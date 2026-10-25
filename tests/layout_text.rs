//! Independent physical-layout goldens through real lopdf extraction.

use tpe::backend::Extractor;
use tpe::backend::lopdf_backend::LopdfBackend;
use tpe::layout_text::{LayoutError, LayoutIssue, LayoutOptions, LayoutText, render_page};
use tpe::schema::{BBox, PageText, Span};

fn extracted(bytes: &[u8]) -> PageText {
    LopdfBackend::default()
        .open(bytes, None)
        .unwrap()
        .page_text(1)
        .unwrap()
}

fn fixed() -> LayoutOptions {
    LayoutOptions {
        column_pitch: Some(6.0),
        row_pitch: Some(12.0),
        ..LayoutOptions::default()
    }
}

fn golden(text: &str) -> &str {
    text.strip_suffix('\n').unwrap()
}

fn conserved(page: &PageText, output: &LayoutText) {
    let mut seen = vec![false; page.spans.len()];
    let mut end = 0;
    let mut source_chars = Vec::new();
    for placement in &output.placements {
        assert!(!seen[placement.span_index], "duplicate span");
        seen[placement.span_index] = true;
        assert!(placement.bytes.start >= end, "overlapping byte ranges");
        let raw = &page.spans[placement.span_index].text;
        assert_eq!(&output.text[placement.bytes.clone()], raw);
        assert!(
            output.text[end..placement.bytes.start]
                .chars()
                .all(|c| c == ' ' || c == '\n')
        );
        end = placement.bytes.end;
        source_chars.extend(raw.chars());
    }
    assert!(seen.into_iter().all(|s| s), "dropped span");
    assert!(output.text[end..].chars().all(|c| c == ' ' || c == '\n'));
    let mut input_chars: Vec<char> = page.spans.iter().flat_map(|s| s.text.chars()).collect();
    source_chars.sort_unstable();
    input_chars.sort_unstable();
    assert_eq!(source_chars, input_chars, "character multiset changed");
}

fn span(text: &str, x: f32, y: f32, seq: u32) -> Span {
    Span {
        text: text.into(),
        bbox: Some(BBox {
            x0: x,
            y0: y,
            x1: x + 12.0,
            y1: y + 10.0,
        }),
        font: None,
        size: Some(10.0),
        seq,
    }
}

fn page(spans: Vec<Span>) -> PageText {
    let mut page = PageText::new(1, 240.0, 180.0, 0);
    page.spans = spans;
    page
}

#[test]
fn columns_heading_indentation_table_and_gaps_match_independent_golden() {
    let page = extracted(include_bytes!("fixtures/layout_text/columns.pdf"));
    assert_eq!(page.spans.len(), 12);
    assert_eq!(
        page.spans[0].text, "R1",
        "stream order is deliberately adversarial"
    );
    let before = serde_json::to_vec(&page).unwrap();
    for options in [fixed(), LayoutOptions::default()] {
        let output = render_page(&page, options).unwrap();
        assert_eq!(
            output.text,
            golden(include_str!("fixtures/layout_text/columns.txt"))
        );
        assert!(output.placements.iter().all(|p| p.issue.is_none()));
        conserved(&page, &output);
        assert_eq!(
            serde_json::to_vec(&page).unwrap(),
            before,
            "evidence mutated"
        );
    }
}

#[test]
fn all_page_quarter_turns_use_real_lopdf_boxes_and_unicode() {
    for bytes in [
        include_bytes!("fixtures/layout_text/rotate-0.pdf").as_slice(),
        include_bytes!("fixtures/layout_text/rotate-90.pdf").as_slice(),
        include_bytes!("fixtures/layout_text/rotate-180.pdf").as_slice(),
        include_bytes!("fixtures/layout_text/rotate-270.pdf").as_slice(),
    ] {
        let page = extracted(bytes);
        let before = page.clone();
        assert_eq!(
            page.spans
                .iter()
                .map(|s| s.text.as_str())
                .collect::<Vec<_>>(),
            ["TOP", "é中😀"]
        );
        let output = render_page(&page, LayoutOptions::default()).unwrap();
        assert_eq!(
            output.text,
            golden(include_str!("fixtures/layout_text/rotated.txt"))
        );
        assert!(output.placements.iter().all(|p| p.issue.is_none()));
        conserved(&page, &output);
        assert_eq!(page, before);
    }
}

