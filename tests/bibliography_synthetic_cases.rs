//! Synthetic page-text regressions for reference-list extraction: heading
//! languages and cases, running heads inside a list, label styles, entries
//! that span page breaks and columns, de-hyphenation, identifiers kept whole
//! across line wraps, and field parsing. Every case is a small page built
//! from positioned lines; no PDF bytes or network are involved.
//!
//! The cases double as the offline scorer quoted in `docs/BIBLIOGRAPHY.md`:
//! each `#[test]` is one case, so the pass count is the score.

use tpe::citations::{extract_citations, find_reference_sections, parse_entry};
use tpe::schema::{BBox, Line, PageText, ReferenceEntry};

/// Left edge of an entry start and of its hanging-indent continuation.
const START_X: f32 = 72.0;
const HANG_X: f32 = 90.0;
/// Left edges of the second column.
const COL2_START_X: f32 = 320.0;
const COL2_HANG_X: f32 = 338.0;
/// First baseline of the body (below the 8% top margin band).
const TOP_Y: f32 = 700.0;
const LEADING: f32 = 14.0;

/// A small count as `f32` (lossless through `u16`), for page geometry.
fn to_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// A 10 pt line at `x0` with its baseline at `y`.
fn line(text: &str, column: u32, x0: f32, y: f32) -> Line {
    let width = to_f32(text.chars().count()) * 5.0;
    Line {
        text: text.to_string(),
        bbox: Some(BBox {
            x0,
            y0: y,
            x1: x0 + width,
            y1: y + 10.0,
        }),
        column,
        spans: Vec::new(),
        role: tpe::schema::default_line_role(),
    }
}

/// A page whose text is its lines joined by newlines.
fn page_of(number: u32, lines: Vec<Line>) -> PageText {
    let mut page = PageText::new(number, 612.0, 792.0, 0);
    page.text = lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<&str>>()
        .join("\n");
    page.lines = lines;
    page
}

/// A single-column page: `(text, x0)` rows laid out top to bottom.
fn page(number: u32, rows: &[(&str, f32)]) -> PageText {
    let lines = rows
        .iter()
        .enumerate()
        .map(|(i, (text, x0))| line(text, 0, *x0, TOP_Y - LEADING * to_f32(i)))
        .collect();
    page_of(number, lines)
}

/// A two-column page: left rows then right rows, each laid out top to bottom.
fn two_column_page(number: u32, left: &[(&str, f32)], right: &[(&str, f32)]) -> PageText {
    let mut lines: Vec<Line> = left
        .iter()
        .enumerate()
        .map(|(i, (text, x0))| line(text, 0, *x0, TOP_Y - LEADING * to_f32(i)))
        .collect();
    lines.extend(
        right
            .iter()
            .enumerate()
            .map(|(i, (text, x0))| line(text, 1, *x0, TOP_Y - LEADING * to_f32(i))),
    );
    page_of(number, lines)
}

fn raws(entries: &[ReferenceEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.raw.as_str()).collect()
}

fn parsed(raw: &str, label: Option<&str>) -> ReferenceEntry {
    let mut entry = ReferenceEntry {
        index: 1,
        label: label.map(str::to_string),
        raw: raw.to_string(),
        page: 1,
        ..ReferenceEntry::default()
    };
    parse_entry(&mut entry);
    entry
}

/// Three numbered entries under `heading`, with a body paragraph before it.
fn numbered_list_under(heading: &str) -> Vec<PageText> {
    vec![page(
        1,
        &[
            ("The last paragraph of the paper ends here.", START_X),
            (heading, START_X),
            (
                "[1] A. Author and B. Writer. A first title. Journal of Tests, 12(3):45–67, 2019.",
                START_X,
            ),
            (
                "[2] C. Coder. A second title. In Proceedings of the Conference, pages 1–9, 2020.",
                START_X,
            ),
            (
                "[3] D. Doe. A third title. Technical report, 2021.",
                START_X,
            ),
        ],
    )]
}

