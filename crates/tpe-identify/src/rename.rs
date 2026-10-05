//! Canonical file names (`<FirstAuthor>_<Year>_<ShortTitle>.pdf`), a
//! collision-free plan over a whole scan, and the apply/undo operations.
//!
//! Applying never touches an input's contents and never deletes: a file
//! outside the output directory is copied into it (the copy is verified
//! by hash before the manifest records it), a file already inside the
//! output directory is renamed there, and nothing is ever overwritten.
//! The manifest lists `from -> to` for every operation; [`undo`] reverses
//! it, removing only copies this tool made whose originals still exist.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tpe::schema::sha256_hex;

use crate::biblio::fold_ascii;
use crate::group::{Group, Role};
use crate::identity::Identity;

/// Default cap on the length of a proposed base name (without `.pdf`).
pub const DEFAULT_MAX_NAME_LEN: usize = 96;

/// Most words of the title kept in a name.
const SHORT_TITLE_WORDS: usize = 6;

/// Words dropped from short titles when something else remains.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "on", "in", "for", "and", "or", "to", "with", "by", "from", "at", "as",
    "is", "are", "its", "into", "via", "toward", "towards", "using", "over", "under", "between",
    "about", "within", "without", "through", "de", "la", "le", "der", "die", "das", "und", "et",
    "des", "du", "el", "los", "las", "il",
];

/// `Firstauthor` part: ASCII letters of the surname, capitalised; `Unknown`
/// when there is none.
pub fn author_part(surname: Option<&str>) -> String {
    let letters: String = surname
        .map(fold_ascii)
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .collect();
    if letters.is_empty() {
        return "Unknown".to_string();
    }
    capitalize(&letters.to_ascii_lowercase())
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Title words usable in a name: ASCII-folded alphanumeric words, stopwords
/// dropped unless nothing else remains, at most [`SHORT_TITLE_WORDS`].
pub fn title_words(title: &str) -> Vec<String> {
    let words: Vec<String> = fold_ascii(title)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| capitalize(&w.to_ascii_lowercase()))
        .collect();
    let content: Vec<String> = words
        .iter()
        .filter(|w| !STOPWORDS.contains(&w.to_ascii_lowercase().as_str()))
        .cloned()
        .collect();
    let chosen = if content.is_empty() { words } else { content };
    chosen.into_iter().take(SHORT_TITLE_WORDS).collect()
}

/// The base name (no extension) for an identity, capped at `max_len`
/// characters by dropping title words, then by truncation.
pub fn canonical_base(id: &Identity, max_len: usize) -> String {
    let author = author_part(id.key.surname.as_deref());
    let year = id
        .key
        .year
        .map_or_else(|| "nd".to_string(), |y| y.to_string());
    let mut words = id.key.title.as_deref().map(title_words).unwrap_or_default();
    if words.is_empty() {
        words.push(id.sha256[..id.sha256.len().min(12)].to_string());
    }
    let max_len = max_len.max(8);
    loop {
        let base = format!("{author}_{year}_{}", words.join("-"));
        if base.len() <= max_len {
            return base;
        }
        if words.len() > 1 {
            words.pop();
        } else {
            return base.chars().take(max_len).collect();
        }
    }
}

/// What applying the plan does with one input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Copy the input into the output directory under the new name.
    Copy,
    /// The input is already inside the output directory: rename it there.
    Rename,
    /// Nothing to do (no file on disk, or already named).
    Skip,
}

/// One input's proposed name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub index: usize,
    pub group: usize,
    pub role: Role,
    pub from: Option<String>,
    /// Proposed file name (no directory).
    pub to: String,
    pub op: Op,
    pub reason: Option<String>,
    pub sha256: String,
}

/// The names proposed for a scan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenamePlan {
    /// The output directory, when one was given.
    pub into: Option<String>,
    pub max_name_len: usize,
    pub entries: Vec<PlanEntry>,
}

