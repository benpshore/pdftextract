# Local-file default continuation

This draft starts from PR213's preserved source tree, now squash-merged into
PR211 at `d8e902be6e409b15f476ea126cb29e861a23b095`, and is stacked on that
existing PR211 branch. Earlier PRs are unchanged by this draft. It
disables URL and webpage fetching, remote image retention and automatic metadata
resolution. Local upload, folder/drop/paste, progress, cache, stored originals,
owner-scoped embedded Office images, and text reading remain available.

Both authenticated capture and remote-asset endpoints return 503 before reading
their remote input or attempting DNS/fetch. Shared destination helpers and the
browser capture client also reject before network work. The browser capture tool
is not registered. URL-only paste shows the disabled message and preserves its
draft; ordinary pasted text remains importable.

Local HTML is parsed through an inert template with external resource elements
removed before building the extraction DOM. Reading permits only the current
document's stored image routes. Local scholarly HTML metadata and link evidence
remain extractable; remote images are omitted. Automatic resolution is disabled
even when an older service advertises it or an old preview requests it. Actual
local scholarly extraction remains opt-in through the existing loopback service.

Functional verification: TypeScript and production build pass; capability tests
5, runtime tests 15, scholarly Workers groups 7 and adapter checks 18 pass.
Workspace/reader/menu/lifecycle regressions pass.
Independent bounded functional review found no blocker; its stale empty-state
"Paste a link" copy was corrected to "Paste text". This was not a security review.
The retained browser report records its tested source hashes before that copy-only
correction; final build and manifest checks include the corrected copy.
Local Chromium runs the actual
app, sign-in, disposable D1/R2 and browser PDF Worker: 11 checks pass, including
local PDF/HTML/Markdown upload and reading, direct disabled API requests, preserved
URL draft, and zero external-resource requests for ordinary HTML/Markdown fixtures.
Captured scholarly output is replayed; no new live native/GROBID execution occurs.

Functional artifacts are in `/tmp/pdftextract-local-only-qa-final/report.json` and
`/tmp/pdftextract-local-only-qa-final/local-only-desktop.png` in the cloud workspace.
The 187-file portable source manifest verifies. Ruff and 92 Python tests pass,
alongside 28 source-export tests. Local OSV audit is blocked by the existing
network proxy; Cargo, Swift and CMake are absent here. Hosted CI must supply those
remaining checks before acceptance.
The parent authorized publishing this benign feature hold as a draft PR. Hosted
CI must complete on the published exact head. Only functional implementation and
its limits are published; potential exploit details remain private.
The first hosted Web run passed the feature checks but exposed an obsolete
remote-capture race fixture. That fixture stripped the new capability import and
waited for a capture barrier that is now unreachable. It now includes the actual
capability guard and verifies 503, zero fetch/write, preserved local document and
ordinary explicit deletion; all 11 lifecycle checks pass locally. No production
code was changed for this test correction. The final published head must pass CI.
Parent visual QA remains a separate acceptance gate. No deployment or production
migration occurred. This change does not verify SSRF/rebinding claims, parser RCE,
injection, or future MCP confidentiality/prompt-injection/symlink risks. Security
assessment remains blocked on the mandatory shared references and ready preflight;
potential exploit details are not published here.

Backend coordination: only web-owned bridge/configuration files change. No native,
OCR or backend-owner branch is changed or rebuilt. The backend owner should keep
registry resolution off for this local-file workflow and independently validate
the assigned native boundaries. This document is functional release-hold evidence,
not a security finding or clearance.
