//! `tpe-pdfops`: PDF operations that always write new files.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tpe_pdfops::clean::{CleanOptions, clean};
use tpe_pdfops::concat::{ConcatOptions, concatenate};
use tpe_pdfops::imagepdf::ImageEncoding;
use tpe_pdfops::inspect::{inspect, render_summary};
use tpe_pdfops::letter::normalise_to_letter;
use tpe_pdfops::linearize::linearize;
use tpe_pdfops::output::{Output, ensure_not_input, save_document};
use tpe_pdfops::paginate::{PaginateOptions, SplitMode, paginate};
use tpe_pdfops::reorient::{Orientation, reorient};
use tpe_pdfops::unlock::remove_password;
use tpe_pdfops::{PageSelection, PdfOpsError};

/// Exit status for an explicit unsupported result.
const EXIT_UNSUPPORTED: u8 = 3;

#[derive(Parser)]
#[command(
    name = "tpe-pdfops",
    version,
    about = "PDF operations that always write new files (inputs are never modified)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Output location shared by the writing operations.
#[derive(Args)]
struct OutArgs {
    /// Output file (a directory for `paginate`). Never an input.
    #[arg(long, value_name = "PATH")]
    out: PathBuf,
    /// Replace an existing output file.
    #[arg(long)]
    force: bool,
}

impl OutArgs {
    fn output(&self) -> Output {
        Output::new(&self.out).force(self.force)
    }
}

