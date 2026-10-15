//! Text folding and field normalisation shared by the scorer.
//!
//! Everything compared is first *folded*: Unicode NFKC (ligatures, full-width
//! forms), diacritics removed through NFKD, a few letters that NFKD leaves
//! alone mapped by hand (`ł`, `ø`, `ß`, `æ`, `œ`, `đ`, `þ`, `ı`), lower-cased,
//! and every non-alphanumeric character turned into a space. Registries often
//! store ASCII forms of names, and printed entries lose accents in extraction,
//! so neither side is trusted for diacritics.

use std::collections::BTreeSet;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Letters and digits of `text`, lower-cased, diacritics removed, words
/// separated by single spaces.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let compat: String = text.nfkc().collect();
    for c in compat.nfkd() {
        if is_combining_mark(c) {
            continue;
        }
        let mapped: Option<&str> = match c {
            'ł' | 'Ł' => Some("l"),
            'đ' | 'Đ' | 'ð' | 'Ð' => Some("d"),
            'ø' | 'Ø' => Some("o"),
            'ı' => Some("i"),
            'ß' => Some("ss"),
            'æ' | 'Æ' => Some("ae"),
            'œ' | 'Œ' => Some("oe"),
            'þ' | 'Þ' => Some("th"),
            _ => None,
        };
        if let Some(m) = mapped {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push_str(m);
            continue;
        }
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(c.to_lowercase());
        } else {
            pending_space = true;
        }
    }
    out
}

/// Folded words of `text`, in order, duplicates kept.
pub fn words(text: &str) -> Vec<String> {
    fold(text).split_whitespace().map(str::to_string).collect()
}

/// Folded words of `text` as a set.
pub fn word_set(text: &str) -> BTreeSet<String> {
    fold(text).split_whitespace().map(str::to_string).collect()
}

/// Function words ignored when comparing titles and container names.
const STOPWORDS: &[&str] = &[
    "a", "an", "and", "the", "of", "for", "in", "on", "to", "with", "by", "at", "from", "or", "de",
    "der", "die", "das", "des", "und", "la", "le", "les", "el", "los", "las", "du", "et", "di",
    "del", "della", "degli", "delle", "il", "lo", "gli", "van", "von", "en",
];

/// Is `word` (folded) a function word?
pub fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Folded content words of `text` (function words removed) as a set.
pub fn content_words(text: &str) -> BTreeSet<String> {
    word_set(text)
        .into_iter()
        .filter(|w| !is_stopword(w))
        .collect()
}

/// Jaccard overlap of two word sets, `0..=1`; `0` when both are empty.
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    ratio(a.intersection(b).count(), union)
}

/// Share of the smaller word set that the larger contains, `0..=1`.
pub fn containment(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    let smaller = a.len().min(b.len());
    if smaller == 0 {
        return 0.0;
    }
    ratio(a.intersection(b).count(), smaller)
}

/// `numerator / denominator` as `f32`; counts are small, precision loss is irrelevant.
#[allow(clippy::cast_precision_loss)]
pub fn ratio(numerator: usize, denominator: usize) -> f32 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f32 / denominator as f32
}

/// Normalised Levenshtein similarity of two folded strings, `0..=1`.
#[allow(clippy::cast_possible_truncation)]
pub fn string_similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    strsim::normalized_levenshtein(a, b) as f32
}

/// Is `token` an initial (`J`, `J.`, `JA`, `J.-P.`) rather than a name?
fn is_initial(token: &str) -> bool {
    let letters: Vec<char> = token.chars().filter(char::is_ascii_alphabetic).collect();
    if letters.is_empty() {
        return true;
    }
    let has_dot = token.contains('.');
    let all_upper = letters.iter().all(char::is_ascii_uppercase);
    (has_dot && letters.len() <= 3) || (all_upper && letters.len() <= 3) || letters.len() == 1
}

