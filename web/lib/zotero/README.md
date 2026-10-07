# Send to Zotero (web)

A self-contained panel that pushes the article shown in the reader into a
Zotero library: metadata as a `journalArticle` (or preprint, web page, …),
the DOIs found in the text as a child note, a `linked_url` attachment for the
source, and optionally the saved original as an uploaded file. The browser
talks to `https://api.zotero.org` directly; the API key never reaches this
site's server.

## Wiring (one line in the reader)

`app/workspace.tsx` is owned by the import track, so the panel is not mounted
yet. In the reader's secondary area (next to the Download / Source links
details), add:

```tsx
import {ZoteroPanel} from '@/lib/zotero';
// …inside the `result ? <>…</>` branch:
<ZoteroPanel key={selected?.id ?? selectedItem?.id} article={result} sourceUrl={selected?.source_url}
  original={selected ? {url: '/api/documents/' + selected.id + '/original', name: selected.original_name, contentType: selected.mime} : null}/>
```

`key` resets the prefilled fields when another article is opened. `original`
is only needed for the "upload the original file" option; the panel fetches
that same-origin URL (with the user's cookies) when the box is ticked.

## What it sends

| Zotero object | Source | Notes |
| --- | --- | --- |
| Top-level item | `Extracted.metadata` (`title`, `authors`/`author`, `date`/`year`, `journal`/`publicationTitle`, `doi`, `url`, `abstract`, `arxiv`, `pmid`) and the first-page DOI link | Every field is shown for editing before sending. Nothing is guessed: unknown fields stay empty. `"Family, Given"` splits a name; other names are sent as single-field creators. |
| Child note | `Extracted.links` with a DOI (deduplicated, the item's own DOI excluded) | HTML, escaped, one `<li>` per reference with page numbers. |
| `linked_url` attachment | `sourceUrl` or the metadata URL | Off when there is no URL. |
| `imported_file` attachment | `original` | Three-step upload (see below). Off by default. |

Item type, library (user or writable group), collection and tags are chosen
in the panel. Keys of created objects and the zotero.org link are shown
afterwards; a failed child (note, link, file) is reported as a warning next
to the created item key, never as a silent loss.

## The API key

* Entered once (`<input type="password" autocomplete="off">`), verified with
  `GET /keys/current` (the key travels in the `Zotero-API-Key` header, never
  in a URL), then kept **only in this browser**:
  * *Until this tab is closed*: `sessionStorage` (default).
  * *On this device*: IndexedDB (`tpe-zotero-credentials`). When IndexedDB is
    unavailable the panel says so and keeps the key for the tab instead.
* "Forget key" clears both places. The key is in no URL, log, request body
  or server of ours; `lib/zotero` has no server route.
* Needed scopes on <https://www.zotero.org/settings/keys>: library access,
  notes access, write access (and per-group write access for groups).

## CORS

`api.zotero.org` serves `Access-Control-Allow-Origin: *`, allows the
`Zotero-API-Key`, `Zotero-API-Version`, `Zotero-Write-Token`, `If-Match`,
`If-None-Match`, `If-Modified-Since-Version` and `If-Unmodified-Since-Version`
request headers for `GET`, `POST`, `PATCH` and `DELETE`, and exposes
`Last-Modified-Version`, `Total-Results`, `Link`, `Backoff` and `Retry-After`,
so every API call is made from the browser and no pass-through route exists
(`web/app/api/zotero/` is intentionally absent). This follows the published
API behaviour; it could not be re-verified from the build container, where
`zotero.org` is blocked by the egress policy.

The one request that goes elsewhere is upload step 2, the `POST` of the file
bytes to the storage host named by the authorisation response. Whether that
host answers browser preflights for a third-party origin is not verified. The
code treats a blocked upload as a warning (the item, note and link are
already saved) and tells the user to use the source link instead. If a real
browser run shows the storage host refusing CORS, the fallback is a minimal
route under `web/app/api/zotero/upload` that forwards exactly one request's
bytes and headers and stores nothing; it is not implemented because nothing
proves it is needed.

## Protocol details

Implemented in `api.ts` and mirrored by `crates/tpe-zotero` (Rust):

* `Zotero-API-Version: 3` on every request; `limit=100` reads that follow
  `Link: rel="next"` only inside `https://api.zotero.org/`.
* `POST <prefix>/items` with at most 50 objects and a fresh 32-hex
  `Zotero-Write-Token`; the 200 body's `successful`/`success`, `unchanged`
  and `failed` maps are parsed per index.
* `PATCH <prefix>/items/<key>` with `If-Unmodified-Since-Version`; `412` →
  "changed since read".
* File upload: `POST <prefix>/items/<key>/file` (`md5`, `filename`,
  `filesize`, `mtime`, `If-None-Match: *`) → `{exists: 1}` or an upload
  target → `POST` to the storage URL (full `prefix + file + suffix` body or a
  multipart form with the `file` part last; no Zotero headers) → `POST
  <prefix>/items/<key>/file` with `upload=<uploadKey>` → `204`.
* `Backoff: <s>` on any response pauses the next request; `429` honours
  `Retry-After`; `5xx` and network failures retry with exponential delays
  (three attempts); `400`, `403`, `404`, `409`, `412`, `413`, `428` map to
  typed `ZoteroApiError`s with user-readable messages.

## Accessibility

Every control has a visible `<label>`; radio and checkbox options are
`<label class="zotero-choice">` rows of at least 44 px; buttons are 48 px
tall; focus is a 3 px ring (`:focus-visible`); progress goes to a
`role="status" aria-live="polite"` region; errors are `role="alert"` and stay
until the user acts (no timed dismissal); the panel is a `<section>` labelled
by its heading. Layout wraps at phone widths (`auto-fit, minmax(14rem, 1fr)`).

## Tests

```sh
cd web && node tests/zotero.test.mjs
```

JSDOM plus a mocked `fetch` (no network): pure helpers, the API client
(headers, paging, write token, `Backoff`, `Retry-After`, error kinds), the
send run with the three-step upload, and the panel flow (connect with a
session-only key → collections → send → forget; refused key). It is a
JSDOM/Chromium-free check, not proof of Safari or VoiceOver behaviour.

## Browser write boundaries

Authenticated API fetches use `redirect: 'error'`, including reads, writes,
pagination and upload authorization/registration. No redirect is followed
with the API key. Storage byte uploads still omit the API key.

The send hook synchronously locks the entire operation before preparing its
plan or fetching the original. An original-read failure releases the lock
without a parent write, allowing a retry or sending without the original.
Once a parent write starts, the mounted panel retains that fact, its saved
outcome (including child warnings), or an explicit uncertain outcome. It
will not start a second parent write, even through direct form events or
Forget key/reconnect. API transport retries keep the same write token.
After an uncertain write, check the library before starting another send.
This guard is scoped to the mounted article panel; reloading/remounting or
another tab is a new operation, not a durable cross-tab deduplication system.

Run `node tests/zotero.browser.test.mjs` with Playwright and Chromium
installed (`PLAYWRIGHT_MODULE` can point to an existing installation).
The fixtures use only loopback servers and synthetic keys/documents. They
exercise a real cross-origin 302, delayed-original double submission,
retained saved outcomes, pre-write acquisition failure, and lost-response
retries with a single write token. CI runs both the JSDOM and Chromium suites.
