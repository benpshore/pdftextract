//! `routed`: PDF Oxide extracts by default and `PDFium` supplies any page whose
//! Oxide text is unusable, decided page by page by [`page_route`]. Every page
//! records which parser supplied it (`routed: <backend> (<reason>)`), the two
//! parsers are never combined within a page, and a `PDFium` page keeps `PDFium`'s
//! own mapping diagnostics, so unresolved mappings stay Partial.
//!
//! `PDFium` is bound lazily: the first page that needs it opens the native
//! document (an eager whole-document read, see `pdfium_backend`). When `PDFium`
//! is missing or fails for a page, the Oxide page is kept and the page says so
//! (`route not taken: ...`); nothing is silently substituted.
//!
//! An Oxide page is cross-read by `PDFium` when available. Text agreement is
//! corroborating evidence, not proof of completeness: both parsers can omit
//! the same content. Every Oxide warning and every native diagnostic survives
//! confirmation, including mapping and resource uncertainty. The Oxide text
//! itself is never edited from `PDFium`'s. Page-count disagreement is explicit.

use std::collections::BTreeMap;

use super::pdf_oxide_backend::PdfOxideBackend;
use super::pdfium_backend::PdfiumBackend;
use super::{BackendError, DocumentSession, Extractor};
use crate::router::{PageBackend, PageEvidence, Task, page_route};
use crate::schema::{BackendIdentity, PageText, config_digest};

/// Identity of the per-page rule; bump when [`page_route`] changes.
pub const POLICY: &str = "oxide-default-pdfium-per-page-corroborate-v3";

/// The two-parser backend selected with `--backend routed`.
#[derive(Clone, Debug, Default)]
pub struct RoutedBackend {
    /// `PDFium` configuration (library location); PDF Oxide needs none.
    pub pdfium: PdfiumBackend,
}

impl Extractor for RoutedBackend {
    fn identity(&self) -> BackendIdentity {
        let oxide = PdfOxideBackend.identity();
        let pdfium = self.pdfium.identity();
        BackendIdentity {
            name: "routed".to_string(),
            version: format!("pdf-oxide-{}+pdfium-{}", oxide.version, pdfium.version),
            config_digest: config_digest(&BTreeMap::from([
                ("policy".to_string(), POLICY.to_string()),
                ("extract".to_string(), oxide.config_digest),
                ("render_and_fallback".to_string(), pdfium.config_digest),
            ])),
        }
    }

    fn open(
        &self,
        bytes: &[u8],
        password: Option<&str>,
    ) -> Result<Box<dyn DocumentSession>, BackendError> {
        let oxide = PdfOxideBackend.open(bytes, password)?;
        Ok(Box::new(RoutedSession {
            oxide,
            pdfium: None,
            pdfium_failure: None,
            backend: self.pdfium.clone(),
            bytes: bytes.to_vec(),
            password: password.map(str::to_owned),
        }))
    }
}

struct RoutedSession {
    oxide: Box<dyn DocumentSession>,
    pdfium: Option<Box<dyn DocumentSession>>,
    /// Why `PDFium` could not be opened; remembered so it is attempted once.
    pdfium_failure: Option<String>,
    backend: PdfiumBackend,
    bytes: Vec<u8>,
    password: Option<String>,
}

impl RoutedSession {
    fn pdfium(&mut self) -> Result<&mut dyn DocumentSession, String> {
        if self.pdfium.is_none() && self.pdfium_failure.is_none() {
            match self.backend.open(&self.bytes, self.password.as_deref()) {
                Ok(session) => self.pdfium = Some(session),
                Err(error) => self.pdfium_failure = Some(error.to_string()),
            }
        }
        match (self.pdfium.as_deref_mut(), &self.pdfium_failure) {
            (Some(session), _) => Ok(session),
            (None, Some(failure)) => Err(failure.clone()),
            (None, None) => Err("pdfium was not opened".to_string()),
        }
    }
}

/// PDF Oxide's standing warning that its completeness was not independently
/// verified (`pdf_oxide_backend`); agreement cannot discharge this uncertainty.
#[cfg(test)]
const UNVERIFIED_PREFIX: &str =
    "extraction_incomplete: pdf-oxide character extraction retains upstream fallback uncertainty";
