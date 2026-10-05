//! Experimental, explicitly trusted review-assisted region selection.
//!
//! Exact artifact retention comes from `tpe-region-evidence`. This crate creates
//! a separate span projection, never modifies a production extraction record,
//! and never treats agreement or an artifact hash as semantic correctness.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tpe_region_evidence::{
    ArtifactRef, Digest, GeometryFrame, Outcome, Sidecar, SourceIdentity, Validated,
};

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_review_bytes: u64,
    pub max_witnesses: usize,
    pub max_render_bytes: u64,
    pub max_total_render_bytes: u64,
    pub max_candidate_attempts: usize,
    pub max_candidate_artifact_bytes: u64,
    pub max_total_candidate_bytes: u64,
    pub max_span_comparisons: u64,
    pub max_projection_spans: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_review_bytes: 1024 * 1024,
            max_witnesses: 128,
            max_render_bytes: 32 * 1024 * 1024,
            max_total_render_bytes: 64 * 1024 * 1024,
            max_candidate_attempts: 8,
            max_candidate_artifact_bytes: 16 * 1024 * 1024,
            max_total_candidate_bytes: 64 * 1024 * 1024,
            max_span_comparisons: 100_000,
            max_projection_spans: 100_000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Box2 {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageGeometry {
    pub width: f64,
    pub height: f64,
    pub rotation: i32,
}

/// Actual runtime identity discovered by geometric matching, never supplied as
/// the correspondence to accept in a review record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanLocator {
    pub attempt_id: String,
    pub artifact: ArtifactRef,
    pub page_index: u32,
    pub page: u32,
    pub span_index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewProvenance {
    pub reviewer: String,
    pub method: String,
    pub renderer: String,
    pub renderer_config_digest: Digest,
    /// External review record reference, not a claim of cryptographic authorship.
    pub record_reference: String,
    /// How this source/render's canonical coordinate frame was established.
    pub frame_basis: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionReview {
    pub id: String,
    pub page_index: u32,
    pub page: u32,
    pub candidate_backend: String,
    pub candidate_version: Option<String>,
    pub frame: GeometryFrame,
    pub page_geometry: PageGeometry,
    /// Same-page region examined in the supplied render. The render may contain
    /// the whole page; its page/region association is the trusted reviewer's claim.
    pub region: Box2,
    pub expected_baseline_text: String,
    pub expected_candidate_text: String,
    /// Independently recorded reading of the rendered region, supplied as input.
    pub adjudicated_text: String,
    pub render_artifact: ArtifactRef,
    pub provenance: ReviewProvenance,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBundle {
    pub contract_version: u32,
    pub source: SourceIdentity,
    pub regions: Vec<RegionReview>,
}

/// Not deserializable. Only explicit caller acceptance can construct this type.
/// The trusted digest must come from caller policy, not the review file or PDF.
pub struct TrustedReview {
    bundle: ReviewBundle,
    digest: Digest,
    trusted_by: String,
    size: u64,
}

impl TrustedReview {
    pub fn accept_explicitly(
        bytes: &[u8],
        expected_digest: &Digest,
        trusted_by: &str,
        limits: &Limits,
    ) -> Result<Self> {
        ensure(
            bytes.len() as u64 <= limits.max_review_bytes,
            "review byte budget exceeded",
        )?;
        ensure(
            &Digest::of(bytes) == expected_digest,
            "trusted review digest mismatch",
        )?;
        nonempty(trusted_by, "caller trust authority")?;
        // All objects are typed with deny_unknown_fields; serde rejects duplicate
        // members at every depth. No untyped maps silently collapse duplicates.
        let bundle: ReviewBundle = serde_json::from_slice(bytes)
            .map_err(|error| invalid(format!("invalid review JSON: {error}")))?;
        ensure(
            bundle.contract_version == CONTRACT_VERSION,
            "unsupported review version",
        )?;
        ensure(
            bundle.regions.len() <= limits.max_witnesses,
            "review witness budget exceeded",
        )?;
        let mut ids = BTreeSet::new();
        for region in &bundle.regions {
            nonempty(&region.id, "review id")?;
            ensure(ids.insert(&region.id), "duplicate review id")?;
            ensure(region.region.valid(), "invalid/nonpositive review region")?;
            ensure(
                region.page_geometry.width.is_finite()
                    && region.page_geometry.width > 0.0
                    && region.page_geometry.height.is_finite()
                    && region.page_geometry.height > 0.0,
                "invalid review page dimensions",
            )?;
            ensure(
                matches!(region.page_geometry.rotation, 0 | 90 | 180 | 270),
                "invalid review rotation",
            )?;
            ensure(
                region.region.x0 >= 0.0
                    && region.region.y0 >= 0.0
                    && region.region.x1 <= region.page_geometry.width
                    && region.region.y1 <= region.page_geometry.height,
                "review region outside page",
            )?;
            ensure(region.page > 0, "invalid review page")?;
            nonempty(&region.candidate_backend, "candidate backend")?;
            if let Some(version) = &region.candidate_version {
                nonempty(version, "candidate version")?;
            }
            nonempty(&region.expected_baseline_text, "expected baseline text")?;
            nonempty(&region.expected_candidate_text, "expected candidate text")?;
            nonempty(&region.adjudicated_text, "adjudicated text")?;
            for (value, label) in [
                (&region.provenance.reviewer, "reviewer"),
                (&region.provenance.method, "review method"),
                (&region.provenance.renderer, "renderer"),
                (
                    &region.provenance.record_reference,
                    "review record reference",
                ),
                (&region.provenance.frame_basis, "frame basis"),
            ] {
                nonempty(value, label)?;
            }
            valid_digest(&region.provenance.renderer_config_digest)?;
            valid_digest(&region.render_artifact.sha256)?;
            ensure(region.render_artifact.size > 0, "empty render witness")?;
        }
        Ok(Self {
            bundle,
            digest: expected_digest.clone(),
            trusted_by: trusted_by.into(),
            size: bytes.len() as u64,
        })
    }

    pub fn bundle(&self) -> &ReviewBundle {
        &self.bundle
    }
    pub fn digest(&self) -> &Digest {
        &self.digest
    }
    pub fn trusted_by(&self) -> &str {
        &self.trusted_by
    }
}

#[derive(Default)]
pub struct WitnessStore(BTreeMap<Digest, Vec<u8>>);

impl WitnessStore {
    pub fn insert(&mut self, bytes: Vec<u8>, limits: &Limits) -> Result<ArtifactRef> {
        let reference = ArtifactRef {
            sha256: Digest::of(&bytes),
            size: bytes.len() as u64,
        };
        ensure(
            reference.size <= limits.max_render_bytes,
            "render artifact byte budget exceeded",
        )?;
        let additional = if self.0.contains_key(&reference.sha256) {
            0
        } else {
            reference.size
        };
        let total = self
            .total_bytes()?
            .checked_add(additional)
            .ok_or_else(|| invalid("render byte count overflow"))?;
        ensure(
            total <= limits.max_total_render_bytes,
            "total render byte budget exceeded",
        )?;
        self.0.entry(reference.sha256.clone()).or_insert(bytes);
        Ok(reference)
    }

    fn total_bytes(&self) -> Result<u64> {
        self.0.values().try_fold(0_u64, |total, bytes| {
            total
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("render byte count overflow"))
        })
    }

    fn contains_exact(&self, reference: &ArtifactRef) -> bool {
        self.0.get(&reference.sha256).is_some_and(|bytes| {
            bytes.len() as u64 == reference.size && Digest::of(bytes) == reference.sha256
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunOutcome {
    Evaluated,
    Cancelled,
    ResourceLimit { resource: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbstentionReason {
    ConflictingReviews,
    AmbiguousCandidateAttempts,
    IneligibleAttempt,
    UnknownFrame,
    PageGeometryMismatch,
    MissingOrInvalidSpanGeometry,
    CrossingSpan,
    AmbiguousRegion,
    ReviewedTextMismatch,
    CandidateDisagreesWithAdjudication,
    BaselineAlreadyMatchesAdjudication,
    WholeRunCancelled,
    WholeRunResourceLimit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum RegionDecision {
    Selected {
        review_id: String,
    },
    RetainedEqual {
        review_id: String,
    },
    Abstained {
        review_id: String,
        reason: AbstentionReason,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectedSpan {
    pub baseline: SpanLocator,
    pub selected: SpanLocator,
    pub text: String,
    pub review_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectedPage {
    pub page: u32,
    /// Baseline span array order, not reconstructed reading order or PageText.text.
    pub spans: Vec<ProjectedSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AcceptedReview {
    pub sha256: Digest,
    pub trusted_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DerivedView {
    pub contract_version: u32,
    pub policy: &'static str,
    /// Original evidence sidecar, preserving baseline/alternative statuses and
    /// artifact references. Every referenced artifact remains unchanged.
    pub evidence: Sidecar,
    pub accepted_review: Option<AcceptedReview>,
    pub outcome: RunOutcome,
    pub comparisons: u64,
    /// None means the projection itself exceeded its bound; use the complete
    /// original baseline artifact. A truncated prefix is never published.
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
fn invalid(message: impl Into<String>) -> Error {
    Error(message.into())
}
fn ensure(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(invalid(message))
    }
}
fn nonempty(value: &str, label: &str) -> Result<()> {
    ensure(
        !value.trim().is_empty(),
        &format!("{label} must not be empty"),
    )
}
fn valid_digest(digest: &Digest) -> Result<()> {
    ensure(
        digest.as_str().len() == 64
            && digest
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "invalid SHA-256 digest",
    )
}

impl Box2 {
    fn valid(self) -> bool {
        [self.x0, self.y0, self.x1, self.y1]
            .into_iter()
            .all(f64::is_finite)
            && self.x0 < self.x1
            && self.y0 < self.y1
    }
    fn contains(self, other: Self) -> bool {
        self.x0 <= other.x0 && self.y0 <= other.y0 && self.x1 >= other.x1 && self.y1 >= other.y1
    }
    fn intersects(self, other: Self) -> bool {
        self.x0 < other.x1 && self.x1 > other.x0 && self.y0 < other.y1 && self.y1 > other.y0
    }
}

/// Project all baseline spans in their existing array order and selectively use
/// unique candidates approved by an explicitly accepted external review.
/// All original artifacts remain unchanged in the caller's validated store.
/// Cancellation/comparison-budget exhaustion discards every proposed selection.
pub fn select(
    validated: &Validated<'_>,
    trusted: Option<&TrustedReview>,
    renders: &WitnessStore,
    limits: &Limits,
    cancellation: &AtomicBool,
) -> Result<DerivedView> {
    let evidence = validated.sidecar();
    let baseline: Value = serde_json::from_slice(validated.retained_baseline_bytes())
        .map_err(|error| invalid(format!("validated baseline JSON: {error}")))?;
    let mut view = DerivedView {
        contract_version: CONTRACT_VERSION,
        policy: "trusted_visible_transcription_v1",
        evidence: evidence.clone(),
        accepted_review: trusted.map(|review| AcceptedReview {
            sha256: review.digest.clone(),
            trusted_by: review.trusted_by.clone(),
        }),
        outcome: RunOutcome::Evaluated,
        comparisons: 0,
        pages: baseline_projection(&baseline, evidence, limits.max_projection_spans),
        regions: Vec::new(),
    };
    let Some(trusted) = trusted else {
        if view.pages.is_none() {
            view.outcome = resource("projection spans");
        }
        if cancellation.load(Ordering::Acquire) {
            view.outcome = RunOutcome::Cancelled;
        }
        return Ok(view);
    };
    ensure(
        trusted.bundle.source == evidence.source,
        "review source identity mismatch",
    )?;
    if cancellation.load(Ordering::Acquire) {
        return Ok(stopped(view, &trusted.bundle, Stop::Cancelled));
    }
    if view.pages.is_none() {
        return Ok(stopped(
            view,
            &trusted.bundle,
            Stop::Limit("projection spans"),
        ));
    }
    if trusted.size > limits.max_review_bytes {
        return Ok(stopped(view, &trusted.bundle, Stop::Limit("review bytes")));
    }
    if trusted.bundle.regions.len() > limits.max_witnesses {
        return Ok(stopped(
            view,
            &trusted.bundle,
            Stop::Limit("review witnesses"),
        ));
    }
    if evidence.alternatives.len() > limits.max_candidate_attempts {
        return Ok(stopped(
            view,
            &trusted.bundle,
            Stop::Limit("candidate attempts"),
        ));
    }
    if renders.total_bytes()? > limits.max_total_render_bytes
        || renders
            .0
            .values()
            .any(|bytes| bytes.len() as u64 > limits.max_render_bytes)
    {
        return Ok(stopped(view, &trusted.bundle, Stop::Limit("render bytes")));
    }
    let mut checked_renders = BTreeSet::new();
    for review in &trusted.bundle.regions {
        if cancellation.load(Ordering::Acquire) {
            return Ok(stopped(view, &trusted.bundle, Stop::Cancelled));
        }
        let page = baseline["pages"]
            .get(review.page_index as usize)
            .ok_or_else(|| invalid("review page index out of bounds"))?;
        ensure(
            page["page"].as_u64() == Some(u64::from(review.page)),
            "review page identity mismatch",
        )?;
        // Hash each distinct supplied render once, not once per region. This
        // validates bytes, not the external renderer's source/page association.
        if checked_renders.insert((&review.render_artifact.sha256, review.render_artifact.size)) {
            ensure(
                renders.contains_exact(&review.render_artifact),
                "review render artifact bytes/hash/size mismatch",
            )?;
        }
    }
    let mut total_candidate_bytes = 0_u64;
    let mut candidates = BTreeMap::new();
    for attempt in &evidence.alternatives {
        if cancellation.load(Ordering::Acquire) {
            return Ok(stopped(view, &trusted.bundle, Stop::Cancelled));
        }
        if let Some(bytes) = validated.alternative_bytes(&attempt.id) {
            total_candidate_bytes = total_candidate_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| invalid("candidate byte count overflow"))?;
            if bytes.len() as u64 > limits.max_candidate_artifact_bytes
                || total_candidate_bytes > limits.max_total_candidate_bytes
            {
                return Ok(stopped(
                    view,
                    &trusted.bundle,
                    Stop::Limit("candidate artifact bytes"),
                ));
            }
            let record: Value = serde_json::from_slice(bytes)
                .map_err(|error| invalid(format!("validated candidate JSON: {error}")))?;
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
        &trusted.bundle,
        &mut counter,
    );
    view.comparisons = counter.used;
    let (decisions, proposals) = match evaluated {
        Ok(evaluated) => evaluated,
        Err(stop) => return Ok(stopped(view, &trusted.bundle, stop)),
    };
    if cancellation.load(Ordering::Acquire) {
        return Ok(stopped(view, &trusted.bundle, Stop::Cancelled));
    }
    view.regions = decisions;
    for proposal in proposals {
        let row = &mut view.pages.as_mut().expect("bounded projection")[proposal.page_index].spans
            [proposal.baseline_span];
        row.text = proposal.text;
        row.selected = proposal.candidate;
        row.review_id = Some(proposal.review_id);
    }
    // If cancellation arrived during construction, atomically publish baseline
    // rows again. No already-selected prefix escapes this function.
    if cancellation.load(Ordering::Acquire) {
        view.pages = baseline_projection(&baseline, evidence, limits.max_projection_spans);
        return Ok(stopped(view, &trusted.bundle, Stop::Cancelled));
    }
    Ok(view)
}

fn baseline_projection(
    record: &Value,
    evidence: &Sidecar,
    limit: usize,
) -> Option<Vec<ProjectedPage>> {
    let pages = record["pages"].as_array()?;
    let mut count = 0_usize;
    let mut projection = Vec::new();
    for (page_index, page) in pages.iter().enumerate() {
        let spans = page["spans"].as_array()?;
        count = count.checked_add(spans.len())?;
        if count > limit {
            return None;
        }
        let mut rows = Vec::new();
        for (span_index, span) in spans.iter().enumerate() {
            let locator = SpanLocator {
                attempt_id: evidence.baseline.id.clone(),
                artifact: evidence.baseline.artifact.clone()?,
                page_index: u32::try_from(page_index).ok()?,
                page: u32::try_from(page["page"].as_u64()?).ok()?,
                span_index: u32::try_from(span_index).ok()?,
            };
            rows.push(ProjectedSpan {
                baseline: locator.clone(),
                selected: locator,
                text: span["text"].as_str()?.into(),
                review_id: None,
            });
        }
        projection.push(ProjectedPage {
            page: u32::try_from(page["page"].as_u64()?).ok()?,
            spans: rows,
        });
    }
    Some(projection)
}

#[derive(Clone, Copy)]
enum Stop {
    Cancelled,
    Limit(&'static str),
}
fn resource(resource: &str) -> RunOutcome {
    RunOutcome::ResourceLimit {
        resource: resource.into(),
    }
}
fn stopped(mut view: DerivedView, bundle: &ReviewBundle, stop: Stop) -> DerivedView {
    let (outcome, reason) = match stop {
        Stop::Cancelled => (RunOutcome::Cancelled, AbstentionReason::WholeRunCancelled),
        Stop::Limit(name) => (resource(name), AbstentionReason::WholeRunResourceLimit),
    };
    view.outcome = outcome;
    view.regions = bundle
        .regions
        .iter()
        .map(|review| RegionDecision::Abstained {
            review_id: review.id.clone(),
            reason: reason.clone(),
        })
        .collect();
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
    review_id: String,
    page_index: usize,
    baseline_span: usize,
    candidate: SpanLocator,
    text: String,
}

fn evaluate(
    baseline: &Value,
    candidates: &BTreeMap<&str, Value>,
    evidence: &Sidecar,
    bundle: &ReviewBundle,
    counter: &mut Counter<'_>,
) -> std::result::Result<(Vec<RegionDecision>, Vec<Proposal>), Stop> {
    let mut conflicts = BTreeSet::new();
    for (i, first) in bundle.regions.iter().enumerate() {
        for (j, second) in bundle.regions.iter().enumerate().skip(i + 1) {
            counter.charge()?;
            if first.page == second.page && first.region.intersects(second.region) {
                conflicts.insert(i);
                conflicts.insert(j);
            }
        }
    }
    let mut decisions = Vec::new();
    let mut proposals = Vec::new();
    for (index, review) in bundle.regions.iter().enumerate() {
        counter.charge()?;
        let abstain = |reason| RegionDecision::Abstained {
            review_id: review.id.clone(),
            reason,
        };
        if conflicts.contains(&index) {
            decisions.push(abstain(AbstentionReason::ConflictingReviews));
            continue;
        }
        let mut matching = Vec::new();
        for attempt in &evidence.alternatives {
            counter.charge()?;
            if attempt.backend.name == review.candidate_backend
                && review
                    .candidate_version
                    .as_ref()
                    .is_none_or(|version| version == &attempt.backend.version)
            {
                matching.push(attempt);
            }
        }
        if matching.len() > 1 {
            decisions.push(abstain(AbstentionReason::AmbiguousCandidateAttempts));
            continue;
        }
        let Some(attempt) = matching.first() else {
            decisions.push(abstain(AbstentionReason::IneligibleAttempt));
            continue;
        };
        if !matches!(attempt.outcome, Outcome::Complete | Outcome::Partial { .. }) {
            decisions.push(abstain(AbstentionReason::IneligibleAttempt));
            continue;
        }
        if review.frame != GeometryFrame::ProducerDeclaredPdfUserSpaceUnrotated
            || attempt.frame != review.frame
            || evidence.baseline.frame != review.frame
        {
            decisions.push(abstain(AbstentionReason::UnknownFrame));
            continue;
        }
        let Some(candidate) = candidates.get(attempt.id.as_str()) else {
            decisions.push(abstain(AbstentionReason::IneligibleAttempt));
            continue;
        };
        let base_page = &baseline["pages"][review.page_index as usize];
        let candidate_page = &candidate["pages"][review.page_index as usize];
        if !geometry_matches(base_page, &review.page_geometry)
            || !geometry_matches(candidate_page, &review.page_geometry)
        {
            decisions.push(abstain(AbstentionReason::PageGeometryMismatch));
            continue;
        }
        let baseline_span = match unique_span(base_page, review.region, counter)? {
            Ok(index) => index,
            Err(reason) => {
                decisions.push(abstain(reason));
                continue;
            }
        };
        let candidate_span = match unique_span(candidate_page, review.region, counter)? {
            Ok(index) => index,
            Err(reason) => {
                decisions.push(abstain(reason));
                continue;
            }
        };
        let baseline_text = base_page["spans"][baseline_span]["text"]
            .as_str()
            .expect("validated text");
        let candidate_text = candidate_page["spans"][candidate_span]["text"]
            .as_str()
            .expect("validated text");
        if baseline_text != review.expected_baseline_text
            || candidate_text != review.expected_candidate_text
        {
            decisions.push(abstain(AbstentionReason::ReviewedTextMismatch));
            continue;
        }
        if candidate_text == baseline_text {
            decisions.push(RegionDecision::RetainedEqual {
                review_id: review.id.clone(),
            });
            continue;
        }
        if baseline_text == review.adjudicated_text {
            decisions.push(abstain(
                AbstentionReason::BaselineAlreadyMatchesAdjudication,
            ));
            continue;
        }
        if candidate_text != review.adjudicated_text {
            decisions.push(abstain(
                AbstentionReason::CandidateDisagreesWithAdjudication,
            ));
            continue;
        }
        proposals.push(Proposal {
            review_id: review.id.clone(),
            page_index: review.page_index as usize,
            baseline_span,
            candidate: SpanLocator {
                attempt_id: attempt.id.clone(),
                artifact: attempt.artifact.clone().expect("candidate artifact"),
                page_index: review.page_index,
                page: review.page,
                span_index: u32::try_from(candidate_span).expect("bounded span index"),
            },
            text: candidate_text.into(),
        });
        decisions.push(RegionDecision::Selected {
            review_id: review.id.clone(),
        });
    }
    Ok((decisions, proposals))
}

fn geometry_matches(page: &Value, geometry: &PageGeometry) -> bool {
    page["width"].as_f64() == Some(geometry.width)
        && page["height"].as_f64() == Some(geometry.height)
        && page["rotation"].as_i64() == Some(i64::from(geometry.rotation))
}

fn unique_span(
    page: &Value,
    region: Box2,
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
        let Ok(bbox) = serde_json::from_value::<Box2>(span["bbox"].clone()) else {
            rejection = Some(AbstentionReason::MissingOrInvalidSpanGeometry);
            continue;
        };
        if !bbox.valid()
            || bbox.x0 < 0.0
            || bbox.y0 < 0.0
            || bbox.x1 > page["width"].as_f64().expect("validated width")
            || bbox.y1 > page["height"].as_f64().expect("validated height")
        {
            rejection = Some(AbstentionReason::MissingOrInvalidSpanGeometry);
            continue;
        }
        if region.intersects(bbox) {
            if !region.contains(bbox) {
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
