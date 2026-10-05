# Web alpha source and Site exchange

`web/` contains the private extraction alpha's application source, browser PDF
worker, HTML/feed extraction, owner-scoped API, database schema and migrations,
UI components, build adapters, and pinned pnpm lockfile. It is a source mirror of
the Site, not a copy of its user documents, credentials, or deployment identity.
The native Rust engine and this browser application remain separate runtimes.
See `web/README.md` for the implemented features and their limits.

The initial mirror was captured after Site source commit
`3b00638a24144c6ff4546e6ee43fab597c050678`. The updated 153-file mirror includes
the upload, OCR, Office, archive, reader and MCP implementation. See
[HANDOFF_TO_DOT.md](HANDOFF_TO_DOT.md) for the final publication checkpoint.
`web/SOURCE_MANIFEST.json` records each current source file, size, normalized mode
and SHA-256; its complete allowlist was round-trip compared with the Site checkout.
The manifest is deterministic: it contains no timestamps, machine paths, or Site
identifiers. `.gitignore` receives a deterministic safety appendix so generated
WASM, local deployment settings, and build state stay out of Git, while the web
build adapter source remains trackable inside the Rust repository.

## Verify and export

From the repository root:

```sh
uv run python scripts/site_source.py verify --source web
uv run pytest web/tests/test_site_source.py
uv run python scripts/site_source.py export \
  --source /path/to/site-checkout --output /path/to/new-source-bundle
```

The output directory must not exist, its parent must already exist, and it must
be outside the source tree. Export reads the allowlisted source twice and refuses
a changed snapshot. It copies regular files only and writes a SHA-256 manifest.
Existing outputs, symlink paths, special files, oversized bundles, missing core
configuration, and recognizable private-key material are rejected.

The allowed roots are `app`, `components`, `lib`, `db`, `drizzle`, `public`,
`scripts`, `vendor`, `build`, `hooks`, and `docs`, plus explicitly named project
configuration files and the lockfile. `.openai`, `.env*` (including examples),
`.npmrc`, `node_modules`, `dist`, `.sites-runtime`, `.wrangler`, `.git`, databases,
credential filenames, generated declarations/caches, and
`public/vendor/pdf-oxide` and `public/ocr` are excluded. Application source must still be reviewed
for inline secrets; filename filtering and checksums are not a secret scanner or
an authenticity signature.

## Edit and stage an import

Edit `web/` normally in Git. Verification will report a mismatch until the edited
source is exported with a new manifest. Create a fresh bundle, then stage it for
comparison with an existing Site:

```sh
uv run python scripts/site_source.py export \
  --source web --output /path/to/edited-web-bundle
uv run python scripts/site_source.py import \
  --source /path/to/edited-web-bundle \
  --output /path/to/new-import-stage \
  --against /path/to/site-checkout
```

Import verifies every allowlisted file's bytes, size, and mode before writing.
It creates a new source directory and `IMPORT_PLAN.json`, listing additions,
modifications, unchanged files, and target-only files to retain. `--against` is
read-only. The script never applies changes to an existing checkout, deletes a
target-only file, starts a preview, deploys, or touches deployment credentials.
Review the staged source and plan, then let the Site owner integrate the intended
files through the Site's normal editing and deployment workflow. In particular,
review build/configuration changes against the destination's current adapters.

The `web/tests/` regression suite belongs to this repository's exchange tooling;
it is not application source and is intentionally omitted from export/import.
Ignored files added beside a bundle are never imported. Extra or missing files
inside the source allowlist cause verification to fail.

## Folder imports

The browser app imports whole folders recursively from `web/lib/folder-traversal.ts`,
wired into the composer's Upload options, the `webkitdirectory` input and the drop
zone in `web/app/workspace.tsx`:

- **Chrome/Edge** use `showDirectoryPicker()` (read mode) and walk the directory
  handles asynchronously. A folder picked in this session is offered again as
  "Import `<name>` again" in the Upload options; the handle is re-used after a
  `queryPermission` check and the browser only re-prompts when its grant lapsed.
  The picker is also passed `startIn` and a stable `id` so it reopens near the
  last location. Where the picker is missing or rejects, nothing is added.
