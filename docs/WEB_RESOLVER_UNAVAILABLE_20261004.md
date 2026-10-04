# Resolver request-failure status repair

Bounded follow-up stacked on unchanged PR211
`c1a3a28027922c6a36e396d7feeccedc00d5b46b`, branch
`feat/web-resolver-unavailable-20261004`. Frozen PR192/207/211 are preserved.

The [backend handoff](https://github.com/benpshore/pdftextract/issues/195#issuecomment-5976310680)
reports a real fresh resolver smoke with 0/2 passed and transport errors. Failed
requests are not successful identity verification or correct unresolved matches.
Reading that evidence exposed two web status errors:

- Native attempts with `outcome: error` and no accepted record displayed unresolved.
  They now display unavailable, retaining exact attempts and extracted identifiers.
- A resolver bridge stage failure left references not-requested after an attempted
  call. It now saves the supplied GROBID bibliography with unavailable resolution,
  the existing warning and prior reading intact; no resolver artifact is invented.

Completed not-found/mismatch/ambiguous lookups remain unresolved; no attempts
remain not-requested. A later accepted compatible record can still recover from
an earlier failed attempt. Conflicting identifiers remain withheld. Saved typed
results and CSV show the same status. No identity/display-name fields are filled
from failed resolver output. Canonical GROBID JSON/raw TEI and supplied resolver
JSON remain byte exact, with hashes/provenance and original filenames preserved.

Nine [resolver contract checks](validation/web-resolver-unavailable-20261004/resolution.json)
and seven [real D1/R2 groups](validation/web-resolver-unavailable-20261004/workers.json)
pass, including actual service→saved result for a simulated failed resolver call.
Independent bounded review passes. Adapter/citation regressions, TypeScript,
production build, Ruff, 92 Python and 28 source-export tests pass. All 184 portable
source files verify. [Verification/hash record](validation/web-resolver-unavailable-20261004/verification.json).
The published PR209 GROBID artifact is byte-identical to the existing captured
adapter fixture, confirmed at exact producer commit and SHA. This is published
artifact validation, not new runtime execution. Resolver responses in the new
web tests are explicitly synthetic contracts; no registry success is fabricated.

Local OSV is proxy-blocked; Cargo/Swift/CMake absent. Hosted CI must be assessed
on the new draft head. The handoff contains no current same-environment live
GROBID bridge; no expensive binary/image builds were repeated. Actual source-bound
native/resolver→browser acceptance remains open with the backend owner.
Parent visual QA remains an unfulfilled gate: its cloud browser rejected loopback
with `net::ERR_BLOCKED_BY_CLIENT`. No port/network bypass or new preview was tried.
No formal scan, deployment, migration, credentials/access expansion, public native
endpoint, native-owner branch edits, merge or release. The private Site is untouched.