/// The family name of one printed or record author: `Smith, J. A.`,
/// `J. A. Smith`, `Smith JA`, `Ada Lovelace`, `van der Berg, A.` and plain
/// organisation names all yield the family part (`van der Berg` keeps its
/// particles). `et al` and conjunctions are dropped first.
pub fn family_name(name: &str) -> String {
    let cleaned = name
        .replace("et al.", " ")
        .replace("et al", " ")
        .replace('&', " ")
        .replace(" and ", " ");
    let cleaned = cleaned.trim().trim_end_matches([',', ';', '.']).trim();
    if cleaned.is_empty() {
        return String::new();
    }
    if let Some((family, _given)) = cleaned.split_once(',') {
        let family = family.trim();
        if !family.is_empty() && !is_initial(family) {
            return family.to_string();
        }
    }
    let tokens: Vec<&str> = cleaned.split_whitespace().collect();
    let names: Vec<&str> = tokens.iter().copied().filter(|t| !is_initial(t)).collect();
    match names.as_slice() {
        [] => cleaned.to_string(),
        [single] => (*single).to_string(),
        _ => {
            // Initials first (`J. A. Smith`): the family name is the last
            // name word, plus the lower-case particles before it. Initials
            // last (`Smith JA`): the family name is the first name word.
            let initials_first = tokens.first().is_some_and(|t| is_initial(t));
            if initials_first {
                let mut start = names.len() - 1;
                while start > 0
                    && names[start - 1]
                        .chars()
                        .next()
                        .is_some_and(char::is_lowercase)
                {
                    start -= 1;
                }
                names[start..].join(" ")
            } else {
                let has_initials = tokens.iter().any(|t| is_initial(t));
                if has_initials {
                    names[0].to_string()
                } else {
                    // `Ada Lovelace` / `Jean van der Berg`: last word plus particles.
                    let mut start = names.len() - 1;
                    while start > 0
                        && names[start - 1]
                            .chars()
                            .next()
                            .is_some_and(char::is_lowercase)
                    {
                        start -= 1;
                    }
                    names[start..].join(" ")
                }
            }
        }
    }
}

/// Folded family name, with particles dropped, to compare surnames by.
pub fn family_key(name: &str) -> String {
    let folded = fold(&family_name(name));
    let parts: Vec<&str> = folded.split_whitespace().collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| !is_stopword(p) || parts.len() == 1)
        .collect();
    if kept.is_empty() {
        folded
    } else {
        kept.join(" ")
    }
}

/// German transliteration folded to the plain vowel (`mueller` to `muller`),
/// so a printed umlaut and a registry `ue` agree.
fn transliterate_umlauts(key: &str) -> String {
    key.replace("ue", "u").replace("oe", "o").replace("ae", "a")
}

/// Similarity of two surnames, `0..=1`: `1` when the folded keys agree (also
/// after umlaut transliteration), when
/// one is a hyphen part or last word of the other (`garcia lopez` /
/// `garcia`), otherwise their string similarity.
pub fn surname_similarity(a: &str, b: &str) -> f32 {
    let (ka, kb) = (family_key(a), family_key(b));
    if ka.is_empty() || kb.is_empty() {
        return 0.0;
    }
    if ka == kb || transliterate_umlauts(&ka) == transliterate_umlauts(&kb) {
        return 1.0;
    }
    let wa: Vec<&str> = ka.split_whitespace().collect();
    let wb: Vec<&str> = kb.split_whitespace().collect();
    if wa.iter().any(|w| w.len() >= 3 && wb.contains(w)) {
        return 1.0;
    }
    let squashed = |s: &str| s.replace(' ', "");
    string_similarity(&squashed(&ka), &squashed(&kb))
}

/// The first plausible publication year (1800–2099) printed in `text`.
pub fn year_in(text: &str) -> Option<u16> {
    let mut digits = String::new();
    let mut found: Option<u16> = None;
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if digits.len() == 4
            && let Ok(year) = digits.parse::<u16>()
            && (1800..=2099).contains(&year)
        {
            found = Some(year);
            break;
        }
        digits.clear();
    }
    found
}

