# TPE web app

This directory is the complete portable application source for the private TPE ChatGPT Site. It belongs to **benpshore/pdftextract**. It is separate from the repository's native Rust engine and macOS app.

- Private Site: https://pdftextract-alpha.junkmail-edu228.chatgpt.site
- Integration PR: https://github.com/benpshore/pdftextract/pull/175
- Full handoff: [HANDOFF_TO_DOT.md](../docs/HANDOFF_TO_DOT.md)
- User request history: [USER_REQUIREMENTS_HISTORY.md](../docs/USER_REQUIREMENTS_HISTORY.md)
- Source exchange and deployment boundary: [WEB_ALPHA.md](../docs/WEB_ALPHA.md)

## Application ownership

| Source | Web-app responsibility |
| --- | --- |
| `app/page.tsx`, `app/workspace.tsx`, `app/globals.css` | Authenticated mobile-first composer, per-item import queue, reader, saved records and navigation recovery |
| `app/chatgpt-auth.ts`, `lib/server.ts` | Managed Site identity, per-owner authorization, D1/R2 access and streamed result responses |
| `app/api/capture`, `lib/source-fetch.ts`, `lib/clip.ts` | Public source capture, HTML article/structured-page cleanup, RSS/Atom, Unicode, URLs/DOIs, tables and images |
| `app/api/uploads`, `lib/uploads.ts`, `lib/upload-client.ts` | Chunked originals/results/assets, SHA-256, completion receipts, atomic result revision updates and content detection |
| `app/api/documents`, `lib/article-assets.ts`, `lib/asset-storage.ts` | Private saved documents, actual retained article images, originals and ranged media playback |
| `lib/imports.ts`, `lib/image-ocr.ts`, `lib/office.ts` | Streaming archives, local image OCR and browser Office extraction |
| `lib/workspace-storage.ts` | Device-local draft/interrupted-queue recovery; durable originals/results remain in R2/D1 |
| `public/pdf-worker.js` | Published upstream PDF Oxide 0.3.77 WASM in a disposable browser worker; **not** the complete native TPE engine compiled to WASM |
| `public/pdf-api/v1/`, `lib/pdf-api.ts` | Callable local Docling.rs WASM PDF API, CPU layout/OCR sessions, bounded jobs and shared browser UI; [contract and evidence](docs/DOCLING_PDF_API.md) |
| `app/mcp/route.ts`, `lib/mcp.ts` | Authenticated read-only, paginated/streamed MCP access |
| `db/`, `drizzle/` | Authoritative document schema and immutable deployment migrations |
| `scripts/`, `build/`, `vite.config.ts`, package/lock files | Reproducible dependency assets, tests and Workers-compatible production build |
| `docs/OFFICE.md` | Office format evidence and explicit limitations |

PDF engines do not parse HTML. PHP-served HTML uses the HTML path; raw PHP source is retained as text, never executed. General binary/media originals can be stored without falsely claiming text extraction. The new local PDF path runs Docling layout and English OCR after explicit asset staging; model coverage remains partial. Audio/video transcription, a native processing service, full iWork decoding, and live JavaScript page rendering remain separate unfinished work.

Original storage and archive transport retain their existing limits. PDF processing now bounds input to 25 MiB, 200 document pages, 20 selected pages, 6 million rendered pixels per page, 16 raster OCR regions, 100,000 text cells and 1 million text characters per page, 8 MiB of structured output and 120 seconds per job. Callers can lower these ceilings. [Measured browser memory and reductions](docs/DOCLING_MEMORY.md) remain substantial; phones are unqualified. Actual browser memory, OPFS quota, storage/hosting limits, and worker budgets still exist. A 50 GB mixed batch has **not** been validated; interrupted upload parts are not yet durably resumable across browser/device loss.

## Reproduce checks

Use Node 22.13+ and the pinned pnpm release. From this directory:

```sh
pnpm install --frozen-lockfile
node scripts/copy-pdf-wasm.mjs
node scripts/stage-docling-assets.mjs # pinned static runtime assets, no models
# Explicit desktop setup for PDF layout/OCR, once; approximately 84 MiB of models:
node scripts/stage-docling-assets.mjs --models
node scripts/copy-ocr-assets.mjs
pnpm exec tsc --noEmit
node scripts/test-clip.mjs
node scripts/test-web-extraction-review.mjs
node scripts/test-import-flow.mjs
node scripts/test-office.mjs
node scripts/test-node-imports.mjs
node ../scripts/test-web-workspace.cjs
node --experimental-strip-types ../scripts/test-site-mcp.mjs
node --experimental-strip-types ../scripts/test-site-uploads-workers.mjs
pnpm run build
```

Before local type checking/building, create the local binding configuration as described in `../docs/WEB_ALPHA.md`. An external host must implement a verified identity boundary rather than trusting caller-supplied authentication headers. Do not disable authorization to make deployment work.

The focused tests use JSDOM, mocks, Node/WASM and real local Workers D1/R2 bindings as identified in each script. They are **not** proof of iPhone/iPad browser behavior or 50 GB capacity. `scripts/test-browser-imports.mjs` is a separate browser harness; its execution has not been verified in this workspace. `scripts/test-browser-ui.mjs` is a second, manual browser harness for the Upload symbol, tooltips and saved-articles dialog (Chromium against a running `pnpm dev`; device emulation only, not Safari or a real iPhone, and not part of CI).

## Source completeness and deployment

`SOURCE_MANIFEST.json` records the portable source's exact hashes, sizes and modes. Root `scripts/site_source.py` exports, verifies, and stages an import into a **new** directory. It does not deploy or overwrite an existing Site checkout.

Managed deployment configuration, credentials, database files, uploaded user files, installed dependencies and generated WASM/OCR assets are intentionally absent from the portable manifest. Logical D1/R2 bindings and MCP capability are reconstructed by the Site owner; dependency assets are reconstructed from the pinned package lock and copy scripts. These exclusions are deployment/data boundaries, not missing application implementation.

The root `.github/workflows/web.yml` owns web-app checks. Root Rust workflows validate the native engine separately. See the handoff for the exact published source revision, GitHub revision and test status rather than inferring production readiness from an open PR.