/// Character 4-gram similarity at or above which texts closely overlap.
const AGREEMENT_THRESHOLD: f64 = 0.98;

/// How two parsers' texts for one page relate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Agreement {
    /// Identical after whitespace normalisation.
    Same,
    /// Character 4-gram multisets overlap at or above the threshold.
    Close(f64),
    /// Below the threshold.
    Different(f64),
}

impl Agreement {
    /// The similarity score behind the verdict.
    #[must_use]
    pub fn score(self) -> f64 {
        match self {
            Self::Same => 1.0,
            Self::Close(score) | Self::Different(score) => score,
        }
    }
}

/// Compare two page texts. Whitespace is irrelevant (the parsers split spans
/// and lines differently); otherwise the Dice coefficient of their character
/// 4-gram multisets decides (`2|A∩B| / (|A|+|B|)`), which is insensitive to
/// where either parser breaks words or lines.
#[must_use]
pub fn agreement(left: &str, right: &str) -> Agreement {
    let left_chars: Vec<char> = left.chars().filter(|c| !c.is_whitespace()).collect();
    let right_chars: Vec<char> = right.chars().filter(|c| !c.is_whitespace()).collect();
    if left_chars == right_chars {
        return Agreement::Same;
    }
    let grams = |chars: &[char]| -> Vec<String> {
        if chars.len() < 4 {
            vec![chars.iter().collect()]
        } else {
            chars.windows(4).map(|w| w.iter().collect()).collect()
        }
    };
    let left_grams = grams(&left_chars);
    let right_grams = grams(&right_chars);
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for gram in &left_grams {
        *counts.entry(gram.as_str()).or_default() += 1;
    }
    let mut shared = 0_i64;
    for gram in &right_grams {
        if let Some(count) = counts.get_mut(gram.as_str())
            && *count > 0
        {
            *count -= 1;
            shared += 1;
        }
    }
    let total = left_grams.len() + right_grams.len();
    #[allow(clippy::cast_precision_loss)]
    let score = if total == 0 {
        1.0
    } else {
        2.0 * shared as f64 / total as f64
    };
    if score >= AGREEMENT_THRESHOLD {
        Agreement::Close(score)
    } else {
        Agreement::Different(score)
    }
}

/// The page's characters in span order, exactly as the parser emitted them
/// (PDF Oxide emits one span per character, `PDFium` one per line).
fn page_words(page: &PageText) -> String {
    page.spans.iter().map(|span| span.text.as_str()).collect()
}

/// Corroborate text without erasing either parser's extraction evidence.
fn corroborate_with_pdfium(page: &mut PageText, native: &PageText) {
    let native_status = native.extraction_status();
    // Keep diagnostic prefixes intact: extraction_status() relies on them.
    // Backend attribution is a suffix rather than a prefix for that reason.
    page.warnings.extend(
        native
            .warnings
            .iter()
            .map(|warning| format!("{warning} [confirming backend: pdfium]")),
    );
    let verdict = agreement(&page_words(page), &page_words(native));
    let outcome = match verdict {
        Agreement::Same => "identical text",
        Agreement::Close(_) => "close text; changed or omitted content remains possible",
        Agreement::Different(_) => "different text",
    };
    page.warnings.push(format!(
        "unconfirmed: pdfium {outcome} (similarity {:.3}, native status {native_status:?}); text comparison does not verify completeness; pdf-oxide text retained",
        verdict.score()
    ));
}

impl RoutedSession {
    /// Read `number` independently, preserving all uncertainty and text.
    fn confirm_with_pdfium(&mut self, number: u32, page: &mut PageText) {
        let oxide_count = self.oxide.page_count();
        match self.pdfium() {
            Ok(session) => {
                let native_count = session.page_count();
                if native_count != oxide_count {
                    page.warnings.push(format!(
                        "extraction_incomplete: pdfium/pdf-oxide page-count disagreement: pdfium={native_count}, pdf-oxide={oxide_count}"
                    ));
                }
                match session.page_text(number) {
                    Ok(native) => corroborate_with_pdfium(page, &native),
                    Err(error) => page
                        .warnings
                        .push(format!("unconfirmed: pdfium page failed: {error}")),
                }
            }
            Err(unavailable) => page
                .warnings
                .push(format!("unconfirmed: pdfium unavailable: {unavailable}")),
        }
    }
}

