//! Selector mechanics tested with independently constructed PDF operators and
//! synthetic schema-5 records. These are not native-engine accuracy results.
//! The separate held-out evaluator owns rendered transcriptions; this test never
//! reads its labels, source inputs, or captured engine outputs.
use lopdf::{Document, Object, Stream, content::Content, dictionary};
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;
use tpe_region_evidence::{
    ArtifactStore, Attempt, BackendIdentity, Decision, Digest, GeometryFrame, Outcome,
    RuntimeIdentity, Sidecar, SourceIdentity,
};
use tpe_source_fusion::source::{SourceLimits, SourceOutcome, verify_source};
use tpe_source_fusion::{
    AbstentionReason, DerivedView, Limits, RegionDecision, RunOutcome, select,
};

const OLD_PDF: &[u8] = include_bytes!("../../region-fusion/fixtures/positive-stream-cmap.pdf");
const FONT: &[u8] = include_bytes!("../../region-fusion/fixtures/font/DejaVuSans-subset.ttf");

/// Give the old development PDF two completely specified Type0 fonts. This is
/// deliberately separate from the new frozen holdout inputs.
fn source_pdf(repeated: bool) -> Vec<u8> {
    let mut doc = Document::load_mem(OLD_PDF).unwrap();
    doc.objects.insert(
        (5, 0),
        Object::Dictionary(dictionary! {
            "Type" => "Font", "Subtype" => "Type0", "BaseFont" => "FixtureDejaVuSans",
            "Encoding" => "Identity-H", "DescendantFonts" => vec![Object::Reference((7, 0))],
            "ToUnicode" => Object::Reference((9, 0)),
        }),
    );
    let face = ttf_parser::Face::parse(FONT, 0).unwrap();
    let codes: Vec<u8> = "ALPHA"
        .chars()
        .flat_map(|ch| face.glyph_index(ch).unwrap().0.to_be_bytes())
        .collect();
    let original = doc
        .get_object((4, 0))
        .unwrap()
        .as_stream()
        .unwrap()
        .content
        .clone();
    let mut content = Content::decode(&original).unwrap();
    let first_show = content
        .operations
        .iter_mut()
        .find(|op| op.operator == "Tj")
        .unwrap();
    first_show.operands = vec![Object::String(codes, lopdf::StringFormat::Hexadecimal)];
    if repeated {
        content.operations.extend(content.operations.clone());
    }
    doc.objects.insert(
        (4, 0),
        Object::Stream(Stream::new(dictionary! {}, content.encode().unwrap())),
    );
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    bytes
}

