//! PDF Oxide operation controller. Uses the same disposable worker, deadline,
//! kill/reap, capture cap, bounded publisher and output relay as extraction.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tpe::oxide::{Context as OperationContext, Input, Limits, OperationOutput};
use tpe::publication::StagedOutputs;

use super::{
    CancelSignals, Capture, ExtractArgs, LimitEvidence, OutputSink, REQUEST_BYTES, deliver,
    read_limited, report_failure, supervise,
};

#[derive(clap::Args)]
pub(crate) struct Args {
    /// Private JSON request file (at most 64 KiB). Passwords never go in argv.
    #[arg(long)]
    request: PathBuf,
    /// Optional directory for atomically published derivative files.
    #[arg(long)]
    out: Option<PathBuf>,
    #[arg(long, default_value_t = 60_000)]
    timeout_ms: u64,
    #[arg(long, default_value_t = 1024)]
    max_memory_growth_mib: u64,
    #[arg(long, default_value_t = tpe::oxide::MAX_OUTPUT_BYTES)]
    max_output_bytes: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    operation: String,
    #[serde(default)]
    inputs: Vec<PathBuf>,
    #[serde(default)]
    pages: Option<(u32, u32)>,
    #[serde(default = "empty_params")]
    params: serde_json::Value,
    #[serde(default)]
    password_file: Option<PathBuf>,
    #[serde(default)]
    limits: Limits,
}