#[derive(Subcommand)]
enum Command {
    /// Merge PDFs in the given order into one new file.
    Concat {
        /// Input PDFs, in order.
        #[arg(required = true, value_name = "PDF")]
        inputs: Vec<PathBuf>,
        /// Add one top-level outline entry per input.
        #[arg(long)]
        outlines: bool,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Split a PDF into new files by page ranges or by N pages per file.
    Paginate {
        /// Input PDF.
        input: PathBuf,
        /// Page selection for one output file, e.g. `1-3,7`; repeatable.
        #[arg(long, value_name = "RANGES", conflicts_with = "every")]
        pages: Vec<String>,
        /// Pages per output file.
        #[arg(long, value_name = "N")]
        every: Option<u32>,
        /// Stamp "Page n of N" (source numbering) at the bottom of every page.
        #[arg(long)]
        stamp: bool,
        /// File stem for the outputs (`<stem>-001.pdf`, ...); defaults to the input's.
        #[arg(long)]
        stem: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Rotate pages via /Rotate, explicitly or from the dominant text direction.
    Reorient {
        /// Input PDF.
        input: PathBuf,
        /// Degrees clockwise to add: 90, 180 or 270.
        #[arg(long, value_name = "DEG", conflicts_with = "auto")]
        rotate: Option<i64>,
        /// Decide per page from the text matrices; undecidable pages are left alone.
        #[arg(long)]
        auto: bool,
        /// Only these pages, e.g. `1-3,7`.
        #[arg(long, value_name = "RANGES")]
        pages: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Scale and centre every page onto US Letter, keeping the aspect ratio.
    Letter {
        /// Input PDF.
        input: PathBuf,
        /// Only these pages, e.g. `1-3,7`.
        #[arg(long, value_name = "RANGES")]
        pages: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Remove the open password (tried once; nothing is guessed).
    Unlock {
        /// Input PDF.
        input: PathBuf,
        /// The user or owner password.
        #[arg(long, value_name = "PASSWORD")]
        password: String,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Linearize for fast web view: reported as unsupported with the reason.
    Linearize {
        /// Input PDF.
        input: PathBuf,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Raster clean-ups: render with `PDFium`, process, write image-backed pages.
    Clean {
        /// Input PDF.
        input: PathBuf,
        /// Adaptive background normalisation (stains, uneven lighting).
        #[arg(long)]
        water_stain: bool,
        /// Simple baseline straightening.
        #[arg(long)]
        dewarp: bool,
        /// Projection-profile deskew (up to 5 degrees either way).
        #[arg(long)]
        deskew: bool,
        /// Render resolution.
        #[arg(long, default_value_t = 200)]
        dpi: u32,
        /// Page image encoding.
        #[arg(long, value_enum, default_value_t = Encoding::Jpeg)]
        encoding: Encoding,
        /// JPEG quality (1-100).
        #[arg(long, default_value_t = 85)]
        quality: u8,
        /// Open password for the input.
        #[arg(long)]
        password: Option<String>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Re-parse a PDF and print its page count, boxes and rotation.
    Inspect {
        /// Input PDF.
        input: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Encoding {
    Jpeg,
    Flate,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if err.is_unsupported() => {
            eprintln!("tpe-pdfops: {err}");
            ExitCode::from(EXIT_UNSUPPORTED)
        }
        Err(err) => {
            eprintln!("tpe-pdfops: error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn selection(spec: Option<&str>) -> Result<Option<PageSelection>, PdfOpsError> {
    spec.map(PageSelection::parse).transpose()
}

fn write(doc: &mut lopdf::Document, inputs: &[&Path], out: &OutArgs) -> Result<(), PdfOpsError> {
    ensure_not_input(&out.out, inputs)?;
    let pages = save_document(doc, &out.output())?;
    println!("wrote {} ({pages} pages)", out.out.display());
    Ok(())
}

fn run(command: Command) -> Result<(), PdfOpsError> {
    match command {
        Command::Concat {
            inputs,
            outlines,
            out,
        } => {
            let paths: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
            ensure_not_input(&out.out, &paths)?;
            let mut doc = concatenate(&paths, &ConcatOptions { outlines })?;
            write(&mut doc, &paths, &out)
        }
        Command::Paginate {
            input,
            pages,
            every,
            stamp,
            stem,
            out,
        } => run_paginate(&input, &pages, every, stamp, stem, &out),
        Command::Reorient {
            input,
            rotate,
            auto,
            pages,
            out,
        } => run_reorient(&input, rotate, auto, pages.as_deref(), &out),
        Command::Letter { input, pages, out } => {
            let (mut doc, fits) =
                normalise_to_letter(&input, selection(pages.as_deref())?.as_ref())?;
            for fit in &fits {
                println!(
                    "page {}: scale {:.4} offset ({:.2}, {:.2}) -> {}x{}",
                    fit.number,
                    fit.scale,
                    fit.offset.0,
                    fit.offset.1,
                    fit.media_box[2],
                    fit.media_box[3]
                );
            }
            write(&mut doc, &[&input], &out)
        }
        Command::Unlock {
            input,
            password,
            out,
        } => {
            let mut doc = remove_password(&input, &password)?;
            write(&mut doc, &[&input], &out)
        }
        Command::Linearize { input, out } => {
            ensure_not_input(&out.out, &[&input])?;
            linearize(&input)
        }
        Command::Clean {
            input,
            water_stain,
            dewarp,
            deskew,
            dpi,
            encoding,
            quality,
            password,
            out,
        } => {
            let options = CleanOptions {
                water_stain,
                dewarp,
                deskew,
                dpi,
                encoding: match encoding {
                    Encoding::Jpeg => ImageEncoding::Jpeg { quality },
                    Encoding::Flate => ImageEncoding::Flate,
                },
                password,
            };
            run_clean(&input, &options, &out)
        }
        Command::Inspect { input } => {
            let summary = inspect(&input)?;
            print!("{}", render_summary(&input, &summary));
            Ok(())
        }
    }
}

fn run_clean(input: &Path, options: &CleanOptions, out: &OutArgs) -> Result<(), PdfOpsError> {
    ensure_not_input(&out.out, &[input])?;
    let (mut doc, report) = clean(input, options)?;
    for page in &report {
        let mut line = format!("page {}:", page.number);
        if let Some(angle) = page.skew_degrees {
            let _ = write!(line, " deskew {angle:.2} deg");
        }
        if let Some(shift) = page.dewarp_shift_px {
            let _ = write!(line, " dewarp max {shift:.1} px");
        }
        println!("{line}");
    }
    write(&mut doc, &[input], out)
}

fn run_paginate(
    input: &Path,
    pages: &[String],
    every: Option<u32>,
    stamp: bool,
    stem: Option<String>,
    out: &OutArgs,
) -> Result<(), PdfOpsError> {
    let mode = match (every, pages.is_empty()) {
        (Some(n), _) => SplitMode::EveryN(n),
        (None, false) => SplitMode::Ranges(
            pages
                .iter()
                .map(|p| PageSelection::parse(p))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        (None, true) => {
            return Err(PdfOpsError::Invalid(
                "give --pages RANGES (repeatable) or --every N".into(),
            ));
        }
    };
    let options = PaginateOptions { mode, stamp, stem };
    let written = paginate(input, &options, &out.output())?;
    for path in &written {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn run_reorient(
    input: &Path,
    rotate: Option<i64>,
    auto: bool,
    pages: Option<&str>,
    out: &OutArgs,
) -> Result<(), PdfOpsError> {
    let orientation = match (rotate, auto) {
        (Some(deg), false) => Orientation::Rotate(deg),
        (None, true) => Orientation::Auto,
        _ => return Err(PdfOpsError::Invalid("give --rotate DEG or --auto".into())),
    };
    let (mut doc, outcomes) = reorient(input, orientation, selection(pages)?.as_ref())?;
    for outcome in &outcomes {
        match &outcome.note {
            Some(note) => println!(
                "page {}: {} (unchanged: {note})",
                outcome.number, outcome.before
            ),
            None => println!(
                "page {}: {} -> {}",
                outcome.number, outcome.before, outcome.after
            ),
        }
    }
    write(&mut doc, &[input], out)
}
