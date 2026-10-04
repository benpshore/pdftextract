# Local scholarly adapter

This opt-in increment is stacked on frozen PR207. It enriches a saved readable
PDF through a private Workers service binding and an explicit loopback bridge.
The production build has no SCHOLARLY binding. It does not deploy a native service.

`TPE_SCHOLARLY_LOCAL_URL` must be an HTTP `127.0.0.1` base URL. The bridge requires
either an absolute native executable plus loopback GROBID URL, or an explicit
captured JSON path. Captured replay is source-hash-bound and labeled separately
in health, saved metadata and the controls. It never falls back from failed live
processing to a fixture. The local bridge rejects browser-origin direct requests.

The adapter consumes the actual PR206 `GrobidDocument`, validates source/TEI hashes,
checks projection against a non-resolving TEI tokenizer and preserves exact JSON,
raw TEI, warnings and optional resolver JSON. It rejects DTDs/entities, mismatched
ranges, metadata and attributes. XML byte offsets stay relative to original UTF-8;
XML newline normalization affects comparisons only. Sparse tag-boundary offsets
avoid per-character memory amplification. A 2 MiB inert-comment check passes;
the 32 MiB buffering ceilings are local configured budgets, not qualified maximum
Worker capacity. JSON, TEI and evidence serialization still allocate multiple copies.

Reference fields and page numbers come from reported bibliography evidence.
Empty coordinates remain empty; no boxes are inferred. Body `ref type=bibr`
relationships are displayed only for explicit matching local XML IDs; absent
relationships remain unavailable/unmatched. Semantic coverage remains partial.
Source links, scattered DOI text and ordinary HTML links never become bibliography.
The captured two-reference fixture supplies no usable body bibliography occurrences,
so its in-text view is unavailable rather than an empty successful extraction.

Derived names use the first reported full header author, `et al.` for multiple
reported authors, and a unique reported title. Missing/conflicting titles fall
back to the original filename. `metadata.scholarly_display_name` records schema,
template, original filename and exact reported arrays. The citation document
contains the derived label. Reading titles and original filenames are preserved;
identical display labels remain distinct by document ID and original hash.
This is parser-reported naming, not registry-authenticated metadata.

Owner/origin checks precede source reads. Each attempt writes immutable evidence
and result objects, then publishes by result-key compare-and-swap. Failed/conflicting
attempts retain the prior reading. Document tombstones are checked around writes;
deletion and cancellation races have real D1/R2 tests. Browser cancellation aborts
its request and reconciles saved state, since server processing may still finish.
Writer controls prevent conflicting retry/removal/reread/delete/cache actions.
A streamed GET helper rechecks local scholarly version after the entire body arrives.
Evidence downloads are owner-bound passive attachments with hash verification.

Optional resolution invokes the existing native bibliography command separately.
Only unique exact raw-reference joins with compatible identifiers and known
providers are accepted; canonical resolver attempts remain separate. Bridge wall,
input/output and child-kill bounds are tested, but native bibliography lacks the
GROBID command's memory supervisor. When resolution is explicitly enabled, the bridge preserves only existing standard
proxy and certificate environment variables, alongside PATH and TPE_GROBID_URL.
It does not log values, create credentials or inherit arbitrary command settings. Actual registry execution was not qualified in this fresh workspace;
coordinate that interface with the native owner before enabling resolution.

## Validation and remaining acceptance

Run `node web/scripts/test-scholarly-adapter.mjs`,
`node web/scripts/test-scholarly-controls.mjs`,
`node web/scripts/test-scholarly-client.mjs`,
`node scripts/test-site-scholarly-workers.mjs` and
`node --test scripts/scholarly-local-runtime.test.mjs` from the repository root.
The full browser harness prepares pinned PDF/OCR assets, local auth and disposable
D1/R2; set PLAYWRIGHT_MODULE if the browser package is outside the project.
`node scripts/test-web-scholarly-browser.mjs` explicitly starts captured replay;
SCHOLARLY_QA_RUNTIME_URL instead selects an existing loopback live runtime.

The October 4 continuation has actual browser upload→captured service→saved result,
CSV inspection/download, native/TEI attachment download and reload evidence. The
checkpoint separately retains a real native/GROBID request; it is not live browser
E2E. The fresh environment has no preserved native executable/GROBID container.
Actual resolver execution, metadata/identifier classification of scholarly HTML
and uncertain PDFs, broader extraction accuracy, full native browser E2E, in-text
navigation where locators exist, mobile/Safari acceptance and parent visual QA
remain open under issues195/198 and service integration182. A PDF MIME/extension
only gates manual eligibility; it does not classify every PDF as scholarly.
Formal security scan is deferred. No deployment, production migration, credentials,
access expansion, public native endpoint, merge or release is authorized here.
