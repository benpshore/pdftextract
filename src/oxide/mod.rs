//! Bounded, typed operation dispatch over pinned PDF Oxide capabilities.
//!
//! This native API accepts immutable owned bytes. Untrusted documents must run
//! through the CLI supervisor: local checks cannot stop upstream allocation,
//! infinite loops, or hidden I/O. Every operation preserves source bytes.

use std::collections::BTreeMap;

use pdf_oxide::PdfDocument;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backend::{BackendError, EncryptionProblem};
use crate::schema::{BackendIdentity, config_digest, sha256_hex};

mod convert;
mod create;
mod edit;
mod flatten;
mod forms;
mod ocr;
mod office;
mod query;
mod render;
mod security;

pub const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_OUTPUT_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_INPUTS: usize = 8;
type Dispatch = fn(&str, &Context<'_>, &Value) -> Option<Result<OperationOutput, BackendError>>;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_pages: u32,
    pub max_items: usize,
    pub max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: MAX_INPUT_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
            max_pages: 256,
            max_items: 10_000,
            max_pixels: 16_000_000,
        }
    }
}

impl Limits {
    pub fn validate(&self) -> Result<(), BackendError> {
        if self.max_input_bytes == 0
            || self.max_input_bytes > MAX_INPUT_BYTES
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_OUTPUT_BYTES
            || self.max_pages == 0
            || self.max_pages > 256
            || self.max_items == 0
            || self.max_items > 10_000
            || self.max_pixels == 0
            || self.max_pixels > 16_000_000
        {
            return Err(BackendError::Limit(
                "PDF Oxide limits exceed fixed ceilings or are zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Input {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

impl Input {
    pub fn new(bytes: Vec<u8>) -> Self {
        let sha256 = sha256_hex(&bytes);
        Self { bytes, sha256 }
    }
}

pub struct Context<'a> {
    pub inputs: &'a [Input],
    pub password: Option<&'a str>,
    pub pages: Option<(u32, u32)>,
    pub limits: &'a Limits,
}

impl<'a> Context<'a> {
    pub fn new(
        inputs: &'a [Input],
        password: Option<&'a str>,
        pages: Option<(u32, u32)>,
        limits: &'a Limits,
    ) -> Self {
        Self {
            inputs,
            password,
            pages,
            limits,
        }
    }

    pub fn input(&self, index: usize) -> Result<&[u8], BackendError> {
        self.inputs
            .get(index)
            .map(|input| input.bytes.as_slice())
            .ok_or_else(|| BackendError::Unsupported(format!("missing input slot {index}")))
    }

    pub fn open_pdf(&self) -> Result<PdfDocument, BackendError> {
        let document = PdfDocument::from_bytes(self.input(0)?.to_vec())
            .map_err(|error| BackendError::Malformed(error.to_string()))?;
        if document.is_encrypted() {
            let authenticated = document
                .authenticate(self.password.unwrap_or_default().as_bytes())
                .map_err(|_| BackendError::Encrypted(EncryptionProblem::UnsupportedCipher))?;
            if !authenticated {
                return Err(BackendError::Encrypted(if self.password.is_some() {
                    EncryptionProblem::WrongPassword
                } else {
                    EncryptionProblem::PasswordRequired
                }));
            }
        }
        Ok(document)
    }

    pub fn open_editor(&self) -> Result<pdf_oxide::editor::DocumentEditor, BackendError> {
        let document = self.open_pdf()?;
        if document.is_encrypted() {
            return Err(BackendError::Unsupported("editing encrypted input can discard encryption; use the explicit security operation first".into()));
        }
        security::reject_signed_mutation(&document, self)?;
        pdf_oxide::editor::DocumentEditor::from_document(document)
            .map_err(|error| BackendError::Malformed(error.to_string()))
    }

    /// Validate the complete selection before allocating its bounded indices.
    pub fn selected_pages(&self, count: usize) -> Result<Vec<usize>, BackendError> {
        let (first, last) = self
            .pages
            .map_or((1_u64, count as u64), |(a, b)| (u64::from(a), u64::from(b)));
        if first == 0 || first > last || last > count as u64 {
            return Err(BackendError::PageRange {
                page: u32::try_from(first).unwrap_or(u32::MAX),
                count: u32::try_from(count).unwrap_or(u32::MAX),
            });
        }
        if last - first + 1 > u64::from(self.limits.max_pages) {
            return Err(BackendError::Limit(
                "selected pages exceed operation limit".into(),
            ));
        }
        (first - 1..last)
            .map(|page| {
                usize::try_from(page)
                    .map_err(|_| BackendError::Limit("page index exceeds usize".into()))
            })
            .collect()
    }

    pub fn require_filesystem_containment(&self) -> Result<(), BackendError> {
        // Set only by verified per-worker confinement, never by request JSON.
        if filesystem_contained() {
            Ok(())
        } else {
            Err(BackendError::Unsupported(
                "this capability requires verified worker filesystem containment".into(),
            ))
        }
    }
}

// The supervisor will install confinement before admitting operations with
// implicit host-file access. Until verified, those capabilities fail closed.
fn filesystem_contained() -> bool {
    tpe_ffi::oxide_containment::installed()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Artifact {
    pub name: String,
    pub mime: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OperationOutput {
    pub data: Value,
    pub artifacts: Vec<Artifact>,
    pub warnings: Vec<String>,
}

/// Exact source/operation provenance, with secrets excluded from the identity.
pub fn identity() -> BackendIdentity {
    BackendIdentity {
        name: "pdf-oxide-operations".into(),
        version: "0.3.78".into(),
        config_digest: config_digest(&BTreeMap::from([
            ("adapter".into(), "1".into()),
            (
                "rendering".into(),
                cfg!(feature = "pdf-oxide-rendering").to_string(),
            ),
            (
                "ocr_tract".into(),
                cfg!(feature = "pdf-oxide-ocr").to_string(),
            ),
            (
                "barcodes".into(),
                cfg!(feature = "pdf-oxide-barcodes").to_string(),
            ),
            (
                "signatures".into(),
                cfg!(feature = "pdf-oxide-signatures").to_string(),
            ),
        ])),
    }
}

/// Build identity complements the pinned dependency identity in operation receipts.
pub fn producer() -> Value {
    json!({
        "name":"text-processing-engine",
        "build_version":env!("TPE_BUILD_VERSION"),
        "dependencies":{"pdf_oxide":"0.3.78","office_oxide":"0.1.9","lopdf":"0.45.0","ammonia":"4.1.2"},
        "qualification":"build identity and pinned versions; not a signed attestation or fidelity guarantee"
    })
}

pub fn operations() -> Vec<&'static str> {
    [
        convert::OPERATIONS,
        create::OPERATIONS,
        edit::OPERATIONS,
        forms::OPERATIONS,
        ocr::OPERATIONS,
        office::OPERATIONS,
        query::OPERATIONS,
        render::OPERATIONS,
        security::OPERATIONS,
    ]
    .into_iter()
    .flatten()
    .copied()
    .collect()
}

pub fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Result<OperationOutput, BackendError> {
    context.limits.validate()?;
    if context.inputs.len() > MAX_INPUTS {
        return Err(BackendError::Limit("at most 8 inputs".into()));
    }
    let mut total = 0_u64;
    for input in context.inputs {
        total = total.saturating_add(input.bytes.len() as u64);
        if total > context.limits.max_input_bytes {
            return Err(BackendError::Limit("total input bytes exceed limit".into()));
        }
        if input.sha256 != sha256_hex(&input.bytes) {
            return Err(BackendError::Malformed(
                "input identity does not match its bytes".into(),
            ));
        }
    }
    if serde_json::to_vec(params).map_err(json_error)?.len() > 64 * 1024 {
        return Err(BackendError::Limit(
            "operation parameters exceed 64 KiB".into(),
        ));
    }
    if operation == "capabilities" {
        let output = OperationOutput {
            data: json!({"backend":identity(),"producer":producer(),"operation_names":operations(),"limits":context.limits,
            "runtime":{
                "filesystem_contained":filesystem_contained(),
                "rendering_compiled":cfg!(feature="pdf-oxide-rendering"),
                "ocr_tract_compiled":cfg!(feature="pdf-oxide-ocr"),
                "barcodes_compiled":cfg!(feature="pdf-oxide-barcodes"),
                "signature_verification_compiled":cfg!(feature="pdf-oxide-signatures"),
                "ocr_models":"required as explicit hash-checked input slots; no downloads or accuracy qualification"
            },
            "known_unsupported_edges":[
                "table creation and HTML/CSS creation: table layout and CSS pixel/PDF point geometry defects",
                "table reconstruction in converters; native extraction candidates carry validation_error",
                "text replacement that would paint black masks",
                "form value/style updates with existing appearances; existing-AcroForm additions; flattening pending appearances",
                "masked image extraction through the pinned memory PNG API",
                "rewriting signed input or silently dropping existing encryption",
                "automatic OCR/model acquisition, network timestamp/revocation checks, signing and persistent MCP",
                "complex raster-region coordinate cases and complete toolkit/platform parity"
            ],
            "note":"Operation names are dispatch entries, not parity or runtime availability claims. Feature, model, input and containment checks can return unsupported."}),
            ..OperationOutput::default()
        };
        validate_output(
            &output,
            context.limits.max_output_bytes,
            context.limits.max_items,
        )?;
        return Ok(output);
    }
    let dispatchers: [Dispatch; 9] = [
        convert::execute,
        create::execute,
        edit::execute,
        forms::execute,
        ocr::execute,
        office::execute,
        query::execute,
        render::execute,
        security::execute,
    ];
    let output = dispatchers
        .into_iter()
        .find_map(|dispatch| dispatch(operation, context, params))
        .ok_or_else(|| {
            BackendError::Unsupported(format!("unknown PDF Oxide operation: {operation}"))
        })??;
    validate_output(
        &output,
        context.limits.max_output_bytes,
        context.limits.max_items,
    )?;
    Ok(output)
}

/// Revalidate derived output at the process boundary before any publication.
pub fn validate_output(
    output: &OperationOutput,
    max_output_bytes: u64,
    max_items: usize,
) -> Result<(), BackendError> {
    if output.artifacts.len() > max_items {
        return Err(BackendError::Limit("artifact count exceeds limit".into()));
    }
    let mut names = std::collections::HashSet::new();
    for artifact in &output.artifacts {
        if artifact.name.is_empty()
            || artifact.name.len() > 128
            || artifact.name.starts_with('.')
            || !artifact
                .name
                .bytes()
                .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch))
            || !names.insert(&artifact.name)
        {
            return Err(BackendError::Unsupported(
                "operation produced an unsafe or duplicate artifact name".into(),
            ));
        }
        if artifact.bytes.len() as u64 > max_output_bytes {
            return Err(BackendError::Limit("artifact bytes exceed limit".into()));
        }
    }
    if serde_json::to_vec(output).map_err(json_error)?.len() as u64 > max_output_bytes {
        return Err(BackendError::Limit(
            "serialized operation output exceeds limit".into(),
        ));
    }
    Ok(())
}

fn json_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::Malformed(error.to_string())
}