/// Keep the Oxide page when `PDFium` cannot supply the routed one, saying why.
fn keep_oxide(
    oxide: Result<PageText, BackendError>,
    not_taken: String,
    reason: &str,
) -> Result<PageText, BackendError> {
    let mut page = oxide?;
    page.warnings.push(not_taken);
    page.warnings
        .push(format!("routed: pdf-oxide (retained; {reason})"));
    Ok(page)
}

impl DocumentSession for RoutedSession {
    fn page_count(&self) -> u32 {
        self.oxide.page_count()
    }

    fn info(&self) -> BTreeMap<String, String> {
        self.oxide.info()
    }

    fn page_text(&mut self, page: u32) -> Result<PageText, BackendError> {
        let number = page;
        let oxide = self.oxide.page_text(number);
        let evidence = PageEvidence::from_oxide(oxide.as_ref());
        let (choice, reason) = page_route(Task::Extract(&evidence));
        match choice {
            PageBackend::PdfOxide => {
                let mut page = oxide?;
                page.warnings.push(format!("routed: pdf-oxide ({reason})"));
                self.confirm_with_pdfium(number, &mut page);
                Ok(page)
            }
            PageBackend::Pdfium => match self.pdfium() {
                Ok(session) => match session.page_text(page) {
                    Ok(mut native) => {
                        native.warnings.push(format!("routed: pdfium ({reason})"));
                        Ok(native)
                    }
                    Err(error) => keep_oxide(
                        oxide,
                        format!("route not taken: pdfium page failed: {error}"),
                        reason,
                    ),
                },
                Err(unavailable) => keep_oxide(
                    oxide,
                    format!("route not taken: pdfium unavailable: {unavailable}"),
                    reason,
                ),
            },
        }
    }

    /// Only `PDFium` pages carry figures, so only its session can have the bytes.
    fn take_figure_bytes(&mut self, page: u32, index: u32) -> Option<Vec<u8>> {
        self.pdfium
            .as_deref_mut()
            .and_then(|session| session.take_figure_bytes(page, index))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use lopdf::content::{Content, Operation};
    use lopdf::{Dictionary, Document, Object, Stream, dictionary};

    use super::*;
    use crate::schema::Status;

    fn native_available() -> bool {
        if std::env::var_os("PDFIUM_DYNAMIC_LIB_PATH").is_none() {
            eprintln!("skipped: set PDFIUM_DYNAMIC_LIB_PATH for routed-backend regressions");
            return false;
        }
        true
    }

    /// A backend whose `PDFium` can never bind: proves which pages need it.
    fn without_pdfium() -> RoutedBackend {
        RoutedBackend {
            pdfium: PdfiumBackend {
                library_dir: Some("/nonexistent/pdfium".to_string()),
            },
        }
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/pdfium-unicode")
                .join(name),
        )
        .unwrap()
    }

