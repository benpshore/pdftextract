//! `tpe-pdfium`: the `tpe pdfium` provisioning commands as a stand-alone
//! binary for builds that do not want the whole CLI. See
//! `tpe::pdfium_provision::cli`.

use std::process::ExitCode;

use clap::Parser;
use tpe::pdfium_provision::cli::{Command, run};

#[derive(Parser)]
#[command(
    name = "tpe-pdfium",
    version,
    about = "Provision the pinned PDFium library"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

fn main() -> ExitCode {
    run(&Cli::parse().command)
}
