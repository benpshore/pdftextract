//! Real extraction compatibility plus synthetic adversarial edits of those
//! artifacts. The fixture review is a selection input, not an accuracy holdout.
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

use serde_json::{Value, json};
use tpe_region_evidence::{
    ArtifactStore, Attempt, Decision, Digest, GeometryFrame, Outcome, RuntimeIdentity, Sidecar,
    SourceIdentity,
};
use tpe_region_fusion::{
    AbstentionReason, DerivedView, Limits, RegionDecision, ReviewBundle, RunOutcome, TrustedReview,
    WitnessStore, select,
};

const SOURCE_HASH: &str = "97779d74bc9f286de6f4578ce86f47a266d7f263a28ce37b955b61b7333f2a1f";
const APPROVED_REVIEW_HASH: &str =
    "926b144c886d90734390df8e951026cc0115610fdb68fb28660ddea7cd48889b";

struct Fixture {
    source: Vec<u8>,
    baseline: Vec<u8>,
    candidate: Vec<u8>,
    review: Vec<u8>,
    render: Vec<u8>,
}

fn read(path: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(path),
    )
    .unwrap()
}

impl Fixture {
    fn positive() -> Self {
        Self {
            source: read("positive-stream-cmap.pdf"),
            baseline: read(&format!(
                "captured/positive-stream-cmap/lopdf/{SOURCE_HASH}.json"
            )),
            candidate: read(&format!(
                "captured/positive-stream-cmap/pdfium/{SOURCE_HASH}.json"
            )),
            review: read("approved-review.json"),
            render: read("captured/positive-stream-cmap/page-1-mupdf.png"),
        }
    }

    /// Test-only explicit acceptance of predeclared generator truth for another
    /// captured source. This does not edit or extend the shipped approved review.
    fn source_truth_case(name: &str) -> Self {
        let source = read(&format!("{name}.pdf"));
        let hash = Digest::of(&source);
        let baseline = read(&format!("captured/{name}/lopdf/{}.json", hash.as_str()));
        let candidate = read(&format!("captured/{name}/pdfium/{}.json", hash.as_str()));
        let render = read(&format!("captured/{name}/page-1-mupdf.png"));
        let truth: Value = serde_json::from_slice(&read(&format!("{name}.truth.json"))).unwrap();
        assert_eq!(truth["input_sha256"], hash.as_str());
        let base_record: Value = serde_json::from_slice(&baseline).unwrap();
        let candidate_record: Value = serde_json::from_slice(&candidate).unwrap();
        let mut bundle = Self::positive().bundle();
        bundle.source = SourceIdentity::of(&source);
        let review = &mut bundle.regions[0];
        review.adjudicated_text = truth["regions"][1]["expected_text"]
            .as_str()
            .unwrap()
            .into();
        review.expected_baseline_text = base_record["pages"][0]["spans"][1]["text"]
            .as_str()
            .unwrap()
            .into();
        review.expected_candidate_text = candidate_record["pages"][0]["spans"][1]["text"]
            .as_str()
            .unwrap()
            .into();
        review.render_artifact.sha256 = Digest::of(&render);
        review.render_artifact.size = render.len() as u64;
        review.provenance.reviewer = "integration-test source-truth policy".into();
        review.provenance.method = "Test-only explicit acceptance of predeclared generator truth; not blind review or independent accuracy evaluation".into();
        review.provenance.record_reference = format!("fixtures/{name}.truth.json");
        Self {
            source,
            baseline,
            candidate,
            render,
            review: serde_json::to_vec(&bundle).unwrap(),
        }
    }

    fn bundle(&self) -> ReviewBundle {
        serde_json::from_slice(&self.review).unwrap()
    }

    fn edit_review(&mut self, edit: impl FnOnce(&mut ReviewBundle)) {
        let mut review = self.bundle();
        edit(&mut review);
        self.review = serde_json::to_vec(&review).unwrap();
    }

