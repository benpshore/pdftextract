# Web citation boundary and Upload interactions — 2026-10-04

Tracking: [#195](https://github.com/benpshore/pdftextract/issues/195) for the
scholarly citation contract and [#184](https://github.com/benpshore/pdftextract/issues/184)
for the concrete Upload/paste defects. Parent epics are #178, #180, #181 and #182.

This increment starts from frozen [PR #192](https://github.com/benpshore/pdftextract/pull/192)
at `66bbb59f524afbf69044b9dddf09dfcc5cb8688c`, on the separate branch
`feat/web-citation-boundary-menu-20261004`. It does not modify PR #192, native
parser branches, production data, or Site configuration.

## Scope and runtime boundary

The web app still uses PDF Oxide WASM 0.3.77 for browser PDF text/link extraction.
That worker does not produce scholarly bibliography. Ordinary article links,
DOI-shaped strings and embedded metadata are not relabeled as Works cited.

The user requires scholarly identity, end-of-paper bibliography, in-text
relationships, applicable DOI/Crossref/PMID/Europe PMC/publisher resolution, and
metadata-derived display naming through the native/GROBID workstream. This
increment provides the web consumer contract, runtime validation, conditional
reference/occurrence browsing and CSV inspection/export for supplied results.
It does not implement or claim that scholarly processing pipeline.

[OCR implementation #197](https://github.com/benpshore/pdftextract/issues/197),
[GROBID runtime #198](https://github.com/benpshore/pdftextract/issues/198), and
[OCR epic #199](https://github.com/benpshore/pdftextract/issues/199) retain their
separate owners. CPU OCR evaluation and a loopback GROBID service on a Codex
executor are not deployed Site services. No model, service URL, credential or
automatic document submission is introduced here. The eventual authenticated
service adapter and end-to-end scholarly qualification remain under #181/#182.

Saved result JSON can be user supplied. A field naming a native/GROBID provider
is not proof that the provider ran. The consumer must distinguish supplied
provenance from independently verified execution, reject invalid or mismatched
data, and expose unavailable states without fabricated records or stub actions.
Original filenames, source identifiers and bytes remain distinct from any
supplied derived display name. The existing result/artifact ownership boundary
is reused; no production migration is applied by this task.

## Interaction regression

The parent's published-UI observation used keyboard and mouse: Tab then Enter
opened the plus menu, Escape left it expanded, and a 900 ms mouse hold merely
focused the textarea. That is not an iOS touch/long-press or native clipboard
test. A focused Chromium probe of PR #192 confirmed named Add files/Add folder/
Add photos actions, but reproduced Escape and outside-click dismissal failures.
An intercepted empty file selection left focus on BODY; that fixture is not
an actual operating-system picker cancellation.

Upload now closes on Escape and returns focus to its trigger, dismisses on
outside interaction without stealing the clicked control's focus, and closes
when focus leaves the menu. Choosing an action restores the trigger before
opening the browser file control; supported native input `cancel` events also
restore it. Ordinary paste stays in the stable labeled textarea. Focusing it
does not read clipboard contents, request clipboard permission, or replace the
draft. Pure file/image paste in the composer queues an import while preserving
the draft; mixed text/file paste keeps the browser's native text behavior.
No unrestricted clipboard or iOS behavior is claimed.

## Consumer behavior

The [versioned contract](../web/docs/CITATIONS.md) requires explicit producer
provenance, independent references/mentions states, stable IDs, and a matching
saved-document identity and known source hash. Declared result revision
mismatches are rejected. An omitted embedded revision is accepted but explicitly
unchecked: current upload/PATCH writers allocate their storage revision after
serializing result JSON. The outer document GET record/result remains the
current context; this does not authenticate embedded producer claims.

Supplied results have separate References, In-text mentions and CSV rows views,
with vertical cards, printed labels/pages, filtering and 25-row pagination.
CSV inspection shows the same projected cell values that download exports,
including all filtered rows across pages. Formula-like spreadsheet cells are
escaped without mutating canonical fields. IDs, provider details and binding
diagnostics are under Details. Missing results expose a clear unavailable state.
Source links remain separate. In-text relationships are inspectable supplied
IDs; clickable navigation between mentions and references is not implemented.

## Verification and publication boundary

Component and real Chromium tests use clearly labeled synthetic service
responses to exercise the consumer and exports. Such fixtures prove UI/data
boundary behavior, not native/GROBID extraction, registry accuracy or production
service availability. Real scholarly end-to-end acceptance remains open in
#195.

Local validation on the frozen product sources passed:

- TypeScript `tsc --noEmit` and the production `pnpm build`.
- Upload/paste: 11 component groups; citations: 5 module/component groups.
- Reader: 15, workspace: 16, integrated lifecycle: 14 component checks.
- Chromium 151.0.7922.173: 32 checks, including keyboard menu behavior, native
  text and PNG clipboard paste, back/forward, repeated imports, real IndexedDB
  reload, malformed/mismatched citation rejection, actual filtered/full CSV
  downloads, reduced motion, and 390/768 px layouts at 200% text. Image OCR and
  remote/citation service responses are synthetic fixtures. There were no
  browser runtime errors.
- Ruff formatting/checks, 92 Python tests and 28 source-export tests.
- Independent read-only review reran the 11 Upload/paste and 5 citation groups
  and found no blocking findings within this consumer-only scope.

The lifecycle fixture now waits for its held synthetic upload/save stage before
clicking Cancel. This removes a test race without changing product timing.
The Web CI workflow now includes the citation and Upload/paste suites.
The portable source export verifies all 171 files; `web/SOURCE_MANIFEST.json`
has SHA-256 `a77b4bb4ba3b731993e35a54f96ed913d7c42afe5a774f41fdcf88b9466d311b`.

Committed browser evidence:

- [Report and tested source hashes](validation/web-citation-boundary-menu-20261004.json)
- [Upload accessibility snapshot](validation/web-upload-accessibility-20261004.txt)
- [References at 390 px / 200% text](validation/web-citation-references-390-200-20261004.png)
- [Mentions at 390 px / 200% text](validation/web-citation-mentions-390-200-20261004.png)
- [CSV at 390 px / 200% text](validation/web-citation-csv-390-200-20261004.png)
- [Reference card](validation/web-citation-reference-card-390-200-20261004.png)
  and [CSV cells](validation/web-citation-csv-cell-card-390-200-20261004.png)
  scrolled into view at 390 px / 200% text, with
  [capture provenance](validation/web-citation-card-captures-20261004.json)
- [Native PNG paste](validation/web-native-png-paste-20261004.png)
- [Full synthetic CSV download](validation/web-citation-supplied-download-20261004.csv)
  and [filtered download](validation/web-citation-supplied-filtered-download-20261004.csv)

Independent review source SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `web/app/workspace.tsx` | `01f00b13555bb531511909ed7734dde7d13223ed372075b6f4f22707007edbab` |
| `web/app/globals.css` | `99d415b709e77d8294b02ef0b41b373b571afc1c6a636d73d1a8d54fb4d6c62b` |
| `web/lib/citations.ts` | `6ea65e5d37e5517b87a3da08c7cf3de64f38103fb5c9c6383b3edf04d8fa9e5d` |
| `web/components/citation-browser.tsx` | `972aab2406ea2de8f99437dbfb32bb19fd0f065286326fb36f68f2f5ba476741` |
| `web/lib/types.ts` | `18c1ca5fb9ed9bc0d2d32b53a1bb05369c5c935d48fd04f68bf64c40d0fea913` |

The local OSV audit failed after proxy CONNECT errors; cargo, Swift and CMake
commands could not run because those tools are absent. These are not local
passes. Exact-head repository CI must supply that evidence. Local published
Site navigation remains tunnel-blocked; authenticated live testing belongs to
the parent and must follow an authorized deployment. No actual iOS/Safari or
native operating-system picker acceptance is established by Chromium fixtures.

No Site deployment, production migration, merge, release, access change or
existing user-document deletion is authorized or performed in this increment.
The parent retains the serialized publication handoff. Any later integration
must preserve new native work on the advancing base; do not overwrite it with
this web checkout.