/// Propose a name for every input. Members of one group get the same base
/// name: the canonical member plain, the others with `_dup2`, `_dup3`, ...;
/// a different work that lands on a taken base gets `_2`, `_3`, ...
/// Names already present in `into` count as taken. Case-insensitive.
pub fn plan(ids: &[Identity], groups: &[Group], into: Option<&Path>, max_len: usize) -> RenamePlan {
    let mut taken: HashMap<String, Option<usize>> = HashMap::new();
    if let Some(dir) = into
        && let Ok(entries) = fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            taken.insert(entry.file_name().to_string_lossy().to_lowercase(), None);
        }
    }
    let into_canonical = into.and_then(|d| d.canonicalize().ok());
    let mut ordered: Vec<(usize, Role, usize)> = Vec::new();
    for group in groups {
        for member in &group.members {
            ordered.push((group.id, member.role, member.index));
        }
    }
    ordered.sort_by(|x, y| {
        x.0.cmp(&y.0)
            .then(role_rank(x.1).cmp(&role_rank(y.1)))
            .then(ids[x.2].path.cmp(&ids[y.2].path))
            .then(x.2.cmp(&y.2))
    });
    let mut entries = Vec::new();
    for (group_id, role, index) in ordered {
        let id = &ids[index];
        let base = canonical_base(id, max_len);
        let name = allocate(&base, group_id, &mut taken);
        let (op, reason) = match &id.path {
            None => (Op::Skip, Some("no PDF file on disk".to_string())),
            Some(path) => {
                let path = Path::new(path);
                let inside = match (&into_canonical, path.canonicalize()) {
                    (Some(dir), Ok(full)) => full.parent() == Some(dir.as_path()),
                    _ => false,
                };
                if inside
                    && path
                        .file_name()
                        .is_some_and(|f| f.to_string_lossy() == name)
                {
                    (Op::Skip, Some("already has this name".to_string()))
                } else if inside {
                    (Op::Rename, None)
                } else {
                    (Op::Copy, None)
                }
            }
        };
        entries.push(PlanEntry {
            index,
            group: group_id,
            role,
            from: id.path.clone(),
            to: name,
            op,
            reason,
            sha256: id.sha256.clone(),
        });
    }
    entries.sort_by_key(|e| e.index);
    RenamePlan {
        into: into.map(|p| p.to_string_lossy().into_owned()),
        max_name_len: max_len,
        entries,
    }
}

fn role_rank(role: Role) -> u8 {
    match role {
        Role::Canonical => 0,
        Role::Duplicate => 1,
        Role::Version => 2,
    }
}

fn allocate(base: &str, group_id: usize, taken: &mut HashMap<String, Option<usize>>) -> String {
    let plain = format!("{base}.pdf");
    let holder = taken.get(&plain.to_lowercase()).copied();
    match holder {
        None => {
            taken.insert(plain.to_lowercase(), Some(group_id));
            plain
        }
        Some(owner) => {
            let infix = if owner == Some(group_id) { "_dup" } else { "_" };
            let mut n = 2;
            loop {
                let candidate = format!("{base}{infix}{n}.pdf");
                if let Entry::Vacant(slot) = taken.entry(candidate.to_lowercase()) {
                    slot.insert(Some(group_id));
                    return candidate;
                }
                n += 1;
            }
        }
    }
}

/// Why applying or undoing failed as a whole (per-file problems are
/// recorded in the manifest instead).
#[derive(Debug, Error)]
pub enum RenameError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("manifest: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the plan has no output directory; pass --rename-into")]
    NoDirectory,
    #[error("{0} is not a directory")]
    NotADirectory(String),
}

/// One applied operation (or one that was skipped, with its reason).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub from: String,
    pub to: String,
    pub op: Op,
    pub sha256: String,
    pub skipped: Option<String>,
}

/// Record of one `--apply`, enough to reverse it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub tool: String,
    pub created_unix: u64,
    pub dir: String,
    pub entries: Vec<ManifestEntry>,
}

/// Carry out the plan inside `into`, then write the manifest there as
/// `tpe-identify-manifest.json` (or `-2`, `-3`, ... when one exists).
/// Returns the manifest and its path.
pub fn apply(plan: &RenamePlan, into: &Path) -> Result<(Manifest, PathBuf), RenameError> {
    fs::create_dir_all(into)?;
    let dir = into.canonicalize()?;
    if !dir.is_dir() {
        return Err(RenameError::NotADirectory(dir.display().to_string()));
    }
    let mut entries = Vec::new();
    let mut written: HashSet<String> = HashSet::new();
    for entry in &plan.entries {
        let Some(from) = &entry.from else {
            continue;
        };
        if entry.op == Op::Skip {
            continue;
        }
        let target = dir.join(&entry.to);
        let skipped = apply_one(entry, Path::new(from), &target, &mut written);
        entries.push(ManifestEntry {
            from: from.clone(),
            to: target.to_string_lossy().into_owned(),
            op: entry.op,
            sha256: entry.sha256.clone(),
            skipped,
        });
    }
    let manifest = Manifest {
        version: 1,
        tool: "tpe-identify".to_string(),
        created_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        dir: dir.to_string_lossy().into_owned(),
        entries,
    };
    let path = free_manifest_path(&dir);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok((manifest, path))
}

