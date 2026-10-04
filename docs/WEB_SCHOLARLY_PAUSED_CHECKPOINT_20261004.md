# Paused scholarly adapter checkpoint

Parent explicitly stopped all new work to resume with Sol 6.1, Standard speed,
medium/high effort. The formal security scan is deferred. This is an unfinished
preservation checkpoint, not a reviewed implementation or deployment candidate.

Branch: `feat/web-scholarly-adapter-local-20261004`, starting at frozen PR207
`32d93cab432623b6818aee2104e9c9f4094557f2`. PR192 and PR207 remain unchanged.
No new draft PR was opened for this checkpoint. No production migration,
deployment, public native endpoint, credentials, access expansion, merge or
release occurred. Existing user documents were not used.

## Saved implementation

- `web/lib/scholarly-adapter.ts`: draft strict mapping from actual PR206
  GrobidDocument JSON to the existing citation projection, with separately
  retained canonical JSON/raw TEI, hashes, warnings, empty geometry and arrays.
  New non-resolving XML consistency checks are unverified. No focused adapter
  test or `web/docs/SCHOLARLY_ADAPTER.md` has been created yet.
- `web/lib/scholarly-service.ts` and document scholarly/evidence routes:
  existing owner/origin checks, stored-original hashing, private service binding,
  optional conservative native resolver output, immutable evidence/result
  objects, and result-key compare-and-swap. Current reading/original metadata
  remain preserved. This draft still needs race/error testing.
- `web/build/scholarly-local-worker.mjs`, `web/vite.config.ts`,
  `web/cloudflare-env.d.ts`: opt-in loopback-only local service binding using
  `TPE_SCHOLARLY_LOCAL_URL`; no production build binding. Existing local sign-in
  middleware is reused. workerd rejects `redirect: error`; the proxy was fixed
  to use `manual` and reject 3xx explicitly after an actual reproduction.
- `web/components/scholarly-controls.tsx` and partial workspace integration:
  capability/mode rendering, request/cancel/reconcile callbacks and writer
  guards. **Controls are not mounted yet.** Finish queue/cache disabling,
  workspace-cleanup cancellation and stale-GET-after-body handling. The focused
  component test does not exist yet. No checks ran on these UI edits.
- `scripts/scholarly-local-runtime.mjs`: explicit loopback Node bridge invoking
  exact native CLI, separate live/captured modes, raw canonical output and
  bounded child lifecycle. Its 14 focused tests passed before the stop.
- `scripts/test-site-scholarly-workers.mjs`: partial real Miniflare D1/R2
  route/adapter suite. `scripts/test-web-scholarly-browser.mjs`: full-app local
  harness with real Vite/vinext/auth/Workers storage and browser PDF processing.
- `web/tests/fixtures/scholarly/`: actual captured PR206 JSON and independently
  authored PDF, with provenance. Captured replay is never a live-service claim.

## Evidence actually obtained

[Runtime checkpoint](validation/scholarly-local-checkpoint-20261004/checkpoint.json)
records one actual live native/GROBID HTTP 200 request taking 29.342337 seconds,
two references, two pages, three empty coordinate attributes, and retained
warnings. [Exact native JSON](validation/scholarly-local-checkpoint-20261004/native-grobid.json)
and [raw TEI](validation/scholarly-local-checkpoint-20261004/native-grobid.tei.xml)
are retained. This was the runner path, **not completed browser-to-saved-result
scholarly E2E**. No actual registry resolver request started.

The [browser upload probe](validation/scholarly-local-checkpoint-20261004/browser-upload-probe.json)
passed six checks: real local sign-in/spoof rejection, foreign-owner rejection,
actual PDF Worker/multipart storage with byte hashes, captured-runtime service
health, and no browser errors. It predates the partial UI edits. Later live-URL
harness support was saved but not run. Full extraction/CSV/evidence browser
acceptance remains unfinished.

The Workers suite passed owner/CSRF/request-authority and original/native/TEI
hash-failure preservation checks, then failed with an unhandled Response -> HTTP
500 during evidence error handling. **First likely fix:** evidence GET returns
`scholarlyEvidence(...)` without `await` inside `try`; asynchronous errors bypass
`failure()`. The remaining cancellation/CAS/deletion tests were not reached.

An early TypeScript run reported two Uint8Array generic mismatches in
`scholarly-service.ts`. Those signatures were corrected, but TypeScript was not
rerun, and subsequent agent edits remain unchecked. No new production build,
complete regression run, manifest regeneration, or independent final approval
has been performed. `web/SOURCE_MANIFEST.json` still describes frozen PR207 and
is intentionally stale at this pause. Do not describe this checkpoint as green.

## Runtime preservation and resume

All agents stopped. Browser/Vite/fixture servers closed. Runner PID 18639 exited
0. Container `tpe-scholarly-local-20261004` is stopped (exit 143), preserved rather
than removed; no active external runtime jobs remain.

- Exact native source: PR206 `df395cc4be6792d0c26e2226a72ad7aa83d9500b`.
- Binary: `/tmp/pdftextract-pr206-native/target/debug/tpe`, SHA-256
  `280fc91832368ebc4420a2a67eb98d7ffa6a4e3b7cb10ef8410063da41aba108`.
- [Build provenance](validation/scholarly-local-checkpoint-20261004/build-provenance.json):
  isolated Rust 1.98.1 debug build with `grobid`; 498 archived source files
  byte-identical; no native source edits. Reuse this binary if the workspace
  persists rather than rebuilding.
- CPU image: `grobid/grobid@sha256:223957791ac2bbe48609dcc58a689b16b60baeae13a8734ef440ae6bfb38f4cd`.
- Before stop: GROBID `http://127.0.0.1:32768`, bridge `http://127.0.0.1:8072`.
  Inspect the retained container before restarting; ports may differ elsewhere.
- Additional logs/config/state stay at `/tmp/pdftextract-scholarly-live-20261004`;
  browser temporary evidence stays at `/tmp/pdftextract-scholarly-browser-qa`.
- The current local bridge defaults to 32 MiB input/output and 60 seconds. The
  web adapter has its own 32 MiB buffering ceiling. These are explicit local
  runtime budgets, not general import or bibliography row limits; review before
  extending scope.

Resume only on the requested Standard-tier Sol model. First read this checkpoint
and inspect saved diffs, then repair evidence error handling, test the adapter
against captured and retained live JSON, finish the UI, and complete real
authenticated local E2E. Optional resolver execution still needs carefully
preserving existing proxy/certificate environment without logging values or
creating credentials. Its native bibliography command has wall/input/output
bounds here but lacks the GROBID CLI's memory supervisor; review that boundary.
No new persistent credentials or security/network changes were required so far.
If later integration requires them, stop that dependent step and report it.

After focused tests and independent review pass, run the required checks once,
regenerate the portable manifest, coordinate issues195/198, and publish the new
draft against the appropriate frozen integration base. Root retains visual QA
and serialized production publication ownership.