fn changed_source(change: impl FnOnce(&mut Document)) -> Vec<u8> {
    let mut document = Document::load_mem(&source_pdf(false)).unwrap();
    change(&mut document);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

#[test]
fn malformed_cmap_wrapper_and_metadata_inside_mapping_block_reject_whole_source() {
    for mutation in 0..3 {
        let source = changed_source(|doc| {
            let stream = doc.get_object_mut((9, 0)).unwrap().as_stream_mut().unwrap();
            let text = String::from_utf8(stream.content.clone()).unwrap();
            stream.content = match mutation {
                0 => format!("{text}\n/CMapType 2 def\n").into_bytes(),
                1 => text
                    .replace("beginbfchar", "beginbfchar\n/WMode 0 def")
                    .into_bytes(),
                2 => text
                    .replace("begincmap", "begincmap\n/CMapName /Duplicate def")
                    .into_bytes(),
                _ => unreachable!(),
            };
        });
        let verified = verify_source(&source, &SourceLimits::default(), &AtomicBool::new(false));
        assert_eq!(*verified.outcome(), SourceOutcome::Unsupported);
        assert!(verified.pages().is_empty());
    }
}

#[test]
fn non_ascii_font_resource_name_rejects_whole_source() {
    let source = changed_source(|doc| {
        let stream = doc.get_object_mut((4, 0)).unwrap().as_stream_mut().unwrap();
        let mut content = Content::decode(&stream.content).unwrap();
        let tf = content
            .operations
            .iter_mut()
            .find(|op| op.operator == "Tf")
            .unwrap();
        tf.operands[0] = Object::Name(vec![0xc3, 0xa9]);
        stream.content = content.encode().unwrap();
    });
    let verified = verify_source(&source, &SourceLimits::default(), &AtomicBool::new(false));
    assert_eq!(*verified.outcome(), SourceOutcome::Unsupported);
    assert!(verified.pages().is_empty());
    assert!(verified.issues()[0].reason.contains("non-ASCII"));
}

#[test]
fn font_byte_and_cmap_probe_limits_accumulate_across_text_shows() {
    let source = source_pdf(false);
    for limits in [
        SourceLimits {
            max_total_font_bytes: 2 * FONT.len(),
            ..SourceLimits::default()
        },
        SourceLimits {
            max_font_cmap_probes: 65_536,
            ..SourceLimits::default()
        },
    ] {
        let verified = verify_source(&source, &limits, &AtomicBool::new(false));
        assert_eq!(
            *verified.outcome(),
            SourceOutcome::ResourceLimit,
            "{:?}",
            verified.issues()
        );
        assert!(verified.pages().is_empty());
    }
}

#[test]
fn embedded_unicode_alias_for_a_shown_glyph_rejects_whole_source() {
    fn word(bytes: &[u8], offset: usize) -> u16 {
        u16::from_be_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }
    let glyph_a = ttf_parser::Face::parse(FONT, 0)
        .unwrap()
        .glyph_index('A')
        .unwrap()
        .0;
    let mut font = FONT.to_vec();
    let tables = word(&font, 4);
    let cmap_record = (0..usize::from(tables))
        .map(|i| 12 + 16 * i)
        .find(|offset| &font[*offset..*offset + 4] == b"cmap")
        .unwrap();
    let cmap =
        u32::from_be_bytes(font[cmap_record + 8..cmap_record + 12].try_into().unwrap()) as usize;
    let subtables = word(&font, cmap + 2);
    let mut changed = false;
    for i in 0..usize::from(subtables) {
        let record = cmap + 4 + 8 * i;
        let platform = word(&font, record);
        if platform != 0 && platform != 3 {
            continue;
        }
        let table =
            cmap + u32::from_be_bytes(font[record + 4..record + 8].try_into().unwrap()) as usize;
        if word(&font, table) != 4 {
            continue;
        }
        let segments = usize::from(word(&font, table + 6) / 2);
        for segment in 0..segments {
            let end = word(&font, table + 14 + segment * 2);
            let start = word(&font, table + 16 + segments * 2 + segment * 2);
            if start > 0x50 || end < 0x50 {
                continue;
            }
            let delta_at = table + 16 + segments * 4 + segment * 2;
            let range_at = table + 16 + segments * 6 + segment * 2;
            let range = usize::from(word(&font, range_at));
            if range == 0 {
                font[delta_at..delta_at + 2]
                    .copy_from_slice(&glyph_a.wrapping_sub(0x50).to_be_bytes());
            } else {
                let glyph_at = range_at + range + 2 * usize::from(0x50 - start);
                let raw = glyph_a.wrapping_sub(word(&font, delta_at));
                font[glyph_at..glyph_at + 2].copy_from_slice(&raw.to_be_bytes());
            }
            changed = true;
        }
    }
    assert!(changed);
    let source = changed_source(|doc| {
        let stream = doc
            .get_object_mut((11, 0))
            .unwrap()
            .as_stream_mut()
            .unwrap();
        stream.content = font;
        stream.dict.remove(b"Filter");
    });
    let verified = verify_source(&source, &SourceLimits::default(), &AtomicBool::new(false));
    assert_eq!(
        *verified.outcome(),
        SourceOutcome::Unsupported,
        "{:?}",
        verified.issues()
    );
    assert!(verified.pages().is_empty());
    assert!(
        verified.issues()[0]
            .reason
            .contains("ambiguous Unicode aliases")
    );
}

struct Fixture {
    source: Vec<u8>,
    baseline: Value,
    candidate: Value,
}
impl Fixture {
    fn new() -> Self {
        let source = source_pdf(false);
        let verified = verify_source(&source, &SourceLimits::default(), &AtomicBool::new(false));
        assert_eq!(
            *verified.outcome(),
            SourceOutcome::Supported,
            "{:?}",
            verified.issues()
        );
        let witnesses = &verified.pages()[0].witnesses;
        assert_eq!(witnesses.len(), 2);
        assert_eq!(witnesses[0].unicode, "ALPHA");
        assert_eq!(witnesses[1].unicode, "RECOVER ALPHA 2026");
        let spans: Vec<_> = witnesses
            .iter()
            .enumerate()
            .map(|(index, witness)| {
                json!({
                    "text": witness.unicode, "bbox": witness.region, "font": "synthetic fixture",
                    "size": 22, "seq": index,
                })
            })
            .collect();
        let source_id = SourceIdentity::of(&source);
        let record = |name: &str, status: &str| {
            json!({
                "schema_version": 5,
                "document": {"hash": source_id.sha256, "size": source_id.size, "pages": 1,
                    "sources": [{"path": "synthetic-development.pdf", "size": source.len(), "mtime_unix": null}]},
                "backend": backend(name), "status": status,
                "pages": [{"page": 1, "width": 612, "height": 792, "rotation": 0,
                    "spans": spans, "lines": [], "figures": [], "links": [],
                    "text": "This existing page text must remain unchanged.", "warnings": ["original warning"]}],
                "chunks": [], "metadata": {"title": "untouched"}, "references": [], "citations": [],
                "warnings": ["original document warning"], "timings": {"parse_ms": 1},
                "future": {"nested": ["retained unknown data"]},
            })
        };
        let mut baseline = record("lopdf", "partial");
        baseline["pages"][0]["spans"][1]["text"] = json!("\u{fffd}");
        let candidate = record("pdfium", "complete");
        Self {
            source,
            baseline,
            candidate,
        }
    }
    fn setup(&self) -> (Sidecar, ArtifactStore, Vec<Vec<u8>>) {
        let limits = tpe_region_evidence::Limits::default();
        let bytes: Vec<_> = [&self.baseline, &self.candidate]
            .iter()
            .map(|record| {
                let mut bytes = b" \n".to_vec();
                bytes.extend(serde_json::to_vec_pretty(record).unwrap());
                bytes.extend_from_slice(b"\n\t");
                bytes
            })
            .collect();
        let mut store = ArtifactStore::default();
        let mut attempts = Vec::new();
        for (index, record) in [&self.baseline, &self.candidate].iter().enumerate() {
            attempts.push(Attempt {
                id: if index == 0 { "baseline" } else { "candidate" }.into(),
                source: SourceIdentity::of(&self.source),
                pages: vec![1],
                backend: serde_json::from_value(record["backend"].clone()).unwrap(),
                runtime: RuntimeIdentity::Unknown {
                    reason: "synthetic selector test".into(),
                },
                outcome: match record["status"].as_str().unwrap() {
                    "complete" => Outcome::Complete,
                    "partial" => Outcome::Partial {
                        reason: "preserved partial status".into(),
                    },
                    "failed" => Outcome::Failed {
                        reason: "synthetic failed artifact".into(),
                    },
                    _ => unreachable!(),
                },
                artifact: Some(store.insert(bytes[index].clone(), &limits).unwrap()),
                frame: GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated,
            });
        }
        let baseline = attempts.remove(0);
        (
            Sidecar {
                contract_version: 1,
                source: SourceIdentity::of(&self.source),
                document_pages: 1,
                pages: vec![1],
                baseline,
                alternatives: attempts,
                regions: Vec::new(),
                decision: Decision::RetainBaselineAndAbstain,
            },
            store,
            bytes,
        )
    }
    fn evaluate(
        &self,
        limits: &Limits,
        cancelled: bool,
        edit: impl FnOnce(&mut Sidecar),
    ) -> DerivedView {
        let (mut sidecar, store, bytes) = self.setup();
        edit(&mut sidecar);
        let validated = sidecar
            .validate(
                &self.source,
                &store,
                &tpe_region_evidence::Limits::default(),
            )
            .unwrap();
        let result = select(
            &validated,
            &self.source,
            limits,
            &AtomicBool::new(cancelled),
        )
        .unwrap();
        assert_eq!(validated.retained_baseline_bytes(), bytes[0]);
        assert_eq!(validated.alternative_bytes("candidate").unwrap(), bytes[1]);
        result
    }
}
fn backend(name: &str) -> BackendIdentity {
    BackendIdentity {
        name: name.into(),
        version: "synthetic".into(),
        config_digest: Digest::of(b"test configuration"),
    }
}
fn selections(view: &DerivedView) -> usize {
    view.regions
        .iter()
        .filter(|decision| matches!(decision, RegionDecision::Selected { .. }))
        .count()
}
fn assert_baseline(view: &DerivedView) {
    if let Some(pages) = &view.pages {
        for span in pages.iter().flat_map(|page| &page.spans) {
            assert_eq!(span.baseline, span.selected);
            assert!(span.source_witness.is_none());
        }
    }
    assert_eq!(selections(view), 0);
}
fn reason(view: &DerivedView, expected: AbstentionReason) -> bool {
    view.regions.iter().any(|decision| {
        matches!(decision,
        RegionDecision::Abstained { reason, .. } if *reason == expected)
    })
}

#[test]
fn source_mapping_selects_one_region_and_retains_good_baseline_with_exact_lineage() {
    let fixture = Fixture::new();
    let view = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_eq!(view.outcome, RunOutcome::Evaluated);
    assert_eq!(view.policy, "source_declared_unicode_v1");
    assert_eq!(selections(&view), 1);
    let page = &view.pages.as_ref().unwrap()[0];
    assert_eq!(page.spans[0].text, "ALPHA");
    assert_eq!(page.spans[0].baseline, page.spans[0].selected);
    assert_eq!(page.spans[1].text, "RECOVER ALPHA 2026");
    assert_eq!(page.spans[1].selected.attempt_id, "candidate");
    assert_eq!(page.spans[1].selected.span_index, 1);
    assert_ne!(
        page.spans[1].baseline.artifact,
        page.spans[1].selected.artifact
    );
    assert!(page.spans[1].source_witness.is_some());
    assert!(matches!(
        view.evidence.baseline.outcome,
        Outcome::Partial { .. }
    ));
    let receipt = serde_json::to_value(&view).unwrap();
    assert!(
        receipt["source_evidence"]["pages"][0]["witnesses"][1]["origin"]["raw_codes_hex"]
            .is_string()
    );
    assert!(receipt.get("approved_review").is_none());
    assert!(receipt.get("adjudicated_text").is_none());
}

#[test]
fn longer_shorter_and_equal_length_wrong_candidates_all_abstain() {
    for text in [
        "A",
        "RECOVER OMEGA 2026",
        "A much longer fabricated extraction with extra content",
    ] {
        let mut fixture = Fixture::new();
        fixture.candidate["pages"][0]["spans"][1]["text"] = json!(text);
        let view = fixture.evaluate(&Limits::default(), false, |_| {});
        assert_baseline(&view);
        assert!(reason(
            &view,
            AbstentionReason::CandidateDisagreesWithSource
        ));
    }
}

#[test]
fn case_and_whitespace_are_not_normalized_into_source_agreement() {
    for text in [
        "recover alpha 2026",
        "RECOVER ALPHA 2026 ",
        "RECOVER  ALPHA 2026",
    ] {
        let mut fixture = Fixture::new();
        fixture.candidate["pages"][0]["spans"][1]["text"] = json!(text);
        assert_baseline(&fixture.evaluate(&Limits::default(), false, |_| {}));
    }
}

#[test]
fn null_crossing_duplicate_and_out_of_page_geometry_abstain() {
    for mutation in 0..5 {
        let mut fixture = Fixture::new();
        match mutation {
            0 => fixture.candidate["pages"][0]["spans"][0]["bbox"] = Value::Null,
            1 => fixture.baseline["pages"][0]["spans"][0]["bbox"] = Value::Null,
            2 => {
                let bbox = &mut fixture.candidate["pages"][0]["spans"][1]["bbox"];
                bbox["x0"] = json!(bbox["x0"].as_f64().unwrap() - 1.0);
            }
            3 => {
                let repeated = fixture.candidate["pages"][0]["spans"][1].clone();
                fixture.candidate["pages"][0]["spans"]
                    .as_array_mut()
                    .unwrap()
                    .push(repeated);
            }
            4 => fixture.candidate["pages"][0]["spans"][0]["bbox"]["x0"] = json!(-1),
            _ => unreachable!(),
        }
        assert_baseline(&fixture.evaluate(&Limits::default(), false, |_| {}));
    }
}

#[test]
fn geometry_frame_and_candidate_attempt_ambiguity_fail_closed() {
    let fixture = Fixture::new();
    for baseline_frame in [false, true] {
        let view = fixture.evaluate(&Limits::default(), false, |sidecar| {
            if baseline_frame {
                sidecar.baseline.frame = GeometryFrame::Unknown;
            } else {
                sidecar.alternatives[0].frame = GeometryFrame::Unknown;
            }
        });
        assert_baseline(&view);
        assert!(reason(&view, AbstentionReason::UnknownFrame));
    }
    let view = fixture.evaluate(&Limits::default(), false, |sidecar| {
        let mut attempt = sidecar.alternatives[0].clone();
        attempt.id = "another actual attempt".into();
        sidecar.alternatives.push(attempt);
    });
    assert_baseline(&view);
    assert!(reason(&view, AbstentionReason::AmbiguousCandidateAttempts));
}

#[test]
fn mismatching_page_geometry_and_failed_artifact_cannot_supply_text() {
    for rotation in [false, true] {
        let mut fixture = Fixture::new();
        fixture.candidate["pages"][0][if rotation { "rotation" } else { "width" }] = json!(90);
        let view = fixture.evaluate(&Limits::default(), false, |_| {});
        assert_baseline(&view);
        assert!(reason(&view, AbstentionReason::PageGeometryMismatch));
    }
    let mut fixture = Fixture::new();
    fixture.candidate["status"] = json!("failed");
    let view = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_baseline(&view);
    assert!(reason(&view, AbstentionReason::NoEligibleCandidate));
}

#[test]
fn every_exhausted_comparison_prefix_rolls_back_the_whole_run() {
    let mut fixture = Fixture::new();
    // Two proposals ensure some budget stops happen after an earlier region
    // would have been selected. No successfully processed prefix may escape.
    fixture.baseline["pages"][0]["spans"][0]["text"] = json!("\u{fffd}");
    let complete = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_eq!(selections(&complete), 2);
    for limit in 0..complete.comparisons {
        let limits = Limits {
            max_span_comparisons: limit,
            ..Limits::default()
        };
        let view = fixture.evaluate(&limits, false, |_| {});
        assert!(matches!(view.outcome, RunOutcome::ResourceLimit { .. }));
        assert_baseline(&view);
        assert!(view.comparisons <= limit);
    }
}

#[test]
fn discovered_candidate_indices_follow_actual_geometry_after_array_permutation() {
    let mut fixture = Fixture::new();
    fixture.candidate["pages"][0]["spans"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    let view = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_eq!(selections(&view), 1);
    let selected = &view.pages.as_ref().unwrap()[0].spans[1];
    assert_eq!(selected.baseline.span_index, 1);
    assert_eq!(selected.selected.span_index, 0);
    assert_eq!(selected.text, "RECOVER ALPHA 2026");
}

#[test]
fn source_consistent_baseline_remains_even_when_candidate_neighbor_is_wrong() {
    let mut fixture = Fixture::new();
    fixture.candidate["pages"][0]["spans"][0]["text"] =
        json!("wrong much longer neighboring candidate");
    let view = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_eq!(selections(&view), 1);
    let neighbor = &view.pages.as_ref().unwrap()[0].spans[0];
    assert_eq!(neighbor.text, "ALPHA");
    assert_eq!(neighbor.baseline, neighbor.selected);
}

#[test]
fn cancellation_projection_and_source_budgets_publish_only_baseline() {
    let fixture = Fixture::new();
    let cancelled = fixture.evaluate(&Limits::default(), true, |_| {});
    assert_eq!(cancelled.outcome, RunOutcome::Cancelled);
    assert_baseline(&cancelled);
    let limits = Limits {
        max_projection_spans: 1,
        ..Limits::default()
    };
    let view = fixture.evaluate(&limits, false, |_| {});
    assert!(view.pages.is_none());
    assert!(matches!(view.outcome, RunOutcome::ResourceLimit { .. }));
    let mut limits = Limits::default();
    limits.source.max_source_bytes = 1;
    assert_baseline(&fixture.evaluate(&limits, false, |_| {}));
    for limits in [
        Limits {
            max_source_witnesses: 1,
            ..Limits::default()
        },
        Limits {
            max_candidate_attempts: 0,
            ..Limits::default()
        },
        Limits {
            max_candidate_artifact_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_total_candidate_bytes: 1,
            ..Limits::default()
        },
    ] {
        let view = fixture.evaluate(&limits, false, |_| {});
        assert!(matches!(view.outcome, RunOutcome::ResourceLimit { .. }));
        assert_baseline(&view);
    }
}

#[test]
fn source_identity_is_checked_again_inside_selector() {
    let fixture = Fixture::new();
    let (sidecar, store, _) = fixture.setup();
    let validated = sidecar
        .validate(
            &fixture.source,
            &store,
            &tpe_region_evidence::Limits::default(),
        )
        .unwrap();
    let other_source = source_pdf(true);
    let view = select(
        &validated,
        &other_source,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(view.outcome, RunOutcome::SourceMismatch);
    assert_baseline(&view);
}

#[test]
fn unsupported_legacy_neighbor_invalidates_all_source_selection() {
    let mut fixture = Fixture::new();
    fixture.source = OLD_PDF.to_vec();
    let source_id = SourceIdentity::of(&fixture.source);
    for record in [&mut fixture.baseline, &mut fixture.candidate] {
        record["document"]["hash"] = json!(source_id.sha256);
        record["document"]["size"] = json!(source_id.size);
    }
    let view = fixture.evaluate(&Limits::default(), false, |_| {});
    assert_eq!(view.outcome, RunOutcome::UnsupportedSource);
    assert_baseline(&view);
}