/// Volume as printed, reduced to its alphanumerics (`Vol. 12` → `12`).
pub fn volume_key(text: &str) -> String {
    let folded = fold(text);
    let mut parts: Vec<&str> = folded.split_whitespace().collect();
    if let Some(first) = parts.first()
        && matches!(
            *first,
            "vol" | "volume" | "v" | "bd" | "band" | "tome" | "t"
        )
    {
        parts.remove(0);
    }
    parts.concat()
}

/// A page range normalised to `first-last` (or `first`): dashes unified,
/// `pp.` removed, abbreviated last pages expanded (`436-44` → `436-444`),
/// article numbers (`e0123`) lower-cased.
pub fn page_range(text: &str) -> Option<(String, Option<String>)> {
    let unified: String = text
        .chars()
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            _ => c,
        })
        .collect();
    let lower = unified.to_lowercase();
    let stripped = lower
        .trim()
        .trim_start_matches("pages")
        .trim_start_matches("pp.")
        .trim_start_matches("pp")
        .trim_start_matches("p.")
        .trim_start_matches("p ")
        .trim();
    let stripped: String = stripped.chars().filter(|c| !c.is_whitespace()).collect();
    let stripped = stripped.trim_end_matches(['.', ',', ';']);
    let mut it = stripped.split('-').filter(|p| !p.is_empty());
    let first = it.next()?.to_string();
    if first.chars().all(|c| !c.is_alphanumeric()) {
        return None;
    }
    let last = it.next().map(|last| {
        let both_digits =
            first.chars().all(|c| c.is_ascii_digit()) && last.chars().all(|c| c.is_ascii_digit());
        if both_digits && last.len() < first.len() {
            format!("{}{}", &first[..first.len() - last.len()], last)
        } else {
            last.to_string()
        }
    });
    Some((first, last))
}

/// Does `short` abbreviate the word `full`: a prefix (`Proc` / `Proceedings`)
/// or, from two letters, the same first letter and every letter of `short`
/// in order inside `full` (`Natl` / `National`, `Sci` / `Sciences`).
fn abbreviates_token(short: &str, full: &str) -> bool {
    if full.starts_with(short) {
        return true;
    }
    let mut short_chars = short.chars();
    let (Some(first), Some(full_first)) = (short_chars.next(), full.chars().next()) else {
        return false;
    };
    if short.chars().count() < 2 || first != full_first {
        return false;
    }
    let mut rest = full.chars().skip(1);
    short_chars.all(|c| rest.any(|f| f == c))
}

/// Does the abbreviated container name `short` abbreviate `full`, token by
/// token (`Proc Natl Acad Sci` / `Proceedings of the National Academy of
/// Sciences`)? Function words of `full` are skipped; every token of `short`
/// must be a prefix of the next content token of `full`, in order.
fn abbreviates(short: &[String], full: &[String]) -> bool {
    if short.is_empty() || short.len() > full.len() {
        return false;
    }
    // Function words and single letters (`U.S.A.`, series letters) carry nothing.
    let keep = |w: &&String| !is_stopword(w) && w.chars().count() > 1;
    let full: Vec<&String> = full.iter().filter(keep).collect();
    let short: Vec<&String> = short.iter().filter(keep).collect();
    if short.is_empty() || short.len() > full.len() {
        return false;
    }
    let mut cursor = 0;
    for token in short {
        let mut matched = false;
        while cursor < full.len() {
            let candidate = full[cursor];
            cursor += 1;
            if abbreviates_token(token, candidate) {
                matched = true;
                break;
            }
        }
        if !matched {
            return false;
        }
    }
    true
}

