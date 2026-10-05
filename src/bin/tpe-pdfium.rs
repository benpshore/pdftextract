//! `tpe-pdfium`: provision and inspect the pinned `PDFium` shared library.
//!
//! `fetch` downloads the archive pinned in `native/manifest.json` for this
//! platform, verifies both digests and installs the library in the per-user
//! data directory; `status` shows what the `pdfium` backend would load;
//! `path` prints the install directory for `PDFIUM_DYNAMIC_LIB_PATH` (needed
//! only by tools that read the variable themselves, such as full `docling`).

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use tpe::pdfium_provision::{self, Effective, FetchAction, Status};

const USER_AGENT: &str =
    "text-processing-engine tpe-pdfium (+https://github.com/benpshore/pdftextract)";

#[derive(Parser)]
#[command(
    name = "tpe-pdfium",
    version,
    about = "Provision the pinned PDFium library"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download, verify (SHA-256 of archive and library) and install the pinned
    /// library for this platform under the per-user data directory.
    Fetch {
        /// Re-download even when a verified copy is installed.
        #[arg(long)]
        force: bool,
    },
    /// Show the pin, the install location, and what the backend would load.
    Status,
    /// Print the directory holding the verified library (exit 1 when absent),
    /// for `export PDFIUM_DYNAMIC_LIB_PATH="$(tpe-pdfium path)"`.
    Path,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Cmd::Fetch { force } => match pdfium_provision::fetch(USER_AGENT, force) {
            Ok(outcome) => {
                let verb = match outcome.action {
                    FetchAction::AlreadyInstalled => "already installed",
                    FetchAction::Installed => "installed",
                };
                println!(
                    "{verb}: {} ({} {}, sha256 {} verified)",
                    outcome.library.path.display(),
                    outcome.library.release,
                    outcome.library.platform,
                    outcome.library.sha256
                );
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("tpe-pdfium fetch: {err}");
                ExitCode::from(2)
            }
        },
        Cmd::Status => {
            let status = Status::inspect();
            print!("{}", status.render());
            if matches!(status.effective, Effective::None) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Cmd::Path => {
            if let Some(library) = pdfium_provision::installed_library() {
                let dir = library.path.parent().map_or_else(
                    || library.path.display().to_string(),
                    |dir| dir.display().to_string(),
                );
                println!("{dir}");
                ExitCode::SUCCESS
            } else {
                eprintln!("tpe-pdfium path: no verified library installed; run `tpe-pdfium fetch`");
                ExitCode::from(1)
            }
        }
    }
}
