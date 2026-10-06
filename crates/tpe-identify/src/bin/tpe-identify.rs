//! `tpe-identify`: find duplicate, near-duplicate and related versions of
//! articles among PDFs, and propose (or apply) canonical names.
//!
//! ```text
//! tpe-identify scan <PATHS>... [--json out.json] [--results DIR] [--db ledger.sqlite]
//!                  [--rename-into DIR [--apply]] [--no-extract]
//! tpe-identify undo <MANIFEST> [--apply]
//! ```
//!
//! `scan` is a dry run unless `--apply` is given with `--rename-into`.
//! Applying copies files into the output directory (or renames files that
//! already live there), never modifies contents, never deletes, and writes
//! a manifest that `undo` reverses.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use tpe::publication::StagedOutputs;
use tpe_identify::rename::{self, DEFAULT_MAX_NAME_LEN};
use tpe_identify::{LoadOptions, Params, ScanOptions, render_plan, render_table, scan};

#[derive(Parser)]
#[command(
    name = "tpe-identify",
    version,
    about = "Article identity, deduplication, cross-referencing and canonical renaming"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Identify and cross-reference PDFs (and engine result JSON files).
    Scan(ScanArgs),
    /// Reverse an applied rename plan from its manifest (dry run unless --apply).
    Undo(UndoArgs),
}

#[derive(Args)]
struct ScanArgs {
    /// PDF files, directories (walked for *.pdf) or `tpe` result JSON files.
    #[arg(required = true)]
    paths: Vec<PathBuf>,
    /// Write the full JSON report to a new file (existing paths are refused).
    #[arg(long)]
    json: Option<PathBuf>,
    /// Directory of `tpe extract --out` results (`<hash>.json`), used before extracting.
    #[arg(long)]
    results: Option<PathBuf>,
    /// A `tpe` ledger; its latest run for each hash is used before extracting.
    #[arg(long)]
    db: Option<PathBuf>,
    /// Never extract text (byte identity and stored results only).
    #[arg(long)]
    no_extract: bool,
    /// Largest PDF to read, in bytes.
    #[arg(long)]
    max_bytes: Option<u64>,
    /// Propose names for this directory; with --apply, copy/rename into it.
    #[arg(long)]
    rename_into: Option<PathBuf>,
    /// Carry out the rename plan (requires --rename-into).
    #[arg(long, requires = "rename_into")]
    apply: bool,
    /// Jaccard estimate at or above which texts are near-duplicates.
    #[arg(long, default_value_t = Params::default().near_threshold)]
    near_threshold: f64,
    /// Jaccard estimate at or above which texts alone link versions of one work.
    #[arg(long, default_value_t = Params::default().text_threshold)]
    text_threshold: f64,
    /// Longest proposed base name (without `.pdf`).
    #[arg(long, default_value_t = DEFAULT_MAX_NAME_LEN)]
    max_name_len: usize,
    /// Print only the table (no evidence or warnings).
    #[arg(long)]
    quiet: bool,
}

#[derive(Args)]
struct UndoArgs {
    /// A `tpe-identify-manifest*.json` written by `scan --apply`.
    manifest: PathBuf,
    /// Carry out the reversal.
    #[arg(long)]
    apply: bool,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Scan(args) => run_scan(&args),
        Command::Undo(args) => run_undo(&args),
    }
}

fn run_scan(args: &ScanArgs) -> ExitCode {
    let unit = 0.0..=1.0;
    if !unit.contains(&args.near_threshold)
        || !unit.contains(&args.text_threshold)
        || args.text_threshold > args.near_threshold
    {
        eprintln!("thresholds must satisfy 0 <= text-threshold <= near-threshold <= 1");
        return ExitCode::from(2);
    }
    let options = ScanOptions {
        load: LoadOptions {
            results_dir: args.results.clone(),
            ledger: args.db.clone(),
            extract: !args.no_extract,
            max_bytes: args.max_bytes,
        },
        params: Params {
            near_threshold: args.near_threshold,
            text_threshold: args.text_threshold,
            ..Params::default()
        },
        rename_into: args.rename_into.clone(),
        max_name_len: args.max_name_len,
    };
    let report = scan(&args.paths, &options);
    let mut failed = !report.errors.is_empty();
    let table = render_table(&report);
    if args.quiet {
        print!("{}", table.split("\nevidence:\n").next().unwrap_or(&table));
    } else {
        print!("{table}");
    }
    if let Some(plan) = &report.rename
        && (args.rename_into.is_some() || args.json.is_none())
    {
        print!("\n{}", render_plan(plan, args.rename_into.as_deref()));
    }
    // Publish the report before applying any rename. An invalid/colliding report
    // destination must fail before sources can move, even with --apply.
    if let Some(path) = &args.json {
        let written = serde_json::to_vec_pretty(&report)
            .map_err(|e| e.to_string())
            .and_then(|mut bytes| {
                bytes.push(b'\n');
                let mut staged =
                    StagedOutputs::stage(path, &[("", bytes)]).map_err(|e| e.to_string())?;
                staged.publish_exact(path)
            });
        if let Err(e) = written {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::from(1);
        }
        println!("report written to {}", path.display());
    }
    if args.apply {
        if let (Some(dir), Some(plan)) = (&args.rename_into, &report.rename) {
            failed |= !apply_plan(plan, dir);
        }
    } else if args.rename_into.is_some() {
        println!("\ndry run: nothing was copied or renamed (add --apply)");
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Apply the plan and print the outcome; `false` when applying failed.
fn apply_plan(plan: &rename::RenamePlan, dir: &std::path::Path) -> bool {
    match rename::apply(plan, dir) {
        Ok((manifest, path)) => {
            let done = manifest
                .entries
                .iter()
                .filter(|e| e.skipped.is_none())
                .count();
            let skipped = manifest.entries.len() - done;
            println!(
                "\napplied {done} operation(s), skipped {skipped}; manifest: {}",
                path.display()
            );
            for e in manifest.entries.iter().filter(|e| e.skipped.is_some()) {
                println!(
                    "  skipped {} -> {}: {}",
                    e.from,
                    e.to,
                    e.skipped.as_deref().unwrap_or("")
                );
            }
            skipped == 0
        }
        Err(e) => {
            eprintln!("apply failed: {e}");
            false
        }
    }
}

fn run_undo(args: &UndoArgs) -> ExitCode {
    let manifest = match rename::read_manifest(&args.manifest) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("cannot read manifest: {e}");
            return ExitCode::from(2);
        }
    };
    let entries = rename::undo(&manifest, !args.apply);
    println!(
        "{} ({} entries):",
        if args.apply { "undone" } else { "undo dry run" },
        entries.len()
    );
    for e in &entries {
        println!(
            "  {:<8} {} -> {}{}",
            e.action,
            e.to,
            e.from,
            e.reason
                .as_ref()
                .map_or_else(String::new, |r| format!("  [{r}]"))
        );
    }
    if !args.apply {
        println!("dry run: nothing was changed (add --apply)");
    }
    if args.apply
        && entries.iter().any(|entry| {
            entry.reason.as_deref().is_some_and(|reason| {
                !reason.starts_with("never applied:") && reason != "nothing was done"
            })
        })
    {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
