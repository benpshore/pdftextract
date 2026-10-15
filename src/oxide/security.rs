//! Offline PDF security and compliance operations over immutable input bytes.
//!
//! This adapter reports the scope of the pinned implementation. In particular,
//! text redaction is not image redaction, catalog sanitization is not a sandbox,
//! and a library compliance/signature result is not independent certification.

use std::collections::{HashMap, HashSet};

use pdf_oxide::PdfDocument;
use pdf_oxide::compliance::{
    self, ConversionConfig, PdfAConverter, PdfALevel, PdfUaLevel, PdfXLevel,
};
use pdf_oxide::content::operators::Operator;
use pdf_oxide::editor::{DocumentEditor, EncryptionAlgorithm, EncryptionConfig, SaveOptions};
use pdf_oxide::object::Object;
use pdf_oxide::redaction::RedactionOptions;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Artifact, Context, OperationOutput};
use crate::backend::BackendError;

pub const OPERATIONS: &[&str] = &[
    "security.inspect",
    "security.encrypt",
    "security.decrypt",
    "security.redact_text",
    "security.sanitize",
    "compliance.validate",
    "compliance.convert_pdf_a",
    "signatures.inspect",
    "signatures.verify",
];

const COMPLIANCE_SCOPE: &str =
    "pdf_oxide 0.3.78 checks only; no independent standards certification";
const SANITIZE_SCOPE: &str = "Catalog /Metadata, /OpenAction, /AA, /Names/JavaScript, /Names/EmbeddedFiles and trailer /Info only; page/field actions, attachment annotations, associated files, image metadata and optional-content layers are not comprehensively sanitized";
const SIGNATURE_SCOPE: &str = "Offline inspection of claimed fields and byte ranges; no certificate trust, chain, revocation, timestamp or document-permission assurance";

pub(super) fn execute(
    operation: &str,
    context: &Context<'_>,
    params: &Value,
) -> Option<Result<OperationOutput, BackendError>> {
    if !OPERATIONS.contains(&operation) {
        return None;
    }
    Some((|| {
        if context.inputs.len() != 1 {
            return Err(invalid("operation requires exactly one immutable input"));
        }
        let mut output = match operation {
            "security.inspect" => inspect(context, params),
            "security.encrypt" => encrypt(context, params),
            "security.decrypt" => decrypt(context, params),
            "security.redact_text" => redact_text(context, params),
            "security.sanitize" => sanitize(context, params),
            "compliance.validate" => validate(context, params),
            "compliance.convert_pdf_a" => convert_pdf_a(context, params),
            "signatures.inspect" => signatures(context, params, false),
            "signatures.verify" => signatures(context, params, true),
            _ => unreachable!(),
        }?;
        if let Some(data) = output.data.as_object_mut() {
            data.insert("source_sha256".into(), json!(context.inputs[0].sha256));
            data.insert("source_preserved".into(), json!(true));
            data.insert("engine".into(), json!("pdf_oxide"));
            data.insert("engine_version".into(), json!("0.3.78"));
        }
        bound_output(context, &output)?;
        Ok(output)
    })())
}

fn invalid(message: &str) -> BackendError {
    BackendError::Malformed(message.to_owned())
}

