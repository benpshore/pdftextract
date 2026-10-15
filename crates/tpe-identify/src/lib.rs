//! `tpe-identify`: article identity, near-duplicate detection,
//! cross-referencing and reversible canonical renaming for PDF corpora.
//!
//! For every input (a PDF, or an engine result JSON) the crate computes
//! three identities ([`identity::Identity`]): the SHA-256 of the bytes,
//! fingerprints of the normalised text ([`text`]) and a bibliographic key
//! ([`biblio`]). Pairwise evidence between inputs ([`group`]) is joined into
//! works, each with a canonical member and a confidence. [`rename`]
//! proposes `<FirstAuthor>_<Year>_<ShortTitle>.pdf` names and applies them
//! by copy or in-directory rename only, with a manifest that [`rename::undo`]
//! reverses. The command-line tool is `tpe-identify`.
//!
//! No network access; the only text extraction is the engine's pure-Rust
//! `lopdf` backend, used when no stored result is found.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::cast_precision_loss
)]

pub mod biblio;
pub mod group;
pub mod identity;
pub mod rename;
pub mod source;
pub mod text;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use group::{Evidence, Group, GroupKind, Params, Relation, Role};
pub use identity::Identity;
pub use rename::{Manifest, RenamePlan};
pub use source::{InputSpec, LoadOptions};

/// An input that could not be loaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputError {
    pub path: String,
    pub error: String,
}

/// Everything a scan found.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub tool: String,
    pub version: String,
    pub params: Params,
    pub inputs: Vec<Identity>,
    pub evidence: Vec<Evidence>,
    pub groups: Vec<Group>,
    pub rename: Option<RenamePlan>,
    pub errors: Vec<InputError>,
}

/// How to scan.
#[derive(Clone, Debug, Default)]
pub struct ScanOptions {
    pub load: LoadOptions,
    pub params: Params,
    /// Propose names for this directory (`None`: still propose, no directory).
    pub rename_into: Option<PathBuf>,
    /// Cap on proposed base names; 0 means [`rename::DEFAULT_MAX_NAME_LEN`].
    pub max_name_len: usize,
}

/// Identify, cross-reference and propose names for `paths` (files or
/// directories). Per-input failures land in `Report::errors`; the report
/// is always produced.
pub fn scan(paths: &[PathBuf], options: &ScanOptions) -> Report {
    let (specs, collect_errors) = source::collect(paths);
    let mut errors: Vec<InputError> = collect_errors
        .into_iter()
        .map(|(path, error)| InputError {
            path: path.to_string_lossy().into_owned(),
            error,
        })
        .collect();
    let mut inputs: Vec<Identity> = Vec::new();
    for spec in &specs {
        match source::load(spec, &options.load) {
            Ok(loaded) => {
                let index = inputs.len();
                let path = loaded
                    .path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .or_else(|| {
                        matches!(spec, InputSpec::Pdf(_))
                            .then(|| spec.path().to_string_lossy().into_owned())
                    });
                inputs.push(Identity::build(
                    index,
                    path,
                    loaded.size,
                    loaded.sha256,
                    loaded.text_source,
                    loaded.result.as_ref(),
                    loaded.warnings,
                ));
            }
            Err(e) => errors.push(InputError {
                path: spec.path().to_string_lossy().into_owned(),
                error: e.to_string(),
            }),
        }
    }
    let evidence = group::pairwise_evidence(&inputs, &options.params);
    let groups = group::group(&inputs, &evidence);
    let max_len = if options.max_name_len == 0 {
        rename::DEFAULT_MAX_NAME_LEN
    } else {
        options.max_name_len
    };
    let plan = rename::plan(&inputs, &groups, options.rename_into.as_deref(), max_len);
    Report {
        tool: "tpe-identify".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        params: options.params,
        inputs,
        evidence,
        groups,
        rename: Some(plan),
        errors,
    }
}

/// The report as a fixed-width text table: one row per input, grouped,
/// followed by the evidence and any errors.
pub fn render_table(report: &Report) -> String {
    let rows = table_rows(report);
    let header = [
        "group",
        "conf",
        "kind",
        "role",
        "variant",
        "sha256",
        "proposed name",
        "path",
    ];
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row.iter()) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    let line = |cells: &[&str], out: &mut String| {
        let parts: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                if i + 1 == cells.len() {
                    (*c).to_string()
                } else {
                    format!("{c:<width$}", width = widths[i])
                }
            })
            .collect();
        out.push_str(parts.join("  ").trim_end());
        out.push('\n');
    };
    line(&header, &mut out);
    for row in &rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        line(&cells, &mut out);
    }
    let linked = report.groups.iter().filter(|g| g.members.len() > 1).count();
    let _ = writeln!(
        out,
        "\n{} inputs, {} works, {} with more than one member, {} errors",
        report.inputs.len(),
        report.groups.len(),
        linked,
        report.errors.len()
    );
    render_notes(report, &mut out);
    out
}

