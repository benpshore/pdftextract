//! Backend routing: decide, from what the cheap `lopdf` pass reports about a
//! page, whether a page needs a stronger backend.
//!
//! `lopdf` parses the PDF structure without rendering, so it is the probe:
//! it says whether a page has text at all, whether that text came out of
//! fonts it could map to Unicode, and whether the page is a full-page
//! image. Three routes follow from that:
//!
//! * [`Route::Lopdf`]: mapped text, keep the fast result;
//! * [`Route::Pdfium`]: text is present but some font had no usable
//!   Unicode mapping (`undecodable`, `decoded as Latin-1`, or a share of
//!   U+FFFD in the text). `pdfium` reads the embedded font program's own
//!   tables and usually recovers the characters;
//! * [`Route::Docling`]: no text layer under a page-sized image (a scan),
//!   without unresolved native mapping evidence. Unverified Unicode mappings
//!   remain Partial; OCR has not proved recovery of those source characters.
//!
//! The optional full-extraction native cascade also uses [`Route::MuPdf`] and
//! [`Route::Poppler`]. The assessment itself never selects them; the pipeline
//! permits each once after the `PDFium` route needs additional help.
//!
//! The `routed` backend (`backend::routed_backend`) uses a different, per-page
//! rule, [`page_route`]: PDF Oxide extracts by default and `PDFium` supplies the
//! pages whose Oxide text is unusable. It is a plain `match` over
//! [`PageEvidence`] so the rule can be read in one place.
//!
//! Nothing here touches geometry or reading order.

use serde::Serialize;

use crate::backend::{self, BackendError, Extractor};
use crate::schema::PageText;

/// Below this many non-whitespace characters a page is "without text".
const MIN_TEXT_CHARS: usize = 20;
/// A raster covering at least this share of the page area makes a text-less
/// page a scan.
const SCAN_COVER: f32 = 0.5;
/// Share of U+FFFD among a page's characters above which its text is unmapped.
const FFFD_SHARE: f32 = 0.01;

/// Where a document (or a page window) should go next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    /// The `lopdf` result is usable.
    Lopdf,
    /// Re-read with `pdfium`: fonts without a usable Unicode mapping.
    Pdfium,
    /// Re-read with docling (layout + OCR): scanned pages.
    Docling,
    /// Optional native fallback after `PDFium` cannot resolve the text.
    MuPdf,
    /// Last optional native fallback after `MuPDF` cannot resolve the text.
    Poppler,
}

impl Route {
    /// The backend name for [`crate::backend::by_name`].
    #[must_use]
    pub fn backend_name(self) -> &'static str {
        match self {
            Self::Lopdf => "lopdf",
            Self::Pdfium => "pdfium",
            Self::Docling => "docling",
            Self::MuPdf => "mupdf",
            Self::Poppler => "poppler",
        }
    }
}

/// What the probe found on a set of pages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Assessment {
    pub pages: usize,
    /// Pages with (almost) no text under a page-sized raster.
    pub scanned: usize,
    /// Pages whose text came, in part, from fonts without a Unicode mapping.
    pub unmapped: usize,
    /// Pages with neither text nor a page-sized raster (blank, vector art).
    pub empty: usize,
}

impl Assessment {
    /// The route for pages assessed like this after a `lopdf` pass.
    #[must_use]
    pub fn route(self) -> Route {
        if self.unmapped > 0 {
            Route::Pdfium
        } else if self.scanned > 0 {
            Route::Docling
        } else {
            Route::Lopdf
        }
    }

    /// After `pdfium`, only scans without unresolved mappings go to OCR.
    /// Mapping uncertainty retains the native result and its Partial status.
    #[must_use]
    pub fn route_after_pdfium(self) -> Route {
        if self.scanned > 0 && self.unmapped == 0 {
            Route::Docling
        } else {
            Route::Pdfium
        }
    }
}

/// The compiled-in extractor for `route`, if this build has it. Optional native
/// cascade routes also require both explicit runtime library files.
#[must_use]
pub fn extractor_for(route: Route) -> Option<Box<dyn Extractor>> {
    if !native_runtime_configured(route, |name| std::env::var_os(name)) {
        return None;
    }
    backend::by_name(route.backend_name())
}

