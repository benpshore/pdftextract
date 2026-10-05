//! Text normalisation and fingerprints for near-duplicate detection.
//!
//! The pipeline is: raw page text -> [`normalize`] (NFKC, lower-case,
//! de-hyphenated, letter-bearing tokens only) -> word [`shingles`] of
//! [`SHINGLE_WORDS`] tokens hashed with FNV-1a -> a [`MinHash`] signature of
//! [`MINHASH_PERMUTATIONS`] minima whose agreement estimates the Jaccard
//! similarity of the shingle sets. A 64-bit [`simhash`] over unigrams and
//! bigrams is kept as compact secondary evidence.
//!
//! Every hash here is defined in this file (no `DefaultHasher`), so
//! fingerprints are reproducible across Rust versions and platforms.

use std::collections::HashSet;

use unicode_normalization::UnicodeNormalization;

/// Words per shingle. Three keeps recall on versions whose copy-edits are
/// scattered through the text (every changed word breaks only three
/// shingles) while unrelated papers in one field still share well under
/// a tenth of their shingles.
pub const SHINGLE_WORDS: usize = 3;

/// Number of `MinHash` permutations. The standard error of the Jaccard
/// estimate is `sqrt(J (1 - J) / 128)`, about 0.03 around the thresholds.
pub const MINHASH_PERMUTATIONS: usize = 128;

/// Fewer tokens than this and a text is too short for shingle evidence:
/// its fingerprints are recorded but never used to link documents.
pub const MIN_TOKENS_FOR_TEXT_EVIDENCE: usize = 20;

/// Mersenne prime 2^61 - 1, the modulus of the universal hash family.
const MERSENNE_61: u64 = (1 << 61) - 1;

/// Fixed seed of the permutation parameters.
const PERMUTATION_SEED: u64 = 0x5eed_1d37_1f1e_0001;

/// 64-bit FNV-1a of `bytes`.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// `splitmix64` step, used to derive permutation parameters from the seed.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Normalise text for fingerprinting: NFKC (folds ligatures such as `ﬁ`),
/// lower-case, line-break hyphenation joined (`exam-\nple` -> `example`),
/// every non-alphanumeric character treated as a separator, and only
/// tokens that contain a letter and are at least two characters long are
/// kept (page numbers, line numbers and equation digits differ between
/// versions and would otherwise dominate the shingles). Tokens are joined
/// by single spaces.
pub fn normalize(raw: &str) -> String {
    let folded: String = raw.nfkc().collect();
    let dehyphenated = join_hyphenated_line_breaks(&folded);
    let mut out = String::with_capacity(dehyphenated.len());
    let mut token = String::new();
    for ch in dehyphenated.chars().chain(std::iter::once(' ')) {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                token.push(lower);
            }
        } else {
            flush_token(&mut token, &mut out);
        }
    }
    out
}

fn flush_token(token: &mut String, out: &mut String) {
    if token.chars().count() >= 2 && token.chars().any(char::is_alphabetic) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
    }
    token.clear();
}

/// Join `word-\nrest` into `wordrest` when both sides are letters.
fn join_hyphenated_line_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if (ch == '-' || ch == '\u{ad}')
            && i > 0
            && chars[i - 1].is_alphabetic()
            && chars.get(i + 1).is_some_and(|c| *c == '\n' || *c == '\r')
        {
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == '\n' || chars[j] == '\r' || chars[j] == ' ') {
                j += 1;
            }
            if chars.get(j).is_some_and(|c| c.is_alphabetic()) {
                i = j;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Tokens of a [`normalize`]d text.
pub fn tokens(normalized: &str) -> Vec<&str> {
    normalized.split(' ').filter(|t| !t.is_empty()).collect()
}

/// Hashes of the distinct word `k`-grams of `tokens`. A text shorter than
/// `k` tokens yields one shingle of all its tokens (none when empty).
pub fn shingles(tokens: &[&str], k: usize) -> HashSet<u64> {
    let k = k.max(1);
    let mut set = HashSet::new();
    if tokens.is_empty() {
        return set;
    }
    if tokens.len() <= k {
        set.insert(fnv1a(tokens.join(" ").as_bytes()));
        return set;
    }
    for window in tokens.windows(k) {
        set.insert(fnv1a(window.join(" ").as_bytes()));
    }
    set
}

/// A `MinHash` signature: the minimum of each of [`MINHASH_PERMUTATIONS`]
/// universal hashes over a shingle set. Two signatures agree in a fraction
/// of positions that estimates the Jaccard similarity of the sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinHash {
    minima: Vec<u64>,
    /// Number of shingles the signature was built from.
    pub shingle_count: usize,
}

