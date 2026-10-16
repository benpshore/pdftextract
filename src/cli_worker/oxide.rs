//! PDF Oxide operation controller. Uses the same disposable worker, deadline,
//! kill/reap, capture cap, bounded publisher and output relay as extraction.

use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
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

#[derive(Deserialize)]
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
}

#[derive(Serialize, Deserialize)]
struct Source {
    sha256: String,
    bytes: u64,
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
    let result = (|| -> anyhow::Result<Capture> {
        let capture = supervise(
            &common,
            "oxide",
            &WorkerRequest {
                path: args.request.clone(),
                output_limit: args.max_output_bytes,
            },
            deadline,
            &cancelled,
            OutputSink::Capture,
        )?;
        let receipt = capture.directory.path().join("receipt.json");
        let published = supervise(
            &common,
            "oxide-publish",
            &Publish {
                response: capture.stdout.clone(),
                receipt: receipt.clone(),
                out: args.out.clone(),
                max_bytes: args.max_output_bytes,
            },
            deadline,
            &cancelled,
            OutputSink::Capture,
        )
        .context("publication outcome unknown: inspect derivative outputs before retrying")?;
        let evidence: OperationReceipt =
            serde_json::from_slice(&read_limited(&receipt, REQUEST_BYTES, true)?)
                .context("publication outcome unknown: publisher receipt could not be verified")?;
        ensure!(
            evidence.version == 1
                && evidence.output_bytes == fs::metadata(&published.stdout)?.len(),
            "publication outcome unknown: invalid receipt"
        );
        Ok(published)
    })();
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

pub(super) fn worker(encoded: &[u8], limits: LimitEvidence) -> anyhow::Result<()> {
    let envelope: WorkerRequest = serde_json::from_slice(encoded)?;
    let encoded = tpe::acquire::read_verified(&envelope.path, Some(REQUEST_BYTES))?.bytes;
    // Never echo private parameter values or password file contents in errors.
    let mut request: Request = serde_json::from_slice(&encoded)
        .map_err(|_| anyhow::anyhow!("invalid PDF Oxide request JSON/schema"))?;
    request.limits.max_output_bytes = request.limits.max_output_bytes.min(envelope.output_limit);
    request.limits.validate()?;
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
            String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("password file is not UTF-8"))
        })
        .transpose()?;
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
    let mut recorded_parameters = request.params;
    if request.operation == "security.encrypt"
        && let Some(parameters) = recorded_parameters.as_object_mut()
    {
        parameters.remove("user_password");
        parameters.remove("owner_password");
    }
    let response = OperationResponse {
        version: 1,
        operation: request.operation.clone(),
        backend: tpe::oxide::identity(),
        producer: tpe::oxide::producer(),
        sources: inputs
            .iter()
            .map(|input| Source {
                sha256: input.sha256.clone(),
                bytes: input.bytes.len() as u64,
            })
            .collect(),
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
        request_options: serde_json::json!({
            "pages":request.pages,
            "parameters_without_passwords":recorded_parameters,
            "limits":request.limits,
            "password_supplied":password.is_some(),
            "new_passwords_omitted":request.operation == "security.encrypt",
        }),
    };
    let encoded = serde_json::to_vec(&response)?;
    ensure!(
        encoded.len() as u64 <= envelope.output_limit,
        "operation response exceeds output limit"
    );
    io::stdout().write_all(&encoded)?;
    Ok(())
}

pub(super) fn publish_worker(encoded: &[u8], limits: LimitEvidence) -> anyhow::Result<()> {
    let request: Publish = serde_json::from_slice(encoded)?;
    let bytes = read_limited(&request.response, request.max_bytes, true)?;
    let mut response: OperationResponse = serde_json::from_slice(&bytes)?;
    ensure!(response.version == 1, "invalid operation response version");
    tpe::oxide::validate_output(&response.output, request.max_bytes, 10_000)?;
    ensure!(
        response
            .sources
            .iter()
            .all(|source| source.sha256.len() == 64
                && source.sha256.bytes().all(|ch| ch.is_ascii_hexdigit())
                && source.bytes <= tpe::oxide::MAX_INPUT_BYTES),
        "invalid source identity in operation response"
    );
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
