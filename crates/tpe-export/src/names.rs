//! Splitting a printed author name into Zotero's `lastName` / `firstName`.
//!
//! Reference lists print names in several shapes; the rules here cover the
//! common ones and never guess beyond them:
//! - `Smith, John A.` and `Smith, J.` (comma): surname before the comma.
//! - `Smith JA` (Vancouver initials after the surname): the trailing
//!   all-capitals token is the given-name part.
//! - `John A. Smith`, `A. B. van der Berg`: the last token is the surname,
//!   together with any lower-case particles (`van`, `de`, ...) before it.
//! - A single token is a surname only (Zotero's single-field creator).

/// Name particles that belong to the surname when they precede it.
const PARTICLES: &[&str] = &[
    "af", "al", "av", "bin", "da", "dal", "das", "de", "degli", "dei", "del", "della", "der",
    "des", "di", "do", "dos", "du", "el", "ibn", "la", "las", "le", "lo", "los", "op", "ten",
    "ter", "van", "vom", "von", "y", "zu", "zur",
];

/// Tokens that stand for omitted authors rather than naming one.
const FILLERS: &[&str] = &[
    "et al",
    "et. al",
    "et al.",
    "others",
    "and others",
    "…",
    "...",
];

/// A name split into Zotero's two creator fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitName {
    /// `lastName` in Zotero (the whole name for single-field creators).
    pub last: String,
    /// `firstName` in Zotero; `None` for a single-field creator.
    pub first: Option<String>,
}

/// Splits one printed name. Returns `None` for an empty string or an
/// "et al." style filler.
pub fn split_name(raw: &str) -> Option<SplitName> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name = collapsed.trim_matches(|c: char| c == ',' || c == ';' || c.is_whitespace());
    for prefix in ["and ", "And ", "& "] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.trim_start();
        }
    }
    if name.is_empty() || is_filler(name) {
        return None;
    }
    if let Some((last, first)) = name.split_once(',') {
        let last = last.trim();
        let first = first.trim();
        if last.is_empty() {
            return split_unpunctuated(first);
        }
        return Some(SplitName {
            last: last.to_string(),
            first: non_empty(first),
        });
    }
    split_unpunctuated(name)
}

fn split_unpunctuated(name: &str) -> Option<SplitName> {
    let tokens: Vec<&str> = name.split(' ').filter(|t| !t.is_empty()).collect();
    let count = tokens.len();
    match count {
        0 => return None,
        1 => {
            return Some(SplitName {
                last: tokens[0].to_string(),
                first: None,
            });
        }
        _ => {}
    }
    let last_token = tokens[count - 1];
    if is_initials(last_token) && !is_initials(tokens[0]) {
        return Some(SplitName {
            last: tokens[..count - 1].join(" "),
            first: Some(last_token.to_string()),
        });
    }
    let mut start = count - 1;
    while start > 0 && is_particle(tokens[start - 1]) {
        start -= 1;
    }
    Some(SplitName {
        last: tokens[start..].join(" "),
        first: non_empty(&tokens[..start].join(" ")),
    })
}

fn is_particle(token: &str) -> bool {
    let lower = token.to_lowercase();
    PARTICLES.contains(&lower.as_str())
}

/// `J`, `JA`, `J.A.`, `J.-P.`: at most three capital letters with periods
/// or hyphens between them.
fn is_initials(token: &str) -> bool {
    let letters = token.chars().filter(|c| c.is_alphabetic()).count();
    (1..=3).contains(&letters)
        && token
            .chars()
            .all(|c| c.is_uppercase() || c == '.' || c == '-')
}

fn is_filler(name: &str) -> bool {
    let lower = name.to_lowercase();
    let lower = lower.trim_end_matches('.');
    FILLERS.contains(&lower)
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(raw: &str) -> (String, Option<String>) {
        let name = split_name(raw).expect("a name");
        (name.last, name.first)
    }

    #[test]
    fn comma_form_keeps_everything_after_the_comma_as_given_names() {
        assert_eq!(
            split("Smith, John A."),
            ("Smith".into(), Some("John A.".into()))
        );
        assert_eq!(split("Smith, J."), ("Smith".into(), Some("J.".into())));
        assert_eq!(split("Smith,"), ("Smith".into(), None));
    }

    #[test]
    fn vancouver_initials_follow_the_surname() {
        assert_eq!(split("Smith JA"), ("Smith".into(), Some("JA".into())));
        assert_eq!(split("Li X-M"), ("Li".into(), Some("X-M".into())));
        assert_eq!(split("Van Dyke D"), ("Van Dyke".into(), Some("D".into())));
    }

    #[test]
    fn natural_order_takes_the_last_token_and_particles() {
        assert_eq!(
            split("John A. Smith"),
            ("Smith".into(), Some("John A.".into()))
        );
        assert_eq!(split("J. Smith"), ("Smith".into(), Some("J.".into())));
        assert_eq!(
            split("A. B. van der Berg"),
            ("van der Berg".into(), Some("A. B.".into()))
        );
        assert_eq!(
            split("Ludwig van Beethoven"),
            ("van Beethoven".into(), Some("Ludwig".into()))
        );
        assert_eq!(split("de Gaulle"), ("de Gaulle".into(), None));
        assert_eq!(split("JOHN SMITH"), ("SMITH".into(), Some("JOHN".into())));
    }

    #[test]
    fn single_tokens_and_fillers() {
        assert_eq!(split("Plato"), ("Plato".into(), None));
        assert_eq!(
            split("  and   J.  Smith "),
            ("Smith".into(), Some("J.".into()))
        );
        assert_eq!(split_name("et al."), None);
        assert_eq!(split_name("others"), None);
        assert_eq!(split_name("   "), None);
    }
}