#[test]
fn overlaps_vertical_and_source_controls_are_preserved_and_reported() {
    let page = extracted(include_bytes!("fixtures/layout_text/special.pdf"));
    assert_eq!(page.spans[0].text, "é中😀");
    assert_eq!(page.spans[4].text, "a\tb\nc");
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(
        output.text,
        golden(include_str!("fixtures/layout_text/special.txt"))
    );
    let issues: Vec<_> = output.placements.iter().filter_map(|p| p.issue).collect();
    assert_eq!(
        issues,
        [
            LayoutIssue::OverlapShift,
            LayoutIssue::VerticalText,
            LayoutIssue::ControlText
        ]
    );
    conserved(&page, &output);
}

#[test]
fn rendering_ignores_cleaned_strings_and_keeps_full_page_immutable() {
    let mut page = extracted(include_bytes!("fixtures/layout_text/columns.pdf"));
    tpe::reading_order::order_page(&mut page);
    tpe::text_cleanup::clean_document(std::slice::from_mut(&mut page));
    page.text = "cleaned strings deliberately replaced".into();
    for line in &mut page.lines {
        line.text = "removed".into();
    }
    let before = serde_json::to_vec(&page).unwrap();
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(
        output.text,
        golden(include_str!("fixtures/layout_text/columns.txt"))
    );
    conserved(&page, &output);
    assert_eq!(serde_json::to_vec(&page).unwrap(), before);
}

#[test]
fn geometry_fallbacks_keep_whitespace_empty_spans_and_tied_sequences() {
    let mut missing = span("  raw  ", 12.0, 144.0, 1);
    missing.bbox = None;
    let mut invalid = span("NaN", 12.0, 144.0, 1);
    invalid.bbox.as_mut().unwrap().x0 = f32::NAN;
    let outside = span("outside", f32::MAX, 144.0, 2);
    let empty = span("", 12.0, 144.0, 0);
    let page = page(vec![missing, invalid, outside, empty]);
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(output.text, "  raw  \nNaN\noutside");
    assert_eq!(
        output
            .placements
            .iter()
            .map(|p| p.span_index)
            .collect::<Vec<_>>(),
        [3, 0, 1, 2]
    );
    assert_eq!(
        output.placements[1].issue,
        Some(LayoutIssue::MissingGeometry)
    );
    assert_eq!(
        output.placements[2].issue,
        Some(LayoutIssue::InvalidGeometry)
    );
    assert_eq!(output.placements[3].issue, Some(LayoutIssue::OutsidePage));
    conserved(&page, &output);
    assert!(page.spans[1].bbox.unwrap().x0.is_nan());
}

#[test]
fn same_box_ties_keep_sequence_then_input_index_without_deduplication() {
    let page = page(vec![
        span("B", 12.0, 144.0, 2),
        span("A", 12.0, 144.0, 1),
        span("A", 12.0, 144.0, 1),
    ]);
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(output.text, "A A B");
    conserved(&page, &output);
}

#[test]
fn subrow_drift_does_not_chain_distinct_rows() {
    let page = page(vec![
        span("A", 12.0, 144.0, 0),
        span("B", 30.0, 141.0, 1),
        span("C", 48.0, 138.0, 2),
    ]);
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(output.text, "A  B\n      C");
    conserved(&page, &output);
}

#[test]
fn combining_marks_cjk_emoji_and_replacement_are_verbatim_scalars() {
    let page = page(vec![
        span("e\u{301}中😀\u{fffd}", 12.0, 144.0, 0),
        span("Z", 48.0, 144.0, 1),
    ]);
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(output.text, "e\u{301}中😀\u{fffd} Z");
    conserved(&page, &output);
}

#[test]
fn invalid_options_dimensions_and_rotation_fail_without_mutation() {
    let base = page(vec![span("A", 12.0, 144.0, 0)]);
    for pitch in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        for options in [
            LayoutOptions {
                column_pitch: Some(pitch),
                ..fixed()
            },
            LayoutOptions {
                row_pitch: Some(pitch),
                ..fixed()
            },
        ] {
            assert_eq!(
                render_page(&base, options),
                Err(LayoutError::InvalidOptions)
            );
        }
    }
    for options in [
        LayoutOptions {
            max_columns: 0,
            ..fixed()
        },
        LayoutOptions {
            max_rows: 20_001,
            ..fixed()
        },
        LayoutOptions {
            max_spans: 100_001,
            ..fixed()
        },
        LayoutOptions {
            max_output_bytes: 8 * 1024 * 1024 + 1,
            ..fixed()
        },
    ] {
        assert_eq!(
            render_page(&base, options),
            Err(LayoutError::InvalidOptions)
        );
    }
    for dim in [0.0, -1.0, f32::INFINITY, f32::NAN] {
        let mut altered = base.clone();
        altered.width = dim;
        assert_eq!(
            render_page(&altered, fixed()),
            Err(LayoutError::InvalidPage)
        );
    }
    let mut altered = base.clone();
    altered.rotation = 45;
    assert_eq!(
        render_page(&altered, fixed()),
        Err(LayoutError::UnsupportedRotation)
    );
    altered.rotation = -90;
    assert!(render_page(&altered, fixed()).is_ok());
}