    fn setup(&self) -> (Sidecar, ArtifactStore) {
        let limits = tpe_region_evidence::Limits::default();
        let source = SourceIdentity::of(&self.source);
        let mut store = ArtifactStore::default();
        let mut attempts = Vec::new();
        let mut page_count = 0;
        for (id, bytes) in [("baseline", &self.baseline), ("candidate", &self.candidate)] {
            let record: Value = serde_json::from_slice(bytes).unwrap();
            page_count = record["document"]["pages"].as_u64().unwrap() as u32;
            attempts.push(Attempt {
                id: id.into(),
                source: source.clone(),
                pages: record["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|page| page["page"].as_u64().unwrap() as u32)
                    .collect(),
                backend: serde_json::from_value(record["backend"].clone()).unwrap(),
                runtime: RuntimeIdentity::Unknown {
                    reason: "test consumes previously captured bytes".into(),
                },
                outcome: match record["status"].as_str().unwrap() {
                    "complete" => Outcome::Complete,
                    "partial" => Outcome::Partial {
                        reason: "preserved artifact partial status".into(),
                    },
                    "failed" => Outcome::Failed {
                        reason: "preserved artifact failed status".into(),
                    },
                    value => panic!("unexpected status {value}"),
                },
                artifact: Some(store.insert(bytes.clone(), &limits).unwrap()),
                frame: GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated,
            });
        }
        let baseline = attempts.remove(0);
        (
            Sidecar {
                contract_version: tpe_region_evidence::CONTRACT_VERSION,
                source,
                document_pages: page_count,
                pages: baseline.pages.clone(),
                baseline,
                alternatives: attempts,
                regions: vec![],
                decision: Decision::RetainBaselineAndAbstain,
            },
            store,
        )
    }

    fn evaluate(
        &self,
        limits: &Limits,
        cancelled: bool,
        trust: bool,
        configure: impl FnOnce(&mut Sidecar),
    ) -> tpe_region_fusion::Result<DerivedView> {
        let (mut sidecar, store) = self.setup();
        configure(&mut sidecar);
        let validated = sidecar
            .validate(
                &self.source,
                &store,
                &tpe_region_evidence::Limits::default(),
            )
            .unwrap();
        // Fixture acceptance is explicit. Mutated tests intentionally supply a new
        // trusted review to probe policy invariants, never to claim independent truth.
        let trusted = TrustedReview::accept_explicitly(
            &self.review,
            &Digest::of(&self.review),
            "test policy",
            &Limits::default(),
        )?;
        let mut renders = WitnessStore::default();
        renders
            .insert(self.render.clone(), &Limits::default())
            .unwrap();
        select(
            &validated,
            trust.then_some(&trusted),
            &renders,
            limits,
            &AtomicBool::new(cancelled),
        )
    }

    fn run(&self) -> DerivedView {
        self.evaluate(&Limits::default(), false, true, |_| {})
            .unwrap()
    }
}

fn edit_artifact(bytes: &mut Vec<u8>, edit: impl FnOnce(&mut Value)) {
    let mut record: Value = serde_json::from_slice(bytes).unwrap();
    edit(&mut record);
    *bytes = serde_json::to_vec(&record).unwrap();
}

fn assert_baseline(view: &DerivedView) {
    if let Some(pages) = &view.pages {
        for row in pages.iter().flat_map(|page| &page.spans) {
            assert_eq!(row.baseline, row.selected);
            assert!(row.review_id.is_none());
        }
    }
    assert!(
        !view
            .regions
            .iter()
            .any(|decision| matches!(decision, RegionDecision::Selected { .. }))
    );
}

fn assert_reason(view: &DerivedView, reason: AbstentionReason) {
    assert_baseline(view);
    assert!(view.regions.iter().any(|decision| matches!(decision, RegionDecision::Abstained { reason: actual, .. } if *actual == reason)), "{:?}", view.regions);
}

#[test]
fn actual_lopdf_pdfium_fixture_selects_one_region_and_retains_good_neighbor() {
    let fixture = Fixture::positive();
    assert_eq!(Digest::of(&fixture.source).as_str(), SOURCE_HASH);
    assert_eq!(Digest::of(&fixture.review).as_str(), APPROVED_REVIEW_HASH);
    let view = fixture.run();
    assert_eq!(view.policy, "trusted_visible_transcription_v1");
    assert_eq!(view.outcome, RunOutcome::Evaluated);
    let rows = &view.pages.as_ref().unwrap()[0].spans;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].text, "BASELINE REGION RETAINS THIS TEXT.");
    assert_eq!(rows[0].baseline, rows[0].selected);
    assert_eq!(rows[1].text, "RECOVER ALPHA 2026");
    assert_eq!(rows[1].baseline.span_index, 1);
    assert_eq!(rows[1].selected.span_index, 1);
    assert_eq!(rows[1].selected.attempt_id, "candidate");
    assert_eq!(
        rows[1].selected.artifact.sha256,
        Digest::of(&fixture.candidate)
    );
    assert_eq!(
        rows[1].baseline.artifact.sha256,
        Digest::of(&fixture.baseline)
    );
    let (sidecar, store) = fixture.setup();
    assert_eq!(view.evidence, sidecar);
    let validated = sidecar
        .validate(
            &fixture.source,
            &store,
            &tpe_region_evidence::Limits::default(),
        )
        .unwrap();
    assert_eq!(validated.retained_baseline_bytes(), fixture.baseline);
    assert_eq!(
        validated.alternative_bytes("candidate").unwrap(),
        fixture.candidate
    );
    assert!(matches!(
        view.evidence.baseline.outcome,
        Outcome::Partial { .. }
    ));
    let encoded: Value = serde_json::from_slice(&serde_json::to_vec(&view).unwrap()).unwrap();
    assert_eq!(encoded["regions"][0]["decision"], "selected");
}

