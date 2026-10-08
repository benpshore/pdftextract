//! Independent semantic acceptance cases preserved for the clean rebuild.
//! These are deliberately ignored in the checkpoint suite: run --ignored
//! to reproduce unresolved legacy failures, not to claim a completed fix.

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
#[ignore = "known legacy reading-order failure; acceptance case for clean rebuild"]
fn tight_heading_then_two_columns() {
    assert_order(
        include_bytes!("fixtures/layout_text/semantic-two.pdf"),
        include_str!("fixtures/layout_text/semantic-two.txt"),
    );
}

#[test]
#[ignore = "known legacy reading-order failure; acceptance case for clean rebuild"]
fn tight_heading_then_three_columns() {
    assert_order(
        include_bytes!("fixtures/layout_text/semantic-three.pdf"),
        include_str!("fixtures/layout_text/semantic-three.txt"),
    );
}
