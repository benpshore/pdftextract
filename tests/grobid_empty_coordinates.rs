#![cfg(feature = "grobid")]

use sha2::{Digest, Sha256};
use tpe::grobid::{GrobidError, parse_tei};

const LIVE_TEI: &str = include_str!("../docs/validation/grobid-cpu/rust-options.tei.xml");
const LIVE_SHA256: &str = "6486c5dbedc34d2c2226bff361b5b195740b164c0734e14009b0193b5dfe561f";

#[test]
fn real_cpu_service_empty_author_coordinates_preserve_evidence() {
    assert_eq!(
        hex::encode(Sha256::digest(LIVE_TEI.as_bytes())),
        LIVE_SHA256
    );
    let document = parse_tei(LIVE_TEI.to_owned(), None).unwrap();
    assert_eq!(document.raw_tei, LIVE_TEI);
    assert_eq!(document.tei_sha256, LIVE_SHA256);
    assert_eq!(document.coverage, "semantic_projection");
    assert_eq!(document.pages.len(), 2);
    assert_eq!(document.citations.len(), 2);
    let absent: Vec<_> = document
        .elements
        .iter()
        .filter(|element| {
            element
                .attributes
                .get("coords")
                .is_some_and(String::is_empty)
        })
        .collect();
    assert_eq!(absent.len(), 3);
    for element in absent {
        assert_eq!(element.kind, "persName");
        assert!(element.coordinates.is_empty());
        assert!(!element.text.is_empty());
        assert!(
            document.raw_tei[element.source_range[0]..element.source_range[1]]
                .contains("coords=\"\"")
        );
    }
    assert!(document.warnings.iter().any(|warning| {
        warning.contains("empty coordinate attributes")
            && warning.contains("no boxes were inferred")
    }));
}

fn minimal_tei(coordinates: &str) -> String {
    format!(
        r#"<TEI xmlns="http://www.tei-c.org/ns/1.0"><text><back><listBibl><biblStruct coords="{coordinates}"><analytic><title>Retained title</title></analytic></biblStruct></listBibl></back></text></TEI>"#
    )
}

#[test]
fn empty_reference_geometry_is_absent_with_warning_and_raw_attribute() {
    for value in ["", "   "] {
        let raw = minimal_tei(value);
        let document = parse_tei(raw.clone(), None).unwrap();
        assert_eq!(document.raw_tei, raw);
        assert_eq!(document.citations.len(), 1);
        assert!(document.citations[0].coordinates.is_empty());
        assert_eq!(document.elements[0].attributes["coords"], value);
        assert!(
            document
                .warnings
                .iter()
                .any(|w| w.contains("empty coordinate attributes"))
        );
    }
}

#[test]
fn nonempty_malformed_coordinates_still_fail() {
    for value in [
        ";",
        "1,2,3,4,5;",
        ";1,2,3,4,5",
        "1,2,3,4,5;;1,2,3,4,5",
        "1,2,3,4",
        "0,2,3,4,5",
        "1,NaN,3,4,5",
        "1,2,3,-4,5",
    ] {
        assert!(matches!(
            parse_tei(minimal_tei(value), None),
            Err(GrobidError::Tei(_))
        ));
    }
}