#[test]
fn actual_missing_mapping_candidate_disagrees_with_source_truth_and_is_not_selected() {
    let fixture = Fixture::source_truth_case("negative-empty-tounicode");
    let review = &fixture.bundle().regions[0];
    assert_ne!(review.expected_candidate_text, review.adjudicated_text);
    let view = fixture.run();
    assert_reason(&view, AbstentionReason::CandidateDisagreesWithAdjudication);
    assert!(matches!(
        view.evidence.alternatives[0].outcome,
        Outcome::Partial { .. }
    ));
}

#[test]
fn actual_equal_control_keeps_baseline_and_repeated_source_is_local_to_reviewed_region() {
    let control = Fixture::source_truth_case("control-named-cmap").run();
    assert_baseline(&control);
    assert!(matches!(
        control.regions[0],
        RegionDecision::RetainedEqual { .. }
    ));
    let repeated = Fixture::source_truth_case("repeated-stream-cmap").run();
    let rows = &repeated.pages.as_ref().unwrap()[0].spans;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].text, "RECOVER ALPHA 2026");
    assert_eq!(rows[1].selected.span_index, 1);
    assert_ne!(rows[1].baseline, rows[1].selected);
    assert_eq!(rows[2].baseline, rows[2].selected);
    assert_eq!(rows[2].text, "\u{fffd}".repeat(18));
}

#[test]
fn no_explicit_trust_never_selects_despite_matching_candidate_text() {
    let view = Fixture::positive()
        .evaluate(&Limits::default(), false, false, |_| {})
        .unwrap();
    assert_baseline(&view);
    assert!(view.accepted_review.is_none());
}

#[test]
fn review_cannot_self_authorize_or_silently_override_fields() {
    let fixture = Fixture::positive();
    assert!(
        TrustedReview::accept_explicitly(
            &fixture.review,
            &Digest::of(b"different approved record"),
            "caller",
            &Limits::default()
        )
        .is_err()
    );
    assert!(
        TrustedReview::accept_explicitly(
            &fixture.review,
            &Digest::of(&fixture.review),
            " ",
            &Limits::default()
        )
        .is_err()
    );
    let duplicate = String::from_utf8(fixture.review).unwrap().replacen(
        "\"contract_version\": 1",
        "\"contract_version\": 1,\"contract_version\": 1",
        1,
    );
    assert!(
        TrustedReview::accept_explicitly(
            duplicate.as_bytes(),
            &Digest::of(duplicate.as_bytes()),
            "caller",
            &Limits::default()
        )
        .is_err()
    );
}

#[test]
fn source_page_and_render_evidence_mismatches_are_errors() {
    let mut fixture = Fixture::positive();
    fixture.edit_review(|review| review.source.sha256 = Digest::of(b"other source"));
    assert!(
        fixture
            .evaluate(&Limits::default(), false, true, |_| {})
            .err()
            .unwrap()
            .to_string()
            .contains("source identity mismatch")
    );
    let mut fixture = Fixture::positive();
    fixture.edit_review(|review| review.regions[0].page = 2);
    assert!(
        fixture
            .evaluate(&Limits::default(), false, true, |_| {})
            .err()
            .unwrap()
            .to_string()
            .contains("page identity mismatch")
    );
    let mut fixture = Fixture::positive();
    fixture.render.push(0);
    assert!(
        fixture
            .evaluate(&Limits::default(), false, true, |_| {})
            .err()
            .unwrap()
            .to_string()
            .contains("render artifact")
    );
}