fn empty_params() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(Serialize, Deserialize)]
struct WorkerRequest {
    path: PathBuf,
    output_limit: u64,
    expected: RequestBinding,
    staged_request_sha256: String,
    staged_password_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Source {
    sha256: String,
    bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestBinding {
    request_id: String,
    operation: String,
    sources: Vec<Source>,
    request_options: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcquisitionRequest {
    path: PathBuf,
    directory: PathBuf,
    request_id: String,
    output_limit: u64,
}

// Private transport only: these hashes are never included in a public response.
// A full request/password digest would expose a low-entropy secret oracle.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcquisitionManifest {
    binding: RequestBinding,
    staged_request_sha256: String,
    staged_password_sha256: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ArtifactReceipt {
    name: String,
    mime: String,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
struct OperationResponse {
    version: u32,
    request_id: String,
    operation: String,
    backend: tpe::schema::BackendIdentity,
    producer: serde_json::Value,
    sources: Vec<Source>,
    output: OperationOutput,
    extraction_limits: LimitEvidence,
    publication_limits: Option<LimitEvidence>,
    output_paths: Vec<PathBuf>,
    artifact_receipts: Vec<ArtifactReceipt>,
    containment: serde_json::Value,
    request_options: serde_json::Value,
}

#[derive(Serialize, Deserialize)]
struct Publish {
    response: PathBuf,
    receipt: PathBuf,
    out: Option<PathBuf>,
    max_bytes: u64,
    expected: RequestBinding,
    response_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct OperationReceipt {
    version: u32,
    output_bytes: u64,
}

pub(crate) fn run(args: &Args) -> anyhow::Result<ExitCode> {
    ensure!(
        (1..=300_000).contains(&args.timeout_ms),
        "--timeout-ms must be 1..=300000"
    );
    ensure!(
        (32..=4096).contains(&args.max_memory_growth_mib),
        "--max-memory-growth-mib must be 32..=4096"
    );
    ensure!(
        (1024..=tpe::oxide::MAX_OUTPUT_BYTES).contains(&args.max_output_bytes),
        "--max-output-bytes must be 1024..=67108864"
    );
    // Shared supervision options; no extraction ledger is opened for operations.
    let common = ExtractArgs {
        paths: vec![args.request.clone()],
        db: PathBuf::new(),
        backend: "pdf-oxide".into(),
        out: None,
        json: true,
        password: None,
        password_file: None,
        pages: None,
        jobs: 1,
        max_bytes: Some(tpe::oxide::MAX_INPUT_BYTES),
        timeout_ms: args.timeout_ms,
        max_memory_growth_mib: args.max_memory_growth_mib,
        max_output_bytes: Some(args.max_output_bytes),
        max_files: 1,
        figures_dir: None,
        progress: false,
        pdf_oxide_cleanup_preview: None,
        pdf_oxide_html: false,
        pdf_oxide_images: false,
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let _signals = CancelSignals::install(&cancelled)?;
    let started = Instant::now();
    let deadline = started + Duration::from_millis(args.timeout_ms);
    let result = capture_operation(args, &common, deadline, &cancelled);
    match result {
        Ok(capture) => {
            deliver(&common, &capture.stdout, false, deadline, &cancelled)?;
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            report_failure(&common, &args.request, started, &error)?;
            Ok(ExitCode::FAILURE)
        }
    }
}

fn capture_operation(
    args: &Args,
    common: &ExtractArgs,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> anyhow::Result<Capture> {
    let staging = tempfile::Builder::new()
        .prefix("tpe-oxide-inputs-")
        .rand_bytes(24)
        .tempdir()?;
    // This is an opaque invocation identity, never a private-request or
    // password digest. The controller alone owns its staging lifetime.
    let request_id = tpe::schema::sha256_hex(staging.path().as_os_str().as_encoded_bytes());
    let acquisition = super::supervise_limits(
        super::WorkerBudget {
            memory_growth_mib: args.max_memory_growth_mib,
            output_bytes: REQUEST_BYTES,
        },
        "oxide-acquire",
        &AcquisitionRequest {
            path: args.request.clone(),
            directory: staging.path().to_path_buf(),
            request_id: request_id.clone(),
            output_limit: args.max_output_bytes,
        },
        deadline,
        cancelled,
        OutputSink::Capture,
    )?;
    let acquired: AcquisitionManifest =
        serde_json::from_slice(&read_limited(&acquisition.stdout, REQUEST_BYTES, true)?)?;
    let expected = acquired.binding;
    ensure!(
        expected.request_id == request_id,
        "acquisition request identity mismatch"
    );
    let capture = supervise(
        common,
        "oxide",
        &WorkerRequest {
            path: staging.path().join("request.json"),
            output_limit: args.max_output_bytes,
            expected: expected.clone(),
            staged_request_sha256: acquired.staged_request_sha256,
            staged_password_sha256: acquired.staged_password_sha256,
        },
        deadline,
        cancelled,
        OutputSink::Capture,
    )?;
    let receipt = capture.directory.path().join("receipt.json");
    let published = supervise(
        common,
        "oxide-publish",
        &Publish {
            response: capture.stdout.clone(),
            receipt: receipt.clone(),
            out: args.out.clone(),
            max_bytes: args.max_output_bytes,
            expected,
            response_sha256: capture_digest(
                &capture.stdout,
                args.max_output_bytes,
                deadline,
                cancelled,
            )?,
        },
        deadline,
        cancelled,
        OutputSink::Capture,
    )
    .context("publication outcome unknown: inspect derivative outputs before retrying")?;
    let evidence: OperationReceipt =
        serde_json::from_slice(&read_limited(&receipt, REQUEST_BYTES, true)?)
            .context("publication outcome unknown: publisher receipt could not be verified")?;
    ensure!(
        evidence.version == 1 && evidence.output_bytes == fs::metadata(&published.stdout)?.len(),
        "publication outcome unknown: invalid receipt"
    );
    Ok(published)
}

fn capture_digest(
    path: &Path,
    maximum: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    let mut total = 0_u64;
    loop {
        super::check_deadline(deadline, cancelled)?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        ensure!(
            total <= maximum,
            "captured operation response exceeds limit"
        );
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn private_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

fn read_request(
    path: &Path,
    output_limit: u64,
    expected_sha256: Option<&str>,
) -> anyhow::Result<Request> {
    let encoded = tpe::acquire::read_verified(path, Some(REQUEST_BYTES))?.bytes;
    if let Some(expected) = expected_sha256 {
        ensure!(
            tpe::schema::sha256_hex(&encoded) == expected,
            "staged private request identity mismatch"
        );
    }
    let mut request: Request = serde_json::from_slice(&encoded)
        .map_err(|_| anyhow::anyhow!("invalid PDF Oxide request JSON/schema"))?;
    request.limits.max_output_bytes = request.limits.max_output_bytes.min(output_limit);
    request.limits.validate()?;
    ensure!(
        request.operation.len() <= 128,
        "operation name exceeds limit"
    );
    ensure!(
        request.inputs.len() <= tpe::oxide::MAX_INPUTS,
        "at most 8 input slots"
    );
    if let Some((first, last)) = request.pages {
        ensure!(
            first > 0 && first <= last && last - first < request.limits.max_pages,
            "invalid or oversized page selection"
        );
    }
    Ok(request)
}

fn public_options(request: &Request) -> serde_json::Value {
    let mut parameters = request.params.clone();
    if request.operation == "security.encrypt"
        && let Some(parameters) = parameters.as_object_mut()
    {
        parameters.remove("user_password");
        parameters.remove("owner_password");
    }
    let target_count = if request.operation == "security.redact_text" {
        parameters
            .as_object_mut()
            .and_then(|parameters| parameters.remove("verify_absent"))
            .and_then(|targets| targets.as_array().map(Vec::len))
    } else {
        None
    };
    serde_json::json!({
        "pages":request.pages, "parameters_without_passwords":parameters,
        "limits":request.limits, "password_supplied":request.password_file.is_some(),
        "new_passwords_omitted":request.operation == "security.encrypt",
        "redaction_targets_omitted":request.operation == "security.redact_text",
        "redaction_target_count":target_count,
    })
}

/// Trusted byte acquisition only; document/model parsers run in the later sealed
/// worker. The supervisor applies the same deadline, memory and cleanup bounds.
pub(super) fn acquire_worker(encoded: &[u8]) -> anyhow::Result<()> {
    let envelope: AcquisitionRequest = serde_json::from_slice(encoded)?;
    let mut request = read_request(&envelope.path, envelope.output_limit, None)?;
    let mut expected = RequestBinding {
        request_id: envelope.request_id,
        operation: request.operation.clone(),
        sources: Vec::new(),
        request_options: public_options(&request),
    };
    let mut remaining = request.limits.max_input_bytes;
    for (slot, path) in request.inputs.iter_mut().enumerate() {
        let input = tpe::acquire::read_verified(path, Some(remaining))?.bytes;
        remaining = remaining
            .checked_sub(input.len() as u64)
            .context("input bytes exceed limit")?;
        expected.sources.push(Source {
            sha256: tpe::schema::sha256_hex(&input),
            bytes: input.len() as u64,
        });
        *path = envelope.directory.join(format!("input-{slot:02}.bin"));
        private_write(path, &input)?;
    }
    let staged_password_sha256 = if let Some(path) = &mut request.password_file {
        let password = tpe::acquire::read_verified_allow_empty(path, 4096)?.bytes;
        ensure!(
            std::str::from_utf8(&password).is_ok(),
            "password file is not UTF-8"
        );
        *path = envelope.directory.join("password.bin");
        private_write(path, &password)?;
        Some(tpe::schema::sha256_hex(&password))
    } else {
        None
    };
    let prepared = serde_json::to_vec(&request)?;
    ensure!(
        prepared.len() as u64 <= REQUEST_BYTES,
        "staged request exceeds 64 KiB"
    );
    private_write(&envelope.directory.join("request.json"), &prepared)?;
    let manifest = serde_json::to_vec(&AcquisitionManifest {
        binding: expected,
        staged_request_sha256: tpe::schema::sha256_hex(&prepared),
        staged_password_sha256,
    })?;
    ensure!(
        manifest.len() as u64 <= REQUEST_BYTES,
        "acquisition manifest exceeds 64 KiB"
    );
    io::stdout().write_all(&manifest)?;
    Ok(())
}

pub(super) fn worker(encoded: &[u8], limits: LimitEvidence) -> anyhow::Result<()> {
    let envelope: WorkerRequest = serde_json::from_slice(encoded)?;
    // Never echo private parameter values or password file contents in errors.
    let request = read_request(
        &envelope.path,
        envelope.output_limit,
        Some(&envelope.staged_request_sha256),
    )?;
    let mut inputs = Vec::new();
    let mut remaining = request.limits.max_input_bytes;
    for path in &request.inputs {
        let source = tpe::acquire::read_verified(path, Some(remaining))?;
        remaining = remaining
            .checked_sub(source.bytes.len() as u64)
            .context("input bytes exceed limit")?;
        inputs.push(Input::new(source.bytes));
    }
    let password = request
        .password_file
        .as_ref()
        .map(|path| -> anyhow::Result<String> {
            let bytes = tpe::acquire::read_verified_allow_empty(path, 4096)?.bytes;
            ensure!(
                Some(tpe::schema::sha256_hex(&bytes)) == envelope.staged_password_sha256,
                "staged private auxiliary identity mismatch"
            );
            String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("password file is not UTF-8"))
        })
        .transpose()?;
    ensure!(
        password.is_some() == envelope.staged_password_sha256.is_some(),
        "staged private auxiliary identity mismatch"
    );
    let binding = RequestBinding {
        request_id: envelope.expected.request_id.clone(),
        operation: request.operation.clone(),
        sources: inputs
            .iter()
            .map(|input| Source {
                sha256: input.sha256.clone(),
                bytes: input.bytes.len() as u64,
            })
            .collect(),
        request_options: public_options(&request),
    };
    ensure!(
        binding == envelope.expected,
        "staged input/request identity mismatch"
    );
    // All source/password/request descriptors are closed. This syscall
    // allowlist synchronizes across the lease thread and denies path IO,
    // network and process creation before parsing untrusted document bytes.
    let confinement =
        tpe_ffi::oxide_containment::install_sealed_process().map_err(anyhow::Error::msg)?;
    let context =
        OperationContext::new(&inputs, password.as_deref(), request.pages, &request.limits);
    let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tpe::oxide::execute(&request.operation, &context, &request.params)
    }))
    .map_err(|_| anyhow::anyhow!("PDF Oxide operation panicked; no result published"))??;
    let response = OperationResponse {
        version: 1,
        request_id: binding.request_id,
        operation: request.operation.clone(),
        backend: tpe::oxide::identity(),
        producer: tpe::oxide::producer(),
        sources: binding.sources,
        output,
        extraction_limits: limits,
        publication_limits: None,
        output_paths: Vec::new(),
        artifact_receipts: Vec::new(),
        containment: serde_json::json!({
            "mode":confinement.mode,
            "landlock_abi":confinement.landlock_abi,
            "filesystem_enforced":confinement.filesystem_enforced,
            "network_enforced":confinement.network_enforced,
            "process_enforced":confinement.process_enforced,
            "seccomp_tsync":confinement.seccomp_tsync,
            "read_only_paths":confinement.read_only_paths,
            "scope":"operation worker only; controller and atomic publisher retain their authorized file access"
        }),
        request_options: binding.request_options,
    };
    let encoded = serde_json::to_vec(&response)?;
    ensure!(
        encoded.len() as u64 <= envelope.output_limit,
        "operation response exceeds output limit"
    );
    io::stdout().write_all(&encoded)?;
    Ok(())
}

fn validate_response(
    response: &OperationResponse,
    expected: &RequestBinding,
    maximum: u64,
) -> anyhow::Result<()> {
    ensure!(response.version == 1, "invalid operation response version");
    ensure!(
        response.request_id == expected.request_id
            && response.operation == expected.operation
            && response.sources == expected.sources
            && response.request_options == expected.request_options,
        "operation response does not match acquired request identity"
    );
    ensure!(
        response.backend == tpe::oxide::identity() && response.producer == tpe::oxide::producer(),
        "operation response producer mismatch"
    );
    ensure!(
        response.output_paths.is_empty()
            && response.artifact_receipts.is_empty()
            && response.publication_limits.is_none(),
        "operation response was already published"
    );
    tpe::oxide::validate_output(&response.output, maximum, 10_000)?;
    ensure!(
        response
            .sources
            .iter()
            .all(|source| source.sha256.len() == 64
                && source.sha256.bytes().all(|ch| ch.is_ascii_hexdigit())
                && source.bytes <= tpe::oxide::MAX_INPUT_BYTES),
        "invalid source identity in operation response"
    );
    Ok(())
}

pub(super) fn publish_worker(encoded: &[u8], limits: LimitEvidence) -> anyhow::Result<()> {
    let request: Publish = serde_json::from_slice(encoded)?;
    let bytes = read_limited(&request.response, request.max_bytes, true)?;
    ensure!(
        tpe::schema::sha256_hex(&bytes) == request.response_sha256,
        "captured operation response digest mismatch"
    );
    let mut response: OperationResponse = serde_json::from_slice(&bytes)?;
    validate_response(&response, &request.expected, request.max_bytes)?;
    response.artifact_receipts = response
        .output
        .artifacts
        .iter()
        .map(|artifact| ArtifactReceipt {
            name: artifact.name.clone(),
            mime: artifact.mime.clone(),
            bytes: artifact.bytes.len() as u64,
            sha256: tpe::schema::sha256_hex(&artifact.bytes),
        })
        .collect();
    response.publication_limits = Some(limits);
    let mut staged = None;
    if let Some(directory) = &request.out {
        fs::create_dir_all(directory)?;
        let base = response.sources.first().map_or_else(
            || tpe::schema::sha256_hex(&bytes),
            |source| source.sha256.clone(),
        );
        let files: Vec<_> = response
            .output
            .artifacts
            .iter_mut()
            .map(|artifact| {
                (
                    format!(".{}", artifact.name),
                    std::mem::take(&mut artifact.bytes),
                )
            })
            .collect();
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.clone()))
            .collect();
        if !files.is_empty() {
            staged = Some(StagedOutputs::stage(
                &directory.join(format!("{base}.result")),
                &files,
            )?);
        }
    }
    let encode = |response: &OperationResponse| -> anyhow::Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(response)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() as u64 <= request.max_bytes,
            "published response exceeds output limit"
        );
        Ok(bytes)
    };
    let mut commit = |paths: &[PathBuf]| -> Result<(), String> {
        response.output_paths = paths.to_vec();
        let output = encode(&response).map_err(|error| error.to_string())?;
        // Keep fallible response/receipt writes inside the rollback boundary.
        // The controller requires both this receipt and a successful worker exit.
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&output)
            .map_err(|error| error.to_string())?;
        stdout.flush().map_err(|error| error.to_string())?;
        let receipt = serde_json::to_vec(&OperationReceipt {
            version: 1,
            output_bytes: output.len() as u64,
        })
        .map_err(|error| error.to_string())?;
        fs::write(&request.receipt, receipt).map_err(|error| error.to_string())?;
        Ok(())
    };
    if let Some(staged) = &mut staged {
        staged
            .publish_then_with_paths(commit)
            .map_err(anyhow::Error::msg)?;
    } else {
        commit(&[]).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}