fn assert_heading_opens_list(heading: &str) {
    let pages = numbered_list_under(heading);
    let sections = find_reference_sections(&pages);
    assert_eq!(sections.len(), 1, "{heading}: {sections:?}");
    assert_eq!(sections[0].heading, heading);
    assert_eq!(sections[0].first_line, 1, "{heading}");
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{heading}: {:?}", raws(&refs));
    assert_eq!(refs[0].label.as_deref(), Some("[1]"));
    assert_eq!(refs[2].year, Some(2021));
}

// ---------------------------------------------------------------------------
// Heading detection: languages and cases
// ---------------------------------------------------------------------------

#[test]
fn german_headings_open_a_list() {
    for heading in [
        "Literaturverzeichnis",
        "Literatur",
        "LITERATURVERZEICHNIS",
        "7 Literaturverzeichnis",
        "Quellenverzeichnis",
        "Quellen",
    ] {
        assert_heading_opens_list(heading);
    }
}

#[test]
fn french_headings_open_a_list() {
    for heading in [
        "Références",
        "Références bibliographiques",
        "Bibliographie",
        "RÉFÉRENCES",
        "6. Références",
    ] {
        assert_heading_opens_list(heading);
    }
}

#[test]
fn spanish_and_portuguese_headings_open_a_list() {
    for heading in [
        "Referencias",
        "Referencias bibliográficas",
        "Bibliografía",
        "REFERENCIAS",
        "Referências",
        "Referências Bibliográficas",
        "Bibliografia",
    ] {
        assert_heading_opens_list(heading);
    }
}

#[test]
fn italian_and_dutch_headings_open_a_list() {
    for heading in [
        "Riferimenti bibliografici",
        "Bibliografia",
        "Referenties",
        "Literatuur",
        "Literatuurlijst",
    ] {
        assert_heading_opens_list(heading);
    }
}

#[test]
fn letter_spaced_and_variant_english_headings_open_a_list() {
    for heading in [
        "R E F E R E N C E S",
        "B I B L I O G R A P H Y",
        "References Cited",
        "Literature",
        "Sources",
        "References.",
        "REFERENCES AND NOTES",
        "Selected Bibliography",
        "Cited literature",
    ] {
        assert_heading_opens_list(heading);
    }
}

/// Heading words inside prose, and a table-of-contents line that no entry
/// follows, open no list in any language: only the real heading does.
#[test]
fn foreign_heading_words_in_prose_are_not_headings() {
    let mut pages = vec![page(
        1,
        &[
            ("Die Literatur zu diesem Thema ist umfangreich.", START_X),
            ("Les références sont citées dans le texte.", START_X),
            ("Inhalt", START_X),
            ("Literaturverzeichnis", START_X),
            ("Anhang", START_X),
            ("Einleitung", START_X),
            ("Der Text beginnt hier.", START_X),
        ],
    )];
    pages.extend(numbered_list_under("Literaturverzeichnis"));
    pages[1].page = 2;
    let sections = find_reference_sections(&pages);
    assert_eq!(sections.len(), 1, "{sections:?}");
    assert_eq!(sections[0].first_page, 2);
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
}

// ---------------------------------------------------------------------------
// Running heads inside the list
// ---------------------------------------------------------------------------

