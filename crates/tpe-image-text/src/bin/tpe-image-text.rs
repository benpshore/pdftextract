//! `tpe-image-text`: OCR every image given (or found under given directories)
//! into `<out>/<stem>.txt` and `<out>/<stem>.json`, with the engine that is
//! really installed. Exit 0 when every input was written, 1 when any failed,
//! 2 when no engine could run or the arguments were unusable.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use tpe_image_text::engine::{self, Availability, Engine, EngineKind, EngineOptions};
use tpe_image_text::{
    FileOutcome, PreprocessOptions, RunOptions, collect_inputs, limits, run_batch,
};

/// OCR still images into one text file and one JSON report per input.
// Each bool is an independent command-line switch; an enum per flag would not read better.
#[allow(clippy::struct_excessive_bools)]
#[derive(Parser, Debug)]
#[command(name = "tpe-image-text", version, about, long_about = None)]
struct Cli {
    /// Image files or directories (png, jpeg, tiff, bmp, webp, gif; heic/heif are reported as unsupported).
    #[arg(required_unless_present = "list_engines")]
    inputs: Vec<PathBuf>,
    /// Directory for `<stem>.txt` and `<stem>.json` (created if missing).
    #[arg(long, required_unless_present = "list_engines")]
    out: Option<PathBuf>,
    /// Descend into subdirectories.
    #[arg(long)]
    recursive: bool,
    /// Engine; `auto` takes the first available of tesseract, ocrs, docling.
    #[arg(long, value_enum, default_value_t = EngineKind::Auto)]
    engine: EngineKind,
    /// Language code for the engine (tesseract codes; ocrs supports `eng` only).
    #[arg(long, default_value = "eng")]
    lang: String,
    /// Overwrite existing outputs.
    #[arg(long)]
    force: bool,
    /// Print one JSON document describing the run instead of text lines.
    #[arg(long)]
    json: bool,
    /// Report every engine's availability and exit.
    #[arg(long)]
    list_engines: bool,
    /// Wall-clock limit per image for an external engine, in seconds.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout_secs: u64,
    /// Address-space limit for an external engine process, in MiB.
    #[arg(long, default_value_t = 2048, value_parser = clap::value_parser!(u64).range(64..=65536))]
    max_memory_mib: u64,
    /// Path of the tesseract executable (default: `TPE_TESSERACT_BIN`, then PATH).
    #[arg(long)]
    tesseract_bin: Option<PathBuf>,
    /// Directory of the ocrs models (default: `TPE_OCRS_MODELS_DIR`, then `.models/ocrs`).
    #[arg(long)]
    ocrs_models: Option<PathBuf>,
    /// Skip deskewing.
    #[arg(long)]
    no_deskew: bool,
    /// Skip the 2x upscale of small text.
    #[arg(long)]
    no_upscale: bool,
    /// Skip auto-contrast.
    #[arg(long)]
    no_contrast: bool,
}

fn main() -> ExitCode {
    let raw: Vec<OsString> = std::env::args_os().collect();
    if raw.get(1).is_some_and(|a| a == limits::EXEC_LIMITED) {
        return ExitCode::from(limits::exec_limited_main(&raw[2..]));
    }
    let cli = Cli::parse();
    let env_path = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let opts = EngineOptions {
        tesseract_bin: cli
            .tesseract_bin
            .clone()
            .or_else(|| env_path("TPE_TESSERACT_BIN")),
        limit_helper: std::env::current_exe().ok(),
        timeout: Duration::from_secs(cli.timeout_secs),
        address_space_bytes: cli.max_memory_mib * 1024 * 1024,
        ocrs_models_dir: cli
            .ocrs_models
            .clone()
            .or_else(|| env_path("TPE_OCRS_MODELS_DIR")),
        docling_models_dir: None,
    };

    if cli.list_engines {
        list_engines(&opts, cli.json);
        return ExitCode::SUCCESS;
    }

    let (engine, probes) = match engine::select(cli.engine, &opts) {
        Ok(selected) => selected,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let out_dir = cli
        .out
        .clone()
        .expect("clap requires --out without --list-engines");
    let (files, notes) = match collect_inputs(&cli.inputs, cli.recursive) {
        Ok(collected) => collected,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let run = RunOptions {
        out_dir,
        force: cli.force,
        lang: cli.lang.clone(),
        preprocess: PreprocessOptions {
            auto_contrast: !cli.no_contrast,
            upscale_small_text: !cli.no_upscale,
            deskew: !cli.no_deskew,
        },
    };
    let outcomes = run_batch(&files, engine.as_ref(), &run);
    let failed = outcomes.iter().filter(|o| !o.ok).count();
    if cli.json {
        print_json_summary(
            engine.as_ref(),
            &probes,
            &run,
            &files,
            &notes,
            &outcomes,
            failed,
        );
    } else {
        print_outcomes(engine.as_ref(), &notes, &outcomes, files.len(), failed);
    }
    if files.is_empty() {
        eprintln!("error: no input images found");
        return ExitCode::from(2);
    }
    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Print every engine's availability.
fn list_engines(opts: &EngineOptions, json: bool) {
    let probes = engine::probe_all(opts);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&probes).unwrap_or_default()
        );
        return;
    }
    for p in &probes {
        println!(
            "{:<10} {:<12} {}",
            p.engine,
            if p.available {
                "available"
            } else {
                "unavailable"
            },
            p.detail
        );
    }
}

/// One JSON document describing the whole run.
fn print_json_summary(
    engine: &dyn Engine,
    probes: &[Availability],
    run: &RunOptions,
    files: &[PathBuf],
    notes: &[String],
    outcomes: &[FileOutcome],
    failed: usize,
) {
    let summary = serde_json::json!({
        "tool": { "name": tpe_image_text::TOOL_NAME, "version": tpe_image_text::TOOL_VERSION },
        "engine": { "name": engine.name(), "version": engine.version(), "lang": run.lang },
        "engines_probed": probes,
        "out_dir": run.out_dir.display().to_string(),
        "inputs": files.len(),
        "written": outcomes.len() - failed,
        "failed": failed,
        "notes": notes,
        "files": outcomes,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).unwrap_or_default()
    );
}

/// Human-readable per-file lines on stdout, run facts on stderr.
fn print_outcomes(
    engine: &dyn Engine,
    notes: &[String],
    outcomes: &[FileOutcome],
    inputs: usize,
    failed: usize,
) {
    eprintln!("engine: {} {}", engine.name(), engine.version());
    for note in notes {
        eprintln!("note: {note}");
    }
    for o in outcomes {
        if o.ok {
            println!(
                "ok    {} -> {} ({} blocks, {} chars, {} ms)",
                o.input,
                o.text_path.as_deref().unwrap_or(""),
                o.blocks,
                o.text_chars,
                o.elapsed_ms
            );
            for w in &o.warnings {
                println!("      warning: {w}");
            }
        } else {
            println!(
                "fail  {}: {}",
                o.input,
                o.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    eprintln!(
        "{} written, {} failed, {} inputs",
        outcomes.len() - failed,
        failed,
        inputs
    );
}
