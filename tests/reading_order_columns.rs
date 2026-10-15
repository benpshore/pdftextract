//! Independent two/three-column regression witnesses preserved from PR269.
//! Expectations are handwritten; fixture identity is recorded beside the PDFs.

use tpe::backend::Extractor;
use tpe::backend::lopdf_backend::LopdfBackend;

fn assert_order(bytes: &[u8], expected: &str) {
    let mut page = LopdfBackend::default()
        .open(bytes, None)
        .unwrap()
        .page_text(1)
        .unwrap();
    let evidence = page.spans.clone();
    tpe::reading_order::order_page(&mut page);
    assert_eq!(page.spans, evidence, "ordering changed raw evidence");
    let mut members: Vec<u32> = page
        .lines
        .iter()
        .flat_map(|l| l.spans.iter().copied())
        .collect();
    members.sort_unstable();
    assert_eq!(
        members,
        (0..u32::try_from(page.spans.len()).unwrap()).collect::<Vec<_>>(),
        "dropped or duplicated span"
    );
    let raw = page.text.clone();
    tpe::text_cleanup::clean_document(std::slice::from_mut(&mut page));
    eprintln!(
        "raw semantic output: {raw:?}\ncleaned semantic output: {:?}",
        page.text
    );
    assert_eq!(
        raw.split_whitespace().collect::<Vec<_>>(),
        expected.split_whitespace().collect::<Vec<_>>(),
        "raw semantic output: {raw:?}"
    );
    assert_eq!(
        page.text.split_whitespace().collect::<Vec<_>>(),
        expected.split_whitespace().collect::<Vec<_>>(),
        "cleaned semantic output: {:?}",
        page.text
    );
}

#[test]
fn tight_heading_then_two_columns() {
    assert_order(
        include_bytes!("fixtures/reading-order/semantic-two.pdf"),
        include_str!("fixtures/reading-order/semantic-two.txt"),
    );
}

#[test]
fn tight_heading_then_three_columns() {
    assert_order(
        include_bytes!("fixtures/reading-order/semantic-three.pdf"),
        include_str!("fixtures/reading-order/semantic-three.txt"),
    );
}

#[test]
fn heading_aligned_with_right_column_stays_with_its_prose() {
    // Shift the independently authored heading into the right column. The
    // fixed-length ASCII substitution preserves the fixture's xref offsets.
    let source = include_str!("fixtures/reading-order/semantic-two.pdf");
    assert_eq!(source.matches("102 144").count(), 1);
    let bytes = source.replace("102 144", "132 144");
    assert_order(
        bytes.as_bytes(),
        "left first row left second row HEADING right first row right second row",
    );
}
