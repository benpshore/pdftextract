//! Opt-in region selection from a deliberately restricted source interpreter.
//!
//! The selector receives no transcription, answer key, or approved review. It
//! recomputes source witnesses, and uses only unique geometric correspondence
//! and exact source-declared Unicode agreement. This is source consistency,
//! not independent proof of the meaning of rendered pixels.

pub mod source;

use serde::Serialize;
use serde_json::Value;
use source::{SourceBox, SourceLimits, SourceOutcome, SourcePage, VerifiedSource};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};
use tpe_region_evidence::{
    ArtifactRef, GeometryFrame, Outcome, Sidecar, SourceIdentity, Validated,
};

pub const CONTRACT_VERSION: u32 = 1;
pub const POLICY: &str = "source_declared_unicode_v1";

#[derive(Clone, Debug)]
pub struct Limits {
    pub source: SourceLimits,
    pub max_source_witnesses: usize,
    pub max_candidate_attempts: usize,
    pub max_candidate_artifact_bytes: u64,
    pub max_total_candidate_bytes: u64,
    pub max_span_comparisons: u64,
    pub max_projection_spans: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            source: SourceLimits::default(),
            max_source_witnesses: 1024,
            max_candidate_attempts: 8,
            max_candidate_artifact_bytes: 16 * 1024 * 1024,
            max_total_candidate_bytes: 64 * 1024 * 1024,
            max_span_comparisons: 100_000,
            max_projection_spans: 100_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SpanLocator {
    pub attempt_id: String,
    pub artifact: ArtifactRef,
    pub page_index: u32,
    pub page: u32,
    pub span_index: u32,
}

/// Index into `source_evidence.pages`, not an index supplied by the caller.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WitnessLocator {
    pub source_page_index: u32,
    pub page: u32,
    pub witness_index: u32,
    pub witness_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunOutcome {
    Evaluated,
    UnsupportedSource,
    SourceMismatch,
    SourcePageCoverageMismatch,
    IneligibleBaseline,
    Cancelled,
    ResourceLimit { resource: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstentionReason {
    ConflictingSourceRegions,
    AmbiguousCandidateAttempts,
    NoEligibleCandidate,
    UnknownFrame,
    PageGeometryMismatch,
    MissingOrInvalidSpanGeometry,
    CrossingSpan,
    AmbiguousRegion,
    CandidateDisagreesWithSource,
    WholeRunCancelled,
    WholeRunResourceLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RegionDecision {
    Selected {
        witness: WitnessLocator,
    },
    RetainedSourceConsistent {
        witness: WitnessLocator,
    },
    Abstained {
        witness: WitnessLocator,
        reason: AbstentionReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectedSpan {
    pub baseline: SpanLocator,
    pub selected: SpanLocator,
    pub text: String,
    pub source_witness: Option<WitnessLocator>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectedPage {
    pub page: u32,
    /// Baseline span array order; this does not reconstruct reading order.
    pub spans: Vec<ProjectedSpan>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DerivedView {
    pub contract_version: u32,
    pub policy: &'static str,
    /// Original attempt status, runtime declarations, and artifact identities.
    /// Exact original bytes remain in the caller's immutable ArtifactStore.
    pub evidence: Sidecar,
    /// Computed by this invocation; no deserialized witness can authorize a selection.
    pub source_evidence: VerifiedSource,
    pub outcome: RunOutcome,
    pub comparisons: u64,
    /// None means the entire projection exceeded its bound. Use the unchanged
    /// original baseline artifact, never a truncated prefix.
    pub pages: Option<Vec<ProjectedPage>>,
    pub regions: Vec<RegionDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// Retain the complete lopdf baseline and propose a candidate only when a
/// freshly computed source witness owns exactly one span in each artifact.
/// No length ranking, confidence score, voting, or external answer is accepted.
pub fn select(
    validated: &Validated<'_>,
    source_bytes: &[u8],
    limits: &Limits,
    cancellation: &AtomicBool,
) -> Result<DerivedView> {
    let evidence = validated.sidecar();
    let baseline: Value = serde_json::from_slice(validated.retained_baseline_bytes())
        .map_err(|error| Error(format!("validated baseline JSON: {error}")))?;
    let source_evidence = source::verify_source(source_bytes, &limits.source, cancellation);
    let mut view = DerivedView {
        contract_version: CONTRACT_VERSION,
        policy: POLICY,
        evidence: evidence.clone(),
        source_evidence,
        outcome: RunOutcome::Evaluated,
        comparisons: 0,
        pages: baseline_projection(&baseline, evidence, limits.max_projection_spans),
        regions: Vec::new(),
    };
    if cancellation.load(Ordering::Acquire)
        || *view.source_evidence.outcome() == SourceOutcome::Cancelled
    {
        return Ok(stopped(view, Stop::Cancelled));
    }
    if view.pages.is_none() {
        return Ok(stopped(view, Stop::Limit("projection spans")));
    }
    if SourceIdentity::of(source_bytes) != evidence.source
        || view.source_evidence.source() != &evidence.source
    {
        view.outcome = RunOutcome::SourceMismatch;
        return Ok(view);
    }
    match view.source_evidence.outcome() {
        SourceOutcome::Supported => {}
        SourceOutcome::Unsupported => {
            view.outcome = RunOutcome::UnsupportedSource;
            return Ok(view);
        }
        SourceOutcome::ResourceLimit => {
            return Ok(stopped(view, Stop::Limit("source verification")));
        }
        SourceOutcome::Cancelled => return Ok(stopped(view, Stop::Cancelled)),
    }
    if evidence.baseline.backend.name != "lopdf" || !eligible(&evidence.baseline.outcome) {
        view.outcome = RunOutcome::IneligibleBaseline;
        return Ok(view);
    }
    if evidence.pages.iter().any(|number| {
        view.source_evidence
            .pages()
            .iter()
            .filter(|page| page.page == *number)
            .count()
            != 1
    }) {
        view.outcome = RunOutcome::SourcePageCoverageMismatch;
        return Ok(view);
    }
    let witness_count = view
        .source_evidence
        .pages()
        .iter()
        .try_fold(0_usize, |sum, page| sum.checked_add(page.witnesses.len()));
    if witness_count.is_none_or(|count| count > limits.max_source_witnesses) {
        return Ok(stopped(view, Stop::Limit("source witnesses")));
    }
    if evidence.alternatives.len() > limits.max_candidate_attempts {
        return Ok(stopped(view, Stop::Limit("candidate attempts")));
    }
    let mut candidates = BTreeMap::new();
    let mut total_bytes = 0_u64;
    for attempt in &evidence.alternatives {
        if cancellation.load(Ordering::Acquire) {
            return Ok(stopped(view, Stop::Cancelled));
        }
        if let Some(bytes) = validated.alternative_bytes(&attempt.id) {
            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            if bytes.len() as u64 > limits.max_candidate_artifact_bytes
                || total_bytes > limits.max_total_candidate_bytes
            {
                return Ok(stopped(view, Stop::Limit("candidate artifact bytes")));
            }
            let record: Value = serde_json::from_slice(bytes)
                .map_err(|error| Error(format!("validated candidate JSON: {error}")))?;
            candidates.insert(attempt.id.as_str(), record);
        }
    }
    let mut counter = Counter {
        used: 0,
        limit: limits.max_span_comparisons,
        cancellation,
    };
    let evaluated = evaluate(
        &baseline,
        &candidates,
        evidence,
        &view.source_evidence,
        &mut counter,
    );
    view.comparisons = counter.used;
    let (decisions, proposals) = match evaluated {
        Ok(value) => value,
        Err(stop) => return Ok(stopped(view, stop)),
    };
    if cancellation.load(Ordering::Acquire) {
        return Ok(stopped(view, Stop::Cancelled));
    }
    view.regions = decisions;
    for proposal in proposals {
        let row = &mut view.pages.as_mut().expect("bounded complete projection")
            [proposal.page_index]
            .spans[proposal.baseline_span];
        row.text = proposal.text;
        row.selected = proposal.candidate;
        row.source_witness = Some(proposal.witness);
    }
    if cancellation.load(Ordering::Acquire) {
        view.pages = baseline_projection(&baseline, evidence, limits.max_projection_spans);
        return Ok(stopped(view, Stop::Cancelled));
    }
    Ok(view)
}

fn eligible(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Complete | Outcome::Partial { .. })
}

fn baseline_projection(
    record: &Value,
    evidence: &Sidecar,
    limit: usize,
) -> Option<Vec<ProjectedPage>> {
    let mut count = 0_usize;
    let mut pages = Vec::new();
    for (page_index, page) in record["pages"].as_array()?.iter().enumerate() {
        let spans = page["spans"].as_array()?;
        count = count.checked_add(spans.len())?;
        if count > limit {
            return None;
        }
        let page_number = u32::try_from(page["page"].as_u64()?).ok()?;
        let mut rows = Vec::new();
        for (span_index, span) in spans.iter().enumerate() {
            let locator = SpanLocator {
                attempt_id: evidence.baseline.id.clone(),
                artifact: evidence.baseline.artifact.clone()?,
                page_index: u32::try_from(page_index).ok()?,
                page: page_number,
                span_index: u32::try_from(span_index).ok()?,
            };
            rows.push(ProjectedSpan {
                baseline: locator.clone(),
                selected: locator,
                text: span["text"].as_str()?.into(),
                source_witness: None,
            });
        }
        pages.push(ProjectedPage {
            page: page_number,
            spans: rows,
        });
    }
    Some(pages)
}

#[derive(Clone, Copy)]
enum Stop {
    Cancelled,
    Limit(&'static str),
}
fn stopped(mut view: DerivedView, stop: Stop) -> DerivedView {
    match stop {
        Stop::Cancelled => {
            view.outcome = RunOutcome::Cancelled;
        }
        Stop::Limit(resource) => {
            view.outcome = RunOutcome::ResourceLimit {
                resource: resource.into(),
            };
        }
    }
    // Do not allocate an unbounded list while reporting an exhausted budget.
    // A whole-run outcome and unchanged baseline are the authoritative result.
    view.regions.clear();
    for page in view.pages.iter_mut().flatten() {
        for span in &mut page.spans {
            span.source_witness = None;
        }
    }
    view
}
struct Counter<'a> {
    used: u64,
    limit: u64,
    cancellation: &'a AtomicBool,
}
impl Counter<'_> {
    fn charge(&mut self) -> std::result::Result<(), Stop> {
        if self.cancellation.load(Ordering::Acquire) {
            return Err(Stop::Cancelled);
        }
        if self.used >= self.limit {
            return Err(Stop::Limit("span/region comparisons"));
        }
        self.used += 1;
        Ok(())
    }
}
struct Proposal {
    witness: WitnessLocator,
    page_index: usize,
    baseline_span: usize,
    candidate: SpanLocator,
    text: String,
}
type Evaluation = (Vec<RegionDecision>, Vec<Proposal>);

fn evaluate(
    baseline: &Value,
    candidates: &BTreeMap<&str, Value>,
    evidence: &Sidecar,
    source: &VerifiedSource,
    counter: &mut Counter<'_>,
) -> std::result::Result<Evaluation, Stop> {
    let mut decisions = Vec::new();
    let mut proposals = Vec::new();
    for (source_page_index, page) in source.pages().iter().enumerate() {
        let Some(page_index) = evidence
            .pages
            .iter()
            .position(|number| *number == page.page)
        else {
            continue;
        };
        let mut conflicts = BTreeSet::new();
        for (i, first) in page.witnesses.iter().enumerate() {
            for (j, second) in page.witnesses.iter().enumerate().skip(i + 1) {
                counter.charge()?;
                if intersects(first.region, second.region) {
                    conflicts.insert(i);
                    conflicts.insert(j);
                }
            }
        }
        for (witness_index, item) in page.witnesses.iter().enumerate() {
            counter.charge()?;
            let witness = WitnessLocator {
                source_page_index: source_page_index as u32,
                page: page.page,
                witness_index: witness_index as u32,
                witness_id: item.id.clone(),
            };
            let abstain = |reason| RegionDecision::Abstained {
                witness: witness.clone(),
                reason,
            };
            if conflicts.contains(&witness_index) {
                decisions.push(abstain(AbstentionReason::ConflictingSourceRegions));
                continue;
            }
            if evidence.baseline.frame != GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated {
                decisions.push(abstain(AbstentionReason::UnknownFrame));
                continue;
            }
            let base_page = &baseline["pages"][page_index];
            if !geometry_matches(base_page, page) {
                decisions.push(abstain(AbstentionReason::PageGeometryMismatch));
                continue;
            }
            let baseline_span = match unique_span(base_page, item.region, counter)? {
                Ok(index) => index,
                Err(reason) => {
                    decisions.push(abstain(reason));
                    continue;
                }
            };
            let baseline_text = base_page["spans"][baseline_span]["text"]
                .as_str()
                .expect("validated text");
            if baseline_text == item.unicode {
                decisions.push(RegionDecision::RetainedSourceConsistent { witness });
                continue;
            }
            let mut matching = Vec::new();
            for attempt in &evidence.alternatives {
                counter.charge()?;
                if eligible(&attempt.outcome) && candidates.contains_key(attempt.id.as_str()) {
                    matching.push(attempt);
                }
            }
            if matching.len() > 1 {
                decisions.push(abstain(AbstentionReason::AmbiguousCandidateAttempts));
                continue;
            }
            let Some(attempt) = matching.first() else {
                decisions.push(abstain(AbstentionReason::NoEligibleCandidate));
                continue;
            };
            if attempt.frame != GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated {
                decisions.push(abstain(AbstentionReason::UnknownFrame));
                continue;
            }
            let candidate_page = &candidates[attempt.id.as_str()]["pages"][page_index];
            if !geometry_matches(candidate_page, page) {
                decisions.push(abstain(AbstentionReason::PageGeometryMismatch));
                continue;
            }
            let candidate_span = match unique_span(candidate_page, item.region, counter)? {
                Ok(index) => index,
                Err(reason) => {
                    decisions.push(abstain(reason));
                    continue;
                }
            };
            let candidate_text = candidate_page["spans"][candidate_span]["text"]
                .as_str()
                .expect("validated text");
            if candidate_text != item.unicode {
                decisions.push(abstain(AbstentionReason::CandidateDisagreesWithSource));
                continue;
            }
            proposals.push(Proposal {
                witness: witness.clone(),
                page_index,
                baseline_span,
                candidate: SpanLocator {
                    attempt_id: attempt.id.clone(),
                    artifact: attempt.artifact.clone().expect("eligible artifact"),
                    page_index: page_index as u32,
                    page: page.page,
                    span_index: candidate_span as u32,
                },
                text: candidate_text.into(),
            });
            decisions.push(RegionDecision::Selected { witness });
        }
    }
    Ok((decisions, proposals))
}

fn geometry_matches(actual: &Value, source: &SourcePage) -> bool {
    actual["page"].as_u64() == Some(u64::from(source.page))
        && actual["width"].as_f64() == Some(source.width)
        && actual["height"].as_f64() == Some(source.height)
        && actual["rotation"].as_i64() == Some(i64::from(source.rotation))
}
fn valid(b: SourceBox) -> bool {
    [b.x0, b.y0, b.x1, b.y1].into_iter().all(f64::is_finite) && b.x0 < b.x1 && b.y0 < b.y1
}
fn contains(a: SourceBox, b: SourceBox) -> bool {
    a.x0 <= b.x0 && a.y0 <= b.y0 && a.x1 >= b.x1 && a.y1 >= b.y1
}
fn intersects(a: SourceBox, b: SourceBox) -> bool {
    a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0
}
fn span_box(value: &Value) -> Option<SourceBox> {
    Some(SourceBox {
        x0: value["x0"].as_f64()?,
        y0: value["y0"].as_f64()?,
        x1: value["x1"].as_f64()?,
        y1: value["y1"].as_f64()?,
    })
}
fn unique_span(
    page: &Value,
    region: SourceBox,
    counter: &mut Counter<'_>,
) -> std::result::Result<std::result::Result<usize, AbstentionReason>, Stop> {
    let mut inside = None;
    let mut rejection = None;
    for (index, span) in page["spans"]
        .as_array()
        .expect("validated spans")
        .iter()
        .enumerate()
    {
        counter.charge()?;
        let Some(bbox) = span_box(&span["bbox"]) else {
            rejection = Some(AbstentionReason::MissingOrInvalidSpanGeometry);
            continue;
        };
        if !valid(bbox)
            || bbox.x0 < 0.0
            || bbox.y0 < 0.0
            || bbox.x1 > page["width"].as_f64().expect("validated width")
            || bbox.y1 > page["height"].as_f64().expect("validated height")
        {
            rejection = Some(AbstentionReason::MissingOrInvalidSpanGeometry);
            continue;
        }
        if intersects(region, bbox) {
            if !contains(region, bbox) {
                rejection = Some(AbstentionReason::CrossingSpan);
                continue;
            }
            if inside.replace(index).is_some() || span["text"].as_str().is_none_or(str::is_empty) {
                rejection = Some(AbstentionReason::AmbiguousRegion);
            }
        }
    }
    Ok(if let Some(reason) = rejection {
        Err(reason)
    } else {
        inside.ok_or(AbstentionReason::AmbiguousRegion)
    })
}