#[test]
fn unknown_frames_and_geometry_mismatch_abstain() {
    let fixture = Fixture::positive();
    assert_reason(
        &fixture
            .evaluate(&Limits::default(), false, true, |sidecar| {
                sidecar.alternatives[0].frame = GeometryFrame::Unknown
            })
            .unwrap(),
        AbstentionReason::UnknownFrame,
    );
    let mut fixture = Fixture::positive();
    fixture.edit_review(|review| review.regions[0].page_geometry.width += 1.0);
    assert_reason(&fixture.run(), AbstentionReason::PageGeometryMismatch);
}

#[test]
fn unlocated_offpage_and_zero_area_spans_anywhere_on_page_abstain() {
    for mode in 0..3 {
        let mut fixture = Fixture::positive();
        edit_artifact(&mut fixture.candidate, |record| {
            let bbox = &mut record["pages"][0]["spans"][0]["bbox"]; // Good neighbor outside target.
            match mode {
                0 => *bbox = Value::Null,
                1 => bbox["x0"] = json!(-1),
                2 => bbox["x1"] = bbox["x0"].clone(),
                _ => unreachable!(),
            }
        });
        assert_reason(
            &fixture.run(),
            AbstentionReason::MissingOrInvalidSpanGeometry,
        );
    }
}

