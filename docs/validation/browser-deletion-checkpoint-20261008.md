# Paused browser deletion checkpoint — 2026-10-08

Ben redirected delivery to a clean rebuild while this task was in progress. This
branch preserves an **unreviewed legacy implementation and reusable acceptance
fixtures**, not a landing recommendation. Stop further legacy feature layering.
No migration, deployment, main merge, release, security-setting change, real
user-document deletion or real browser storage clearing was performed.

## Baseline and ownership

- Inspected main `1bc92ccb7b566f11a3b750f21007cfeb310593b2`, root `AGENTS.md`,
  the empty environment `.agents` directory, and current open PR metadata.
- Main has no document DELETE route or control. Authoritative data is D1
  `documents` plus R2 `{id}/original`, `{id}/results/*`, `{id}/assets/*` and global
  `uploads/*` receipts/multipart sessions. Browser recovery is IndexedDB
  `tpe-private-workspace`, owner-keyed `workspaces`. Read-only MCP uses the same
  D1 rows and R2 keys. No document Cache API/localStorage/service-worker store was
  found. OPFS `tpe-import-tmp` holds temporary archive members, not the library.
- Historical PR #187 (`5c0a3aa6d33ac22814e5906785aede118d772c87`) already owns
  reusable server deletion work. This checkpoint copies only its lifecycle,
  upload/asset guards, additive D1 schema/migration and Workers tests; it does not
  import the branch or its batch UI/dependency changes. The client helper comes
  from #192 (`b315c36a9d93c03ef1acac9339f758431587ff51`) with a timeout, redirect
  rejection and exact 204 success check.
- Inspected historical web integration #192/#229/#234 and concurrent folder
  #244/#259 and Zotero #248 scope. Existing branches/PRs were not edited.
- OCR/lopdf layout owner remains reviewer thread
  `01a119f7-ffae-72cb-b9fa-1bf2ea9e70c9`; OCR, PDF worker and native sources are
  unchanged. The small workspace orchestration diff must be reconciled explicitly
  if any independently owned UI changes are reused.

## Preserved implementation

A named, irreversible native confirmation precedes deletion from the reader or
Saved articles. A separate IndexedDB v3 journal atomically records confirmed
intent and removes matching cached queue/selection data before the HTTP request.
Only the matching document ID is removed; archive members remain independent
stored documents. Stale checkpoints read this journal in their own transaction.
Completed entries retain only the ID; pending entries retain the title for an
explicit Retry delete action. Lost responses, quota/transaction failures and
partial remote cleanup never produce a confirmed completion message.

D1 atomically reserves the deleted ID/owner and removes the document row. R2
cleanup scans the document prefix and matching owned receipts, aborting known
multipart sessions. Atomic insertion predicates, result CAS and late-writer
checks prevent old requests resurrecting that ID. Repeated deletion is
idempotent for the owner. Client aborts, late callbacks and cross-tab journal
notifications prevent local recovery/reader reappearance. No restore/undo exists;
confirmation explicitly says this cannot be undone.

## Independently useful fixtures and observed results

These are synthetic, disposable tests. They do not validate the deployed Site.

| Fixture | Observed result before checkpoint |
| --- | --- |
| `scripts/test-site-lifecycle-workers.mjs` | 11 grouped checks pass against real local D1/R2: ownership/no foreign bucket access, exact confirmation, pagination, repeated deletion, completed receipts, original-insert/result/asset/legacy PATCH races, late session creation, retry after cleanup failure |
| `web/scripts/test-browser-deletion.mjs` | 11 grouped Chromium checks pass with the real Workspace, upload client, IndexedDB and API routes, bundled into a loopback test app using disposable D1/R2; only identity and the deliberately stalled PDF worker are synthetic |
| `web/scripts/test-built-deletion.mjs` | 8 grouped desktop/touch-emulation checks pass against the **actual production build**, served by local Wrangler with isolated temporary D1/R2; real framework hydration, visible Upload/picker, Delete hit target, Open/Delete separation, Escape, cancelled/repeated clicks, keyboard confirmation, focus return, intended record/original removal, neighbor preservation, reload, raw IndexedDB and repeated server deletion |
| Existing web checks | TypeScript, build, 19 workspace component checks, clip/extraction/import-flow/Office/Node-WASM OCR/archive/MCP/Workers upload suites pass |
| Root checks | Ruff formatting/lint and 210 Python tests pass; source-manifest checks are run when checkpointing |