/// A journal prints `References` as the running head of every page of the
/// list. The entry that spans the page break must keep its continuation,
/// and the list must stay one list (the backward scan would otherwise stop
/// at the last page's running head).
#[test]
fn running_head_references_does_not_split_the_list() {
    let first = page_of(
        2,
        vec![
            line("Journal of Tests 12 (2020)", 0, START_X, 770.0),
            line("Final body sentence.", 0, START_X, 700.0),
            line("References", 0, START_X, 686.0),
            line(
                "[1] A. Author. A first title. Journal of Tests, 12:45–67, 2019.",
                0,
                START_X,
                672.0,
            ),
            line(
                "[2] B. Writer. A second title that wraps to the next page. In Pro-",
                0,
                START_X,
                658.0,
            ),
        ],
    );
    let second = page_of(
        3,
        vec![
            line("References", 0, START_X, 770.0),
            line(
                "ceedings of the Conference on Testing, pages 1–9, 2020.",
                0,
                HANG_X,
                700.0,
            ),
            line(
                "[3] C. Coder. A third title. Technical report, 2021.",
                0,
                START_X,
                686.0,
            ),
            line(
                "[4] D. Doe. A fourth title. Technical report, 2022.",
                0,
                START_X,
                672.0,
            ),
        ],
    );
    let pages = vec![first, second];
    let sections = find_reference_sections(&pages);
    assert_eq!(
        sections.len(),
        1,
        "the running head is not a second heading: {sections:?}"
    );
    assert_eq!(sections[0].first_page, 2);
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 4, "{:?}", raws(&refs));
    assert_eq!(
        refs[1].raw,
        "[2] B. Writer. A second title that wraps to the next page. In Proceedings of the Conference on Testing, pages 1–9, 2020."
    );
    assert_eq!(refs[1].pages.as_deref(), Some("1–9"));
}

/// The running head `REFERENCES` set in capitals at the page top, with the
/// folio on the same row, over an author-year list.
#[test]
fn capitalised_running_head_with_folio_over_author_year_list() {
    let first = page_of(
        5,
        vec![
            line("REFERENCES", 0, START_X, 770.0),
            line(
                "Smith, A. (2019). A first title. Journal of Tests, 12, 45–67.",
                0,
                START_X,
                700.0,
            ),
            line(
                "Writer, B., & Coder, C. (2020). A second title that is long",
                0,
                START_X,
                686.0,
            ),
        ],
    );
    let second = page_of(
        6,
        vec![
            line("REFERENCES", 0, START_X, 770.0),
            line("54", 0, 500.0, 770.0),
            line(
                "enough to wrap. Journal of Tests, 13, 1–9.",
                0,
                HANG_X,
                700.0,
            ),
            line(
                "Doe, D. (2021). A third title. Technical report.",
                0,
                START_X,
                686.0,
            ),
            line(
                "Eve, E. (2022). A fourth title. Technical report.",
                0,
                START_X,
                672.0,
            ),
        ],
    );
    let heading = page_of(
        4,
        vec![
            line("Final body sentence.", 0, START_X, 700.0),
            line("REFERENCES", 0, START_X, 686.0),
            line(
                "Adams, Z. (2018). An opening title. Journal of Tests, 11, 1–2.",
                0,
                START_X,
                672.0,
            ),
        ],
    );
    let pages = vec![heading, first, second];
    let sections = find_reference_sections(&pages);
    assert_eq!(sections.len(), 1, "{sections:?}");
    assert_eq!(sections[0].first_page, 4);
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 5, "{:?}", raws(&refs));
    assert!(
        refs[2]
            .raw
            .ends_with("enough to wrap. Journal of Tests, 13, 1–9."),
        "{}",
        refs[2].raw
    );
}

// ---------------------------------------------------------------------------
// Label styles and segmentation
// ---------------------------------------------------------------------------

#[test]
fn parenthesised_labels_segment_the_list() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            (
                "(1) A. Author and B. Writer. A first title. Journal of Tests, 12(3):45–67, 2019.",
                START_X,
            ),
            (
                "(2) C. Coder. A second title. In Proceedings of the Conference,",
                START_X,
            ),
            ("pages 1–9, 2020.", HANG_X),
            (
                "(3) D. Doe. A third title. Technical report, 2021.",
                START_X,
            ),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
    assert_eq!(refs[0].label.as_deref(), Some("(1)"));
    assert_eq!(
        refs[1].raw,
        "(2) C. Coder. A second title. In Proceedings of the Conference, pages 1–9, 2020."
    );
    assert_eq!(refs[1].year, Some(2020));
    assert_eq!(refs[1].title.as_deref(), Some("A second title"));
}