#[test]
fn crossing_region_boundary_and_duplicate_atoms_abstain() {
    let mut fixture = Fixture::positive();
    fixture.edit_review(|review| review.regions[0].region.x1 = 100.0);
    assert_reason(&fixture.run(), AbstentionReason::CrossingSpan);
    let mut fixture = Fixture::positive();
    edit_artifact(&mut fixture.candidate, |record| {
        let duplicate = record["pages"][0]["spans"][1].clone();
        record["pages"][0]["spans"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
    });
    assert_reason(&fixture.run(), AbstentionReason::AmbiguousRegion);
}

#[test]
fn repeated_text_outside_region_is_not_confused_with_geometric_identity() {
    let mut fixture = Fixture::positive();
    edit_artifact(&mut fixture.candidate, |record| {
        record["pages"][0]["spans"][0]["text"] = json!("RECOVER ALPHA 2026");
        record["pages"][0]["spans"][0]["seq"] = record["pages"][0]["spans"][1]["seq"].clone();
    });
    let view = fixture.run();
    assert_eq!(view.pages.unwrap()[0].spans[1].selected.span_index, 1);
}

#[test]
fn overlapping_reviews_abstain_every_conflict_independent_of_order() {
    for reverse in [false, true] {
        let mut fixture = Fixture::positive();
        fixture.edit_review(|review| {
            let mut duplicate = review.regions[0].clone();
            duplicate.id = "second conflicting review".into();
            duplicate.region.x0 += 1.0;
            review.regions.push(duplicate);
            if reverse {
                review.regions.reverse();
            }
        });
        let view = fixture.run();
        assert_eq!(view.regions.len(), 2);
        assert!(view.regions.iter().all(|decision| matches!(
            decision,
            RegionDecision::Abstained {
                reason: AbstentionReason::ConflictingReviews,
                ..
            }
        )));
        assert_baseline(&view);
    }
}

#[test]
fn competing_attempts_abstain_and_terminal_attempts_are_ineligible() {
    let fixture = Fixture::positive();
    assert_reason(
        &fixture
            .evaluate(&Limits::default(), false, true, |sidecar| {
                let mut other = sidecar.alternatives[0].clone();
                other.id = "competing".into();
                sidecar.alternatives.push(other);
            })
            .unwrap(),
        AbstentionReason::AmbiguousCandidateAttempts,
    );
    for terminal in [
        Outcome::Failed {
            reason: "failed".into(),
        },
        Outcome::Cancelled {
            reason: "cancelled".into(),
            requested_by: "caller".into(),
        },
        Outcome::ResourceLimit {
            resource: "time".into(),
            limit: 1,
            observed: Some(2),
        },
    ] {
        let mut fixture = Fixture::positive();
        edit_artifact(&mut fixture.candidate, |record| {
            record["status"] = json!("failed")
        });
        assert_reason(
            &fixture
                .evaluate(&Limits::default(), false, true, |sidecar| {
                    sidecar.alternatives[0].outcome = terminal
                })
                .unwrap(),
            AbstentionReason::IneligibleAttempt,
        );
    }
}

#[test]
fn longer_text_is_never_authority_and_equal_text_keeps_baseline() {
    let mut fixture = Fixture::positive();
    edit_artifact(&mut fixture.candidate, |record| {
        record["pages"][0]["spans"][1]["text"] =
            json!("Much longer text that the independent render never approved")
    });
    fixture.edit_review(|review| {
        review.regions[0].expected_candidate_text =
            "Much longer text that the independent render never approved".into()
    });
    assert_reason(
        &fixture.run(),
        AbstentionReason::CandidateDisagreesWithAdjudication,
    );
    let mut fixture = Fixture::positive();
    let baseline_text = fixture.bundle().regions[0].expected_baseline_text.clone();
    edit_artifact(&mut fixture.candidate, |record| {
        record["pages"][0]["spans"][1]["text"] = json!(baseline_text)
    });
    fixture.edit_review(|review| review.regions[0].expected_candidate_text = baseline_text);
    let view = fixture.run();
    assert_baseline(&view);
    assert!(matches!(
        view.regions[0],
        RegionDecision::RetainedEqual { .. }
    ));
}

#[test]
fn cancellation_and_every_selector_limit_preserve_baseline_atomically() {
    let fixture = Fixture::positive();
    let cancelled = fixture
        .evaluate(&Limits::default(), true, true, |_| {})
        .unwrap();
    assert_eq!(cancelled.outcome, RunOutcome::Cancelled);
    assert_baseline(&cancelled);
    for limits in [
        Limits {
            max_review_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_witnesses: 0,
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
        Limits {
            max_render_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_total_render_bytes: 1,
            ..Limits::default()
        },
        Limits {
            max_span_comparisons: 4,
            ..Limits::default()
        },
        Limits {
            max_projection_spans: 1,
            ..Limits::default()
        },
    ] {
        let view = fixture.evaluate(&limits, false, true, |_| {}).unwrap();
        assert!(matches!(view.outcome, RunOutcome::ResourceLimit { .. }));
        assert_baseline(&view);
    }
}

#[test]
fn budget_exhaustion_after_first_proposal_discards_it_instead_of_publishing_prefix() {
    let mut fixture = Fixture::positive();
    fixture.edit_review(|review| {
        let mut neighbor = review.regions[0].clone();
        neighbor.id = "neighbor".into();
        neighbor.region.y0 = 708.0;
        neighbor.region.y1 = 742.0;
        neighbor.region.x1 = 330.0;
        neighbor.expected_baseline_text = "BASELINE REGION RETAINS THIS TEXT.".into();
        neighbor.expected_candidate_text = neighbor.expected_baseline_text.clone();
        neighbor.adjudicated_text = neighbor.expected_baseline_text.clone();
        review.regions.push(neighbor);
    });
    // One conflict check + first region/attempt + two baseline and two candidate
    // spans = seven. Eight expires in the second region after first proposal.
    let view = fixture
        .evaluate(
            &Limits {
                max_span_comparisons: 8,
                ..Limits::default()
            },
            false,
            true,
            |_| {},
        )
        .unwrap();
    assert_eq!(view.comparisons, 8);
    assert!(matches!(view.outcome, RunOutcome::ResourceLimit { .. }));
    assert_baseline(&view);
}

#[test]
fn review_regions_require_finite_positive_in_page_bounds() {
    for mode in 0..3 {
        let mut fixture = Fixture::positive();
        fixture.edit_review(|review| match mode {
            0 => review.regions[0].region.x0 = -1.0,
            1 => review.regions[0].region.x1 = review.regions[0].region.x0,
            2 => review.regions[0].region.y1 = 900.0,
            _ => unreachable!(),
        });
        assert!(
            TrustedReview::accept_explicitly(
                &fixture.review,
                &Digest::of(&fixture.review),
                "caller",
                &Limits::default()
            )
            .is_err()
        );
    }
}
