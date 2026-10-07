//! `tpe-refcheck`: validate the reference entries of an engine JSON output
//! against DOI and Crossref records. See `docs/REFCHECK.md`.

use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use tpe_refcheck::{
    Cache, Checker, Client, ClientConfig, MAILTO_ENV, Thresholds, load_entries, report,
};

/// What goes to standard output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Stdout {
    /// One summary line.
    Summary,
    /// The JSON report.
    Json,
    /// The Markdown table.
    Markdown,
}

/// Validate extracted references against DOI and Crossref records.
#[derive(Debug, Parser)]
#[command(name = "tpe-refcheck", version, about)]
struct Args {
    /// Engine JSON output: an `extract` result, a `tpe bibliography` record
    /// (or JSON Lines of them), or a list of reference entries.
    input: PathBuf,
    /// Directory that receives `refcheck.json` and `refcheck.md`.
    #[arg(long)]
    out: Option<PathBuf>,
    /// What to print on standard output.
    #[arg(long, value_enum, default_value_t = Stdout::Summary)]
    stdout: Stdout,
    /// Directory for the on-disk response cache (created when missing).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Contact address sent to the registries (polite pool); also `TPE_MAILTO`.
    #[arg(long)]
    mailto: Option<String>,
    /// Exit with status 2 when any entry is not verified.
    #[arg(long)]
    strict: bool,
    /// Never touch the network; answer from the cache only.
    #[arg(long)]
    offline: bool,
    /// Do not run bibliographic queries (entries without a DOI stay `not-found`).
    #[arg(long)]
    no_query: bool,
    /// Whole-request timeout in seconds.
    #[arg(long, default_value_t = 30)]
    timeout_secs: u64,
    /// Least spacing between two requests to the same host, in milliseconds.
    #[arg(long, default_value_t = 200)]
    interval_ms: u64,
    /// Retries after HTTP 429/5xx or a transport failure.
    #[arg(long, default_value_t = 4)]
    retries: u32,
    /// Check only the first N entries.
    #[arg(long)]
    limit: Option<usize>,
    /// Least title similarity for the titles to agree (0..1).
    #[arg(long)]
    title_match: Option<f32>,
}

fn run(args: &Args) -> Result<u8, String> {
    let text = fs::read_to_string(&args.input)
        .map_err(|e| format!("cannot read {}: {e}", args.input.display()))?;
    let mut entries = load_entries(&text).map_err(|e| e.to_string())?;
    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }
    let mailto = args
        .mailto
        .clone()
        .or_else(|| std::env::var(MAILTO_ENV).ok())
        .filter(|m| !m.trim().is_empty());
    if mailto.is_none() && !args.offline {
        eprintln!(
            "tpe-refcheck: no contact address; set {MAILTO_ENV} or --mailto to use the polite pool"
        );
    }
    let mut config = ClientConfig::new(mailto);
    config.timeout = Duration::from_secs(args.timeout_secs.max(1));
    config.interval = Duration::from_millis(args.interval_ms);
    config.retries = args.retries;
    config.offline = args.offline;
    let cache = match &args.cache_dir {
        Some(dir) => Some(Cache::open(dir).map_err(|e| format!("cache {}: {e}", dir.display()))?),
        None => None,
    };
    if args.offline && cache.is_none() {
        return Err("--offline needs --cache-dir".to_string());
    }
    let client = Client::new(config, cache);
    let mut thresholds = Thresholds::default();
    if let Some(t) = args.title_match {
        if !(0.0..=1.0).contains(&t) {
            return Err("--title-match must be between 0 and 1".to_string());
        }
        thresholds.title_match = t;
    }
    let checker = Checker::new(&client)
        .with_thresholds(thresholds)
        .with_query(!args.no_query);
    let report = checker.check_entries(&entries, &args.input.display().to_string());

    let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    let markdown = report::markdown(&report);
    if let Some(dir) = &args.out {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        fs::write(dir.join("refcheck.json"), format!("{json}\n"))
            .map_err(|e| format!("cannot write refcheck.json: {e}"))?;
        fs::write(dir.join("refcheck.md"), &markdown)
            .map_err(|e| format!("cannot write refcheck.md: {e}"))?;
    }
    let s = &report.summary;
    match args.stdout {
        Stdout::Summary => println!(
            "{} entries: {} verified, {} mismatch, {} not found, {} offline/error ({} requests, {} cache hits)",
            s.entries, s.verified, s.mismatch, s.not_found, s.error, s.requests, s.cache_hits
        ),
        Stdout::Json => println!("{json}"),
        Stdout::Markdown => print!("{markdown}"),
    }
    Ok(if args.strict && s.verified != s.entries {
        2
    } else {
        0
    })
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("tpe-refcheck: {e}");
            ExitCode::from(1)
        }
    }
}