/// `1.` labels with a continuation that opens with a number
/// (`2020;12(3):45–67.`) do not start a new entry.
#[test]
fn dot_labels_with_numeric_continuations() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            ("1. Smith J, Jones K. A first title. J Biol Chem.", START_X),
            ("2019;294(12):4567–78.", HANG_X),
            ("2. Coder C. A second title. Nature.", START_X),
            ("2020;580:1–9. doi:10.1038/s41586-020-1234-5", HANG_X),
            (
                "3. Doe D. A third title. Science. 2021;371:100–101.",
                START_X,
            ),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
    assert_eq!(refs[0].volume.as_deref(), Some("294"));
    assert_eq!(refs[0].issue.as_deref(), Some("12"));
    assert_eq!(refs[0].pages.as_deref(), Some("4567–78"));
    assert_eq!(refs[1].doi.as_deref(), Some("10.1038/s41586-020-1234-5"));
}

/// An entry that spans a page break across a footer and a hyphenated word.
#[test]
fn entry_spanning_a_page_break_rejoins_its_hyphenated_word() {
    let first = page_of(
        1,
        vec![
            line("Final body sentence.", 0, START_X, 700.0),
            line("References", 0, START_X, 686.0),
            line(
                "[1] A. Author. A first title. Journal of Tests, 12:45–67, 2019.",
                0,
                START_X,
                672.0,
            ),
            line("[2] B. Writer. Learning the recon-", 0, START_X, 658.0),
            line("12", 0, 300.0, 40.0),
        ],
    );
    let second = page_of(
        2,
        vec![
            line(
                "struction of surfaces. Journal of Tests, 13:1–9, 2020.",
                0,
                HANG_X,
                700.0,
            ),
            line(
                "[3] C. Coder. A third title. Technical report, 2021.",
                0,
                START_X,
                686.0,
            ),
            line("13", 0, 300.0, 40.0),
        ],
    );
    let (refs, _) = extract_citations(&[first, second]);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
    assert_eq!(
        refs[1].raw,
        "[2] B. Writer. Learning the reconstruction of surfaces. Journal of Tests, 13:1–9, 2020."
    );
    assert_eq!(
        refs[1].title.as_deref(),
        Some("Learning the reconstruction of surfaces")
    );
    assert_eq!(refs[1].page, 1);
    assert_eq!(refs[2].page, 2);
}