fn upstream(error: pdf_oxide::Error) -> BackendError {
    match error {
        pdf_oxide::Error::Unsupported(message) => BackendError::Unsupported(message),
        other => invalid(&other.to_string()),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, BackendError> {
    // Do not echo request values: encryption parameters can contain passwords.
    serde_json::from_value(params.clone()).map_err(|_| invalid("invalid operation parameters"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}

fn no_params(params: &Value) -> Result<(), BackendError> {
    let _: Empty = parse(params)?;
    Ok(())
}

fn require_whole_document(context: &Context<'_>) -> Result<(), BackendError> {
    if context.pages.is_some() {
        return Err(BackendError::Unsupported(
            "this document operation requires a whole-document request; select pages in a separate derivative first".into(),
        ));
    }
    Ok(())
}

fn bounded_document(context: &Context<'_>) -> Result<PdfDocument, BackendError> {
    require_whole_document(context)?;
    let document = context.open_pdf()?;
    let count = document.page_count().map_err(upstream)?;
    if count == 0 {
        return Err(invalid("document has no pages"));
    }
    if count > context.limits.max_pages as usize {
        return Err(BackendError::Limit("whole-document page limit".into()));
    }
    Ok(document)
}

fn bounded_editor(context: &Context<'_>) -> Result<DocumentEditor, BackendError> {
    let document = bounded_document(context)?;
    reject_signed_mutation(&document, context)?;
    DocumentEditor::from_document(document).map_err(upstream)
}

/// Reject any filled signature value before a rewrite can invalidate it.
/// Blank fields (missing `/V` or explicit null) do not constitute signatures.
/// There is deliberately no invalidation override or key/signing path here.
pub(super) fn reject_signed_mutation(
    document: &PdfDocument,
    context: &Context<'_>,
) -> Result<(), BackendError> {
    if signature_dicts(document, context)?.is_empty()
        && !catalog_signature_permissions(document, context)?
    {
        Ok(())
    } else {
        Err(BackendError::Unsupported(
            "rewriting a PDF with filled signature values would invalidate them; signature invalidation is not enabled".into(),
        ))
    }
}

fn catalog_signature_permissions(
    document: &PdfDocument,
    context: &Context<'_>,
) -> Result<bool, BackendError> {
    let catalog = document.catalog().map_err(upstream)?;
    let dict = catalog
        .as_dict()
        .ok_or_else(|| invalid("catalog is not a dictionary"))?;
    let Some(permissions) = dict.get("Perms") else {
        return Ok(false);
    };
    let mut budget = WalkBudget::new(context);
    let permissions = resolve(document, permissions, &mut budget)?;
    let permissions = permissions
        .as_dict()
        .ok_or_else(|| invalid("catalog Perms is not a dictionary"))?;
    for key in ["DocMDP", "UR", "UR3"] {
        if let Some(value) = permissions.get(key)
            && !matches!(resolve(document, value, &mut budget)?, Object::Null)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn item_limit(context: &Context<'_>, count: usize) -> Result<(), BackendError> {
    if count > context.limits.max_items {
        return Err(BackendError::Limit("security/compliance item limit".into()));
    }
    Ok(())
}

fn bound_output(context: &Context<'_>, output: &OperationOutput) -> Result<(), BackendError> {
    let mut total = serde_json::to_vec(&output.data)
        .map_err(|_| invalid("could not serialize operation report"))?
        .len() as u64;
    for warning in &output.warnings {
        total = total.saturating_add(warning.len() as u64);
    }
    for artifact in &output.artifacts {
        total = total.saturating_add(artifact.bytes.len() as u64);
    }
    if total > context.limits.max_output_bytes {
        return Err(BackendError::Limit(
            "security/compliance output byte limit".into(),
        ));
    }
    Ok(())
}

fn warnings(document: &PdfDocument) -> Vec<String> {
    document
        .take_structured_warnings()
        .into_iter()
        .map(|warning| {
            format!(
                "pdf_oxide:{}:page={}:{}",
                warning.category.as_str(),
                warning
                    .page
                    .map_or_else(|| "unknown".into(), |page| (page + 1).to_string()),
                warning.message
            )
        })
        .collect()
}

fn pdf_artifact(name: &str, bytes: Vec<u8>) -> Artifact {
    Artifact {
        name: name.into(),
        mime: "application/pdf".into(),
        bytes,
    }
}

fn inspect(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    no_params(params)?;
    require_whole_document(context)?;
    let document = PdfDocument::from_bytes(context.input(0)?.to_vec()).map_err(upstream)?;
    let encrypted = document.is_encrypted();
    let authentication = if !encrypted {
        "not_required"
    } else if let Some(password) = context.password {
        match document.authenticate(password.as_bytes()) {
            Ok(true) => "authenticated",
            Ok(false) => "wrong_password",
            Err(_) => "unsupported_or_malformed_encryption",
        }
    } else if document.is_authenticated() {
        "authenticated_with_empty_password"
    } else {
        "password_required_or_unsupported_encryption"
    };
    let page_count = if document.is_authenticated() {
        let count = document.page_count().map_err(upstream)?;
        if count > context.limits.max_pages as usize {
            return Err(BackendError::Limit("whole-document page limit".into()));
        }
        Some(count)
    } else {
        None
    };
    let mut budget = WalkBudget::new(context);
    let encryption = document
        .trailer()
        .as_dict()
        .and_then(|dict| dict.get("Encrypt"))
        .map(|object| resolve(&document, object, &mut budget))
        .transpose()?;
    let encryption = encryption.as_ref().and_then(Object::as_dict);
    Ok(OperationOutput {
        data: json!({
            "encrypted": encrypted,
            "authentication": authentication,
            "page_count": page_count,
            "permissions": document.permissions(),
            "encryption_dictionary": encryption.map(|dict| json!({
                "filter": dict.get("Filter").and_then(Object::as_name),
                "version": dict.get("V").and_then(Object::as_integer),
                "revision": dict.get("R").and_then(Object::as_integer),
                "key_bits": dict.get("Length").and_then(Object::as_integer),
            })),
            "permission_enforcement": "reported flags only; not enforced by this inspection",
        }),
        artifacts: Vec::new(),
        warnings: warnings(&document),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptParams {
    user_password: String,
    owner_password: String,
    #[serde(default = "aes256")]
    algorithm: String,
}

fn aes256() -> String {
    "aes256".into()
}

fn encrypt(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: EncryptParams = parse(params)?;
    if params.algorithm != "aes256" {
        return Err(BackendError::Unsupported(
            "only AES-256 output is enabled; legacy RC4/AES-128 writing is not enabled".into(),
        ));
    }
    // Revision 6 truncates at 127 UTF-8 bytes; reject instead of silently
    // changing the caller's password. No password appears in returned data.
    if params.user_password.len() > 127
        || params.owner_password.len() > 127
        || params.owner_password.is_empty()
    {
        return Err(invalid(
            "passwords must fit 127 UTF-8 bytes and owner password must be nonempty",
        ));
    }
    let mut editor = bounded_editor(context)?;
    let mut notes = warnings(editor.source());
    let config = EncryptionConfig::new(params.user_password.clone(), params.owner_password)
        .with_algorithm(EncryptionAlgorithm::Aes256);
    let bytes = editor
        .save_to_bytes_with_options(SaveOptions::with_encryption(config))
        .map_err(upstream)?;
    let bytes = complete_revision_six_permissions(bytes, &params.user_password)?;
    notes.extend(warnings(editor.source()));
    Ok(OperationOutput {
        data: json!({"algorithm":"AES-256", "revision":6, "rewrite":"full", "permissions":"all_allowed", "compatibility_repair":"mandatory_revision_6_Perms_added", "ciphertext_serializer":"lopdf/0.45.0", "independent_decryption_check":"lopdf/0.45.0"}),
        artifacts: vec![pdf_artifact("encrypted.pdf", bytes)],
        warnings: notes,
    })
}

/// The pinned upstream R6 builder leaves `perms: None`, omitting the
/// mandatory 16-byte permission-validation block. Keep its generated file
/// key and ciphertext; add only the missing Algorithm 10 block, then verify
/// the actual serialized derivative with the independent existing parser.
fn complete_revision_six_permissions(
    bytes: Vec<u8>,
    password: &str,
) -> Result<Vec<u8>, BackendError> {
    use pdf_oxide::crypto::{AesKeySize, Padding};
    use pdf_oxide::encryption::EncryptionHandler;

    let document = PdfDocument::from_bytes(bytes.clone()).map_err(upstream)?;
    let trailer = document
        .trailer()
        .as_dict()
        .ok_or_else(|| invalid("encrypted writer output has no trailer dictionary"))?;
    let encryption_ref = trailer
        .get("Encrypt")
        .and_then(Object::as_reference)
        .ok_or_else(|| invalid("encrypted writer output has no encryption reference"))?;
    let encryption = document.load_object(encryption_ref).map_err(upstream)?;
    let dictionary = encryption
        .as_dict()
        .ok_or_else(|| invalid("encrypted writer output has no encryption dictionary"))?;
    if dictionary.get("V").and_then(Object::as_integer) != Some(5)
        || dictionary.get("R").and_then(Object::as_integer) != Some(6)
        || dictionary.contains_key("Perms")
    {
        return Err(BackendError::Unsupported(
            "revision-6 compatibility repair requires the exact pinned missing-Perms output".into(),
        ));
    }
    let file_id = trailer
        .get("ID")
        .and_then(Object::as_array)
        .and_then(|ids| ids.first())
        .and_then(|value| match value {
            Object::String(bytes) => Some(bytes.clone()),
            _ => None,
        })
        .ok_or_else(|| invalid("encrypted writer output has no file ID"))?;
    let mut handler = EncryptionHandler::new(&encryption, file_id).map_err(upstream)?;
    if !handler
        .authenticate(password.as_bytes())
        .map_err(upstream)?
    {
        return Err(invalid("encrypted writer output failed authentication"));
    }
    let key = handler
        .encryption_key()
        .filter(|key| key.len() == 32)
        .ok_or_else(|| invalid("revision-6 output has no 256-bit file key"))?;
    let permissions = dictionary
        .get("P")
        .and_then(Object::as_integer)
        .and_then(|number| i32::try_from(number).ok())
        .ok_or_else(|| invalid("revision-6 output has invalid permission flags"))?;
    let encrypt_metadata = match dictionary.get("EncryptMetadata") {
        None | Some(Object::Boolean(true)) => true,
        Some(Object::Boolean(false)) => false,
        _ => return Err(invalid("revision-6 output has invalid EncryptMetadata")),
    };
    // ISO 32000-2 Algorithm 10: little-endian signed /P extended to 64
    // bits, metadata flag, "adb", four random bytes; AES-256 ECB.
    let mut permission_block = [0_u8; 16];
    permission_block[..8].copy_from_slice(&i64::from(permissions).to_le_bytes());
    permission_block[8] = if encrypt_metadata { b'T' } else { b'F' };
    permission_block[9..12].copy_from_slice(b"adb");
    let provider = pdf_oxide::crypto::active();
    provider
        .random_bytes(&mut permission_block[12..])
        .map_err(|_| invalid("permission-block randomness unavailable"))?;
    // For exactly one block, CBC with a zero IV and no padding is ECB.
    // Reuse the pinned crypto provider instead of implementing a cipher.
    let encrypted_permissions = provider
        .symmetric()
        .aes_cbc_encrypt(
            AesKeySize::Aes256,
            key,
            &[0_u8; 16],
            &permission_block,
            Padding::None,
        )
        .map_err(|_| invalid("permission-block encryption failed"))?;
    if encrypted_permissions.len() != 16 {
        return Err(invalid(
            "permission-block encryption returned the wrong length",
        ));
    }
    drop(handler);
    drop(document);

    // Missing /Perms prevents lopdf's empty-password auto-decryption too.
    // Require the encrypted state and serialize opaque original ciphertext;
    // never re-encrypt streams or substitute an independently generated key.
    let mut opaque = lopdf::Document::load_mem(&bytes)
        .map_err(|_| invalid("could not read encrypted writer output for compatibility repair"))?;
    if !opaque.is_encrypted() {
        return Err(invalid(
            "compatibility repair requires opaque encrypted objects",
        ));
    }
    let reference = opaque
        .trailer
        .get(b"Encrypt")
        .and_then(lopdf::Object::as_reference)
        .map_err(|_| invalid("encrypted output lacks Encrypt reference"))?;
    opaque
        .get_object_mut(reference)
        .and_then(lopdf::Object::as_dict_mut)
        .map_err(|_| invalid("encrypted output lacks Encrypt dictionary"))?
        .set(
            "Perms",
            lopdf::Object::String(encrypted_permissions, lopdf::StringFormat::Hexadecimal),
        );
    let mut repaired = Vec::new();
    opaque
        .save_to(&mut repaired)
        .map_err(|_| invalid("could not serialize encryption compatibility repair"))?;
    let mut independent = lopdf::Document::load_mem(&repaired)
        .map_err(|_| invalid("repaired encryption output is not independently readable"))?;
    if independent.is_encrypted() {
        independent
            .decrypt(password)
            .map_err(|_| invalid("repaired encryption output failed independent decryption"))?;
    }
    Ok(repaired)
}

fn decrypt(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    no_params(params)?;
    let mut editor = bounded_editor(context)?;
    let was_encrypted = editor.source().is_encrypted();
    let mut notes = warnings(editor.source());
    let bytes = editor.save_to_bytes().map_err(upstream)?;
    notes.extend(warnings(editor.source()));
    Ok(OperationOutput {
        data: json!({"was_encrypted":was_encrypted, "output_encrypted":false, "rewrite":"full"}),
        artifacts: vec![pdf_artifact("decrypted.pdf", bytes)],
        warnings: notes,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    page: u32,
    rect: [f32; 4],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RedactParams {
    regions: Vec<Region>,
}

fn redact_text(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: RedactParams = parse(params)?;
    item_limit(context, params.regions.len())?;
    if params.regions.is_empty() {
        return Err(invalid(
            "text redaction requires at least one explicit region",
        ));
    }
    let document = bounded_document(context)?;
    reject_signed_mutation(&document, context)?;
    if document.is_encrypted() {
        // The pinned redactor's content loader uses decode_stream_data,
        // bypassing the document's encryption-aware stream decoder.
        return Err(BackendError::Unsupported(
            "decrypt to a separate derivative before text redaction".into(),
        ));
    }
    let catalog = document.catalog().map_err(upstream)?;
    if catalog
        .as_dict()
        .is_some_and(|dict| dict.contains_key("StructTreeRoot"))
    {
        return Err(BackendError::Unsupported(
            "text redaction of tagged documents is not verified; alternate structure text may survive".into(),
        ));
    }
    let count = document.page_count().map_err(upstream)?;
    let mut checked_pages = HashSet::new();
    let mut total_content = 0_u64;
    let mut total_operators = 0_usize;
    for region in &params.regions {
        if region.page == 0 || region.page as usize > count {
            return Err(BackendError::PageRange {
                page: region.page,
                count: count as u32,
            });
        }
        if !region.rect.iter().all(|number| number.is_finite())
            || region.rect[0] >= region.rect[2]
            || region.rect[1] >= region.rect[3]
            || !(region.rect[2] - region.rect[0]).is_finite()
            || !(region.rect[3] - region.rect[1]).is_finite()
        {
            return Err(invalid(
                "redaction rectangle must be finite and have positive width and height",
            ));
        }
        let page = region.page as usize - 1;
        if checked_pages.insert(page) {
            preflight_text_redaction(
                &document,
                page,
                context,
                &mut total_content,
                &mut total_operators,
            )?;
        }
    }
    let mut notes = warnings(&document);
    let mut editor = DocumentEditor::from_document(document).map_err(upstream)?;
    for region in &params.regions {
        editor
            .add_redaction(region.page as usize - 1, region.rect, None)
            .map_err(upstream)?;
    }
    let report = editor
        .apply_redactions_destructive(RedactionOptions::default())
        .map_err(upstream)?;
    let bytes = editor.save_to_bytes().map_err(upstream)?;
    notes.extend(warnings(editor.source()));
    notes.push("Text content in the explicit regions was processed; duplicate text on other pages, metadata, fonts and other document structures is not guaranteed removed. Review the derivative before sharing.".into());
    Ok(OperationOutput {
        data: json!({
            "scope":"text_content_only", "report":report, "rewrite":"full",
            "regions":params.regions.iter().map(|region| json!({"page":region.page,"rect":region.rect})).collect::<Vec<_>>(),
            "global_secret_removal_verified":false,
        }),
        artifacts: vec![pdf_artifact("text-redacted.pdf", bytes)],
        warnings: notes,
    })
}

fn preflight_text_redaction(
    document: &PdfDocument,
    page: usize,
    context: &Context<'_>,
    total_content: &mut u64,
    total_operators: &mut usize,
) -> Result<(), BackendError> {
    let object = document.get_page(page).map_err(upstream)?;
    let dict = object
        .as_dict()
        .ok_or_else(|| invalid("page is not a dictionary"))?;
    if dict.contains_key("Annots") {
        return Err(BackendError::Unsupported(
            "text redaction of pages with annotations is not verified; annotation copies may survive".into(),
        ));
    }
    // Match the editor's supported content representation, but fail on an
    // invalid member instead of its array loader's silent skip-on-decode-error.
    let contents: Vec<&Object> = match dict.get("Contents") {
        None | Some(Object::Null) => Vec::new(),
        Some(reference @ Object::Reference(_)) => vec![reference],
        Some(Object::Array(array)) => array.iter().collect(),
        _ => {
            return Err(BackendError::Unsupported(
                "unsupported content stream representation for text redaction".into(),
            ));
        }
    };
    item_limit(context, contents.len())?;
    for object in contents {
        let reference = object
            .as_reference()
            .ok_or_else(|| invalid("content array member is not a stream reference"))?;
        let stream = document.load_object(reference).map_err(upstream)?;
        if !matches!(stream, Object::Stream { .. }) {
            return Err(invalid("content reference is not a stream"));
        }
        let bytes = stream.decode_stream_data().map_err(upstream)?;
        *total_content = total_content.saturating_add(bytes.len() as u64);
        if *total_content > context.limits.max_input_bytes {
            return Err(BackendError::Limit(
                "decoded redaction content byte limit".into(),
            ));
        }
        let operators =
            pdf_oxide::content::parser::parse_content_stream(&bytes).map_err(upstream)?;
        *total_operators = total_operators.saturating_add(operators.len());
        item_limit(context, *total_operators)?;
        if operators.iter().any(|operator| {
            matches!(
                operator,
                Operator::Do { .. }
                    | Operator::InlineImage { .. }
                    | Operator::BeginMarkedContent { .. }
                    | Operator::BeginMarkedContentDict { .. }
                    | Operator::Other { .. }
            )
        }) {
            return Err(BackendError::Unsupported(
                "text redaction refuses XObjects, inline images, marked content and unknown operators; their content removal is not verified".into(),
            ));
        }
    }
    Ok(())
}

fn sanitize(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    no_params(params)?;
    let mut editor = bounded_editor(context)?;
    let mut notes = warnings(editor.source());
    let was_encrypted = editor.source().is_encrypted();
    let report = editor
        .sanitize_document(RedactionOptions::default())
        .map_err(upstream)?;
    let bytes = editor.save_to_bytes().map_err(upstream)?;
    notes.extend(warnings(editor.source()));
    notes.push(SANITIZE_SCOPE.into());
    if was_encrypted {
        notes.push("The sanitized derivative is unencrypted; source encryption is preserved only in the original input.".into());
    }
    Ok(OperationOutput {
        data: json!({"scope":"document_catalog_and_info", "report":report, "output_encrypted":false, "comprehensive_sanitization":false, "rewrite":"full"}),
        artifacts: vec![pdf_artifact("catalog-sanitized.pdf", bytes)],
        warnings: notes,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComplianceParams {
    standard: String,
}

fn pdf_a_level(name: &str) -> Option<PdfALevel> {
    match name {
        "PDF/A-1a" => Some(PdfALevel::A1a),
        "PDF/A-1b" => Some(PdfALevel::A1b),
        "PDF/A-2a" => Some(PdfALevel::A2a),
        "PDF/A-2b" => Some(PdfALevel::A2b),
        "PDF/A-2u" => Some(PdfALevel::A2u),
        "PDF/A-3a" => Some(PdfALevel::A3a),
        "PDF/A-3b" => Some(PdfALevel::A3b),
        "PDF/A-3u" => Some(PdfALevel::A3u),
        _ => None,
    }
}

fn validate(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: ComplianceParams = parse(params)?;
    let mut document = bounded_document(context)?;
    let data = if let Some(level) = pdf_a_level(&params.standard) {
        pdf_a_result(
            context,
            &compliance::validate_pdf_a(&mut document, level).map_err(upstream)?,
        )?
    } else if matches!(params.standard.as_str(), "PDF/UA-1" | "PDF/UA-2") {
        let level = if params.standard == "PDF/UA-1" {
            PdfUaLevel::Ua1
        } else {
            PdfUaLevel::Ua2
        };
        let result = compliance::validate_pdf_ua(&mut document, level).map_err(upstream)?;
        item_limit(
            context,
            result.errors.len().saturating_add(result.warnings.len()),
        )?;
        json!({
            "standard":result.level.to_string(), "upstream_is_compliant":result.is_compliant,
            "independent_certification":false,
            "errors":result.errors.iter().map(|error| json!({"code":format!("{:?}",error.code),"message":error.message,"location":error.location,"clause":error.clause,"wcag":error.wcag_ref})).collect::<Vec<_>>(),
            "warnings":compliance_warnings(&result.warnings),
            "stats":{"pages_checked":result.stats.pages_checked,"structure_elements_checked":result.stats.structure_elements_checked,"images_checked":result.stats.images_checked,"images_with_alt":result.stats.images_with_alt,"tables_checked":result.stats.tables_checked,"form_fields_checked":result.stats.form_fields_checked,"annotations_checked":result.stats.annotations_checked},
        })
    } else if let Some(level) = PdfXLevel::from_gts_version(&params.standard) {
        let result = compliance::validate_pdf_x(&mut document, level).map_err(upstream)?;
        item_limit(
            context,
            result.errors.len().saturating_add(result.warnings.len()),
        )?;
        let issue = |error: &compliance::XComplianceError| json!({"code":format!("{:?}",error.code),"message":error.message,"page":error.page.map(|page|page+1),"object_id":error.object_id,"severity":format!("{:?}",error.severity),"clause":error.clause});
        json!({
            "standard":result.level.to_string(), "upstream_is_compliant":result.is_compliant,
            "independent_certification":false, "detected_standard":result.detected_level.map(|level|level.to_string()),
            "errors":result.errors.iter().map(issue).collect::<Vec<_>>(),
            "warnings":result.warnings.iter().map(issue).collect::<Vec<_>>(),
            "stats":{"pages_checked":result.stats.pages_checked,"fonts_checked":result.stats.fonts_checked,"fonts_embedded":result.stats.fonts_embedded,"images_checked":result.stats.images_checked,"annotations_checked":result.stats.annotations_checked,"color_spaces_found":result.stats.color_spaces_found,"has_transparency":result.stats.has_transparency,"has_layers":result.stats.has_layers,"output_intent":result.stats.output_intent},
        })
    } else {
        return Err(BackendError::Unsupported(
            "unsupported PDF compliance standard".into(),
        ));
    };
    let mut notes = warnings(&document);
    notes.push(COMPLIANCE_SCOPE.into());
    Ok(OperationOutput {
        data,
        artifacts: Vec::new(),
        warnings: notes,
    })
}

fn pdf_a_result(
    context: &Context<'_>,
    result: &compliance::ValidationResult,
) -> Result<Value, BackendError> {
    item_limit(
        context,
        result.errors.len().saturating_add(result.warnings.len()),
    )?;
    Ok(json!({
        "standard":result.level.to_string(), "upstream_is_compliant":result.is_compliant,
        "independent_certification":false, "detected_standard":result.detected_level.map(|level|level.to_string()),
        "errors":result.errors.iter().map(|error| json!({"code":format!("{:?}",error.code),"message":error.message,"location":error.location,"clause":error.clause})).collect::<Vec<_>>(),
        "warnings":compliance_warnings(&result.warnings),
        "stats":{"pages_checked":result.stats.pages_checked,"fonts_checked":result.stats.fonts_checked,"fonts_embedded":result.stats.fonts_embedded,"images_checked":result.stats.images_checked,"color_spaces_checked":result.stats.color_spaces_checked,"annotations_checked":result.stats.annotations_checked},
    }))
}

fn compliance_warnings(warnings: &[compliance::ComplianceWarning]) -> Vec<Value> {
    warnings.iter().map(|warning| json!({"code":format!("{:?}",warning.code),"message":warning.message,"location":warning.location})).collect()
}

fn convert_pdf_a(context: &Context<'_>, params: &Value) -> Result<OperationOutput, BackendError> {
    let params: ComplianceParams = parse(params)?;
    let level = pdf_a_level(&params.standard).ok_or_else(|| {
        BackendError::Unsupported("PDF/A conversion requires a supported PDF/A standard".into())
    })?;
    let mut document = bounded_document(context)?;
    reject_signed_mutation(&document, context)?;
    if document.is_encrypted() {
        return Err(BackendError::Unsupported("decrypt to a separate derivative before PDF/A conversion; pinned converter reopens source bytes without authentication".into()));
    }
    let mut notes = warnings(&document);
    // Avoid implicit system-font lookup and rasterization. These operations
    // need their own bounded explicit inputs/options in the rendering adapter.
    let config = ConversionConfig::default()
        .embed_fonts(false)
        .flatten_transparency(false)
        .add_structure(false);
    let result = PdfAConverter::new(level)
        .with_config(config)
        .convert(&mut document)
        .map_err(upstream)?;
    item_limit(
        context,
        result.actions.len().saturating_add(result.errors.len()),
    )?;
    let converter_validation = pdf_a_result(context, &result.validation)?;
    // The pinned converter downgrades some unembedded-font errors. A new
    // unmodified validator result must accompany, not inherit, that success.
    let fresh = compliance::validate_pdf_a(&mut document, level).map_err(upstream)?;
    let validation = pdf_a_result(context, &fresh)?;
    notes.extend(warnings(&document));
    let bytes = document.source_bytes.clone();
    if bytes.is_empty() {
        return Err(invalid("PDF/A converter produced no source bytes"));
    }
    notes.push(COMPLIANCE_SCOPE.into());
    notes.push("Font embedding, transparency flattening and synthesized accessibility structure are disabled; unmet requirements remain in the fresh validation report. The derivative may not conform to the target standard.".into());
    Ok(OperationOutput {
        data: json!({
            "standard":params.standard, "upstream_conversion_success":result.success,
            "converter_validation":converter_validation,"validation":validation,
            "independent_certification":false,
            "actions":result.actions.iter().map(|action|json!({"type":format!("{:?}",action.action_type),"description":action.description,"fixed_error":action.fixed_error.map(|error|format!("{error:?}"))})).collect::<Vec<_>>(),
            "conversion_errors":result.errors.iter().map(|error|json!({"code":format!("{:?}",error.error_code),"reason":error.reason})).collect::<Vec<_>>(),
        }),
        artifacts: vec![pdf_artifact("pdfa-candidate.pdf", bytes)],
        warnings: notes,
    })
}

struct WalkBudget {
    remaining: usize,
}

impl WalkBudget {
    fn new(context: &Context<'_>) -> Self {
        Self {
            remaining: context.limits.max_items,
        }
    }
    fn take(&mut self) -> Result<(), BackendError> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or_else(|| BackendError::Limit("PDF field/object walk item limit".into()))?;
        Ok(())
    }
}

fn resolve(
    document: &PdfDocument,
    object: &Object,
    budget: &mut WalkBudget,
) -> Result<Object, BackendError> {
    let mut object = object.clone();
    let mut seen = HashSet::new();
    while let Object::Reference(reference) = object {
        budget.take()?;
        if !seen.insert((reference.id, reference.r#gen)) {
            return Err(invalid("cyclic PDF object reference"));
        }
        if seen.len() > 64 {
            return Err(BackendError::Limit("PDF object reference depth".into()));
        }
        object = document.load_object(reference).map_err(upstream)?;
    }
    Ok(object)
}

fn signature_dicts(
    document: &PdfDocument,
    context: &Context<'_>,
) -> Result<Vec<Object>, BackendError> {
    let mut budget = WalkBudget::new(context);
    let catalog = document.catalog().map_err(upstream)?;
    let catalog = catalog
        .as_dict()
        .ok_or_else(|| invalid("catalog is not a dictionary"))?;
    let Some(form) = catalog.get("AcroForm") else {
        return Ok(Vec::new());
    };
    let form = resolve(document, form, &mut budget)?;
    let form = form
        .as_dict()
        .ok_or_else(|| invalid("AcroForm is not a dictionary"))?;
    let Some(fields) = form.get("Fields") else {
        return Ok(Vec::new());
    };
    let fields = resolve(document, fields, &mut budget)?;
    let fields = fields
        .as_array()
        .ok_or_else(|| invalid("AcroForm Fields is not an array"))?;
    item_limit(context, fields.len())?;
    let mut stack: Vec<_> = fields
        .iter()
        .rev()
        .map(|field| (field.clone(), false, 0_usize))
        .collect();
    let mut seen = HashSet::new();
    let mut signatures = Vec::new();
    while let Some((field, inherited_signature, depth)) = stack.pop() {
        budget.take()?;
        if depth >= 64 {
            return Err(BackendError::Limit("AcroForm field depth".into()));
        }
        if let Object::Reference(reference) = &field
            && !seen.insert((reference.id, reference.r#gen))
        {
            return Err(invalid("cyclic or repeated AcroForm field reference"));
        }
        let field = resolve(document, &field, &mut budget)?;
        let field = field
            .as_dict()
            .ok_or_else(|| invalid("AcroForm field is not a dictionary"))?;
        let is_signature = field
            .get("FT")
            .and_then(Object::as_name)
            .map_or(inherited_signature, |name| name == "Sig");
        if is_signature && let Some(value) = field.get("V") {
            let value = resolve(document, value, &mut budget)?;
            if !matches!(value, Object::Null) {
                if value.as_dict().is_none() {
                    return Err(invalid("signature value is not a dictionary"));
                }
                signatures.push(value);
                item_limit(context, signatures.len())?;
            }
        }
        if let Some(kids) = field.get("Kids") {
            let kids = resolve(document, kids, &mut budget)?;
            let kids = kids
                .as_array()
                .ok_or_else(|| invalid("field Kids is not an array"))?;
            item_limit(context, stack.len().saturating_add(kids.len()))?;
            stack.extend(
                kids.iter()
                    .rev()
                    .map(|kid| (kid.clone(), is_signature, depth + 1)),
            );
        }
    }
    Ok(signatures)
}

fn signatures(
    context: &Context<'_>,
    params: &Value,
    verify: bool,
) -> Result<OperationOutput, BackendError> {
    no_params(params)?;
    if verify && !cfg!(feature = "pdf-oxide-signatures") {
        return Err(BackendError::Unsupported("offline signature verification requires the pdf-oxide-signatures build feature; signing and network validation are not exposed".into()));
    }
    let document = bounded_document(context)?;
    let signatures = signature_dicts(&document, context)?;
    let input = context.input(0)?;
    let mut records = Vec::new();
    for (index, signature) in signatures.iter().enumerate() {
        let dict = signature
            .as_dict()
            .ok_or_else(|| invalid("signature is not a dictionary"))?;
        let claimed_range = dict.get("ByteRange").and_then(Object::as_array);
        let range: Option<Vec<i64>> =
            claimed_range.and_then(|array| array.iter().map(Object::as_integer).collect());
        let range_valid = range
            .as_deref()
            .is_some_and(|range| valid_range(range, input.len()));
        let claimed_ranges_end_at_eof = range.as_deref().is_some_and(|range| {
            range_valid && range[2].checked_add(range[3]) == Some(input.len() as i64)
        });
        let contents = dict.get("Contents").and_then(|object| {
            if let Object::String(bytes) = object {
                Some(bytes.as_slice())
            } else {
                None
            }
        });
        if contents.is_some_and(|bytes| bytes.len() as u64 > context.limits.max_output_bytes) {
            return Err(BackendError::Limit("signature CMS byte limit".into()));
        }
        let text = |key| {
            dict.get(key).and_then(|object| {
                if let Object::String(bytes) = object {
                    Some(String::from_utf8_lossy(bytes).into_owned())
                } else {
                    None
                }
            })
        };
        let mut record = json!({
            "index":index + 1,"claimed_name":text("Name"),"claimed_time":text("M"),
            "claimed_reason":text("Reason"),"claimed_location":text("Location"),
            "subfilter":dict.get("SubFilter").and_then(Object::as_name),
            "byte_range":range,"byte_range_valid":range_valid,"claimed_ranges_end_at_eof":claimed_ranges_end_at_eof,
            "whole_document_coverage_verified":false,
            "cms_bytes":contents.map(<[u8]>::len),"cryptographic_status":"not_checked",
            "certificate_trust":"not_checked","revocation":"not_checked","timestamp":"not_checked",
        });
        if verify {
            verify_signature(input, dict, contents, range_valid, &mut record)?;
        }
        records.push(record);
    }
    let mut notes = warnings(&document);
    notes.push(SIGNATURE_SCOPE.into());
    Ok(OperationOutput {
        data: json!({"signatures":records,"field_walk_complete":true,"enumeration_scope":"AcroForm signature fields","catalog_signature_permissions_present":catalog_signature_permissions(&document,context)?,"trust_verified":false,"network_used":false}),
        artifacts: Vec::new(),
        warnings: notes,
    })
}

fn valid_range(range: &[i64], size: usize) -> bool {
    range.len() == 4
        && range[0] == 0
        && range[1] > 0
        && range[2] > range[1]
        && range[3] >= 0
        && range[2]
            .checked_add(range[3])
            .is_some_and(|end| end <= size as i64)
}

#[cfg(not(feature = "pdf-oxide-signatures"))]
fn verify_signature(
    _: &[u8],
    _: &HashMap<String, Object>,
    _: Option<&[u8]>,
    _: bool,
    _: &mut Value,
) -> Result<(), BackendError> {
    Err(BackendError::Unsupported(
        "offline signature verification is not enabled".into(),
    ))
}

#[cfg(feature = "pdf-oxide-signatures")]
fn verify_signature(
    input: &[u8],
    dict: &HashMap<String, Object>,
    contents: Option<&[u8]>,
    range_valid: bool,
    record: &mut Value,
) -> Result<(), BackendError> {
    use pdf_oxide::signatures::{ByteRangeCalculator, SignerVerify, verify_signer_detached};
    if !range_valid || contents.is_none() {
        record["cryptographic_status"] = json!("invalid");
        record["reason"] = json!("invalid ByteRange or missing CMS contents");
        return Ok(());
    }
    if !matches!(
        dict.get("SubFilter").and_then(Object::as_name),
        Some("adbe.pkcs7.detached" | "ETSI.CAdES.detached")
    ) {
        record["cryptographic_status"] = json!("unsupported");
        record["reason"] = json!("only detached CMS signature subfilters are enabled");
        return Ok(());
    }
    let array = dict
        .get("ByteRange")
        .and_then(Object::as_array)
        .ok_or_else(|| invalid("missing ByteRange"))?;
    let mut range = [0_i64; 4];
    for (number, object) in range.iter_mut().zip(array) {
        *number = object
            .as_integer()
            .ok_or_else(|| invalid("noninteger ByteRange"))?;
    }
    let signed = ByteRangeCalculator::extract_signed_bytes(input, &range).map_err(upstream)?;
    let status = match verify_signer_detached(contents.unwrap_or_default(), &signed) {
        Ok(SignerVerify::Valid) => "valid_for_claimed_byte_ranges",
        Ok(SignerVerify::Invalid) => "invalid",
        Ok(SignerVerify::Unknown) => "unsupported_or_incomplete",
        Err(_) => "invalid_cms",
    };
    record["cryptographic_status"] = json!(status);
    record["scope"] = json!(
        "CMS signature and digest over claimed ranges only; signature-gap binding, trust chains, revocation, timestamps and modification permissions are not validated"
    );
    Ok(())
}