/// Similarity of two container (journal, proceedings) names, `0..=1`:
/// `1` for equal folded names or a token-wise abbreviation in either
/// direction, otherwise the best of word Jaccard and string similarity.
pub fn container_similarity(a: &str, b: &str) -> f32 {
    let (fa, fb) = (fold(a), fold(b));
    if fa.is_empty() || fb.is_empty() {
        return 0.0;
    }
    if fa == fb {
        return 1.0;
    }
    let (wa, wb) = (words(a), words(b));
    if abbreviates(&wa, &wb) || abbreviates(&wb, &wa) {
        return 1.0;
    }
    let (sa, sb) = (content_words(a), content_words(b));
    jaccard(&sa, &sb).max(string_similarity(&fa, &fb))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn folding_removes_diacritics_ligatures_and_punctuation() {
        assert_eq!(
            fold("Obtułowicz, Ł.—“Ødegård” ﬁne"),
            "obtulowicz l odegard fine"
        );
        assert_eq!(fold("  Müller & Straße "), "muller strasse");
        assert_eq!(fold("Ｆｕｌｌｗｉｄｔｈ"), "fullwidth");
    }

    #[test]
    fn family_names_from_every_common_layout() {
        assert_eq!(family_name("Smith, J. A."), "Smith");
        assert_eq!(family_name("J. A. Smith"), "Smith");
        assert_eq!(family_name("Smith JA"), "Smith");
        assert_eq!(family_name("Ada Lovelace"), "Lovelace");
        assert_eq!(family_name("van der Berg, A."), "van der Berg");
        assert_eq!(family_name("A. van der Berg"), "van der Berg");
        assert_eq!(family_name("World Health Organization"), "Organization");
        assert_eq!(family_name("García-López, M. et al."), "García-López");
        assert_eq!(family_name("Vaswani"), "Vaswani");
        assert_eq!(family_name(""), "");
    }

    #[test]
    fn surnames_match_across_particles_hyphens_and_accents() {
        assert_eq!(surname_similarity("van der Berg, A.", "Berg"), 1.0);
        assert_eq!(surname_similarity("García-López, M.", "Garcia"), 1.0);
        assert_eq!(surname_similarity("Müller, K.", "Mueller"), 1.0);
        assert_eq!(surname_similarity("Obtułowicz, Ł.", "Obtulowicz"), 1.0);
        assert!(surname_similarity("Smith, J.", "Smyth") > 0.7);
        assert!(surname_similarity("Smith, J.", "Jones") < 0.5);
        assert_eq!(surname_similarity("", "Jones"), 0.0);
    }

    #[test]
    fn years_pages_and_volumes_normalise() {
        assert_eq!(year_in("Nature 521, 436–444 (2015)"), Some(2015));
        assert_eq!(year_in("arXiv:1706.03762, 2017"), Some(2017));
        assert_eq!(year_in("no year 12345"), None);
        assert_eq!(volume_key("Vol. 12"), "12");
        assert_eq!(volume_key("521"), "521");
        assert_eq!(
            page_range("pp. 436–44"),
            Some(("436".to_string(), Some("444".to_string())))
        );
        assert_eq!(
            page_range("436-444."),
            Some(("436".to_string(), Some("444".to_string())))
        );
        assert_eq!(page_range("e0123456"), Some(("e0123456".to_string(), None)));
        assert_eq!(page_range("–"), None);
    }

    #[test]
    fn container_abbreviations_and_variants() {
        assert_eq!(
            container_similarity(
                "Proc. Natl. Acad. Sci. U.S.A.",
                "Proceedings of the National Academy of Sciences"
            ),
            1.0
        );
        assert_eq!(container_similarity("Nature", "Nature"), 1.0);
        assert_eq!(
            container_similarity("J Mach Learn Res", "Journal of Machine Learning Research"),
            1.0
        );
        assert!(container_similarity("Nature", "Science") < 0.5);
        assert!(
            container_similarity(
                "Advances in Neural Information Processing Systems 30",
                "Advances in Neural Information Processing Systems"
            ) > 0.8
        );
    }

    #[test]
    fn set_measures() {
        let a = word_set("deep learning for images");
        let b = word_set("deep learning for images and text");
        assert!((jaccard(&a, &b) - 4.0 / 6.0).abs() < 1e-6);
        assert!((containment(&a, &b) - 1.0).abs() < 1e-6);
        assert_eq!(jaccard(&BTreeSet::new(), &BTreeSet::new()), 0.0);
    }
}
