# Busybody Bob's separate read-only makeghrepo review

Exact scope: `benpshore/makeghrepo` PR142, head
`d6fae9a8e97eb40111742b9d2aebe034b9c657d1`, tree
`14865825f1ad543d1533db1121ab18bf3148501b`, base
`bf1ed67859d8786c16f41196765d3ef1582e685f`. The fetched detached checkout
and independently fetched GitHub PR metadata agree. Test merge-ref
`9c2ed68392fb39fb5225d4f7308c19f5273cb1c6` has the same candidate tree.
This review is separate from pdftextract's held hardening integration.

No new blocking code finding was identified in the bounded opt-out/default/
resume/live-rule/template review. This is engineering review evidence, not a
formal security assessment, exhaustive correctness proof or merge authorization.
The release side effect remains a concrete blocker for the parent to reconcile.
No makeghrepo edit, API mutation, protection change, agent spawn or merge occurred.

The four policy choices are consistent across Python/Rust payload construction:

| no-ci | no-pr | Generated merge/push requirements |
| --- | --- | --- |
| false | false | PR/review-thread resolution and strict Actions-bound ci |
| true | false | PR/review-thread resolution |
| false | true | Strict Actions-bound ci |
| true | true | Neither gate; history protections remain |

Both implementations build the established gate parameters then filter only the
selected rule types. They retain exact-owner bypass scope and history protection,
never submit an empty gate, and do not delete existing owner rulesets. Templates
carry the same two bools into AGENTS/README; workflow selection does not depend
on them. Generated policy says optional CI is enforcement, not disabled execution.
Neither flag changes release CI gates, local bootstrap checks or license defaults.
Private mode refuses these public-only options before rendering/configuration.

Resume uses explicit stored bools, written before a first-run smoke failure can
interrupt creation. Missing legacy fields mean false/enforced; an omitted current
flag preserves a recorded true; conflicting true against recorded false and invalid
marker types fail before smoke/commit/configuration. Both implementations then run
read-only opt-out preflight before settings/push/fanout. Existing omitted live rules,
legacy gate migration or introduction of an owner bypass to a stricter live gate are
refused before writes; remaining stricter parameters and unrelated rules survive.
Installation repeats validation; concurrent administrators remain a documented
nontransactional limit. This review did not exercise live GitHub protection APIs.

Fresh local stdlib execution: 12 creation-policy/owner-migration tests passed via
uv using the existing Python runtime, no installation or heavy compilation. I read
the CLI/resume/offline tests, renderer changes, shared four-case fixture and parity
harness; I did not locally rerun all hosted render/compiler suites. The review
checkout remained clean. Rust is unavailable in this environment.

Independently read hosted exact-head CI37214198335: all45 jobs are successful,
including required ci, generated-project and three native distribution jobs.
CodeQL37214198088 is also successful at the same SHA. Bounded log reads confirm
536 Python tests,24 Rust unit plus5 CLI tests,41 byte-identical Rust golden renders
and16 Python/Rust policy render comparisons including names/bytes/executable bits.
These are fresh hosted execution observations, not local rerun claims. Broader
bootstrap counts reported by Rick were not separately recomputed by this review.

The unchanged `.github/workflows/auto-release.yml` triggers on push to main
(lines46–49), waits for that exact commit's Actions ci (around lines100–116), creates
an annotated next-minor tag and builds (around lines130–145), then pushes the tag and
calls `gh release create` (lines146–152). Three native binaries are subsequently
attached. An ordinary successful merge can therefore publish a release automatically.
The flags do not disable this. A prior no-release instruction cannot be satisfied
by simply treating merge and publication as separate manual steps. Parent must
reconcile that consequence before exercising any otherwise conditional merge
permission; I did not weaken/cancel the workflow as a workaround.

Source links: [PR142](https://github.com/benpshore/makeghrepo/pull/142),
[exact Python rule handling](https://github.com/benpshore/makeghrepo/blob/d6fae9a8e97eb40111742b9d2aebe034b9c657d1/src/makeghrepo/github.py#L84),
[exact Rust rule handling](https://github.com/benpshore/makeghrepo/blob/d6fae9a8e97eb40111742b9d2aebe034b9c657d1/rust/src/github.rs#L124),
[release trigger/publication](https://github.com/benpshore/makeghrepo/blob/d6fae9a8e97eb40111742b9d2aebe034b9c657d1/.github/workflows/auto-release.yml#L46),
[hosted CI](https://github.com/benpshore/makeghrepo/actions/runs/37214198335),
[CodeQL](https://github.com/benpshore/makeghrepo/actions/runs/37214198088).

Responsible reviewer: Busybody Bob (Mara Keel alias), fingerprint
`sha256:2b6c731a6eed3711be7c08910d82c6a5fd60b74c96b9cceec0464400a53d1e6f`,
configuration `4c95bc39e9e3ba047139418455b3f6bdc3e61174472c9f614205a7aa124b2cb3`,
immutable steward task `pdftextract/stewardship/2026-10-04/busybody-bob`, parent
`01a0fe84-0716-7474-b2b7-709e4359b0fb`. Parent explicitly queued this bounded
review after the pdftextract checkpoint. Runtime model/tier and usage unknown;
registry hashes identify responsibility/content, not signing or proof of correctness.