/// Optional native fallbacks never discover a library through PATH or the
/// platform loader. Both exact library files must be explicitly configured;
/// the provider then verifies their fingerprints and ABI when opening them.
fn native_runtime_configured(
    route: Route,
    mut lookup: impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> bool {
    let variables = match route {
        Route::MuPdf => ["TPE_MUPDF_PROVIDER_PATH", "MUPDF_DYNAMIC_LIB_PATH"],
        Route::Poppler => ["TPE_POPPLER_PROVIDER_PATH", "POPPLER_DYNAMIC_LIB_PATH"],
        _ => return true,
    };
    variables.into_iter().all(|name| {
        lookup(name).is_some_and(|value| {
            let path = std::path::Path::new(&value);
            path.is_absolute() && path.is_file()
        })
    })
}

/// Which parser supplies one page of the `routed` backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PageBackend {
    /// PDF Oxide, the default extractor.
    PdfOxide,
    /// `PDFium`: every render, and the pages PDF Oxide cannot read usably.
    Pdfium,
}

impl PageBackend {
    /// The backend name for [`crate::backend::by_name`].
    #[must_use]
    pub fn backend_name(self) -> &'static str {
        match self {
            Self::PdfOxide => "pdf-oxide",
            Self::Pdfium => "pdfium",
        }
    }
}

/// What the `routed` backend is being asked to do with a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task<'a> {
    /// Rasterise or draw the page.
    Render,
    /// Produce the page's text, given what PDF Oxide's pass reported.
    Extract(&'a PageEvidence),
}

/// How PDF Oxide's pass ended for one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OxideOutcome {
    /// PDF Oxide returned an error for the page.
    Failed,
    /// PDF Oxide read the page; `unmapped_font` is true when a font on it has
    /// no usable Unicode mapping (it reported `to_unicode_missing` or an
    /// unresolved mapping).
    Read { unmapped_font: bool },
}

/// What PDF Oxide's text for the page looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextShape {
    /// Whitespace only, or no text at all.
    Empty,
    /// Contains U+FFFD or private-use characters.
    ReplacementOrPrivateUse,
    /// Ordinary mapped text.
    #[default]
    Plain,
}

/// What PDF Oxide's pass reported about one page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageEvidence {
    pub oxide: OxideOutcome,
    pub text: TextShape,
}

impl Default for PageEvidence {
    /// A page PDF Oxide read cleanly.
    fn default() -> Self {
        Self {
            oxide: OxideOutcome::Read {
                unmapped_font: false,
            },
            text: TextShape::Plain,
        }
    }
}

impl PageEvidence {
    /// Read the evidence off PDF Oxide's result for a page.
    #[must_use]
    pub fn from_oxide(page: Result<&PageText, &BackendError>) -> Self {
        let Ok(page) = page else {
            return Self {
                oxide: OxideOutcome::Failed,
                text: TextShape::Empty,
            };
        };
        let text: String = if page.text.is_empty() {
            page.spans.iter().map(|s| s.text.as_str()).collect()
        } else {
            page.text.clone()
        };
        let unmapped_font = page.warnings.iter().any(|w| {
            w.starts_with("unicode_mapping:") || w.contains("pdf-oxide to_unicode_missing")
        });
        let shape = if text.trim().is_empty() {
            TextShape::Empty
        } else if text.chars().any(is_replacement_or_private_use) {
            TextShape::ReplacementOrPrivateUse
        } else {
            TextShape::Plain
        };
        Self {
            oxide: OxideOutcome::Read { unmapped_font },
            text: shape,
        }
    }
}

/// U+FFFD, or a code point in one of the three Private Use Areas.
#[must_use]
pub fn is_replacement_or_private_use(ch: char) -> bool {
    matches!(
        ch,
        '\u{fffd}' | '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}'
    )
}

