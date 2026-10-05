# Proposal assessment for issue239 — 2026-10-05

Responsible agent: Busybody Bob (Mara Keel alias), existing registered task
`pdftextract/stewardship/2026-10-04/busybody-bob`. Parent requested an assessment;
[issue239](https://github.com/benpshore/pdftextract/issues/239) supplies the proposal
scope, not implementation authority. No checkpoint workflow, schedule, dependency,
credential, permission or deployment is added. This document belongs to the separate
evidence branch, not the application repair in PR240. Owner of this assessment: Bob;
implementation owner remains to be assigned by Ben. Next review: Ben's implementation
decision, or a change to the native pins/baseline. Assessment done criteria: an
inspectable recommendation, evidence limits and a bounded first step are delivered.

The useful replacement for a weekly ChatGPT safety opinion is a deterministic report
of changed boundaries and executed evidence. It can remove the recurring dependency
on an LLM, but ownership, aliasing and foreign-library contracts still require human
review. A green report must describe its supported checks and remaining assumptions.

## Existing material and its actual baseline

The inventory in [PR162](https://github.com/benpshore/pdftextract/pull/162) was merged
into the held `codex/pinned-rust-release-pr` stack, not current main. Its
[document](https://github.com/benpshore/pdftextract/blob/6c2c92d8d79a4fb5a5befec54fb2bed9da6cb2cb/docs/unsafe-rust-inventory-ffi.md)
and [summary](https://github.com/benpshore/pdftextract/blob/6c2c92d8d79a4fb5a5befec54fb2bed9da6cb2cb/docs/unsafe-rust-inventory-ffi/summary.json)
describe source `68ee9b4e08fde2f28e1042e1d6abb89e3547770d`: 95 tracked Rust files,
35 unsafe constructs (34 blocks and one function), including target/cfg/test and
macro-token contexts; registry dependencies and generated code were excluded.
The published JSONL/source/build manifests are useful provenance and schema examples.
They do not establish a reusable generator on main, or a current count of 35.
The inventory files are absent main `88d65b565bbf1037b1b76df1cc2462a1ae324add`.

Current boundaries include raw PDFium document/page/text handles and safe-looking
binding calls in `src/backend/pdfium_backend/unicode_mapping.rs`, dynamically loaded
provider ABI/functions and foreign output ownership in `src/backend/native_provider.rs`,
OS process/limit FFI in `src/worker_limits.rs`, allocation delegation in
`src/worker_allocator.rs`, and macOS Objective-C/speech interfaces. Counting the word
`unsafe` would miss the mapping wrapper's foreign handles and library contracts.
Dependency/lockfile, feature, toolchain, native-library, header and ABI changes must
invalidate the relevant evidence even when application unsafe syntax is unchanged.

## Smallest useful design

Regenerate a syntax-aware inventory at the chosen immutable baseline and head, with
an explicit declaration of excluded generated/dependency code. Retain old records;
identify items by path, containing symbol and construct/boundary identity, with a
source digest and explicit moved/superseded links. Line numbers alone are unstable.
Added, changed, removed and unchanged items should be searchable. Removal of an item
is a recorded source change, not an automatic closure of a safety concern.

Join each item to a compact assumption/evidence record: byte and library lifetimes,
handle/drop order, aliasing and mutability, ABI/layout/signedness, thread/global
initialization rules, callbacks and unwind boundaries, null/error/size behavior,
review owner, next review and done criteria. Keep scanner facts and test results
separate from reviewer judgments. Changed source/configuration invalidates a previous
closure unless its continued applicability is explicitly reviewed.

Each check needs `passed`, `failed`, `not_run`, `unsupported` or `inconclusive`, an
explicit reason, exact command/exit status, target/features/profile, source/tree,
toolchain/lock/native/header/fixture hashes and immutable log/artifact references.
Include report schema and generator version, baseline/head, run/job/attempt and
artifact digest. Missing, empty, expired or wrong-head artifacts are visible failures
of evidence collection, never an empty green report. Actual executions and unsupported
targets cannot be collapsed into a common success. Human approval remains a separate
decision. Hashes aid lookup/integrity; they do not sign authorship or prove correctness.

## Reuse the native lanes before adding a large matrix

| Existing evidence | What it can support | Limit to retain |
| --- | --- | --- |
| `ci.yml` | Default workspace checks, Linux x64/ARM, Python/Swift | Does not qualify every optional native feature |
| `pdfium-probe.yml`, `pdfium_unicode.rs`, mapping unit tests | Pinned real PDFium; count conversion, failed open/close, Unicode/status preservation | Tests can print `skipped:` when the library is absent; assert actual execution |
| `native_provider.rs` integration/unit tests | Retained input after caller bytes drop, independent sessions, malformed input, bounded output, free/close | Requires the matching provider/engine and declared ABI/features |
| `native-repair-checks.yml` | Three target native/public fixture evidence | Current main's scope is narrower; PR235's expansion is not yet landed |
| `build-binaries.yml`, containment/startup/build identity tests | Release worker resource/process behavior and build provenance on shipped targets | Containment is not a memory-soundness proof |
| `native.yml` | Corpus coverage and extraction completeness | Its 209 Partial failures on PR235 remain a separate quality gate |

PR240 at `52ff2462ce3c0d030a78da4f5f3b852a38489ef3` restores workflow parsing.
Its required CI passes all six jobs, and Linux x64 runs the real native handle and
five Unicode integration tests without the missing-library skip message. ARM fails
on `c_char`/`u8` `cast_unsigned` compilation (already repaired in active PR235).
macOS fails in LiteParse header provisioning when its `sha256sum` rejects `--check`;
the held PR234 contains Bob's previously qualified checksum-interface repair.
These failures are prerequisites to reliable multi-target reporting, not clearance.
That successful Linux job also ignores all three licensed MuPDF/Poppler provider
integration tests and the native output-limit unit test: their actual runtime paths
are not provisioned. PR235 compiles those optional provider modules; its expanded
repair lane still does not execute these ignored tests. The report must show them as
`not_run`, even though their containing Cargo command exits successfully. Provisioning
the licensed native providers would need a separate explicit decision.

For relevant PR changes, run the deterministic diff/report and only affected existing
checks plus supported Rust-only Miri harnesses. Dependency/ABI/native-pin changes
need the affected native targets; prose-only changes should have a visible `not_run`
reason. Proposed initial budgets are five minutes for inventory/report and ten minutes
for supported Miri tests, reusing bounded native jobs rather than duplicating them.
These are planning ceilings to qualify with measured runs, not observed timings.

A weekly deeper run should select an immutable main SHA, cover the three shipped
targets with existing native tests and add a bounded Linux native harness for open,
partial initialization, page/text failure, close/free/drop order, repeated sessions
and malformed public/synthetic PDFs. Cap each new harness at twenty minutes initially;
timeout is `inconclusive`. Keep a separate pinned analysis toolchain and record its
identity instead of changing the production toolchain. Heavy native dependency builds
need an explicit cost/owner decision; use previously qualified instrumented artifacts
only when their source/configuration hashes match. None is provisioned by this task.

## Miri and native instrumentation

Miri is useful for supported Rust pointer/ownership/bounds harnesses with controlled
mock boundaries. Its normal foreign-call support is limited. The current experimental
Unix `-Zmiri-native-lib` bridge has argument/memory/thread restrictions and loses
provenance tracking around native calls; it does not generally check native code for
UB. It is not evidence of full PDFium C++/ABI/lifecycle safety. Passing sampled Miri
executions cannot prove soundness. [Miri documentation](https://github.com/rust-lang/miri).

Exercise the real provider/PDFium boundary separately. ASan can detect executed
out-of-bounds, use-after-free and invalid/double-free paths when the relevant native
code is instrumented and the runtime is linked. A harness around an uninstrumented
prebuilt library does not provide equivalent coverage inside that library.
[Clang AddressSanitizer documentation](https://clang.llvm.org/docs/AddressSanitizer.html).
Use UBSan where supported for native undefined operations; qualify TSan separately
against the actual thread/initialization contract. MSan requires instrumented
dependencies; partial synchronization coverage also limits TSan conclusions.
[Rust sanitizer guidance](https://doc.rust-lang.org/unstable-book/compiler-flags/sanitizer.html).
Record unsupported instrumentation explicitly. No Miri or sanitizer run was performed
here, and ordinary native CI passing is not labeled sanitized evidence.

## First verification and acceptance

Use an inspectable priority order: an observed failing boundary check, then a changed
boundary with no matching evidence, then an ABI/native-pin change, then the oldest
unverified assumption. Show why the selected item wins; permit a named reviewer to
override with recorded rationale. This is a proposed policy, not a calibrated risk score.

First establish a current inventory/evidence join that includes the provider and
safe-looking PDFium handle wrapper, with fixture expectations for changed/removed
items, wrong-head/missing logs, native skip detection and unsupported targets. The
highest-leverage subsequent deeper check is the real instrumented handle/output
ownership harness: input lifetime, null/partial opens, drop/free order and repeated
sessions. Do not assume cross-thread use is legal; verify that contract first.
Repair or explicitly mark the observed ARM/macOS blockers before presenting a weekly
three-target checkpoint as complete.

Acceptance should require reproducible output from identical inputs, truthful
pass/fail/not-run/unsupported fixtures, source/configuration invalidation, retained
history, secret-free untrusted-PR execution with read-only permissions, bounded costs
and a named human reviewer for assumptions. No LLM, new persistent credential or
automatic merge/security exception is needed. Issue239 is left unchanged; no duplicate
ticket or proposed-checkpoint implementation was created.
