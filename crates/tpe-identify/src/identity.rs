//! One input's identities: exact (SHA-256 of the bytes), textual (digest
//! and fingerprints of the normalised text) and bibliographic (DOI,
//! `arXiv` id, title, first author, year).

use serde::{Deserialize, Serialize};
use tpe::schema::{ExtractionResult, Status, sha256_hex};

use crate::biblio::{BiblioKey, Variant, classify_variant, key_of};
use crate::text::{
    MIN_TOKENS_FOR_TEXT_EVIDENCE, MinHash, SHINGLE_WORDS, normalize, shingles, simhash, tokens,
};

/// Everything computed for one input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Identity {
    /// Position in the scan's input list.
    pub index: usize,
    /// The PDF on disk, when there is one (a result JSON whose source has
    /// moved has none; it can be grouped but not renamed).
    pub path: Option<String>,
    pub size: u64,
    /// Lower-case hex SHA-256 of the PDF bytes.
    pub sha256: String,
    /// Where the text came from: `json:<path>`, `results:<path>`,
    /// `ledger:<db>#<run>`, `extract:lopdf` or `none:<reason>`.
    pub text_source: String,
    pub pages: u32,
    /// Tokens of the normalised text.
    pub words: usize,
    /// SHA-256 of the normalised text; `None` without text.
    pub text_sha256: Option<String>,
    /// 64-bit `SimHash` as 16 hex digits; `0000000000000000` without text.
    pub simhash: String,
    /// The `MinHash` signature (kept in memory only; not part of the report).
    #[serde(skip)]
    pub minhash: Option<MinHash>,
    pub key: BiblioKey,
    pub variant: Variant,
    pub warnings: Vec<String>,
}

impl Identity {
    /// Build the identity of one input from its bytes' hash and, when
    /// available, the engine's result for it.
    pub fn build(
        index: usize,
        path: Option<String>,
        size: u64,
        sha256: String,
        text_source: String,
        result: Option<&ExtractionResult>,
        mut warnings: Vec<String>,
    ) -> Self {
        let Some(result) = result else {
            return Self {
                index,
                path,
                size,
                sha256,
                text_source,
                pages: 0,
                words: 0,
                text_sha256: None,
                simhash: "0000000000000000".to_string(),
                minhash: None,
                key: BiblioKey::default(),
                variant: Variant::Unknown,
                warnings,
            };
        };
        let full_text = result
            .pages
            .iter()
            .map(|page| page.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let page1 = result.pages.first().map_or("", |page| page.text.as_str());
        let normalized = normalize(&full_text);
        let toks = tokens(&normalized);
        let words = toks.len();
        let minhash = MinHash::of_shingles(&shingles(&toks, SHINGLE_WORDS));
        let sim = simhash(&toks);
        let key = key_of(&result.metadata, page1);
        let variant = classify_variant(&key, page1);
        if result.status != Status::Complete {
            warnings.push(format!("extraction status: {}", result.status.as_str()));
        }
        if words < MIN_TOKENS_FOR_TEXT_EVIDENCE {
            warnings.push(format!(
                "only {words} text tokens: text evidence is not used (minimum {MIN_TOKENS_FOR_TEXT_EVIDENCE})"
            ));
        }
        warnings.extend(key.notes.iter().cloned());
        Self {
            index,
            path,
            size,
            sha256,
            text_source,
            pages: result.document.pages,
            words,
            text_sha256: (words > 0).then(|| sha256_hex(normalized.as_bytes())),
            simhash: format!("{sim:016x}"),
            minhash: Some(minhash),
            key,
            variant,
            warnings,
        }
    }

    /// Whether the text is long enough to count as evidence.
    pub fn has_text_evidence(&self) -> bool {
        self.words >= MIN_TOKENS_FOR_TEXT_EVIDENCE && self.minhash.is_some()
    }

    /// Estimated Jaccard similarity of the two texts' shingles, or `None`
    /// when either side lacks text evidence.
    pub fn text_similarity(&self, other: &Self) -> Option<f64> {
        if !self.has_text_evidence() || !other.has_text_evidence() {
            return None;
        }
        Some(self.minhash.as_ref()?.similarity(other.minhash.as_ref()?))
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::collections::BTreeMap;

    use tpe::schema::{
        Author, BackendIdentity, ContentHash, Document, ExtractionResult, Metadata, PageText,
        SCHEMA_VERSION, StageTimings, Status, sha256_hex,
    };

    /// A synthetic engine result with one page per `pages` entry.
    pub(crate) fn result(bytes: &[u8], pages: &[&str], metadata: Metadata) -> ExtractionResult {
        let pages: Vec<PageText> = pages
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let mut page =
                    PageText::new(u32::try_from(i + 1).unwrap_or(u32::MAX), 612.0, 792.0, 0);
                page.text = (*text).to_string();
                page
            })
            .collect();
        ExtractionResult {
            schema_version: SCHEMA_VERSION,
            document: Document {
                hash: ContentHash(sha256_hex(bytes)),
                size: bytes.len() as u64,
                pages: u32::try_from(pages.len()).unwrap_or(u32::MAX),
                sources: vec![],
            },
            backend: BackendIdentity {
                name: "test".into(),
                version: "0".into(),
                config_digest: String::new(),
            },
            status: Status::Complete,
            pages,
            chunks: vec![],
            metadata,
            references: vec![],
            citations: vec![],
            warnings: vec![],
            timings: StageTimings::default(),
        }
    }

    pub(crate) fn metadata(
        title: &str,
        author: &str,
        year: Option<u16>,
        doi: Option<&str>,
    ) -> Metadata {
        Metadata {
            title: (!title.is_empty()).then(|| title.to_string()),
            authors: if author.is_empty() {
                vec![]
            } else {
                vec![Author {
                    name: author.to_string(),
                    ..Author::default()
                }]
            },
            doi: doi.map(str::to_string),
            year,
            info: BTreeMap::new(),
            ..Metadata::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{metadata, result};
    use super::*;

    #[test]
    fn identity_without_result_has_no_text() {
        let id = Identity::build(0, None, 3, "abc".into(), "none:test".into(), None, vec![]);
        assert_eq!(id.words, 0);
        assert!(!id.has_text_evidence());
        assert!(id.key.is_empty());
        assert_eq!(id.variant, Variant::Unknown);
    }

    #[test]
    fn identity_from_result_carries_text_and_key() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon phi chi psi omega";
        let r = result(
            b"bytes",
            &[text, text],
            metadata(
                "A Title Of Note",
                "Ann Turing",
                Some(2020),
                Some("10.1000/x"),
            ),
        );
        let id = Identity::build(
            1,
            Some("a.pdf".into()),
            5,
            "h".into(),
            "json:a".into(),
            Some(&r),
            vec![],
        );
        assert_eq!(id.pages, 2);
        assert_eq!(id.words, 48);
        assert!(id.has_text_evidence());
        assert_eq!(id.key.doi.as_deref(), Some("10.1000/x"));
        assert_eq!(id.variant, Variant::Published);
        assert_eq!(id.simhash.len(), 16);
        assert!(id.warnings.is_empty(), "{:?}", id.warnings);
        let twin = Identity::build(2, None, 5, "h2".into(), "json:b".into(), Some(&r), vec![]);
        assert_eq!(id.text_similarity(&twin), Some(1.0));
        assert_eq!(id.text_sha256, twin.text_sha256);
    }
}