/// Author-year entries in two columns: the entry at the foot of the left
/// column continues at the head of the right column.
#[test]
fn author_year_entries_continue_across_columns() {
    let pages = vec![two_column_page(
        1,
        &[
            ("References", START_X),
            (
                "Adams, A. (2018). A first title. Journal of Tests,",
                START_X,
            ),
            ("11(2), 1–2.", HANG_X),
            ("Brown, B., & Coder, C. (2019). A second title", START_X),
            ("that wraps. Journal of Tests, 12(3),", HANG_X),
            ("45–67.", HANG_X),
            ("Doe, D. (2020). A third title whose venue", START_X),
        ],
        &[
            ("sits in the other column. Journal of", COL2_HANG_X),
            ("Tests, 13(1), 1–9.", COL2_HANG_X),
            ("Eve, E. (2021). A fourth title. Technical", COL2_START_X),
            ("report, University of Testing.", COL2_HANG_X),
            ("Frank, F. (2022). A fifth title. Journal of", COL2_START_X),
            ("Tests, 14(1), 10–19.", COL2_HANG_X),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 5, "{:?}", raws(&refs));
    assert_eq!(
        refs[2].raw,
        "Doe, D. (2020). A third title whose venue sits in the other column. Journal of Tests, 13(1), 1–9."
    );
    assert_eq!(refs[2].label.as_deref(), Some("Doe2020"));
    assert_eq!(refs[2].pages.as_deref(), Some("1–9"));
    assert_eq!(refs[4].year, Some(2022));
}

/// Numbered footnotes at the foot of the last body page are not a
/// heading-less reference list.
#[test]
fn numbered_footnotes_are_not_a_reference_list() {
    let pages = vec![page(
        1,
        &[
            (
                "The body of the paper discusses the result in detail.",
                START_X,
            ),
            ("1 See the appendix for the full derivation.", START_X),
            (
                "2 The dataset is available from the authors on request.",
                START_X,
            ),
            ("3 Cf. the discussion in Section 2.", START_X),
        ],
    )];
    assert!(find_reference_sections(&pages).is_empty());
    let (refs, _) = extract_citations(&pages);
    assert!(refs.is_empty(), "{:?}", raws(&refs));
}

// ---------------------------------------------------------------------------
// De-hyphenation and line joining
// ---------------------------------------------------------------------------

#[test]
fn hyphens_at_line_ends_are_resolved_by_policy() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            ("[1] A. Author. Multi-task learning with self-", START_X),
            (
                "supervised objectives. Journal of Tests, 12:45–67, 2019.",
                HANG_X,
            ),
            ("[2] B. Writer. On the approxi-", START_X),
            (
                "mation of functions. Journal of Tests, 13:1–9, 2020.",
                HANG_X,
            ),
            (
                "[3] C. Coder. A third title. https://doi.org/10.1038/s41586-",
                START_X,
            ),
            ("020-1234-5, 2021.", HANG_X),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
    assert!(
        refs[0].raw.contains("self-supervised objectives"),
        "{}",
        refs[0].raw
    );
    assert!(
        refs[1].raw.contains("approximation of functions"),
        "{}",
        refs[1].raw
    );
    assert_eq!(refs[2].doi.as_deref(), Some("10.1038/s41586-020-1234-5"));
}

// ---------------------------------------------------------------------------
// Identifiers kept whole across line wraps
// ---------------------------------------------------------------------------

#[test]
fn arxiv_ids_survive_line_wraps_and_old_style_ids_parse() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            (
                "[1] A. Author. A first title. arXiv preprint arXiv:2301.",
                START_X,
            ),
            ("12345, 2023.", HANG_X),
            (
                "[2] B. Writer. A second title. arXiv:hep-th/9901001, 1999.",
                START_X,
            ),
            (
                "[3] C. Coder. A third title. https://arxiv.org/abs/",
                START_X,
            ),
            ("2105.00001v2, 2021.", HANG_X),
            ("[4] D. Doe. A fourth title. arXiv preprint", START_X),
            ("arXiv:1706.03762, 2017.", HANG_X),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 4, "{:?}", raws(&refs));
    assert_eq!(refs[0].arxiv_id.as_deref(), Some("2301.12345"));
    assert_eq!(refs[0].year, Some(2023));
    assert_eq!(refs[1].arxiv_id.as_deref(), Some("hep-th/9901001"));
    assert_eq!(refs[2].arxiv_id.as_deref(), Some("2105.00001v2"));
    assert_eq!(refs[3].arxiv_id.as_deref(), Some("1706.03762"));
    assert_eq!(refs[3].title.as_deref(), Some("A fourth title"));
}

#[test]
fn dois_and_urls_survive_line_wraps() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            (
                "[1] A. Author. A first title. Journal of Tests, 2019. doi:10.1007/",
                START_X,
            ),
            ("978-3-030-12345-6_7.", HANG_X),
            (
                "[2] B. Writer. A second title. Journal of Tests, 2020. https://doi.",
                START_X,
            ),
            ("org/10.1016/j.jmp.2013.05.005", HANG_X),
            (
                "[3] C. Coder. A third title. 2021. https://example.org/some-",
                START_X,
            ),
            ("path/file.pdf", HANG_X),
            (
                "[4] D. Doe. A fourth title. 2022. URL https://openreview.net/",
                START_X,
            ),
            ("forum?id=AbC123.", HANG_X),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 4, "{:?}", raws(&refs));
    assert_eq!(refs[0].doi.as_deref(), Some("10.1007/978-3-030-12345-6_7"));
    assert_eq!(refs[1].doi.as_deref(), Some("10.1016/j.jmp.2013.05.005"));
    assert_eq!(
        refs[2].url.as_deref(),
        Some("https://example.org/some-path/file.pdf")
    );
    assert_eq!(
        refs[3].url.as_deref(),
        Some("https://openreview.net/forum?id=AbC123")
    );
    assert_eq!(refs[3].title.as_deref(), Some("A fourth title"));
}

