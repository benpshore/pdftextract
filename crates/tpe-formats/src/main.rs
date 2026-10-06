//! `tpe-formats`: extract text and structure from non-PDF inputs, one
//! `<stem>.txt` and `<stem>.json` per input. Inputs are never modified.
//!
//! Exit codes: 0 every input produced a complete or partial result; 1 at
//! least one input failed (unreadable or malformed; other inputs are still
//! written); 2 usage error (no inputs, a directory without `--recursive`,
//! unwritable output directory); 3 at least one input was unsupported here
//! and none failed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use serde::Serialize;
use tpe_formats::{
    Format, FormatsError, Options, Status, create_output_directory, extract_path,
    write_outputs_with_inputs,
};

/// Extract text from docx, pptx, xlsx, csv/tsv, html, md, txt, Pages/Numbers
/// packages and audio (through a local engine), one output pair per input.
#[derive(Parser)]
#[command(name = "tpe-formats", version, about)]
struct Cli {
    /// Files to extract; directories need `--recursive`.
    #[arg(required = true)]
    inputs: Vec<PathBuf>,
    /// Directory that receives `<stem>.txt` and `<stem>.json`.
    #[arg(long)]
    out: PathBuf,
    /// Walk directories and take every file with a recognised extension.
    #[arg(long)]
    recursive: bool,
    /// Overwrite outputs that already exist (they are skipped otherwise).
    #[arg(long)]
    force: bool,
    /// Print one JSON report for the run on stdout instead of text lines.
    #[arg(long)]
    json: bool,
}

/// One line of the run report.
#[derive(Serialize)]
struct Report {
    input: String,
    status: String,
    format: Option<Format>,
    #[serde(skip_serializing_if = "Option::is_none")]
    json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    txt: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extra: Vec<String>,
    warnings: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let inputs = match expand_inputs(&cli.inputs, cli.recursive) {
        Ok(inputs) => inputs,
        Err(message) => {
            eprintln!("tpe-formats: {message}");
            return ExitCode::from(2);
        }
    };
    if inputs.is_empty() {
        eprintln!("tpe-formats: no inputs with a recognised extension");
        return ExitCode::from(2);
    }
    if let Err(e) = create_output_directory(&cli.out, &inputs) {
        eprintln!("tpe-formats: cannot create {}: {e}", cli.out.display());
        return ExitCode::from(2);
    }
    let options = Options::default();
    let mut used_stems: BTreeSet<String> = BTreeSet::new();
    let mut reports = Vec::new();
    let mut failed = false;
    let mut unsupported = false;
    for input in &inputs {
        let stem = unique_stem(input, &mut used_stems);
        let report = process_one(input, &stem, &cli, &options, &inputs);
        match report.status.as_str() {
            "failed" => failed = true,
            "unsupported" => unsupported = true,
            _ => {}
        }
        if !cli.json {
            print_line(&report);
        }
        reports.push(report);
    }
    if cli.json {
        match serde_json::to_string_pretty(&reports) {
            Ok(text) => println!("{text}"),
            Err(e) => eprintln!("tpe-formats: cannot serialise report: {e}"),
        }
    }
    if failed {
        ExitCode::from(1)
    } else if unsupported {
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    }
}

/// Extract one input and report typed failures without stopping the batch.
fn process_one(
    input: &Path,
    stem: &str,
    cli: &Cli,
    options: &Options,
    inputs: &[PathBuf],
) -> Report {
    let mut report = Report {
        input: input.display().to_string(),
        status: "failed".to_string(),
        format: None,
        json: None,
        txt: None,
        extra: Vec::new(),
        warnings: Vec::new(),
    };
    let result = match extract_path(input, options) {
        Ok(result) => result,
        Err(FormatsError::Unsupported(reason)) => {
            report.status = "unsupported".to_string();
            report.warnings.push(format!("unsupported: {reason}"));
            return report;
        }
        Err(e) => {
            report.warnings.push(format!("failed: {e}"));
            return report;
        }
    };
    report.format = Some(result.format);
    report.warnings.clone_from(&result.warnings);
    match write_outputs_with_inputs(&result, &cli.out, stem, cli.force, inputs) {
        Ok(Some(outputs)) => {
            report.status = result.status.to_string();
            report.json = Some(outputs.json.display().to_string());
            report.txt = outputs.txt.map(|p| p.display().to_string());
            report.extra = outputs
                .extra
                .iter()
                .map(|p| p.display().to_string())
                .collect();
        }
        Ok(None) => {
            report.status = if result.status == Status::Unsupported {
                "unsupported".to_string()
            } else {
                "skipped".to_string()
            };
            report
                .warnings
                .push("outputs exist; not overwritten (use --force)".to_string());
        }
        Err(e) => report
            .warnings
            .push(format!("failed: writing outputs: {e}")),
    }
    report
}

fn print_line(report: &Report) {
    let format = report.format.map_or("-", Format::as_str);
    let target = report.json.as_deref().unwrap_or("-");
    println!(
        "{}\t{}\t{} -> {}",
        report.status, format, report.input, target
    );
    for warning in &report.warnings {
        println!("\t{warning}");
    }
}

/// `<stem>`, with `-2`, `-3`, ... appended when another input in this run
/// already took it.
fn unique_stem(input: &Path, used: &mut BTreeSet<String>) -> String {
    let base = input
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("input")
        .to_string();
    let mut stem = base.clone();
    let mut n = 1;
    while used.contains(&stem) {
        n += 1;
        stem = format!("{base}-{n}");
    }
    used.insert(stem.clone());
    stem
}

/// Files to process: given files as they are; directories walked when
/// `recursive`, keeping files with recognised extensions and treating
/// `.pages`/`.numbers` directories as single inputs.
fn expand_inputs(inputs: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for input in inputs {
        if !input.exists() {
            return Err(format!("{} does not exist", input.display()));
        }
        if !input.is_dir() || is_bundle(input) {
            out.push(input.clone());
            continue;
        }
        if !recursive {
            return Err(format!(
                "{} is a directory; pass --recursive to walk it",
                input.display()
            ));
        }
        let mut walker = walkdir::WalkDir::new(input).sort_by_file_name().into_iter();
        while let Some(entry) = walker.next() {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if entry.file_type().is_dir() {
                if is_bundle(path) {
                    out.push(path.to_path_buf());
                    walker.skip_current_dir();
                }
                continue;
            }
            if path
                .extension()
                .and_then(|e| e.to_str())
                .and_then(Format::from_extension)
                .is_some()
            {
                out.push(path.to_path_buf());
            }
        }
    }
    Ok(out)
}

fn is_bundle(path: &Path) -> bool {
    path.is_dir()
        && matches!(
            path.extension()
                .and_then(|e| e.to_str())
                .and_then(Format::from_extension),
            Some(Format::Pages | Format::Numbers)
        )
}