/// The per-page rule of the `routed` backend, with the reason recorded on the
/// page. Rendering always goes to `PDFium`. Extraction goes to PDF Oxide unless
/// its page failed, a font has no usable Unicode mapping, its text is empty,
/// or its text contains replacement or private-use characters.
#[must_use]
pub fn page_route(task: Task<'_>) -> (PageBackend, &'static str) {
    match task {
        Task::Render => (PageBackend::Pdfium, "render"),
        Task::Extract(PageEvidence {
            oxide: OxideOutcome::Failed,
            ..
        }) => (PageBackend::Pdfium, "pdf-oxide failed to read the page"),
        Task::Extract(PageEvidence {
            oxide: OxideOutcome::Read {
                unmapped_font: true,
            },
            ..
        }) => (PageBackend::Pdfium, "a font has no usable Unicode mapping"),
        Task::Extract(PageEvidence {
            text: TextShape::Empty,
            ..
        }) => (PageBackend::Pdfium, "pdf-oxide text is empty"),
        Task::Extract(PageEvidence {
            text: TextShape::ReplacementOrPrivateUse,
            ..
        }) => (
            PageBackend::Pdfium,
            "pdf-oxide text contains replacement or private-use characters",
        ),
        Task::Extract(PageEvidence {
            oxide: OxideOutcome::Read {
                unmapped_font: false,
            },
            text: TextShape::Plain,
        }) => (PageBackend::PdfOxide, "text mapped cleanly"),
    }
}

/// Non-whitespace characters and U+FFFD characters of `text`.
fn char_counts(text: &str) -> (usize, usize) {
    let mut chars = 0;
    let mut fffd = 0;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        chars += 1;
        if ch == '\u{fffd}' {
            fffd += 1;
        }
    }
    (chars, fffd)
}

/// Is `page` a scan: text below [`MIN_TEXT_CHARS`] and a raster covering
/// at least [`SCAN_COVER`] of the page? Uses the spans when the page has
/// not been ordered yet (empty `text`).
#[must_use]
pub fn looks_scanned(page: &PageText) -> bool {
    let text: String = if page.text.is_empty() {
        page.spans.iter().map(|s| s.text.as_str()).collect()
    } else {
        page.text.clone()
    };
    let (chars, _) = char_counts(&text);
    if chars >= MIN_TEXT_CHARS {
        return false;
    }
    let area = page.width * page.height;
    if area <= 0.0 {
        return false;
    }
    page.figures.iter().any(|f| {
        f.kind == "raster"
            && f.bbox.is_some_and(|b| {
                let cover = ((b.x1 - b.x0).max(0.0) * (b.y1 - b.y0).max(0.0)) / area;
                cover >= SCAN_COVER
            })
    })
}

/// Did some of `page`'s text come from a font the backend could not map to
/// Unicode? True on the backend's own warning, or on a share of U+FFFD.
#[must_use]
pub fn has_unmapped_text(page: &PageText) -> bool {
    if page.warnings.iter().any(|w| {
        w.starts_with("unicode_mapping:")
            || w.contains("undecodable")
            || w.contains("decoded as Latin-1")
    }) {
        return true;
    }
    let text: String = if page.text.is_empty() {
        page.spans.iter().map(|s| s.text.as_str()).collect()
    } else {
        page.text.clone()
    };
    let (chars, fffd) = char_counts(&text);
    chars > 0 && (fffd as f32) / (chars as f32) >= FFFD_SHARE
}

/// Record unresolved extraction evidence before ordering/cleanup can remove
/// short spans. A blank page with no large raster remains complete. These
/// diagnostics describe uncertainty; they do not claim that OCR recovered it.
pub fn mark_incomplete(page: &mut PageText) {
    if looks_scanned(page)
        && !page
            .warnings
            .iter()
            .any(|warning| warning.starts_with("extraction_incomplete:"))
    {
        page.warnings.push(
            "extraction_incomplete: page-sized raster with insufficient extracted text; OCR completeness unverified"
                .to_string(),
        );
    }
    if has_unmapped_text(page)
        && !page
            .warnings
            .iter()
            .any(|warning| warning.starts_with("unicode_mapping:"))
    {
        page.warnings
            .push("unicode_mapping: extracted text has unresolved character mappings".to_string());
    }
}