fn free_manifest_path(dir: &Path) -> PathBuf {
    let first = dir.join("tpe-identify-manifest.json");
    if !first.exists() {
        return first;
    }
    let mut n = 2;
    loop {
        let candidate = dir.join(format!("tpe-identify-manifest-{n}.json"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Apply one entry; `Some(reason)` when it was skipped.
fn apply_one(
    entry: &PlanEntry,
    from: &Path,
    target: &Path,
    written: &mut HashSet<String>,
) -> Option<String> {
    let bytes = match fs::read(from) {
        Ok(b) => b,
        Err(e) => return Some(format!("cannot read source: {e}")),
    };
    if sha256_hex(&bytes) != entry.sha256 {
        return Some("source changed since the scan".to_string());
    }
    if target.exists() || written.contains(&target.to_string_lossy().to_lowercase()) {
        return Some("target exists".to_string());
    }
    match entry.op {
        Op::Rename => {
            if let Err(e) = fs::rename(from, target) {
                return Some(format!("rename failed: {e}"));
            }
        }
        Op::Copy => {
            let mut file = match OpenOptions::new().write(true).create_new(true).open(target) {
                Ok(f) => f,
                Err(e) => return Some(format!("cannot create target: {e}")),
            };
            if let Err(e) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
                return Some(format!("write failed: {e}"));
            }
            drop(file);
            match fs::read(target) {
                Ok(back) if sha256_hex(&back) == entry.sha256 => {}
                Ok(_) => return Some("copy verification failed: hash differs".to_string()),
                Err(e) => return Some(format!("copy verification failed: {e}")),
            }
        }
        Op::Skip => return Some("skipped".to_string()),
    }
    written.insert(target.to_string_lossy().to_lowercase());
    None
}

/// What undoing one manifest entry would do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UndoEntry {
    pub from: String,
    pub to: String,
    pub op: Op,
    /// `restore` (rename back), `remove` (delete the verified copy), or `skip`.
    pub action: String,
    pub reason: Option<String>,
}

/// Read a manifest file.
pub fn read_manifest(path: &Path) -> Result<Manifest, RenameError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

/// Reverse a manifest, newest entry first. With `dry_run`, nothing is
/// touched and the entries say what would happen. A copy is removed only
/// when its hash still matches the manifest and the original still exists
/// with the same hash; a rename is reversed only when the renamed file is
/// unchanged and the original name is free.
pub fn undo(manifest: &Manifest, dry_run: bool) -> Vec<UndoEntry> {
    let mut out = Vec::new();
    for entry in manifest.entries.iter().rev() {
        let mut report = UndoEntry {
            from: entry.from.clone(),
            to: entry.to.clone(),
            op: entry.op,
            action: "skip".to_string(),
            reason: None,
        };
        if let Some(reason) = &entry.skipped {
            report.reason = Some(format!("never applied: {reason}"));
            out.push(report);
            continue;
        }
        let to = Path::new(&entry.to);
        let from = Path::new(&entry.from);
        let to_hash = fs::read(to).ok().map(|b| sha256_hex(&b));
        if to_hash.as_deref() != Some(entry.sha256.as_str()) {
            report.reason = Some("renamed file is missing or changed".to_string());
            out.push(report);
            continue;
        }
        match entry.op {
            Op::Rename => {
                if from.exists() {
                    report.reason = Some("original name is taken".to_string());
                } else {
                    report.action = "restore".to_string();
                    if !dry_run && let Err(e) = fs::rename(to, from) {
                        report.action = "skip".to_string();
                        report.reason = Some(format!("rename failed: {e}"));
                    }
                }
            }
            Op::Copy => {
                let from_hash = fs::read(from).ok().map(|b| sha256_hex(&b));
                if from_hash.as_deref() == Some(entry.sha256.as_str()) {
                    report.action = "remove".to_string();
                    if !dry_run && let Err(e) = fs::remove_file(to) {
                        report.action = "skip".to_string();
                        report.reason = Some(format!("remove failed: {e}"));
                    }
                } else {
                    report.reason = Some("original is missing or changed; copy kept".to_string());
                }
            }
            Op::Skip => report.reason = Some("nothing was done".to_string()),
        }
        out.push(report);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biblio::BiblioKey;

    fn identity(
        index: usize,
        path: Option<&str>,
        title: Option<&str>,
        surname: Option<&str>,
        year: Option<u16>,
        sha: &str,
    ) -> Identity {
        Identity {
            index,
            path: path.map(str::to_string),
            size: 1,
            sha256: sha.to_string(),
            text_source: "test".into(),
            pages: 1,
            words: 0,
            text_sha256: None,
            simhash: "0".repeat(16),
            minhash: None,
            key: BiblioKey {
                title: title.map(str::to_string),
                surname: surname.map(str::to_string),
                year,
                ..BiblioKey::default()
            },
            variant: crate::biblio::Variant::Unknown,
            warnings: vec![],
        }
    }

    #[test]
    fn names_are_ascii_capped_and_fall_back() {
        let id = identity(
            0,
            None,
            Some("Über die Elektrodynamik bewegter Körper: eine Studie"),
            Some("einstein"),
            Some(1905),
            "abcdef0123456789ff",
        );
        assert_eq!(
            canonical_base(&id, DEFAULT_MAX_NAME_LEN),
            "Einstein_1905_Uber-Elektrodynamik-Bewegter-Korper-Eine-Studie"
        );
        assert_eq!(canonical_base(&id, 30), "Einstein_1905_Uber");
        assert_eq!(canonical_base(&id, 10), "Einstein_1");
        let bare = identity(1, None, None, None, None, "abcdef0123456789ff");
        assert_eq!(
            canonical_base(&bare, DEFAULT_MAX_NAME_LEN),
            "Unknown_nd_abcdef012345"
        );
        let stop = identity(
            2,
            None,
            Some("On the Of"),
            Some("Ó'Néill-Smith"),
            Some(2000),
            "ab",
        );
        assert_eq!(
            canonical_base(&stop, DEFAULT_MAX_NAME_LEN),
            "Oneillsmith_2000_On-The-Of"
        );
    }

    #[test]
    fn title_words_drop_stopwords_and_cap_count() {
        assert_eq!(
            title_words("A Study of the Effects of Caffeine on Memory in Mice and Men"),
            ["Study", "Effects", "Caffeine", "Memory", "Mice", "Men"]
        );
        assert_eq!(author_part(Some("van der Berg")), "Vanderberg");
        assert_eq!(author_part(None), "Unknown");
        assert_eq!(author_part(Some("李")), "Unknown");
    }

    #[test]
    fn plan_assigns_dup_suffixes_within_a_group_and_numbers_across_groups() {
        let ids = vec![
            identity(
                0,
                Some("/in/a.pdf"),
                Some("Same Title"),
                Some("smith"),
                Some(2020),
                "a1",
            ),
            identity(
                1,
                Some("/in/b.pdf"),
                Some("Same Title"),
                Some("smith"),
                Some(2020),
                "a1",
            ),
            identity(
                2,
                Some("/in/c.pdf"),
                Some("Same Title"),
                Some("smith"),
                Some(2020),
                "c3",
            ),
            identity(3, None, Some("Same Title"), Some("smith"), Some(2020), "d4"),
        ];
        let member = |index: usize, role: Role| crate::group::Member {
            index,
            path: ids[index].path.clone(),
            sha256: ids[index].sha256.clone(),
            variant: crate::biblio::Variant::Unknown,
            role,
            link_score: 1.0,
        };
        let groups = vec![
            Group {
                id: 1,
                kind: crate::group::GroupKind::ExactDuplicates,
                confidence: 1.0,
                canonical: 1,
                members: vec![member(1, Role::Canonical), member(0, Role::Duplicate)],
                evidence: vec![],
            },
            Group {
                id: 2,
                kind: crate::group::GroupKind::Single,
                confidence: 1.0,
                canonical: 2,
                members: vec![member(2, Role::Canonical)],
                evidence: vec![],
            },
            Group {
                id: 3,
                kind: crate::group::GroupKind::Single,
                confidence: 1.0,
                canonical: 3,
                members: vec![member(3, Role::Canonical)],
                evidence: vec![],
            },
        ];
        let plan = plan(&ids, &groups, None, DEFAULT_MAX_NAME_LEN);
        let names: Vec<(usize, &str, Op)> = plan
            .entries
            .iter()
            .map(|e| (e.index, e.to.as_str(), e.op))
            .collect();
        assert_eq!(
            names,
            vec![
                (0, "Smith_2020_Same-Title_dup2.pdf", Op::Copy),
                (1, "Smith_2020_Same-Title.pdf", Op::Copy),
                (2, "Smith_2020_Same-Title_2.pdf", Op::Copy),
                (3, "Smith_2020_Same-Title_3.pdf", Op::Skip),
            ]
        );
        assert_eq!(
            plan.entries[3].reason.as_deref(),
            Some("no PDF file on disk")
        );
    }
}