#[test]
fn span_input_and_generated_byte_limits_are_exact() {
    let page = page(vec![span("é", 12.0, 144.0, 0), span("中", 24.0, 144.0, 1)]);
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                max_spans: 1,
                ..fixed()
            }
        ),
        Err(LayoutError::SpanLimit)
    );
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                max_output_bytes: 4,
                ..fixed()
            }
        ),
        Err(LayoutError::OutputLimit)
    );
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                max_output_bytes: 5,
                ..fixed()
            }
        ),
        Err(LayoutError::OutputLimit)
    );
    let output = render_page(
        &page,
        LayoutOptions {
            max_output_bytes: 6,
            ..fixed()
        },
    )
    .unwrap();
    assert_eq!(output.text, "é 中");
    assert_eq!(output.text.len(), 6);
    conserved(&page, &output);
}

#[test]
fn positioned_and_fallback_output_respect_row_column_bounds() {
    let page = page(vec![span("AB", 12.0, 144.0, 0), span("CD", 30.0, 132.0, 1)]);
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                max_rows: 1,
                ..fixed()
            }
        ),
        Err(LayoutError::RowLimit)
    );
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                max_columns: 4,
                ..fixed()
            }
        ),
        Err(LayoutError::ColumnLimit)
    );
    let output = render_page(
        &page,
        LayoutOptions {
            max_rows: 2,
            max_columns: 5,
            ..fixed()
        },
    )
    .unwrap();
    assert_eq!(output.text, "AB\n   CD");
    let mut fallback = page.clone();
    fallback.spans[1].bbox = None;
    fallback.spans[1].text = "C\nD".into();
    assert_eq!(
        render_page(
            &fallback,
            LayoutOptions {
                max_rows: 3,
                ..fixed()
            }
        ),
        Err(LayoutError::RowLimit)
    );
    assert!(
        render_page(
            &fallback,
            LayoutOptions {
                max_rows: 4,
                ..fixed()
            }
        )
        .is_ok()
    );
    fallback.spans[1].text = "CDEFG".into();
    assert_eq!(
        render_page(
            &fallback,
            LayoutOptions {
                max_columns: 4,
                ..fixed()
            }
        ),
        Err(LayoutError::ColumnLimit)
    );
}

#[test]
fn extreme_finite_pages_and_tiny_pitches_do_not_allocate_coordinate_canvases() {
    let mut page = page(vec![
        span("A", 0.0, 0.0, 0),
        span("B", 0.0, f32::MAX / 2.0, 1),
    ]);
    page.width = f32::MAX;
    page.height = f32::MAX;
    assert_eq!(render_page(&page, fixed()), Err(LayoutError::RowLimit));
    page.spans[1].bbox.as_mut().unwrap().x0 = f32::MAX / 2.0;
    page.spans[1].bbox.as_mut().unwrap().x1 = f32::MAX / 2.0;
    page.spans[1].bbox.as_mut().unwrap().y0 = 0.0;
    page.spans[1].bbox.as_mut().unwrap().y1 = 10.0;
    assert_eq!(render_page(&page, fixed()), Err(LayoutError::ColumnLimit));
    page.width = 240.0;
    page.height = 180.0;
    page.spans[1] = span("B", 12.0, 0.0, 1);
    assert_eq!(
        render_page(
            &page,
            LayoutOptions {
                column_pitch: Some(f32::from_bits(1)),
                ..fixed()
            }
        ),
        Err(LayoutError::ColumnLimit)
    );
}

#[test]
fn many_coincident_spans_are_bounded_and_preserved_or_fail_closed() {
    let page = page((0..2000).map(|seq| span("X", 12.0, 144.0, seq)).collect());
    let output = render_page(
        &page,
        LayoutOptions {
            max_columns: 4096,
            ..fixed()
        },
    )
    .unwrap();
    assert_eq!(output.text.chars().count(), 3999);
    conserved(&page, &output);
    assert_eq!(render_page(&page, fixed()), Err(LayoutError::ColumnLimit));
}

#[test]
fn empty_page_and_empty_strings_have_no_generated_text() {
    let page = page(vec![span("", 12.0, 144.0, 0)]);
    let output = render_page(&page, fixed()).unwrap();
    assert_eq!(output.text, "");
    conserved(&page, &output);
    assert_eq!(
        render_page(&PageText::new(1, 240.0, 180.0, 0), fixed())
            .unwrap()
            .text,
        ""
    );
}