    /// One page showing `shown` with `font` (a complete font dictionary).
    fn pdf_with_font(font: Dictionary, shown: Object) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let tree_id = doc.new_object_id();
        let font_id = doc.add_object(font);
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let operations = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 24_i32.into()]),
            Operation::new("Td", vec![72_i32.into(), 700_i32.into()]),
            Operation::new("Tj", vec![shown]),
            Operation::new("ET", vec![]),
        ];
        let content = Content { operations }.encode().unwrap();
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => tree_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0_i32.into(), 0_i32.into(), 612_i32.into(), 792_i32.into()],
        });
        doc.objects.insert(
            tree_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![Object::Reference(page_id)],
                "Count" => Object::Integer(1),
            }),
        );
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => tree_id });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    /// A non-embedded Type0/Identity-H font with no `ToUnicode`: PDF Oxide
    /// reports `to_unicode_missing` and emits U+FFFD.
    fn identity_without_tounicode() -> Vec<u8> {
        let descriptor = dictionary! {
            "Type" => "FontDescriptor", "FontName" => "Helvetica", "Flags" => 32,
            "FontBBox" => vec![0_i32.into(), 0_i32.into(), 1000_i32.into(), 1000_i32.into()],
            "ItalicAngle" => 0, "Ascent" => 800, "Descent" => -200, "CapHeight" => 700, "StemV" => 80,
        };
        let descendant = dictionary! {
            "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "Helvetica",
            "CIDSystemInfo" => dictionary! { "Registry" => Object::string_literal("Adobe"), "Ordering" => Object::string_literal("Identity"), "Supplement" => 0 },
            "FontDescriptor" => descriptor, "DW" => 600,
        };
        pdf_with_font(
            dictionary! {
                "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "Helvetica",
                "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Dictionary(descendant)],
            },
            Object::String(vec![0, 1, 0, 2, 0, 3], lopdf::StringFormat::Hexadecimal),
        )
    }

    /// A simple font whose `Differences` name private-use glyphs.
    fn private_use_differences() -> Vec<u8> {
        pdf_with_font(
            dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                "Encoding" => dictionary! {
                    "Type" => "Encoding",
                    "Differences" => vec![1.into(), "uniE001".into(), "uniE002".into(), "uniE003".into()],
                },
            },
            Object::string_literal(vec![1_u8, 2, 3]),
        )
    }

    /// A simple font whose `Differences` name glyphs nothing can map:
    /// PDF Oxide's text for the page is empty.
    fn unknown_glyph_names() -> Vec<u8> {
        pdf_with_font(
            dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
                "Encoding" => dictionary! {
                    "Type" => "Encoding",
                    "Differences" => vec![1.into(), "g17".into(), "g23".into(), "g42".into()],
                },
            },
            Object::string_literal(vec![1_u8, 2, 3]),
        )
    }

    fn page_of(backend: &RoutedBackend, bytes: &[u8]) -> PageText {
        let mut session = backend.open(bytes, None).unwrap();
        assert_eq!(session.page_count(), 1);
        session.page_text(1).unwrap()
    }

    fn supplier(page: &PageText) -> String {
        let routed: Vec<&String> = page
            .warnings
            .iter()
            .filter(|w| w.starts_with("routed: "))
            .collect();
        assert_eq!(routed.len(), 1, "exactly one supplier per page: {routed:?}");
        routed[0].clone()
    }

    fn text_of(page: &PageText) -> String {
        page.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn confirmation(page: &PageText) -> String {
        let lines: Vec<&String> = page
            .warnings
            .iter()
            .filter(|w| w.starts_with("confirmed: ") || w.starts_with("unconfirmed: "))
            .collect();
        assert_eq!(lines.len(), 1, "exactly one confirmation line: {lines:?}");
        lines[0].clone()
    }

    #[test]
    fn agreement_ignores_whitespace_and_scores_character_overlap() {
        assert_eq!(agreement("a  b\nc", "a b c"), Agreement::Same);
        assert_eq!(agreement("F a i t h f u l", "Faithful"), Agreement::Same);
        assert_eq!(agreement("", ""), Agreement::Same);
        assert_eq!(agreement("ab", "ab"), Agreement::Same);
        let words: Vec<String> = (0..1000).map(|i| format!("word{i}")).collect();
        let mut one_off = words.clone();
        one_off[500] = "changed".to_string();
        match agreement(&words.join(" "), &one_off.join("\n")) {
            Agreement::Close(score) => assert!(score > 0.99 && score < 1.0, "{score}"),
            other => panic!("{other:?}"),
        }
        let half: Vec<String> = words[..500].to_vec();
        match agreement(&words.join(" "), &half.join(" ")) {
            Agreement::Different(score) => assert!(score > 0.5 && score < 0.75, "{score}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            agreement("alpha beta", "gamma delta"),
            Agreement::Different(0.0)
        );
        assert!((Agreement::Same.score() - 1.0).abs() < f64::EPSILON);
    }

    fn synthetic_page(text: &str, warnings: &[&str]) -> PageText {
        let mut page = PageText::new(1, 612.0, 792.0, 0);
        page.spans.push(crate::schema::Span {
            text: text.to_string(),
            bbox: None,
            font: None,
            size: None,
            seq: 0,
        });
        page.warnings = warnings
            .iter()
            .map(|warning| (*warning).to_string())
            .collect();
        page
    }

    struct FixedSession {
        page: PageText,
        count: u32,
    }

    impl DocumentSession for FixedSession {
        fn page_count(&self) -> u32 {
            self.count
        }

        fn info(&self) -> BTreeMap<String, String> {
            BTreeMap::new()
        }

        fn page_text(&mut self, number: u32) -> Result<PageText, BackendError> {
            assert_eq!(number, 1);
            Ok(self.page.clone())
        }
    }

    fn confirm_synthetic(oxide: PageText, native: PageText, native_count: u32) -> PageText {
        let mut session = RoutedSession {
            oxide: Box::new(FixedSession {
                page: oxide,
                count: 1,
            }),
            pdfium: Some(Box::new(FixedSession {
                page: native,
                count: native_count,
            })),
            pdfium_failure: None,
            backend: PdfiumBackend::default(),
            bytes: Vec::new(),
            password: None,
        };
        // Exercise the production routing and confirmation path, not just comparison.
        session.page_text(1).unwrap()
    }

    #[test]
    fn equal_text_preserves_partial_native_mapping_resource_and_failure_evidence() {
        for diagnostic in [
            "unicode_mapping: unresolved character",
            "resource_limit: inspection budget exhausted",
            "extraction_incomplete: native omitted content",
            "failed: native extraction failed",
        ] {
            let oxide = synthetic_page("ALPHA BETA", &[UNVERIFIED_PREFIX, "oxide diagnostic"]);
            let native = synthetic_page("ALPHA BETA", &[diagnostic, "native diagnostic"]);
            assert_eq!(native.extraction_status(), Status::Partial);
            let page = confirm_synthetic(oxide, native, 1);
            assert_eq!(text_of(&page), "ALPHA BETA");
            assert_eq!(page.extraction_status(), Status::Partial);
            assert!(page.warnings.iter().any(|w| w == UNVERIFIED_PREFIX));
            assert!(page.warnings.iter().any(|w| w == "oxide diagnostic"));
            assert!(
                page.warnings
                    .iter()
                    .any(|w| w == &format!("{diagnostic} [confirming backend: pdfium]"))
            );
            assert!(
                page.warnings
                    .iter()
                    .any(|w| w == "native diagnostic [confirming backend: pdfium]")
            );
            assert!(confirmation(&page).contains("native status Partial"));
        }
    }

    #[test]
    fn close_changed_or_omitted_text_cannot_promote_completeness() {
        let words: Vec<String> = (0..1000).map(|i| format!("word{i}")).collect();
        let original = words.join(" ");
        let mut changed = words.clone();
        changed[500] = "changed".to_string();
        let mut omitted = words;
        omitted.remove(500);
        for text in [changed.join(" "), omitted.join(" ")] {
            assert!(matches!(agreement(&original, &text), Agreement::Close(_)));
            let page = confirm_synthetic(
                synthetic_page(&original, &[UNVERIFIED_PREFIX]),
                synthetic_page(&text, &[]),
                1,
            );
            assert_eq!(text_of(&page), original);
            assert_eq!(page.extraction_status(), Status::Partial);
            assert!(confirmation(&page).contains("changed or omitted content"));
            assert!(page.warnings.iter().any(|w| w == UNVERIFIED_PREFIX));
        }
    }

    #[test]
    fn different_text_preserves_both_backends_evidence() {
        let page = confirm_synthetic(
            synthetic_page("ALPHA BETA", &[UNVERIFIED_PREFIX]),
            synthetic_page("GAMMA DELTA", &["resource_limit: native limit"]),
            1,
        );
        assert_eq!(text_of(&page), "ALPHA BETA");
        assert_eq!(page.extraction_status(), Status::Partial);
        assert!(confirmation(&page).contains("different text"));
        assert!(
            page.warnings
                .iter()
                .any(|w| w.starts_with("resource_limit:"))
        );
        assert!(page.warnings.iter().any(|w| w == UNVERIFIED_PREFIX));
    }

    #[test]
    fn shared_omissions_are_not_disproved_by_identical_clean_native_text() {
        // Ground truth contains an extra clause neither extraction captured.
        let source_text = "ALPHA BETA omitted clause";
        let extracted = "ALPHA BETA";
        assert_ne!(source_text, extracted);
        let native = synthetic_page(extracted, &[]);
        assert_eq!(native.extraction_status(), Status::Complete);
        let page = confirm_synthetic(synthetic_page(extracted, &[UNVERIFIED_PREFIX]), native, 1);
        assert_eq!(page.extraction_status(), Status::Partial);
        assert!(confirmation(&page).contains("identical text"));
        assert!(confirmation(&page).contains("does not verify completeness"));
        assert!(page.warnings.iter().any(|w| w == UNVERIFIED_PREFIX));
    }

    #[test]
    fn page_count_disagreement_survives_equal_text_confirmation() {
        for count in [0, 2] {
            let page = confirm_synthetic(
                synthetic_page("ALPHA BETA", &[UNVERIFIED_PREFIX]),
                synthetic_page("ALPHA BETA", &["native diagnostic"]),
                count,
            );
            assert_eq!(text_of(&page), "ALPHA BETA");
            assert_eq!(page.extraction_status(), Status::Partial);
            assert!(page.warnings.iter().any(|w| w == &format!(
                "extraction_incomplete: pdfium/pdf-oxide page-count disagreement: pdfium={count}, pdf-oxide=1"
            )));
            assert!(page.warnings.iter().any(|w| w == UNVERIFIED_PREFIX));
            assert!(
                page.warnings
                    .iter()
                    .any(|w| w == "native diagnostic [confirming backend: pdfium]")
            );
        }
    }

    #[test]
    fn clean_oxide_pages_stay_partial_and_unconfirmed_without_pdfium() {
        let backend = without_pdfium();
        let page = page_of(&backend, &fixture("native.pdf"));
        assert_eq!(supplier(&page), "routed: pdf-oxide (text mapped cleanly)");
        assert!(confirmation(&page).starts_with("unconfirmed: pdfium unavailable:"));
        assert!(
            page.warnings
                .iter()
                .any(|w| w.starts_with(UNVERIFIED_PREFIX)),
            "{:?}",
            page.warnings
        );
        assert_eq!(page.extraction_status(), Status::Partial);
    }

    #[test]
    fn pdfium_corroborates_clean_oxide_pages_without_claiming_completeness() {
        if !native_available() {
            return;
        }
        let backend = RoutedBackend::default();
        let page = page_of(&backend, &fixture("native.pdf"));
        assert_eq!(supplier(&page), "routed: pdf-oxide (text mapped cleanly)");
        let line = confirmation(&page);
        assert!(
            line.starts_with("unconfirmed: pdfium identical text"),
            "{line}"
        );
        assert!(
            page.warnings
                .iter()
                .any(|w| w.starts_with(UNVERIFIED_PREFIX)),
            "{:?}",
            page.warnings
        );
        assert_eq!(page.extraction_status(), Status::Partial);
        assert!(text_of(&page).contains("Faithful native text"));
    }

    #[test]
    fn native_partial_fixture_keeps_mapping_evidence_on_oxide_route() {
        if !native_available() {
            return;
        }
        let bytes = fixture("partial-cmap.pdf");
        let mut session = PdfiumBackend::default().open(&bytes, None).unwrap();
        let native = session.page_text(1).unwrap();
        assert_eq!(native.extraction_status(), Status::Partial);
        let page = page_of(&RoutedBackend::default(), &bytes);
        assert_eq!(supplier(&page), "routed: pdf-oxide (text mapped cleanly)");
        assert_eq!(page.extraction_status(), Status::Partial);
        assert!(
            page.warnings
                .iter()
                .any(|w| w.starts_with(UNVERIFIED_PREFIX))
        );
        for warning in &native.warnings {
            assert!(
                page.warnings
                    .contains(&format!("{warning} [confirming backend: pdfium]"))
            );
        }
        assert!(confirmation(&page).contains("native status Partial"));
    }

    #[test]
    fn identity_names_both_parsers_and_the_policy() {
        let identity = RoutedBackend::default().identity();
        assert_eq!(identity.name, "routed");
        assert!(identity.version.starts_with("pdf-oxide-0.3.78+pdfium-"));
        assert_ne!(
            without_pdfium().identity().config_digest,
            identity.config_digest
        );
    }

    #[test]
    fn clean_pages_come_from_pdf_oxide_without_binding_pdfium() {
        let backend = without_pdfium();
        let probe = page_of(&backend, &crate::backend::probe_pdf().unwrap());
        assert_eq!(supplier(&probe), "routed: pdf-oxide (text mapped cleanly)");
        assert_eq!(text_of(&probe), "probe");
        // The committed fixtures map cleanly through PDF Oxide, so `PDFium` is
        // never opened for them (the unbindable library proves it).
        let native = page_of(&backend, &fixture("native.pdf"));
        assert_eq!(supplier(&native), "routed: pdf-oxide (text mapped cleanly)");
        assert!(text_of(&native).contains("Faithful native text remains available"));
        let partial = page_of(&backend, &fixture("partial-cmap.pdf"));
        assert_eq!(
            supplier(&partial),
            "routed: pdf-oxide (text mapped cleanly)"
        );
        assert_eq!(text_of(&partial).trim(), "ALPHA BETA GAMMA");
        for page in [&probe, &native, &partial] {
            assert!(
                !page
                    .warnings
                    .iter()
                    .any(|w| w.starts_with("route not taken:")),
                "{:?}",
                page.warnings
            );
        }
    }

    #[test]
    fn missing_pdfium_is_recorded_rather_than_substituted() {
        let backend = without_pdfium();
        for (bytes, reason) in [
            (
                identity_without_tounicode(),
                "a font has no usable Unicode mapping",
            ),
            (unknown_glyph_names(), "pdf-oxide text is empty"),
            (
                private_use_differences(),
                "pdf-oxide text contains replacement or private-use characters",
            ),
        ] {
            let page = page_of(&backend, &bytes);
            assert!(
                page.warnings
                    .iter()
                    .any(|w| w.starts_with("route not taken: pdfium unavailable:")),
                "{:?}",
                page.warnings
            );
            assert_eq!(
                supplier(&page),
                format!("routed: pdf-oxide (retained; {reason})")
            );
            assert_eq!(page.extraction_status(), Status::Partial);
        }
    }

    #[test]
    fn unusable_oxide_text_routes_the_page_to_pdfium() {
        if !native_available() {
            return;
        }
        let backend = RoutedBackend::default();
        let unmapped = page_of(&backend, &identity_without_tounicode());
        assert_eq!(
            supplier(&unmapped),
            "routed: pdfium (a font has no usable Unicode mapping)"
        );
        // `PDFium`'s own mapping diagnostics travel with its page: still Partial,
        // and no PDF Oxide text is mixed in.
        assert!(
            crate::router::has_unmapped_text(&unmapped),
            "{:?}",
            unmapped.warnings
        );
        assert!(!text_of(&unmapped).contains('\u{fffd}'));
        assert_eq!(unmapped.extraction_status(), Status::Partial);

        let empty = page_of(&backend, &unknown_glyph_names());
        assert_eq!(supplier(&empty), "routed: pdfium (pdf-oxide text is empty)");

        let private = page_of(&backend, &private_use_differences());
        assert_eq!(
            supplier(&private),
            "routed: pdfium (pdf-oxide text contains replacement or private-use characters)"
        );
    }

    #[test]
    fn pipeline_result_carries_the_routed_identity_and_per_page_supplier() {
        if !native_available() {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("unmapped.pdf");
        std::fs::write(&path, identity_without_tounicode()).unwrap();
        let result = crate::pipeline::run_job(&crate::schema::Job {
            path: path.to_string_lossy().into_owned(),
            backend: "routed".into(),
            pages: None,
            password: None,
            max_bytes: None,
            figures_dir: None,
        })
        .unwrap();
        assert_eq!(result.backend.name, "routed");
        assert_eq!(result.status, Status::Partial);
        assert_eq!(
            supplier(&result.pages[0]),
            "routed: pdfium (a font has no usable Unicode mapping)"
        );
    }
}