/// One table row per input, grouped: group id, confidence, kind, role,
/// variant, short hash, proposed name and path.
fn table_rows(report: &Report) -> Vec<[String; 8]> {
    let mut rows: Vec<[String; 8]> = Vec::new();
    for group in &report.groups {
        for member in &group.members {
            let id = &report.inputs[member.index];
            let name = report
                .rename
                .as_ref()
                .and_then(|plan| plan.entries.iter().find(|e| e.index == member.index))
                .map_or_else(String::new, |e| e.to.clone());
            rows.push([
                group.id.to_string(),
                format!("{:.2}", group.confidence),
                kind_name(group.kind).to_string(),
                role_name(member.role).to_string(),
                variant_name(member.variant).to_string(),
                id.sha256[..id.sha256.len().min(12)].to_string(),
                name,
                id.path
                    .clone()
                    .unwrap_or_else(|| format!("(no file) {}", id.text_source)),
            ]);
        }
    }
    rows
}

/// Evidence, warnings and errors under the table.
fn render_notes(report: &Report, out: &mut String) {
    if !report.evidence.is_empty() {
        out.push_str("\nevidence:\n");
        for e in &report.evidence {
            let _ = writeln!(
                *out,
                "  #{} ~ #{}  {:<20} {:.2}  {}",
                e.a,
                e.b,
                e.relation.as_str(),
                e.score,
                e.detail
            );
        }
    }
    let warned: Vec<&Identity> = report
        .inputs
        .iter()
        .filter(|i| !i.warnings.is_empty())
        .collect();
    if !warned.is_empty() {
        out.push_str("\nwarnings:\n");
        for id in warned {
            for w in &id.warnings {
                let _ = writeln!(*out, "  #{}  {}", id.index, w);
            }
        }
    }
    if !report.errors.is_empty() {
        out.push_str("\nerrors:\n");
        for e in &report.errors {
            let _ = writeln!(*out, "  {}: {}", e.path, e.error);
        }
    }
}

/// A dry-run or applied rename plan as text.
pub fn render_plan(plan: &RenamePlan, dir: Option<&Path>) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "rename plan ({}):",
        dir.map_or_else(
            || "no output directory; names only".to_string(),
            |d| format!("into {}", d.display())
        )
    );
    for e in &plan.entries {
        let op = match e.op {
            rename::Op::Copy => "copy",
            rename::Op::Rename => "rename",
            rename::Op::Skip => "skip",
        };
        let _ = writeln!(
            out,
            "  {op:<6} {} -> {}{}",
            e.from.as_deref().unwrap_or("(no file)"),
            e.to,
            e.reason
                .as_ref()
                .map_or_else(String::new, |r| format!("  [{r}]"))
        );
    }
    out
}

fn kind_name(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::Single => "single",
        GroupKind::ExactDuplicates => "exact_duplicates",
        GroupKind::NearDuplicates => "near_duplicates",
        GroupKind::Versions => "versions",
    }
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Canonical => "canonical",
        Role::Duplicate => "duplicate",
        Role::Version => "version",
    }
}

fn variant_name(variant: biblio::Variant) -> &'static str {
    match variant {
        biblio::Variant::Preprint => "preprint",
        biblio::Variant::Published => "published",
        biblio::Variant::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn scan_reports_errors_and_renders() {
        let dir = tempdir().unwrap();
        let bad = dir.path().join("bad.pdf");
        std::fs::write(&bad, b"not a pdf at all").unwrap();
        let report = scan(
            &[bad.clone(), dir.path().join("missing.pdf")],
            &ScanOptions::default(),
        );
        assert!(report.inputs.is_empty());
        assert_eq!(report.errors.len(), 2);
        let table = render_table(&report);
        assert!(table.contains("errors:"));
        assert!(table.contains("missing.pdf"));
        let json = serde_json::to_string(&report).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.errors, report.errors);
    }
}
