# Repository stewardship ledger — 2026-10-04

Owner: **Busybody Bob** (alias **Mara Keel**). [Agent maintenance epic #225](https://github.com/benpshore/pdftextract/issues/225).
Next review: **2026-10-05 UTC**, or immediately after main/active-head changes. This is a review date, not an automation.
Scope authority: Ben Shore's sequential delegation in source thread `01a0fe84-0716-7474-b2b7-709e4359b0fb`.
Root owns queue/review/follow-through. No child agents, feature fanout, main merges, releases, deployment or Mac execution.

## Read this first

Live main was independently fetched at `72890e6f9d23b1c10ee2cd9d368c7c0817319c10`.
Ben's recent squash merges #168/#173/#212/#219 are preserved. Snapshot: **89 open PRs**.
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
PR214 is a Dependabot web candidate, separately qualified against Workers/browser/runtime tests.
No pins, toolchain or automatic-update permissions are changed by this stewardship draft.

## Active dependency map and gates

- Native: #175 → #191 → #194 → #203 → #204 → #205 → #223.
  #172/#194/#203/#204 contribution files exactly equal current main. Closing those PRs
  does not remove their branches or erase #205's base. #175 remains a broad unqualified
  draft; its residual feature requirements are not accepted merely because some repairs landed.
- Web: #175 → #192 → #211 → #221. #187/#188/#189 heads are contained by #192
  and retained as duplicate source/history, not independently promoted.
  **#211's head does not contain #192's latest head**; inspect its merge base and newer
  changes before integration. Do not retarget or combine stacks casually.
- OCR/GROBID #206 remains based on #175. #202 remains an isolated region contract;
  #207/#208/#209/#210/#213/#218 are verified merged into their feature-stack bases,
  rather than main. Their exact heads/bases are retained in
  [merged-stack-prs.json](stewardship/merged-stack-prs.json). Do not reopen or reland them separately.
- Native #223 `7385a3bcd4592a2071cd5eb93d566fe4ec472723` and web #221
  `3809145fc07c0a33be5a85514a50bc3dfcff4938` retain network-off boundaries.
  Prior green required CI is reported in those exact-head descriptions, not rerun or
  counted as current-main integration/security qualification by this steward.
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
agent-created and labeled. Other historical issue authorship is unresolved, so their
labels/bodies/state/comments are preserved; [historical attribution](stewardship/historical-attribution.json)
records unknowns and body hashes. Existing epic scopes #176–#183/#199 do not exactly cover
maintenance, so #225 fills that gap without editing those issues.

All newly created agent issues need `agent-created` plus explicit agent/task attribution.
Known historical review/research agents require evidence before retrospective linkage:
their missing names/configurations/fingerprints are not fabricated. ChatGPTWork is registered
for the current observed parent delegation, not falsely attributed to all earlier work.

## Full bounded PR classification

“Blocked residual review” means preserved open work awaiting comparison/qualification,
not a defect or security clearance. “Duplicate preserved” remains open to preserve its
review history and stacked dependencies. Human/unknown #100/#101 are untouched.

| PR | Classification | Base / exact head prefix | Why / next step |
|---|---|---|---|
| [#223](https://github.com/benpshore/pdftextract/pull/223) | active-draft-gated | `fix/native-eval-provenance-20261004` / `7385a3bcd459` | Qualify focused residual against current main |
| [#222](https://github.com/benpshore/pdftextract/pull/222) | active-separately-owned-security | `main` / `6fc3266e0860` | Read-only; separate author/owner |
| [#221](https://github.com/benpshore/pdftextract/pull/221) | active-draft-gated | `feat/web-scholarly-adapter-local-20261004` / `3809145fc07c` | Qualify focused residual against current main |
| [#214](https://github.com/benpshore/pdftextract/pull/214) | active-dependency-candidate | `main` / `9aca9bf58b2a` | Read-only; separate author/owner |
| [#211](https://github.com/benpshore/pdftextract/pull/211) | active-draft-gated | `fix/web-alpha-integration-20261004` / `d8e902be6e40` | Base head not contained; inspect graph |
| [#206](https://github.com/benpshore/pdftextract/pull/206) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `f7b41b93ba87` | Qualify focused residual against current main |
| [#205](https://github.com/benpshore/pdftextract/pull/205) | active-draft-gated | `fix/docling-bibliography-order-20261004` / `3b73e37101e0` | Qualify focused residual against current main |
| [#204](https://github.com/benpshore/pdftextract/pull/204) | superseded-exact-files | `fix/native-bibliography-boundaries-20261004` / `490e39f9320c` | Exact contribution files equal main; archive before closure |
| [#203](https://github.com/benpshore/pdftextract/pull/203) | superseded-exact-files | `diagnostics/native-bibliography-20261004` / `5cd375e6b2c4` | Exact contribution files equal main; archive before closure |
| [#202](https://github.com/benpshore/pdftextract/pull/202) | active-draft-gated | `main` / `d964e9a7d939` | Qualify focused residual against current main |
| [#194](https://github.com/benpshore/pdftextract/pull/194) | superseded-exact-files | `integrate/native-bibliography-base-20261004` / `0ece13395f83` | Exact contribution files equal main; archive before closure |
| [#192](https://github.com/benpshore/pdftextract/pull/192) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `b315c36a9d93` | Qualify focused residual against current main |
| [#191](https://github.com/benpshore/pdftextract/pull/191) | active-draft-gated | `feat/native-toolkit-private-alpha-20261004` / `e9e41ceb3083` | Qualify focused residual against current main |
| [#189](https://github.com/benpshore/pdftextract/pull/189) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `8194834e7797` | Head retained inside an active stack |
| [#188](https://github.com/benpshore/pdftextract/pull/188) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `0d92038160df` | Head retained inside an active stack |
| [#187](https://github.com/benpshore/pdftextract/pull/187) | duplicate-preserved-in-active-stack | `feat/native-toolkit-private-alpha-20261004` / `5c0a3aa6d33a` | Head retained inside an active stack |
| [#175](https://github.com/benpshore/pdftextract/pull/175) | active-draft-gated | `main` / `baafb4728737` | Qualify focused residual against current main |
| [#174](https://github.com/benpshore/pdftextract/pull/174) | duplicate-preserved-in-active-stack | `publish/paragraph-region` / `c484b73a6882` | Head retained inside an active stack |
| [#172](https://github.com/benpshore/pdftextract/pull/172) | superseded-exact-files | `publish/pdfium-native-hyphen` / `5c6225c7e1dc` | Exact contribution files equal main; archive before closure |
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
| [#83](https://github.com/benpshore/pdftextract/pull/83) | active-draft-gated | `maintenance/docling-1.74.1` / `dda67a8981fb` | Qualify focused residual against current main |
| [#82](https://github.com/benpshore/pdftextract/pull/82) | blocked-residual-review | `main` / `edb33dfc08ef` | Qualify focused residual against current main |
| [#81](https://github.com/benpshore/pdftextract/pull/81) | blocked-residual-review | `main` / `6170f3059259` | Qualify focused residual against current main |
| [#80](https://github.com/benpshore/pdftextract/pull/80) | blocked-residual-review | `main` / `d0a5d30b3af2` | Base head not contained; inspect graph |
| [#79](https://github.com/benpshore/pdftextract/pull/79) | blocked-residual-review | `main` / `2f4f026affc4` | Qualify focused residual against current main |
| [#78](https://github.com/benpshore/pdftextract/pull/78) | active-draft-gated | `feat/isolated-ingest-batch` / `f23f392efec7` | Qualify focused residual against current main |
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

Policy fixtures currently pass (15); Ruff passes. Full local Python suite passes **225 tests**. OSV audit was attempted and is proxy-blocked;
no successful local audit is claimed. Required Cargo/Swift/CMake commands were attempted but those
executables are absent in this environment; hosted required CI is the remaining integration gate.
No runnable extraction tool was run on Ben's Mac. No version, release tag, credential, Git object
format, external storage permission, Site setting or security policy permission was changed.

At the next review, refresh main and active exact heads before using this snapshot; inspect
the four archive/closure receipts, finish active-head checks, and qualify any genuinely needed
residual as a focused draft. User merging a runnable change remains a separate decision.

Stable steward fingerprint: `sha256:2b6c731a6eed3711be7c08910d82c6a5fd60b74c96b9cceec0464400a53d1e6f`.
