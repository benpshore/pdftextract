Separate makeghrepo read-only review checkpoint: 103 records / `63f7140fb99ef5a728bffa51d507bc391a5cb37daee1da9be5a86040f1999f9e`; prior102-record prefix unchanged. [Review/release blocker](stewardship/makeghrepo-review-20261004.md). This grants no pdftextract promotion authority.

Current reviewer-registration checkpoint: 2026-10-04T16:10:45Z; 102 records / `72de7bf12c68903f9a483f33cd2b6db14232d5f5f438b13be5c07c65738d367e`. Prior 100-record checkpoint `9d9cc6c364c9be91174ece3b694e2df67d635aa4a0d74d465428961288ee88e0` remains an unchanged prefix. Independent reports cover frozen #234 head82f95ad; focused repairs on Bob WIP require delta review. Archie supplied canonical identity/configuration registered after receipt; Sylvie bytes and original companion artifacts still requested. See [registration receipt](stewardship/independent-review-registration-20261004.json). Older final checkpoints below are historical. Main/release/deploy remain held.

# Repository stewardship ledger — 2026-10-04

Owner: **Busybody Bob** (alias **Mara Keel**). [Agent maintenance epic #225](https://github.com/benpshore/pdftextract/issues/225).
Next review: **2026-10-05 UTC**, or immediately after main/active-head changes. This is a review date, not an automation.
Scope authority: Ben Shore's sequential delegation in source thread `01a0fe84-0716-7474-b2b7-709e4359b0fb`.
Root owns queue/review/follow-through. No child agents, feature fanout, main merges, releases, deployment or Mac execution.

## Current hold and queued work — live-call updates

Ben briefly authorized #192/#228/#229 merges, then put **all pending merges on hold**
at 2026-10-04 13:51:48 UTC. No merge was attempted or completed. All source heads and
main remain unchanged. Preparation added and verified only
`archive/steward-20261004/pr-192-b315c36a9d93` at
`b315c36a9d93c03ef1acac9339f758431587ff51`; source branches/reviews remain.
[Hold/version-mechanics receipt](https://github.com/benpshore/pdftextract/issues/225#issuecomment-5980730852).

#222 is now **open/draft**, at unchanged `6fc3266e0860de85b0bf5bd39a364d783a1a2796`,
after Ben explicitly requested not-ready status. [Draft receipt](https://github.com/benpshore/pdftextract/pull/222#issuecomment-5980737771).
The [limited source/usability observations](https://github.com/benpshore/pdftextract/pull/222#issuecomment-5980868244)
are not a completed security audit or validated patch-risk assessment. All seven changed
files and direct capture/assets/error/UI callers were read; fetch/DNS/redirect application
logic is byte-identical after the added contract comment. The changed runtime flag affects
same-zone routing; deployed consequences remain unverified. Main-based #222 does not include
held #229's network-off guards. Those guards/local-file segmentation must be preserved in
any later explicitly scoped capture reimplementation. No source rewrite or defense removal.

Exact-head CI/Web and its added test step are green; the 155-file source manifest was
independently hash-checked. Hosted mocked/local connection tests do not qualify production
own-zone routing, deployed provider policy, real devices or the held-stack combination.
Formal security diff preflight reference/tools are missing. Patch-risk shared storage/schema
references are missing; its validator source/risk rubric are readable. No substitute scan,
scan goal, sealed report, risk rating or clearance was produced. [Resource refinement](https://github.com/benpshore/pdftextract/pull/222#issuecomment-5980887800).
#215 remains untouched/open. Ben now defers further hardening while prioritizing usable
Bear-like ingestion including images; this does not authorize silent protection/config changes.

The [machine-readable queue](stewardship/work-queue.json) and agent-labeled issues below
are durable work records, **queued, not started**. Owner: Bob. Review date: 2026-10-05 UTC
or explicit parent resume, not an automation. No separate research agents are launched.

| Queue | Acceptance / provenance |
|---|---|
| [#233 Alpha v2 current design priority](https://github.com/benpshore/pdftextract/issues/233) | Remove large hero; upload **symbol** with tooltip, accessible name/focus and sensible iPhone touch; two-line saved-article menu preserves content/state. Earlier permanent Upload-word interpretation is superseded. |
| [#230 deferred research 1](https://github.com/benpshore/pdftextract/issues/230) | Zotero PDF metadata/configuration and actual GROBID browser/native/service execution; distinguish fixtures/replay from execution. |
| [#231 deferred research 2](https://github.com/benpshore/pdftextract/issues/231) | Verify Bear sources/strategy and smallest practical capture path **including images**; PWA cannot be assumed to read another tab's DOM. Initial Clipper Clive sources are preliminary/incomplete, not completed research. |
| [#232 deferred research 3](https://github.com/benpshore/pdftextract/issues/232) | Verify PDFium/Docling.rs binding/routing/build/packaging and scholarly bibliography end to end, separately from generic web hyperlinks. Selectable Crossref-validated title-linked entries, Select all, Zotero RDF default/alternatives and real import/round-trip proof; preserve raw citation/status/provenance. |

Bibliography checkbox selection is independent of following the verified paper-title link.
A landing-page link does not promise open full text. Crossref uses only an explicitly
permitted network path. Parent suggestions for an Export selected button/selection count
are recorded as unconfirmed advice, not Ben's accepted implementation. All detailed done
criteria and original live-call timestamps remain in the linked issues; user issues are untouched.

No version bump has occurred. Private web `0.1.0` → `0.2.0` is a suitable minor proposal
for integrated behavior, queued with #233. Published project `v0.55.0` is unchanged;
Git tags derive public CLI/Python versions, and main/tag operations automatically publish.
Separate release authority and reconciled merge hold are needed before those operations.
No third-party dependency or unrelated toolchain pin is changed.

## Read this first

Live main was independently fetched at `72890e6f9d23b1c10ee2cd9d368c7c0817319c10`.
Ben's recent squash merges #168/#173/#212/#219 are preserved. Initial bounded snapshot: **89 open PRs**.
The later web-stack repair is recorded below; this initial count is not a current global inventory.
No old head is an ancestor of current main; squash ancestry cannot decide supersession.
The machine-readable [repository map](stewardship/repository-map.json) has full exact
heads/bases, contribution paths, containment, owners, next actions/reviews and done criteria.
[Branch snapshot](stewardship/branches.json) includes heads without open PRs; unmapped branches
are preserved with ownership/purpose unresolved. [Action history](stewardship/actions.jsonl)
records this steward's decisions and receipts; [policy](AGENT_ACCOUNTABILITY.md) explains assurance.

## Maintenance queue

| Concern | Owner / next action | Done criteria |
|---|---|---|
| Version control | Bob: individually archive exact superseded heads, then reversible closure; keep branch copies and provenance | Archive SHA equals inspected head; closure receipt verified; reopening instructions retained |
| Distribution/build | Bob coordinates existing #201/#153/#150; human #220/#5 are read-only requirements | Exact candidate identity and locked builds agree; interrupted-release recovery tested before separate promotion |
| Dependencies/toolchain | Bob reviews #114/#83/#78/#214 against current manifests; no automatic update fanout | Deliberate pin rationale, compatibility evidence, owner/review date, unchanged working accepted pins |
| Runtime/browser compatibility | Existing native/web owners, reported idle; Bob owns queue without reactivating them | Linux x64/ARM64 and macOS boundaries qualified; real browser/Workers contract documented; exact evidence and limitations |
| Performance | #224 agent ticket, build owner evidence pending | Controlled same-input dev/release timings identify contributors; currently cause unknown; no heavy benchmark or Mac execution |
| Accountability | Bob: identity registry, all-action records, voluntary trailer checks and external checkpoint receipts | Human exemption/agent requirement/review-only and integrity fixtures pass; missing usage/platform evidence explicit |

Rust is pinned to **1.98.1** (`rust-toolchain.toml`), with lockfiles and existing checksum-pinned
Actions/native/container artifacts. Dependabot currently uses weekly grouped updates and a 7-day
cooldown. PR114's old claim that Rust floats is stale; PR83's scheduled write-token update mechanism
is a candidate, not accepted policy. Reconcile their residuals rather than land either full old tree.
PR78/#83 are blocked by failed exact-head required ci (cancelled macOS jobs retained).
PR214 is a Dependabot web candidate, separately qualified against Workers/browser/runtime tests.
No pins, toolchain or automatic-update permissions are changed by this stewardship draft.

## Active dependency map and gates

- Native: #175 → #191 → #194 → #203 → #204 → #205 → #223.
  #172/#194/#203/#204 contribution files exactly equal current main. Closing those PRs
  does not remove their branches or erase #205's base. #175 remains a broad unqualified
  draft; its residual feature requirements are not accepted merely because some repairs landed.
- Web: #175 → #192 → **#228 → #229**. #187/#188/#189 heads are contained by #192
  and retained as duplicate source/history, not independently promoted.
  Original #211 descended from #207's original head while #192 contained its squash.
  Both base trees are identical. New single-parent #228/#229 preserve the entire original
  #211/#221 trees; original drafts are archived and closed without merging. No source
  branch is rewritten. Broader #175/main qualification remains a separate gate.
- OCR/GROBID #206 remains based on #175. #202 retains the isolated region contract plus merged #208 reviewed whole-span pilot;
  #207/#208/#209/#210/#213/#218 are verified merged into their feature-stack bases,
  rather than main. Their exact heads/bases are retained in
  [merged-stack-prs.json](stewardship/merged-stack-prs.json). Do not reopen or reland them separately.
- Native #223 `7385a3bcd4592a2071cd5eb93d566fe4ec472723` and repaired web #229
  `b752bae170ae628c07b12800a44a0a37b235db28` retain network-off boundaries.
  Their exact-head required ci was independently read back as terminal success;
  this is not current-main integration, full quality or security qualification.
- Frozen native `eb5f711b` evidence (parent-reported) includes containment18/startup2/PDFium
  smokes2, but **209 genuine Partials remain a gate**. No source quality waiver is granted.
- Separate #222 stays open at `6fc3266e0860de85b0bf5bd39a364d783a1a2796`.
  Its body reports private Site submission from `9fbca29d8a22e4b2e8f580946e282e722ebaaf33`;
  deployed runtime is unverified here. Its [review comment](https://github.com/benpshore/pdftextract/pull/222#issuecomment-5977112347)
  reports a usage-limit block. #215 remains open; functional CI is not security clearance.
  Mandatory formal security resources remain unavailable in prior context.

Browser is one endpoint for a reusable Rust core. Eventual GPUI/native, Office/Excel,
Slack/Dropbox thin adapters share extraction/provenance with endpoint-specific permissions.
Browser-first session/user-bound processing with fewer server-retained files is an intentional
security goal. These future directions do not authorize implementations here and do not make
native work redundant.

## Issue provenance and preservation

User-authored issues remain entirely untouched, including #220/#215/#216/#217 and any
uncertain historical issue. A shared GitHub username never decides agent authorship.
#224 is explicitly identified by the parent as a Luna-created issue; its original body
is preserved, with an additive agent-created label and provenance comment. #225 is newly
agent-created and labeled. New explicitly requested [#227](https://github.com/benpshore/pdftextract/issues/227) is agent-created
and attributed to Bob; it covers region plus within-page chunk schemas, alignment and bounded streaming.
Other historical issue authorship is unresolved, so their
labels/bodies/state/comments are preserved; [historical attribution](stewardship/historical-attribution.json)
records unknowns and body hashes. Existing epic scopes #176–#183/#199 do not exactly cover
maintenance, so #225 fills that gap without editing those issues.

All newly created agent issues need `agent-created` plus explicit agent/task attribution.
Known historical review/research agents require evidence before retrospective linkage:
their missing names/configurations/fingerprints are not fabricated. ChatGPTWork is registered
for the current observed parent delegation, not falsely attributed to all earlier work.

## Verified cleanup and rationale receipts

Six superseded PRs are closed **without merging**. Their individually named archive refs
were independently read back at the exact heads before closure; original branches remain.
Full method/receipt/reopening instructions: [cleanup-decisions.json](stewardship/cleanup-decisions.json).

| PR | Preserved archive ref | Exact head | Decision receipt |
|---|---|---|---|
| #172 | `archive/steward-20261004/pr-172-5c6225c7e1dc` | `5c6225c7e1dc76cea97fca6d3019cc31094fa8ac` | [receipt](https://github.com/benpshore/pdftextract/pull/172#issuecomment-5979663305) |
| #194 | `archive/steward-20261004/pr-194-0ece13395f83` | `0ece13395f8372a6e9b32aa93f16b612a850b0fe` | [receipt](https://github.com/benpshore/pdftextract/pull/194#issuecomment-5979665508) |
| #203 | `archive/steward-20261004/pr-203-5cd375e6b2c4` | `5cd375e6b2c46792cd5f70cf9ab58e990ebbe5d0` | [receipt](https://github.com/benpshore/pdftextract/pull/203#issuecomment-5979667406) |
| #204 | `archive/steward-20261004/pr-204-490e39f9320c` | `490e39f9320c1b9e2766f136c27f0dccf9ca78e0` | [receipt](https://github.com/benpshore/pdftextract/pull/204#issuecomment-5979669416) |
| #211 | `archive/steward-20261004/pr-211-d8e902be6e40` | `d8e902be6e409b15f476ea126cb29e861a23b095` | [receipt](https://github.com/benpshore/pdftextract/pull/211#issuecomment-5980042744) |
| #221 | `archive/steward-20261004/pr-221-3809145fc07c` | `3809145fc07c0a33be5a85514a50bc3dfcff4938` | [receipt](https://github.com/benpshore/pdftextract/pull/221#issuecomment-5980045854) |


Active rationale reconciled for **#78/#83/#114/#175/#191/#192/#202/#205/#206/#211/#221/#223**;
original evidence remains underneath a dated current-state note. [Body hashes/heads](stewardship/pr-rationale-updates.json)
and [exact-head checks](stewardship/active-checks.json) record the revision. #222 and human/unknown
#100/#101 remain untouched. Original #211 had no exact-head checks returned and did not contain
its latest base. Its exact-tree replacement now has current green checks; #78/#83 required ci
still fails and those separate dependency proposals were only classified in this follow-up.
Green ci does not waive Native/registry/quality/security failures.

## Verified linear web-stack repair

Owner: Bob. Next review: 2026-10-05 UTC or any relevant main/head change.
The [repair report](stewardship/web-stack-repair-20261004.json) records exact parents,
trees, focused paths, ownership evidence, closures and all terminal-green check receipts.
#207 original `32d93cab432623b6818aee2104e9c9f4094557f2` and #192 squash
`b315c36a9d93c03ef1acac9339f758431587ff51` both have tree
`0345a6d79cc9ba16150ea4194c865b76656103df`. This resolves the reported conflict as
duplicate ancestry rather than a reason to discard dependent changes.

| Review order | Exact head / single parent | Preserved tree / focused diff | Exact-head checks |
|---|---|---|---|
| [#228 adapter](https://github.com/benpshore/pdftextract/pull/228) | `e15e0e081fdaf8d5904b6154065b16dcd0658f5b` / #192 `b315c36a9d93c03ef1acac9339f758431587ff51` | Original #211 tree `d226930b2ffec77168e66bd1fa7ae14a5557b841`; 47 files | [CI](https://github.com/benpshore/pdftextract/actions/runs/37202349278), [Web](https://github.com/benpshore/pdftextract/actions/runs/37202349187): success |
| [#229 network-off](https://github.com/benpshore/pdftextract/pull/229) | `b752bae170ae628c07b12800a44a0a37b235db28` / #228 `e15e0e081fdaf8d5904b6154065b16dcd0658f5b` | Original #221 tree `86e10a3baf49802df4ed3b8599a4d5f540c61f3c`; 25 files | [CI](https://github.com/benpshore/pdftextract/actions/runs/37202697546), [Web](https://github.com/benpshore/pdftextract/actions/runs/37202697554): success |

Both new drafts are agent-created and carry this steward's validated responsible-identity
trailers. Historical application contributors are not retroactively attributed to Bob.
Local existing web commands passed (27 adapter, 28 network-off), as did 9/11 actual desktop
Chromium checks, 92 Python plus 28 source-export tests on each tree and 184/187-file manifests.
Published [command/browser reports](stewardship/web-stack-repair-20261004/) preserve exact
source/fixture hashes. The browser bridge replays captured PR206 native output; no fresh
native/GROBID/registry accuracy or actual-device claim. Local OSV is proxy-blocked;
Cargo/Swift/CMake are absent. Existing system Chromium was used after the browser-download
CDN denied access; no alternate download or permission workaround was attempted.

Original owners are parent-reported idle; repeated live heads/comments found no newer claim.
Platform runtime ownership/usage evidence remains unavailable. Done for this repair: exact
source-tree equality, intended single-parent ancestry, green current checks, reversible
source closure and readable current rationale/ledger. Subsequent promotion must separately
qualify #175/main residuals and native/security gates. No merge, release or deployment occurred.

## Full bounded PR classification

“Blocked residual review” means preserved open work awaiting comparison/qualification,
not a defect or security clearance. “Duplicate preserved” remains open to preserve its
review history and stacked dependencies. Human/unknown #100/#101 are untouched.

| PR | Classification | Base / exact head prefix | Why / next step |
|---|---|---|---|
| [#229](https://github.com/benpshore/pdftextract/pull/229) | active-linear-draft-green | `steward/web-scholarly-linear-20261004` / `b752bae170ae` | Exact #221 tree; review after #228 |
| [#228](https://github.com/benpshore/pdftextract/pull/228) | active-linear-draft-green | `fix/web-alpha-integration-20261004` / `e15e0e081fda` | Exact #211 tree; review after #192 |
| [#223](https://github.com/benpshore/pdftextract/pull/223) | active-draft-gated | `fix/native-eval-provenance-20261004` / `7385a3bcd459` | Qualify focused residual against current main |
| [#222](https://github.com/benpshore/pdftextract/pull/222) | active-separately-owned-security | `main` / `6fc3266e0860` | Read-only; separate author/owner |
| [#221](https://github.com/benpshore/pdftextract/pull/221) | superseded-exact-tree / CLOSED | `feat/web-scholarly-adapter-local-20261004` / `3809145fc07c` | Archived; exact tree retained by #229 |
| [#214](https://github.com/benpshore/pdftextract/pull/214) | active-dependency-candidate | `main` / `9aca9bf58b2a` | Read-only; separate author/owner |
| [#211](https://github.com/benpshore/pdftextract/pull/211) | superseded-exact-tree / CLOSED | `fix/web-alpha-integration-20261004` / `d8e902be6e40` | Archived; exact tree retained by #228 |
| [#206](https://github.com/benpshore/pdftextract/pull/206) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `f7b41b93ba87` | Qualify focused residual against current main |
| [#205](https://github.com/benpshore/pdftextract/pull/205) | active-draft-gated | `fix/docling-bibliography-order-20261004` / `3b73e37101e0` | Qualify focused residual against current main |
| [#204](https://github.com/benpshore/pdftextract/pull/204) | superseded-exact-files / CLOSED | `fix/native-bibliography-boundaries-20261004` / `490e39f9320c` | Archived and closed; exact contribution files equal main |
| [#203](https://github.com/benpshore/pdftextract/pull/203) | superseded-exact-files / CLOSED | `diagnostics/native-bibliography-20261004` / `5cd375e6b2c4` | Archived and closed; exact contribution files equal main |
| [#202](https://github.com/benpshore/pdftextract/pull/202) | active-draft-gated | `main` / `d964e9a7d939` | Qualify focused residual against current main |
| [#194](https://github.com/benpshore/pdftextract/pull/194) | superseded-exact-files / CLOSED | `integrate/native-bibliography-base-20261004` / `0ece13395f83` | Archived and closed; exact contribution files equal main |
| [#192](https://github.com/benpshore/pdftextract/pull/192) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `b315c36a9d93` | Qualify focused residual against current main |
| [#191](https://github.com/benpshore/pdftextract/pull/191) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `e9e41ceb3083` | Qualify focused residual against current main |
| [#189](https://github.com/benpshore/pdftextract/pull/189) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `8194834e7797` | Head retained inside an active stack |
| [#188](https://github.com/benpshore/pdftextract/pull/188) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `0d92038160df` | Head retained inside an active stack |
| [#187](https://github.com/benpshore/pdftextract/pull/187) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `5c0a3aa6d33a` | Head retained inside an active stack |
| [#175](https://github.com/benpshore/pdftextract/pull/175) | active-draft-gated | `main` / `baafb4728737` | Qualify focused residual against current main |
| [#174](https://github.com/benpshore/pdftextract/pull/174) | duplicate-preserved-in-active-stack | `publish/paragraph-region` / `c484b73a6882` | Head retained inside an active stack |
| [#172](https://github.com/benpshore/pdftextract/pull/172) | superseded-exact-files / CLOSED | `publish/pdfium-native-hyphen` / `5c6225c7e1dc` | Archived and closed; exact contribution files equal main |
| [#171](https://github.com/benpshore/pdftextract/pull/171) | duplicate-preserved-in-active-stack | `main` / `23880f84e896` | Head retained inside an active stack |
| [#170](https://github.com/benpshore/pdftextract/pull/170) | blocked-residual-review | `main` / `627325b08bfa` | Qualify focused residual against current main |
| [#163](https://github.com/benpshore/pdftextract/pull/163) | blocked-residual-review | `codex/resolver-independent-cohort` / `7a1f7417d527` | Qualify focused residual against current main |
| [#161](https://github.com/benpshore/pdftextract/pull/161) | blocked-residual-review | `codex/resolver-independent-cohort` / `cbe575e0cfab` | Qualify focused residual against current main |
| [#160](https://github.com/benpshore/pdftextract/pull/160) | blocked-residual-review | `codex/lopdf-module-boundaries` / `4859e45d89f3` | Qualify focused residual against current main |
| [#159](https://github.com/benpshore/pdftextract/pull/159) | blocked-residual-review | `codex/pinned-rust-release-pr` / `1de685874dfa` | Qualify focused residual against current main |
| [#158](https://github.com/benpshore/pdftextract/pull/158) | blocked-residual-review | `main` / `cc6479221710` | Qualify focused residual against current main |
| [#157](https://github.com/benpshore/pdftextract/pull/157) | blocked-residual-review | `codex/pinned-rust-release-pr` / `2208c2c95762` | Qualify focused residual against current main |
| [#156](https://github.com/benpshore/pdftextract/pull/156) | blocked-residual-review | `codex/lopdf-module-boundaries` / `0a43c3a883c6` | Qualify focused residual against current main |
| [#155](https://github.com/benpshore/pdftextract/pull/155) | blocked-residual-review | `codex/lopdf-module-boundaries` / `ae0cf2493489` | Qualify focused residual against current main |
| [#143](https://github.com/benpshore/pdftextract/pull/143) | blocked-residual-review | `codex/complete-geometry-search` / `b56a05ebe7be` | Qualify focused residual against current main |
| [#142](https://github.com/benpshore/pdftextract/pull/142) | blocked-residual-review | `codex/pdfium-evidence-worker` / `d7a7c2c36dfe` | Qualify focused residual against current main |
| [#141](https://github.com/benpshore/pdftextract/pull/141) | blocked-residual-review | `codex/complete-geometry-search` / `6eafc85591c2` | Qualify focused residual against current main |
| [#140](https://github.com/benpshore/pdftextract/pull/140) | blocked-residual-review | `codex/pdfium-evidence-worker` / `eb7f3b7d3f63` | Qualify focused residual against current main |
| [#139](https://github.com/benpshore/pdftextract/pull/139) | blocked-residual-review | `codex/integrated-resource-repairs` / `86531f68fd6d` | Qualify focused residual against current main |
| [#138](https://github.com/benpshore/pdftextract/pull/138) | blocked-residual-review | `codex/form-execution-evidence` / `f2ddf313604e` | Qualify focused residual against current main |
| [#137](https://github.com/benpshore/pdftextract/pull/137) | blocked-residual-review | `codex/integrated-resource-repairs` / `85845b0fdfbb` | Qualify focused residual against current main |
| [#136](https://github.com/benpshore/pdftextract/pull/136) | blocked-residual-review | `codex/vancouver-colon-titles` / `77793d531574` | Qualify focused residual against current main |
| [#135](https://github.com/benpshore/pdftextract/pull/135) | blocked-residual-review | `codex/integrated-resource-repairs` / `e439b79c04d5` | Qualify focused residual against current main |
| [#134](https://github.com/benpshore/pdftextract/pull/134) | blocked-residual-review | `main` / `7de09bfa473e` | Qualify focused residual against current main |
| [#133](https://github.com/benpshore/pdftextract/pull/133) | blocked-residual-review | `main` / `102638dfa417` | Qualify focused residual against current main |
| [#132](https://github.com/benpshore/pdftextract/pull/132) | blocked-residual-review | `main` / `bee5459dd1be` | Qualify focused residual against current main |
| [#126](https://github.com/benpshore/pdftextract/pull/126) | blocked-residual-review | `main` / `c6ab5f8ceb07` | Base head not contained; inspect graph |
| [#119](https://github.com/benpshore/pdftextract/pull/119) | blocked-residual-review | `main` / `80bbab4caef5` | Qualify focused residual against current main |
| [#118](https://github.com/benpshore/pdftextract/pull/118) | blocked-residual-review | `app/text-size` / `96f019b53b91` | Base head not contained; inspect graph |
| [#117](https://github.com/benpshore/pdftextract/pull/117) | blocked-residual-review | `main` / `bfff85ea484d` | Qualify focused residual against current main |
| [#116](https://github.com/benpshore/pdftextract/pull/116) | blocked-residual-review | `main` / `d4f5aea806b4` | Qualify focused residual against current main |
| [#115](https://github.com/benpshore/pdftextract/pull/115) | blocked-residual-review | `app/text-size` / `317d65793053` | Qualify focused residual against current main |
| [#114](https://github.com/benpshore/pdftextract/pull/114) | active-draft-gated | `main` / `d547cd688b8c` | Qualify focused residual against current main |
| [#113](https://github.com/benpshore/pdftextract/pull/113) | blocked-residual-review | `main` / `4ffb5585fe92` | Qualify focused residual against current main |
| [#112](https://github.com/benpshore/pdftextract/pull/112) | blocked-residual-review | `app/launch-smoke` / `bd07c9501f5f` | Qualify focused residual against current main |
| [#111](https://github.com/benpshore/pdftextract/pull/111) | blocked-residual-review | `app/timings` / `34d59882f506` | Qualify focused residual against current main |
| [#110](https://github.com/benpshore/pdftextract/pull/110) | blocked-residual-review | `main` / `6ba9b0550f4f` | Qualify focused residual against current main |
| [#109](https://github.com/benpshore/pdftextract/pull/109) | blocked-residual-review | `app/cancel-job` / `d8508d52d575` | Qualify focused residual against current main |
| [#108](https://github.com/benpshore/pdftextract/pull/108) | blocked-residual-review | `app/virtual-list` / `34438e4bcc19` | Qualify focused residual against current main |
| [#107](https://github.com/benpshore/pdftextract/pull/107) | blocked-residual-review | `app/headless-tests` / `39cd4450f5db` | Qualify focused residual against current main |
| [#106](https://github.com/benpshore/pdftextract/pull/106) | blocked-residual-review | `main` / `51c231411bb7` | Qualify focused residual against current main |
| [#101](https://github.com/benpshore/pdftextract/pull/101) | human-or-unknown-preserve | `main` / `bc653883fc80` | Read-only; separate author/owner |
| [#100](https://github.com/benpshore/pdftextract/pull/100) | human-or-unknown-preserve | `main` / `6a84632cf035` | Read-only; separate author/owner |
| [#99](https://github.com/benpshore/pdftextract/pull/99) | blocked-residual-review | `main` / `0d4bf7b4edf0` | Qualify focused residual against current main |
| [#98](https://github.com/benpshore/pdftextract/pull/98) | blocked-residual-review | `main` / `e85c8e966156` | Qualify focused residual against current main |
| [#97](https://github.com/benpshore/pdftextract/pull/97) | blocked-residual-review | `main` / `bc7f4a1b193d` | Qualify focused residual against current main |
| [#96](https://github.com/benpshore/pdftextract/pull/96) | blocked-residual-review | `main` / `6beda59896b9` | Qualify focused residual against current main |
| [#95](https://github.com/benpshore/pdftextract/pull/95) | blocked-residual-review | `main` / `1d8021935c09` | Qualify focused residual against current main |
| [#94](https://github.com/benpshore/pdftextract/pull/94) | blocked-residual-review | `main` / `e96fb95b0c3f` | Qualify focused residual against current main |
| [#93](https://github.com/benpshore/pdftextract/pull/93) | blocked-residual-review | `main` / `1d74899107d3` | Qualify focused residual against current main |
| [#92](https://github.com/benpshore/pdftextract/pull/92) | blocked-residual-review | `main` / `a814e9bab05c` | Qualify focused residual against current main |
| [#91](https://github.com/benpshore/pdftextract/pull/91) | blocked-residual-review | `main` / `e6bbf0458225` | Qualify focused residual against current main |
| [#90](https://github.com/benpshore/pdftextract/pull/90) | blocked-residual-review | `main` / `0af4b62af2ab` | Qualify focused residual against current main |
| [#89](https://github.com/benpshore/pdftextract/pull/89) | blocked-residual-review | `main` / `33bb311e5f3f` | Qualify focused residual against current main |
| [#88](https://github.com/benpshore/pdftextract/pull/88) | blocked-residual-review | `main` / `1a8a388120d9` | Qualify focused residual against current main |
| [#87](https://github.com/benpshore/pdftextract/pull/87) | blocked-residual-review | `main` / `e725632d3eb4` | Qualify focused residual against current main |
| [#86](https://github.com/benpshore/pdftextract/pull/86) | blocked-residual-review | `main` / `861a4758230c` | Qualify focused residual against current main |
| [#85](https://github.com/benpshore/pdftextract/pull/85) | blocked-residual-review | `main` / `5b81323bd788` | Qualify focused residual against current main |
| [#84](https://github.com/benpshore/pdftextract/pull/84) | blocked-residual-review | `main` / `bf25f48307e9` | Qualify focused residual against current main |
| [#83](https://github.com/benpshore/pdftextract/pull/83) | blocked-dependency-ci | `maintenance/docling-1.74.1` / `dda67a8981fb` | Qualify focused residual against current main |
| [#82](https://github.com/benpshore/pdftextract/pull/82) | blocked-residual-review | `main` / `edb33dfc08ef` | Qualify focused residual against current main |
| [#81](https://github.com/benpshore/pdftextract/pull/81) | blocked-residual-review | `main` / `6170f3059259` | Qualify focused residual against current main |
| [#80](https://github.com/benpshore/pdftextract/pull/80) | blocked-residual-review | `main` / `d0a5d30b3af2` | Base head not contained; inspect graph |
| [#79](https://github.com/benpshore/pdftextract/pull/79) | blocked-residual-review | `main` / `2f4f026affc4` | Qualify focused residual against current main |
| [#78](https://github.com/benpshore/pdftextract/pull/78) | blocked-dependency-ci | `feat/isolated-ingest-batch` / `f23f392efec7` | Qualify focused residual against current main |
| [#77](https://github.com/benpshore/pdftextract/pull/77) | blocked-residual-review | `main` / `e7cf5ad9a83e` | Qualify focused residual against current main |
| [#76](https://github.com/benpshore/pdftextract/pull/76) | blocked-residual-review | `main` / `4176bb4d729f` | Qualify focused residual against current main |
| [#75](https://github.com/benpshore/pdftextract/pull/75) | blocked-residual-review | `main` / `d7e208bc6540` | Qualify focused residual against current main |
| [#74](https://github.com/benpshore/pdftextract/pull/74) | blocked-residual-review | `main` / `dda9a6f5f18c` | Qualify focused residual against current main |
| [#68](https://github.com/benpshore/pdftextract/pull/68) | blocked-residual-review | `main` / `4557fbc34ecd` | Base head not contained; inspect graph |
| [#64](https://github.com/benpshore/pdftextract/pull/64) | blocked-residual-review | `main` / `afce5697081b` | Base head not contained; inspect graph |
| [#57](https://github.com/benpshore/pdftextract/pull/57) | blocked-residual-review | `main` / `4188236f65e7` | Base head not contained; inspect graph |
| [#55](https://github.com/benpshore/pdftextract/pull/55) | blocked-residual-review | `feat/native-format-ingestion` / `cd14f587b112` | Qualify focused residual against current main |
| [#54](https://github.com/benpshore/pdftextract/pull/54) | blocked-residual-review | `main` / `0e7b498f3bc8` | Qualify focused residual against current main |

## Validation and next review

Policy fixtures pass (15); Ruff passes. Initial draft #226 at `6d163348` has
[green required CI](https://github.com/benpshore/pdftextract/actions/runs/37200247331); final-head CI is recorded separately. Full local Python suite passes **225 tests**. OSV audit was attempted and is proxy-blocked;
no successful local audit is claimed. Required Cargo/Swift/CMake commands were attempted but those
executables are absent in this environment; hosted required CI is the remaining integration gate.
No runnable extraction tool was run on Ben's Mac. No version, release tag, credential, Git object
format, external storage permission, Site setting or security policy permission was changed.

At the next review, refresh main and active exact heads before using this snapshot; inspect
the six archive/closure receipts, refresh repaired-stack exact-head checks, and qualify any genuinely needed
residual as a focused draft. User merging a runnable change remains a separate decision.

Stable steward fingerprint: `sha256:2b6c731a6eed3711be7c08910d82c6a5fd60b74c96b9cceec0464400a53d1e6f`.

Usage oversight: this steward task remains active and sequential, with no child agents,
speed escalation or credit purchase. Requested Sol6.1 / STANDARD / extra-high is recorded;
actual runtime model/tier and provider token/credit usage are not exposed. No account-wide
meter tool is available. Parent reports cloud ChatGPT signed out, Mac inspection unauthorized,
and Ben's credits UI disrupting the call. Last reported 57,000 credits is self-reported,
not live. No activity-derived balance estimate or hard spending-cap enforcement is claimed.
Record any actual provider receipt if later exposed; do not ask Ben to switch UI during work.

## Explicit follow-up requirements from the live call

[Agent data engineering #227](https://github.com/benpshore/pdftextract/issues/227) owns the missing
**region AND chunk within each page** schema/alignment/coordinate-frame/boundary/reading-order/
provenance/correction contract. It preserves candidates and distinguishes confidence from truth.
Design must use adaptive native/browser host budgets with conservative unavailable-probe fallback:
**no blanket 8 MB input cap**, and legitimate **50 GB gunzipped tar** browser streaming feasibility
must be investigated, not categorically excluded or claimed supported. Separate file/object/
reference/metadata/nesting/queue/index budgets; spill, throttle, checkpoint and resume before
killing legitimate work while preserving firm hostile-input ceilings. Test background worker,
cache and buffer lifecycle/growth. These are recorded directions, not new builds or benchmark authority.

Ben refuses real-device testing. Desired option: isolated Tart macOS VM plus Xcode iOS Simulator,
only with an authorized Apple host; no Mac installation permission is implied and simulator results
do not prove actual iPhone capacity. No actual-device test, Mac run or new runtime was attempted.