/// The `(a, b)` parameters of every permutation, derived once from the seed.
fn permutation_parameters() -> Vec<(u64, u64)> {
    let mut state = PERMUTATION_SEED;
    (0..MINHASH_PERMUTATIONS)
        .map(|_| {
            // `a` must be non-zero modulo the prime for a proper permutation.
            let a = (splitmix64(&mut state) % (MERSENNE_61 - 1)) + 1;
            let b = splitmix64(&mut state) % MERSENNE_61;
            (a, b)
        })
        .collect()
}

/// `(a * x + b) mod (2^61 - 1)` without overflow.
fn universal_hash(a: u64, b: u64, x: u64) -> u64 {
    let product = u128::from(a) * u128::from(x & MERSENNE_61) + u128::from(b);
    let folded = (product & u128::from(MERSENNE_61)) + (product >> 61);
    let folded = (folded & u128::from(MERSENNE_61)) + (folded >> 61);
    // After two folds the value is below 2^62; one conditional subtraction
    // brings it under the prime.
    let value = u64::try_from(folded).unwrap_or(u64::MAX);
    if value >= MERSENNE_61 {
        value - MERSENNE_61
    } else {
        value
    }
}

impl MinHash {
    /// Signature of a shingle set. An empty set gives all-maximum minima
    /// and a `shingle_count` of zero; such a signature never matches.
    pub fn of_shingles(shingles: &HashSet<u64>) -> Self {
        let params = permutation_parameters();
        let mut minima = vec![u64::MAX; MINHASH_PERMUTATIONS];
        for shingle in shingles {
            for (slot, (a, b)) in minima.iter_mut().zip(&params) {
                let value = universal_hash(*a, *b, *shingle);
                if value < *slot {
                    *slot = value;
                }
            }
        }
        Self {
            minima,
            shingle_count: shingles.len(),
        }
    }

    /// Signature of already normalised text (see [`normalize`]).
    pub fn of_normalized(normalized: &str) -> Self {
        Self::of_shingles(&shingles(&tokens(normalized), SHINGLE_WORDS))
    }

    /// Estimated Jaccard similarity in `0..=1`; zero when either side has
    /// no shingles.
    pub fn similarity(&self, other: &Self) -> f64 {
        if self.shingle_count == 0 || other.shingle_count == 0 {
            return 0.0;
        }
        let agree = self
            .minima
            .iter()
            .zip(&other.minima)
            .filter(|(a, b)| a == b)
            .count();
        agree as f64 / self.minima.len() as f64
    }
}

/// 64-bit `SimHash` over the unigrams and bigrams of `tokens`: bits of a
/// token hash vote on each position. Hamming distance between two values
/// is small for near-identical texts (typically at most 3 bits for texts
/// that differ in a few words) and about 32 for unrelated texts.
pub fn simhash(tokens: &[&str]) -> u64 {
    let mut votes = [0i64; 64];
    let mut vote = |hash: u64| {
        for (bit, slot) in votes.iter_mut().enumerate() {
            if (hash >> bit) & 1 == 1 {
                *slot += 1;
            } else {
                *slot -= 1;
            }
        }
    };
    for token in tokens {
        vote(fnv1a(token.as_bytes()));
    }
    for pair in tokens.windows(2) {
        vote(fnv1a(format!("{} {}", pair[0], pair[1]).as_bytes()));
    }
    votes
        .iter()
        .enumerate()
        .filter(|(_, v)| **v > 0)
        .fold(0u64, |acc, (bit, _)| acc | (1 << bit))
}

