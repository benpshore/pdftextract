# Source-derived region selection review

Review date: 2026-10-04 UTC. Worktree: `/workspace/pdftextract-source-fusion`.
Starting commit: `cd95a8b7243c690b24f59b1994ecb7670c95c1c0` (frozen reviewed-region
pilot). This specialist review owns this new document only. Production source,
PR #202, PR #208, native providers and root manifests are outside its edit scope.

## Decision boundary

This increment must derive its selection evidence from the exact PDF bytes and
retained extraction artifacts. Fixture transcripts, authored region boxes,
render-review labels and a manufactured `TrustedReview` are not permitted inputs
to an automatic source-derived decision. Independently authored evaluation truth
must stay outside the selector until policy is frozen.

A source `/ToUnicode` mapping is a PDF author's semantic declaration. Matching it
can establish consistency with that declaration under a supported parsing profile;
it does not prove the visible glyph means the same character. Character code,
CID, glyph ID and Unicode value are different identities. An identity Encoding
CMap or `/CIDToGIDMap /Identity` does not identify a Unicode character. A misleading
ToUnicode negative must be included in the evaluation and classified honestly;
it cannot be omitted from the denominator because it defeats the intended claim.

The narrow policy may abstain for the whole source whenever it cannot establish
complete occurrence ownership. Supported-format restrictions are legitimate;
silently skipping unsupported content while claiming uniqueness is not.

## Required source evidence

| Evidence | Necessary distinction |
| --- | --- |
| Full source SHA-256 and byte length | Identifies the immutable PDF examined; a path or producer assertion is insufficient. |
| Page/content/font/mapping object references | Object number and generation belong to that exact PDF revision. A dictionary reserialization hash is not a raw source-byte hash. |
| Content stream bytes/hash and operator ordinal | An ordinal identifies a parsed operation, not a source byte offset. Decoded-stream bytes must be distinguished from compressed on-file bytes. |
| Raw `Tj` string bytes | Original encoded character codes must remain available before Unicode decoding or NFC normalization. |
| Encoding CMap and ToUnicode provenance | Record the actual code-to-CID and code-to-Unicode rule used. Do not infer Unicode from glyph number or character count. |
| Font and metrics | A declared FontBBox or advance width is not a verified painted-glyph boundary. Unknown metrics must not make another occurrence disappear from overlap analysis. |
| Text/graphics state and source page frame | Preserve the interpreted text matrix, CTM, font size, spacing/scaling, page boxes/rotation and supported state assumptions. Derived regions must not come from fixture rectangles. |
| Exact chosen artifact locator | The selected text must be copied from one existing candidate span, with its actual artifact hash/page/array index. Source decoding must not synthesize replacement output. |

## Parsing and geometry gates

- Use full-consumption content parsing. In pinned lopdf 0.45.0,
  `Content::decode_strict` exists; the convenience
  `get_and_decode_page_content` uses lenient decoding. A successfully parsed
  prefix is not a complete content-stream interpretation.
- CMap counts, code spaces, mappings and termination must be validated, including
  duplicate/overlapping rules, malformed destinations and trailing syntax.
  Unsupported `usecmap`, writing modes or mapping forms must abstain unless
  implemented explicitly. Do not silently accept an inner valid block from an
  otherwise unsupported program.
- The initial profile can require one content stream and a small operator set.
  Multiple streams can continue a graphics/text state; independent reset-and-parse
  behavior would assign incorrect matrices or occurrence ordinals.
- Every visible text occurrence on the affected page must participate in overlap
  and ownership analysis. An unsupported Type1 neighbor is an obstacle or a cause
  for page/source abstention, never ignorable whitespace.
- Text matrix composition, advances, spacing, horizontal scaling and glyph metrics
  determine source-derived envelopes. Unknown graphics state, transforms,
  clipping, Forms, hidden text and marked-content semantics need explicit handling
  or rejection. Matching page dimensions does not resolve an unknown transform.
- Coincident/repeated source shows and source-derived region overlaps must cause
  abstention where ownership is ambiguous, independent of ordering or text equality.
  A candidate span crossing a region, missing geometry or overlapping another
  text atom cannot be accepted because one string matches the decoded source.

## Implementation review checkpoint

Concrete findings, final source fingerprints, observed validation and remaining
limits will be recorded here after the new implementation and separately held
evaluation evidence are available. No production build or security scan is
duplicated by this review.

## Standard continuation review and outcome

The transferred archive matches SHA-256
`71e9b3168f19e67ed7f4917c14b5c7aa4562771ac89e06da83378756d9b0ce5f`;
all 91 preservation manifest file hashes and lengths were verified. Truth bytes
were hashed without interpretation and remained sealed during policy review.
One Sol6.1 high reviewer established three evaluator defects: exceptions could
drop the remaining cohort, uncommitted projection files could count as available,
and the qualification test expected the wrong unsupported enum. All were fixed.
Follow-up review found that source and journal receipts could change together
without matching the frozen case hash. Source-to-frozen-input binding and baseline
receipt checks were added before scoring, with a dedicated synthetic regression.

The latest source parsing gates are directly tested with development sources:
malformed CMap wrappers/metadata, non-ASCII font resource names, cumulative font
byte/probe exhaustion, and conflicting Unicode aliases for shown glyphs.
The experiment passes 7 CLI tests, 16 selector/parser tests, 15 evaluator tests,
8 new runner tests and 22 retained lifecycle tests, plus formatting and strict
Clippy. These are mechanism tests, not native recovery or visible accuracy.

Policy fingerprint `c895a4657dfa416f5c99985ce39e48f6ccda1934989bda55e98d70cee1ff055f`
was recorded at 03:27:12 UTC before unsealing. The evaluator checks the frozen
implementation before execution and again before any scoring truth read. Actual
six-case native evaluation retained all attempts: three evaluated projections,
three unsupported sources, zero selected regions and all six neighbors preserved.
Fresh-engine qualification fails four of five tests because the supported
positive cases abstain on native/source geometry correspondence. The policy and
failed assertions remain unchanged. No automatic visible accuracy is established.
Detailed timing, hashes, raw receipts and limitations are in
[the qualification record](SOURCE_REGION_SELECTION_VALIDATION.md).