Browser used: Chromium `151.0.7922.173`. The first built-app run was blocked by
Wrangler trying to create its config under a read-only home. The fixture now uses
its own temporary `XDG_CONFIG_HOME`; the actual built-app checks then passed.
The first quota-injection browser run exposed an uncaught IndexedDB callback
exception; the preserved transaction wrapper aborts and rejects explicitly, and
all 11 checks then passed with no uncaught browser errors.

Focused helper/API ESLint passes. The actual workspace and unmodified main both
have five existing `react-hooks/refs` errors and two warnings; full workspace
ESLint is **not** a passing gate. Cargo, Swift and CMake are absent locally. The
local uv audit cannot reach OSV through the environment proxy. Hosted CI results
belong to the exact PR head and are reported separately; the inherited Web
workflow does not automatically run the two new manual browser fixtures.

The built-app fixture prints git commit, dirty-state flag, server bundle SHA-256
and browser version. Its exact-head rerun must have `worktreeDirty: false` and
follow a fresh build. A bundled component fixture alone is insufficient evidence
that the delivered site's controls work.

## Reproduction

After installing the frozen web lockfile, from `web/`:

```sh
pnpm exec tsc --noEmit
node --experimental-strip-types ../scripts/test-site-lifecycle-workers.mjs
node --experimental-strip-types ../scripts/test-site-uploads-workers.mjs
node scripts/test-browser-deletion.mjs
pnpm run build
node scripts/test-built-deletion.mjs
```

Install/provide Playwright separately. `PLAYWRIGHT_MODULE` may point to its ESM
entry; `CHROMIUM_EXECUTABLE_PATH` may select an installed Chromium. Both browser
fixtures create fresh profiles. The built-app fixture cannot target an arbitrary
Site URL; it creates only loopback Wrangler and its own temporary storage.

## Limits and clean-rebuild handoff

- **Do not merge or deploy this checkpoint.** The parent requested a fresh
  architecture. Start from acceptance criteria, then independently review any
  selected reuse; passing local tests are not default permission to reuse code.
- This is not an atomic physical-erasure transaction across D1/R2/browser storage.
  A process killed between object publication and its final guard can leave
  unreachable bytes until another cleanup sweep. Server tombstones block library
  resurrection but no durable background cleanup queue exists. Global upload
  receipt scans grow with stored receipt count. Minimal tombstones are retained.
- General multi-tab snapshot merging remains unsupported. The deletion journal
  protects deleted IDs, not concurrent changes to unrelated queue entries.
  Another device/profile's already-held bytes, old deployed clients, offline
  copies, downloads and backups are not remotely erased. BroadcastChannel plus
  focus/pageshow/visibility reconciliation covers current same-origin tabs only.
- OPFS disposal remains owned by the archive iterator. No global directory sweep
  is safe without ownership/leases; crash-orphan temporary files and the existing
  swallowed disposal errors remain unqualified. An independently stored archive
  can still contain the original member bytes and metadata references; deleting
  a member deliberately does not delete its parent or siblings.
- The named deployed Site was inaccessible to the read-only web tool. No deployed
  build identity or authenticated live-button result was obtained. Safari,
  VoiceOver, real iPhone/iPad, migration blocking by a held legacy connection,
  storage eviction and arbitrary power-loss behavior were not validated.
- If reusing the legacy API, `0002_document_deletions.sql` must precede it; retain
  tombstones, insertion/late-writer guards and IndexedDB v3 on rollback. Never
  deploy a rollback that permits old IDs to reappear. No production migration was
  applied here.
- For the new vertical slice, retain the useful behavioral tests: create two
  synthetic documents through visible controls; cancel confirmation; delete one;
  inspect persistence and neighbor bytes; repeat/reload; interrupt before and
  after remote commit; race writers and stale tabs; fail the local journal; verify
  actual delivery build/hydration/accessibility. Redesign the storage contract
  first, then adapt the fixtures instead of importing the legacy architecture.

Independent review and the parent's next scoped assignment remain pending.