// ---------------------------------------------------------------------------
// Field parsing
// ---------------------------------------------------------------------------

#[test]
fn chicago_author_date_entry_parses() {
    let entry = parsed(
        "Smith, John, and Jane Doe. 2020. “A Title of Note.” Journal of Things 12 (3): 45–67. https://doi.org/10.1000/j.123.",
        None,
    );
    // Names are kept as printed (surname-first for the first author).
    assert_eq!(entry.authors, vec!["Smith, John", "Jane Doe"]);
    assert_eq!(entry.year, Some(2020));
    assert_eq!(entry.title.as_deref(), Some("A Title of Note"));
    assert_eq!(entry.doi.as_deref(), Some("10.1000/j.123"));
    assert_eq!(entry.volume.as_deref(), Some("12"));
    assert_eq!(entry.issue.as_deref(), Some("3"));
    assert_eq!(entry.pages.as_deref(), Some("45–67"));
}

#[test]
fn vancouver_entry_parses() {
    let entry = parsed(
        "1. Smith J, Jones K. A title of work. J Biol Chem. 2020;295(12):4567-78. doi:10.1074/jbc.2020.123456",
        Some("1."),
    );
    assert_eq!(entry.authors, vec!["Smith J", "Jones K"]);
    assert_eq!(entry.title.as_deref(), Some("A title of work"));
    assert_eq!(entry.year, Some(2020));
    assert_eq!(entry.volume.as_deref(), Some("295"));
    assert_eq!(entry.issue.as_deref(), Some("12"));
    // Page ranges are reported with an en dash whatever the print used.
    assert_eq!(entry.pages.as_deref(), Some("4567–78"));
    assert_eq!(entry.doi.as_deref(), Some("10.1074/jbc.2020.123456"));
}

#[test]
fn ieee_entry_with_wrapped_quoted_title_parses() {
    let pages = vec![page(
        1,
        &[
            ("References", START_X),
            (
                "[1] A. B. Author and C. Writer, “A quoted title that wraps onto",
                START_X,
            ),
            (
                "the next line,” IEEE Trans. Tests, vol. 12, no. 3, pp. 45–67, Mar.",
                HANG_X,
            ),
            ("2019.", HANG_X),
            (
                "[2] D. Doe, “A second title,” in Proc. Conf. Testing, 2020, pp. 1–9.",
                START_X,
            ),
            (
                "[3] E. Eve, “A third title,” Tech. Rep. 7, Univ. of Testing, 2021.",
                START_X,
            ),
        ],
    )];
    let (refs, _) = extract_citations(&pages);
    assert_eq!(refs.len(), 3, "{:?}", raws(&refs));
    assert_eq!(
        refs[0].title.as_deref(),
        Some("A quoted title that wraps onto the next line")
    );
    assert_eq!(refs[0].year, Some(2019));
    assert_eq!(refs[0].volume.as_deref(), Some("12"));
    assert_eq!(refs[0].issue.as_deref(), Some("3"));
    assert_eq!(refs[0].pages.as_deref(), Some("45–67"));
    assert_eq!(refs[1].pages.as_deref(), Some("1–9"));
}

#[test]
fn german_style_entry_with_in_and_seiten_parses() {
    let entry = parsed(
        "Müller, H. (2019): Ein Titel des Werkes. In: Zeitschrift für Tests 12, S. 45–67.",
        None,
    );
    assert_eq!(entry.authors, vec!["Müller, H."]);
    assert_eq!(entry.year, Some(2019));
    assert_eq!(entry.title.as_deref(), Some("Ein Titel des Werkes"));
    assert_eq!(entry.pages.as_deref(), Some("45–67"));
}