- **Safari/Firefox** (and every other browser) get `<input type="file"
  webkitdirectory>`; relative paths come from `webkitRelativePath`. Dropped
  folders use `DataTransferItem.webkitGetAsEntry()`: entries are taken
  synchronously inside the drop event and every directory reader is drained with
  `readEntries` until it returns an empty batch.
- **Policy** (`collectFolder`): hidden and system files are skipped (`.DS_Store`,
  `._*`, any dot-file or dot-folder, `Thumbs.db`, `desktop.ini`, `__MACOSX`,
  `~$` lock files); files are classified by extension, PDFs are queued first and
  the rest keep path order; unsupported types, files over the per-file limit
  (512 MB) and unreadable files stay in the import queue as "Not imported" with
  the reason instead of vanishing; the queue-entry limit (2000) and the
  directory-entry limit (50 000) stop a scan with a message that says where it
  stopped; duplicates are dropped once per batch by relative path or by equal
  size plus content digest (full SHA-256 up to 64 MB, a sampled digest above);
  progress reports "n of m files (bytes)"; Cancel aborts between entries and
  adds nothing. Nothing on disk is modified.
- **Queue and saved list**: the relative path names each queue entry and is sent
  as `path` with the original, where `app/api/uploads` keeps it (sanitised: no
  leading slash, `..` or control characters) as `original_name`, so the saved
  articles list shows "In `<folder>`" for every file imported from a folder.
- **Accessibility**: the scan status is a live region with a 48 px Cancel button,
  the summary stays until dismissed, options are keyboard reachable with visible
  focus, and focus returns to the Upload control after choosing an option.

Tests: `node scripts/test-import-flow.mjs` covers the three sources and the
policy without a DOM; `scripts/test-browser-ui.mjs` (Chromium against a running
`pnpm dev`, `TPE_UI_ONLY=folder` runs only that scenario) imports a generated
nested folder through the directory input, a mocked directory picker, Cancel and
a synthetic drop, and verifies the queue entries. Headless Chromium rejects the
real picker immediately, so the picker itself is mocked, not proven there.

## Local setup and deployment boundary

Use Node and pnpm versions declared in `web/package.json`. The lockfile pins
dependencies; install with a frozen lockfile. The Site's managed configuration is
intentionally absent. For an isolated local preview, create an ignored
`web/.openai/hosting.json` containing only local binding names:

```json
{"d1":"DB","r2":"BUCKET","capabilities":["mcp"]}
```

Do not replace an existing hosting file. In a managed Site, let Sites provision
that file and its actual D1/R2 bindings. From `web/`, the build commands are:

```sh
pnpm install --frozen-lockfile
node scripts/copy-pdf-wasm.mjs
node scripts/copy-ocr-assets.mjs
pnpm build
```

The copy scripts recreate the PDF Oxide JavaScript/WASM and Tesseract OCR
worker/core/English-model assets from pinned installed packages. Framework builds regenerate `next-env.d.ts` and route types.
Database migrations are under `drizzle/`; the deployment owner must apply them to
the intended database. Never commit local database contents or object storage.

**Authentication is deployment-specific.** `app/chatgpt-auth.ts` trusts the
`oai-authenticated-*` identity headers supplied by the trusted Sites edge.
`lib/server.ts` then scopes database and object access to that verified owner.
This header adapter must not be exposed behind an arbitrary public server where
clients can supply those headers. An alternative host needs its own verified
session/authentication adapter and equivalent owner isolation. Portable preview
mode includes mock authentication for isolated local development; never expose
that preview as a public deployment. The build's Cloudflare/D1/R2 and Sites
adapters are included so their boundary is visible, not silently bypassed.

The source mirror does not export private user data. Moving an application's
source between Sites does not migrate existing documents, buckets, database
ownership, or authentication identities.
