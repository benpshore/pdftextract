# Web scholarly local continuation

Resumed authorized WIP c9e762c on `feat/web-scholarly-adapter-local-20261004`,
stacked on unchanged frozen PR207 (`32d93cab432623b6818aee2104e9c9f4094557f2`).
PR192 remains frozen at `66bbb59f524afbf69044b9dddf09dfcc5cb8688c`.

The missing evidence-route await is repaired. Actual canonical PR206 JSON/raw
TEI now pass strict adapter checks against both captured and checkpoint-live
artifacts. Controls are mounted under Citations, preserving immediate reading.
Cancellation reconciles rather than assuming rollback. Writer guards cover
queue retry/removal, reread, delete and cache clear; unmount aborts requests.
Streamed GET checks run after body consumption and synchronously after JSON
parsing at the selection write. Five focused response tests cover both race windows. Source filenames
stay intact; deterministic header-derived names retain template, raw field and
fallback provenance separately. Explicit resolver mode preserves existing
allowlisted proxy/certificate configuration without logging or creating credentials.

Independent review found and verified fixes for per-character XML offset memory
amplification and XML CRLF normalization. The 2 MiB probe's heap fell from about
193 MB to 15 MB. The configured 32 MiB ceiling is not max-size Worker acceptance.
See [contract and limits](../web/docs/SCHOLARLY_ADAPTER.md).

Verified evidence: [checks/source hashes](validation/web-scholarly-continuation-20261004/verification.json),
[adapter](validation/web-scholarly-continuation-20261004/adapter.json),
[React controls](validation/web-scholarly-continuation-20261004/controls.json),
[streamed GET](validation/web-scholarly-continuation-20261004/client.json),
[real D1/R2 races](validation/web-scholarly-continuation-20261004/workers.json),
[runtime tests](validation/web-scholarly-continuation-20261004/runtime.txt), and
[browser](validation/web-scholarly-continuation-20261004/browser.json).
The browser uses actual local auth, PDF Worker, multipart uploads and workerd
storage. Nine checks cover saved references, passive native/TEI downloads, CSV
inspection/download and reload with zero browser errors. It explicitly replays
captured native output; no new live native/GROBID or registry request is claimed.
The older checkpoint's live 29.342 s request remains separate evidence.

TypeScript, production build, Ruff, 92 Python tests and 28 source-export tests
pass. Existing citation/reader/workspace/Upload/lifecycle/queue/storage/HTML
regressions pass. All 183 portable source files verify. Production build has no
SCHOLARLY service binding. New focused checks are included in Web CI.
Local OSV is proxy-blocked; Cargo, Swift and CMake are absent. Exact-head hosted
CI remains to be assessed; local build/test success is not full repository CI.

Remaining blockers: this fresh environment lacks the preserved native binary
and GROBID container. Actual resolver execution and native resolver memory
qualification need coordination with backend owner
`01a104e2-af9d-736f-a9ca-4da4148a4020`. General scholarly metadata/identifier
classification of HTML and uncertain PDFs, broader semantic accuracy, usable
in-text navigation, mobile/Safari acceptance and parent visual QA remain open.
PDF eligibility does not classify every PDF as scholarly. Issues195/198 remain
open; no completion of their entire acceptance is implied.

No deployment, production migration, persistent credentials, access expansion,
public native endpoint, native-owner branch edits, merge or release occurred.
The existing private Site is old and untouched. Parent retains serialized visual
QA/publication ownership and scheduled email. Formal security scan is deferred.
