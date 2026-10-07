//! `tpe-export`: reverse-citation exports of one extracted article.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use tpe_export::input::{self, RunSelector};
use tpe_export::model::Export;
use tpe_export::{Format, write_export_preserving};

/// Export an extracted article and its reference list as Zotero RDF,
/// Zotero CSV or a SQLite database.
#[derive(Debug, Parser)]
#[command(name = "tpe-export", version, about, long_about = None)]
struct Cli {
    /// Output format: zotero-rdf, csv or sqlite.
    #[arg(long, value_name = "FORMAT")]
    format: Format,
    /// Engine JSON output (`<hash>.json`, `tpe extract --json` or a
    /// `tpe bibliography` record) or a SQLite ledger.
    #[arg(long, value_name = "FILE")]
    input: PathBuf,
    /// File to write; refused when it exists unless --force is given.
    #[arg(long, value_name = "FILE")]
    output: PathBuf,
    /// Replace an existing output file.
    #[arg(long)]
    force: bool,
    /// Ledger input only: the run to export, by row id.
    #[arg(long, value_name = "ID", conflicts_with = "hash")]
    run: Option<i64>,
    /// Ledger input only: the latest run of the document whose SHA-256
    /// starts with this hex prefix. Without --run or --hash the latest
    /// run in the ledger is exported.
    #[arg(long, value_name = "HEX")]
    hash: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), tpe_export::ExportError> {
    let selector = match (cli.run, &cli.hash) {
        (Some(id), _) => RunSelector::RunId(id),
        (None, Some(prefix)) => RunSelector::HashPrefix(prefix.clone()),
        (None, None) => RunSelector::Latest,
    };
    let (article, identity) = input::load_preserving(&cli.input, &selector)?;
    let export = Export::from_article(&article);
    write_export_preserving(&export, cli.format, &cli.output, cli.force, &identity)?;
    println!(
        "wrote {} ({}): 1 article, {} references",
        cli.output.display(),
        cli.format,
        export.cited().len()
    );
    Ok(())
}
