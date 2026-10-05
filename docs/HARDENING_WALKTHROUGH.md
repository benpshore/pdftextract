# The held hardening integration, read as a processing story

Owner: Busybody Bob. Next review: 2026-10-05 UTC or a relevant head change.
This explains the focused current-main adaptation in draft #234, not the older
scholarly/UI stack or an accepted security/release result. The original independent
reviews cover exactly `82f95ad99185b6641023169f8dd99d6f92438ac1`; the follow-up
code snapshot is `3d122ac7e12a1b2b951a1f9ea48ffb4667157fbf` and needs delta review.

The first decision is whether an input requires acquisition. Local files and pasted
bytes are already available to process; a remote URL asks the system to obtain new
bytes. The native `network` Cargo feature and the web's fixed false capability
module make that distinction explicit. A build feature is not a sandbox: unrelated
AI/model-download paths and third-party libraries are outside this limited claim.

The native CLI therefore refuses a disabled registry operation before opening its
inputs or output CSV. The library resolver independently refuses HTTP when its own
root feature is off, even if Cargo feature unification enables a dependency's HTTP
code. The two checks belong at different boundaries: CLI ergonomics cannot protect
another library consumer. Local PDF parsing and cached corpus use remain possible.
GROBID needs both its optional module and explicit network capability; no browser to
native to GROBID end-to-end execution is established by this integration.

The web tells the same story through its client and server. The composer retains a
URL draft and explains the hold. Capture and remote-asset routes reject enabled
acquisition requests, while shared destination/fetch helpers enforce the same hold
for other callers. Authentication still happens at the owner boundary. Existing
owner-local assets are handled separately from remote image URLs: disabling new
acquisition is not an instruction to discard a user's already-owned stored bytes.
The public-only Workers flag remains additional transport policy, with its known
limits; a DoH preflight does not pin a later connection to that DNS result.

A local HTML upload then reaches one inert parsing boundary in
`web/lib/passive-html.ts`. A template holds markup without joining the live page.
Before removing link elements, the boundary copies canonical/feed URLs as strings.
Template fragment parsing discards a complete document's outer `<html>` tag, so a
small leading-prologue recognizer separately preserves its declared `lang`. Only
that opening tag enters an inert attribute carrier; the browser decodes attribute
entities, and only language is transferred to the sanitized document. Later tags
inside comments/scripts cannot become invented root provenance. Unrecognized or
malformed prologues deliberately have no recovered root language; this recognizer
is not a full HTML tokenizer or a semantic language validator.

With evidence retained, the boundary removes acquisition-bearing elements, handlers
and attributes. Hidden styles are inspected before style removal so hidden content
does not become visible. JSON-LD survives only as inert data for the clipper's limited
projection. No resource-bearing source document is handed directly to DOMParser.
Existing owner-local image callers can supply their own acceptance predicate; local
HTML clipping uses the default that accepts none. Browser fixtures verify this
particular path; they do not establish an OS-wide network guarantee.

`clipHtml` now visibly proceeds from that sanitized snapshot to provenance projection,
editorial cleanup, Readability selection, publication sanitization and text/Markdown
conversion. The former lazy/noscript image restoration machinery was unreachable
after passive stripping and contradicted this local-only path, so it is removed here.
Navigation links remain evidence; captions remain text; `metadata.images` retains
an empty array for client compatibility. Existing article-boundary fallbacks still
preserve omitted tables, code, footnotes and introductory text. This change does not
promise perfect article selection, image capture or historical storage migration.

The native provider repair addresses a separate ABI portability failure, not parser
accuracy. C's `char` aliases a signed byte on some targets and an unsigned byte on
others. Calling a signed-only conversion failed on Linux ARM64. `c_char_byte` copies
the one-byte bit pattern through `to_ne_bytes`/`from_ne_bytes`, preserving UTF-8 bytes
without changing pointer ownership or length bounds. Identity strings reject bad
UTF-8; diagnostic strings deliberately decode it lossily. A bounded loop cannot make
an arbitrary dangling pointer safe: callers still must supply live provider memory.
The new unit fixtures check high bytes, Unicode, NUL termination and the 255-byte
boundary; actual native execution remains a hosted qualification gate locally.

Provisioning happens before that provider can be used. The existing reviewed archive
and library hashes remain mandatory. The first repair tried short `sha256sum -c`, but hosted macOS rejected that too.
The shell now prefers the same `shasum -a 256 --check` interface already successful
in that job, with GNU sha256sum fallback when shasum is absent. Synthetic fixtures
exercise real GNU/shasum and a refusing macOS alias, including bad
archive/library pins and cleanup. They neither load a library nor run on Ben's Mac;
the actual macOS/ARM lane must verify the repair independently.

Finally, evidence collection must preserve failure meaning. The CLI commits bounded
Partial outputs and then exits nonzero. The Poppler diagnostic exporter now validates
that exact structured publication, its source identity, ordered page outcomes and
matching disk JSON/text before copying the same three development inputs. It finishes
those diagnostic exports, writes their statuses, and still exits nonzero if any result
is Partial. Failed/cancelled or inconsistent publications are rejected; a held-out
split is refused before reading input bytes. Diagnostic artifact availability does
not make extraction Complete, and no corpus artifact was downloaded by the steward.

Tests follow these boundaries. Production capability refusal is tested unmodified;
retained enabled-transport fixtures use a separate virtual test-only module. The real
Chromium harness uses actual local app routes with disposable D1/R2 and local sign-in,
then releases its browser/server/storage on failure or success. Synthetic observations
and hosted execution are recorded separately. At the original reviewed SHA, two live
registry tests passed, native/binary execution genuinely ran, and Native still failed
209 incomplete-paper Partial validations with zero historical exceptions. No change
to reviewed exceptions, pins, permissions or release semantics hides that gate.

Done for this follow-up means the exact delta, evidence, remaining failures and
reviewer records are inspectable. Main promotion also needs independent delta review,
required checks, separate quality/security/release acceptance and explicit authority.
The missing formal-security prerequisites and issues #215/#216/#217/#220 remain
separate requirements; neither this narrative nor engineering fixtures closes them.
