# Native evaluation diagnostics after historical pin drift

This change separates current evaluation diagnostics from historical Partial
eligibility. It does not qualify a backend, repin a model, or renew an exception.
It is independent of the Python app/library architecture tracked in #164.

## Trigger and evidence boundary

At PR #273 head `7cabac188b5d89c5f0a9127392f5213df7d349a1`, Native run
[38169034277](https://github.com/benpshore/pdftextract/actions/runs/38169034277),
job 114557539328 passed strict model provisioning, compilation, tests and the
four evaluation commands. The retrieved job log then reports four validator
failures: `reviewed-partial provenance mismatch: native_manifest_sha256`.
The current-source diagnostics were hidden by that early exception.

The connected GitHub reader returned artifact 11714096388 as a downloadable
file reference; fetching its ZIP URL in this environment returned HTTP 403.
No claim is made that the artifact's dumps were downloaded or revalidated here.
The implementation is tested with controlled evidence fixtures, including
counterexamples. The real artifact remains a follow-up verification target.

## Preserved gates

`load_reviewed_partials` still rejects model/scorer/truth drift. Its only
existing drift allowance remains a verified dependency-lock change, which
returns baselines with no eligible Partial exceptions. The existing test that
rejects other drift remains intact. Shared per-input policy validation was
extracted without removing its conditions.

`native/reviewed-partials.json` is byte-identical to main, SHA-256
`ff92be145cb8fa57e0948f984088c21079c9ad5a85bdbdb27b53d5ceed33e234`.
No provenance pins, model hashes or historical warning hashes are changed.

A separate diagnostic baseline path may compare pages and references across
model/dependency changes only when current-source identity is verified and the
corpus, metric source, truth source, host, split, backend and input hashes still
match their relevant expectations. Every such baseline sets
`partial_eligible=False`. Invalid/noncomparable baselines are reported as errors;
they cannot authorize an exception. All Partial outcomes remain strict errors
in the independent current diagnostics.

Historical rejection remains in the overall errors even if every current paper
is Complete. Invalid dump evidence disables all historical exceptions. Missing
or tampered source provenance fails before claiming current-source diagnostics.
A valid historical allowance can still satisfy the existing gate while its
Partial status remains visible in the separate strict diagnostic list.

## Output

`--diagnostics-json PATH` writes a new JSON file with schema version 1 containing:

- Verified source provenance, exact policy-byte hash, backend, split and host.
- Historical eligibility errors and number of exceptions actually applied.
- Baseline compatibility errors and IDs retaining page/reference comparisons.
- Dump-integrity errors and strict current coverage/status/reference errors.
- Overall errors, including historical rejection, which determine failure.

The destination must not exist. Reports, dumps and policy files cannot be
silently overwritten. Early malformed/missing input or unverified-source errors
may prevent this artifact from being created; the CLI still exits nonzero.
The Native workflow requests `eval-native/<backend>/validation.json` and retains
its existing failure aggregation and artifact upload behavior.

## Validation and remaining work

Python fixtures exercise model-only and model-plus-lock drift, Partial and
Complete outcomes, zero references, page/reference regressions, malformed policy,
wrong input/source identities, tampered dump data, valid historical allowances,
and non-overwriting CLI output. Existing strict policy tests remain enabled.
Ruff and the full Python suite passed during development. Exact-head counts and
hosted outcomes are recorded in the draft PR/handoff.

Retrieve the original artifact through a working authorized download path, bind
it to its producer source and execution host, and inspect each backend's emitted
diagnostics. This change intentionally does not turn that historical run green
or certify extraction correctness. No deployment, release or merge is included.