/// Number of differing bits between two `SimHash` values.
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic pseudo-prose: `n` words drawn from a fixed vocabulary.
    pub(crate) fn prose(n: usize, seed: u64) -> String {
        const VOCAB: [&str; 40] = [
            "protein", "folding", "network", "gradient", "sample", "measure", "latent", "signal",
            "cortex", "neuron", "method", "result", "random", "matrix", "kernel", "bound",
            "theorem", "lemma", "proof", "data", "model", "train", "error", "noise", "layer",
            "policy", "reward", "agent", "graph", "node", "edge", "path", "cycle", "prime",
            "field", "group", "ring", "ideal", "module", "space",
        ];
        let mut state = seed;
        (0..n)
            .map(|_| {
                VOCAB[usize::try_from(splitmix64(&mut state) % VOCAB.len() as u64).unwrap_or(0)]
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn normalize_folds_case_ligatures_hyphenation_and_digits() {
        let raw = "The ﬁrst Exam-\nple: Page 12, α-helix β2 x";
        assert_eq!(normalize(raw), "the first example page helix β2");
    }

    #[test]
    fn normalize_keeps_soft_hyphen_joins_and_drops_single_letters() {
        assert_eq!(
            normalize("inter\u{ad}\nnational a b cd"),
            "international cd"
        );
        assert_eq!(normalize("well-known"), "well known");
        assert_eq!(normalize(""), "");
    }

    #[test]
    fn shingles_cover_short_texts() {
        assert!(shingles(&[], 3).is_empty());
        assert_eq!(shingles(&["one"], 3).len(), 1);
        assert_eq!(shingles(&["a", "b", "c", "d"], 3).len(), 2);
    }

    #[test]
    fn minhash_identical_texts_are_fully_similar() {
        let text = normalize(&prose(400, 1));
        let a = MinHash::of_normalized(&text);
        let b = MinHash::of_normalized(&text);
        assert!((a.similarity(&b) - 1.0).abs() < f64::EPSILON);
        assert_eq!(a, b);
    }

    #[test]
    fn minhash_tracks_jaccard_of_edited_texts() {
        let base: Vec<String> = prose(600, 7).split(' ').map(str::to_string).collect();
        // Change every 50th word: about 6% of shingles break.
        let mut edited = base.clone();
        for (i, word) in edited.iter_mut().enumerate() {
            if i % 50 == 25 {
                *word = "changed".to_string();
            }
        }
        let a = MinHash::of_normalized(&base.join(" "));
        let b = MinHash::of_normalized(&edited.join(" "));
        let sim = a.similarity(&b);
        assert!(sim > 0.85, "near-duplicate estimate too low: {sim}");

        // Change every 8th word: roughly a third of shingles survive.
        let mut heavy = base.clone();
        for (i, word) in heavy.iter_mut().enumerate() {
            if i % 8 == 3 {
                *word = "rewritten".to_string();
            }
        }
        let c = MinHash::of_normalized(&heavy.join(" "));
        let sim = a.similarity(&c);
        assert!(
            (0.2..0.6).contains(&sim),
            "revision estimate out of range: {sim}"
        );
    }

    #[test]
    fn minhash_distinct_texts_are_dissimilar() {
        let a = MinHash::of_normalized(&normalize(&prose(500, 3)));
        let b = MinHash::of_normalized(&normalize(&prose(500, 4)));
        let sim = a.similarity(&b);
        assert!(sim < 0.1, "unrelated texts look similar: {sim}");
    }

    #[test]
    fn minhash_empty_never_matches() {
        let empty = MinHash::of_normalized("");
        assert_eq!(empty.shingle_count, 0);
        assert!(empty.similarity(&empty).abs() < f64::EPSILON);
    }

    #[test]
    fn universal_hash_stays_below_the_prime() {
        for (a, b) in permutation_parameters() {
            for x in [0, 1, u64::MAX, MERSENNE_61, 0x1234_5678_9abc_def0] {
                assert!(universal_hash(a, b, x) < MERSENNE_61);
            }
        }
    }

    #[test]
    fn simhash_distance_separates_near_and_far() {
        let base = normalize(&prose(300, 11));
        let base_tokens = tokens(&base);
        let mut edited = base_tokens.clone();
        edited[10] = "swapped";
        edited[200] = "swapped";
        let other = normalize(&prose(300, 12));
        let near = hamming(simhash(&base_tokens), simhash(&edited));
        let far = hamming(simhash(&base_tokens), simhash(&tokens(&other)));
        assert!(near <= 6, "near distance {near}");
        assert!(far >= 12, "far distance {far}");
    }

    #[test]
    fn fnv1a_matches_reference_vector() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