/// Assess `pages` (any subset of a document, ordered or not).
#[must_use]
pub fn assess(pages: &[PageText]) -> Assessment {
    let mut a = Assessment {
        pages: pages.len(),
        ..Assessment::default()
    };
    for page in pages {
        let scanned = looks_scanned(page);
        let unmapped = has_unmapped_text(page);
        if scanned {
            a.scanned += 1;
        }
        if unmapped {
            a.unmapped += 1;
        }
        if !scanned && !unmapped {
            let text: String = if page.text.is_empty() {
                page.spans.iter().map(|s| s.text.as_str()).collect()
            } else {
                page.text.clone()
            };
            if char_counts(&text).0 < MIN_TEXT_CHARS {
                a.empty += 1;
            }
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{BBox, Figure, Span};

    fn page(text: &str) -> PageText {
        let mut p = PageText::new(1, 600.0, 800.0, 0);
        p.spans.push(Span {
            text: text.to_string(),
            bbox: Some(BBox {
                x0: 50.0,
                y0: 700.0,
                x1: 300.0,
                y1: 712.0,
            }),
            font: None,
            size: Some(10.0),
            seq: 0,
        });
        p
    }

    fn raster(cover: f32) -> Figure {
        Figure {
            index: 0,
            bbox: Some(BBox {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: 800.0 * cover,
            }),
            kind: "raster".to_string(),
            mime: None,
            width_px: Some(2000),
            height_px: Some(2600),
            sha256: None,
            file: None,
            caption: None,
        }
    }

    #[test]
    fn mapped_text_stays_on_lopdf() {
        let p = page("This page has ordinary mapped text of some length.");
        assert!(!looks_scanned(&p));
        assert!(!has_unmapped_text(&p));
        assert_eq!(assess(&[p]).route(), Route::Lopdf);
    }

    #[test]
    fn page_sized_raster_without_text_is_a_scan() {
        let mut p = page("");
        p.figures.push(raster(0.9));
        assert!(looks_scanned(&p));
        assert_eq!(assess(std::slice::from_ref(&p)).route(), Route::Docling);
        let mut small = page("");
        small.figures.push(raster(0.2));
        assert!(!looks_scanned(&small));
        assert_eq!(assess(&[small]).empty, 1);
    }

    #[test]
    fn undecodable_font_warning_routes_to_pdfium() {
        let mut p = page("Readable text next to a font that failed.");
        p.warnings
            .push("font F3: undecodable; U+FFFD substituted".to_string());
        assert!(has_unmapped_text(&p));
        let a = assess(std::slice::from_ref(&p));
        assert_eq!(a.route(), Route::Pdfium);
        assert_eq!(a.route_after_pdfium(), Route::Pdfium);
    }

    #[test]
    fn replacement_share_routes_to_pdfium() {
        let p = page("abc\u{fffd}\u{fffd}\u{fffd}defghijklmnopqrstuvwxyz");
        assert!(has_unmapped_text(&p));
        let clean = page(&format!(
            "one stray \u{fffd} then fine text {}",
            "and on ".repeat(60)
        ));
        assert!(!has_unmapped_text(&clean));
    }

    #[test]
    fn scan_with_unmapped_text_preserves_both_signals() {
        let mut p = page("bad \u{fffd}");
        p.figures.push(raster(0.9));
        mark_incomplete(&mut p);
        mark_incomplete(&mut p);
        assert_eq!(p.warnings.len(), 2);
        let assessment = assess(&[p]);
        assert_eq!(assessment.scanned, 1);
        assert_eq!(assessment.unmapped, 1);
        assert_eq!(assessment.route(), Route::Pdfium);
    }

    #[test]
    fn page_route_is_the_documented_match() {
        assert_eq!(page_route(Task::Render), (PageBackend::Pdfium, "render"));
        let clean = PageEvidence::default();
        assert_eq!(
            page_route(Task::Extract(&clean)),
            (PageBackend::PdfOxide, "text mapped cleanly")
        );
        let unmapped = OxideOutcome::Read {
            unmapped_font: true,
        };
        for (evidence, reason) in [
            (
                PageEvidence {
                    oxide: OxideOutcome::Failed,
                    text: TextShape::Empty,
                },
                "pdf-oxide failed to read the page",
            ),
            (
                PageEvidence {
                    oxide: unmapped,
                    ..clean
                },
                "a font has no usable Unicode mapping",
            ),
            (
                PageEvidence {
                    text: TextShape::Empty,
                    ..clean
                },
                "pdf-oxide text is empty",
            ),
            (
                PageEvidence {
                    text: TextShape::ReplacementOrPrivateUse,
                    ..clean
                },
                "pdf-oxide text contains replacement or private-use characters",
            ),
        ] {
            assert_eq!(
                page_route(Task::Extract(&evidence)),
                (PageBackend::Pdfium, reason)
            );
        }
        // Precedence: an unmapped font is reported before an empty page.
        let both = PageEvidence {
            oxide: unmapped,
            text: TextShape::Empty,
        };
        assert_eq!(
            page_route(Task::Extract(&both)).1,
            "a font has no usable Unicode mapping"
        );
        assert_eq!(PageBackend::PdfOxide.backend_name(), "pdf-oxide");
        assert_eq!(PageBackend::Pdfium.backend_name(), "pdfium");
    }

    #[test]
    fn page_evidence_reads_pdf_oxide_output() {
        let failed = PageEvidence::from_oxide(Err(&BackendError::Malformed("x".into())));
        assert_eq!(failed.oxide, OxideOutcome::Failed);
        let clean = PageEvidence::from_oxide(Ok(&page("Readable text of a page.")));
        assert_eq!(clean, PageEvidence::default());
        let empty = PageEvidence::from_oxide(Ok(&page("   \n ")));
        assert_eq!(empty.text, TextShape::Empty);
        let private = PageEvidence::from_oxide(Ok(&page("abc\u{e001}def")));
        assert_eq!(private.text, TextShape::ReplacementOrPrivateUse);
        let replaced = PageEvidence::from_oxide(Ok(&page("abc\u{fffd}def")));
        assert_eq!(replaced.text, TextShape::ReplacementOrPrivateUse);
        let mut unmapped = page("abc");
        unmapped.warnings.push(
            "extraction_incomplete: pdf-oxide to_unicode_missing: font F1 has no ToUnicode".into(),
        );
        assert_eq!(
            PageEvidence::from_oxide(Ok(&unmapped)).oxide,
            OxideOutcome::Read {
                unmapped_font: true
            }
        );
        assert!(is_replacement_or_private_use('\u{f8ff}'));
        assert!(is_replacement_or_private_use('\u{10fffd}'));
        assert!(!is_replacement_or_private_use('a'));
        assert!(!is_replacement_or_private_use('\u{e9}'));
    }

    #[test]
    fn route_names_match_backends() {
        assert_eq!(Route::Lopdf.backend_name(), "lopdf");
        assert_eq!(Route::Pdfium.backend_name(), "pdfium");
        assert_eq!(Route::Docling.backend_name(), "docling");
        assert_eq!(Route::MuPdf.backend_name(), "mupdf");
        assert_eq!(Route::Poppler.backend_name(), "poppler");
    }

    #[test]
    fn optional_native_routes_require_both_explicit_library_files() {
        let file = tempfile::NamedTempFile::new().unwrap();
        for route in [Route::MuPdf, Route::Poppler] {
            assert!(!native_runtime_configured(route, |_| None));
            assert!(!native_runtime_configured(route, |_| Some(
                "libnative.so".into()
            )));
            assert!(!native_runtime_configured(route, |name| {
                name.starts_with("TPE_")
                    .then(|| file.path().as_os_str().to_owned())
            }));
            assert!(native_runtime_configured(route, |_| {
                Some(file.path().as_os_str().to_owned())
            }));
            assert!(!native_runtime_configured(route, |_| {
                Some(file.path().parent().unwrap().as_os_str().to_owned())
            }));
        }
        assert!(native_runtime_configured(Route::Pdfium, |_| None));
    }
}
