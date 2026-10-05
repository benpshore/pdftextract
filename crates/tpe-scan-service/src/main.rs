//! `tpe-scan-service`: start the loopback scan service for the browser alpha,
//! print the per-launch token once, and stop cleanly on SIGINT/SIGTERM.
//! The hidden `worker` subcommand is the disposable per-scan process.

use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::{Parser, Subcommand};
use tpe_scan_service::server::{Config, Service, normalize_origin};
use tpe_scan_service::{auth, models, worker};

#[derive(Parser)]
#[command(
    name = "tpe-scan-service",
    about = "Loopback-only OCR service for scanned PDFs and images (docling backends)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    /// Port to listen on (127.0.0.1 only); 0 picks a free port.
    #[arg(long, default_value_t = 0)]
    port: u16,
    /// Loopback address to bind; anything else is refused.
    #[arg(long, default_value = "127.0.0.1")]
    bind: IpAddr,
    /// Web-app origin allowed to call the service from a browser
    /// (`scheme://host[:port]`); repeat for several.
    #[arg(long = "origin", value_name = "ORIGIN")]
    origins: Vec<String>,
    /// Largest request body in MiB.
    #[arg(long, default_value_t = 64)]
    max_body_mib: u64,
    /// Most pages one scan may cover (clients window larger documents).
    #[arg(long, default_value_t = 50)]
    max_pages: u32,
    /// Scans allowed to run at once (each is one worker process).
    #[arg(long, default_value_t = 1)]
    max_concurrent: usize,
    /// Seconds a scan may take before its worker is killed.
    #[arg(long, default_value_t = 120)]
    scan_timeout_s: u64,
    /// Address-space growth a worker may use above its start-up mappings, in MiB.
    #[arg(long, default_value_t = 4096)]
    worker_memory_mib: u64,
    /// The `.models` directory (exported to workers as `DOCLING_RS_MODELS_DIR`).
    #[arg(long, value_name = "DIR")]
    models_dir: Option<PathBuf>,
    /// Skip hashing the model files at start.
    #[arg(long)]
    no_hash: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Internal: the disposable scan worker (JSON request as the argument).
    #[command(hide = true)]
    Worker { request: String },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(Cmd::Worker { request }) = cli.command {
        let code = worker::run_worker(&request);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    match serve(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn serve(cli: &Cli) -> Result<(), String> {
    let mut origins = Vec::new();
    for origin in &cli.origins {
        let normalised = normalize_origin(origin)
            .ok_or_else(|| format!("--origin {origin}: expected scheme://host[:port]"))?;
        if !origins.contains(&normalised) {
            origins.push(normalised);
        }
    }
    let token = auth::generate_token().map_err(|error| format!("random source: {error}"))?;
    let mut config = Config::new(token.clone());
    config.bind = cli.bind;
    config.port = cli.port;
    config.origins = origins;
    config.max_body_bytes = cli.max_body_mib.saturating_mul(1024 * 1024);
    config.max_pages = cli.max_pages;
    config.max_concurrent = cli.max_concurrent.max(1);
    config.scan_timeout = Duration::from_secs(cli.scan_timeout_s.max(1));
    config.worker_memory_growth_mib = cli.worker_memory_mib.max(1);
    config.models_dir = cli
        .models_dir
        .as_ref()
        .map(|dir| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.clone()));
    config.hash_models = !cli.no_hash;
    config.worker_exe =
        std::env::current_exe().map_err(|error| format!("locating this executable: {error}"))?;

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stop))
            .map_err(|error| format!("installing the signal handler: {error}"))?;
    }
    let service = Service::start(config.clone()).map_err(|error| error.to_string())?;
    let models = service.models();
    let pdfium = models::pdfium();
    println!(
        "tpe-scan-service {} listening on http://{} (loopback only, no telemetry)",
        tpe_scan_service::VERSION,
        service.addr()
    );
    if config.origins.is_empty() {
        println!("allowed browser origins: none (pass --origin <web app origin> to allow one)");
    } else {
        println!("allowed browser origins: {}", config.origins.join(", "));
    }
    match models::provisioning_problem(&models, &pdfium) {
        None => println!("ocr: models and PDFium provisioned"),
        Some(problem) => println!("ocr: {problem}"),
    }
    println!(
        "limits: {} MiB per request, {} pages per scan, {} concurrent, {} s per scan",
        cli.max_body_mib,
        config.max_pages,
        config.max_concurrent,
        config.scan_timeout.as_secs()
    );
    println!("token (shown once; paste it into the web app's scan-service dialog):");
    println!("{token}");
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("stopping");
    service.shutdown();
    Ok(())
}
