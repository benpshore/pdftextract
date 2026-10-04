# Security hardening integration ledger — 2026-10-04

Owner: **Busybody Bob** (Mara Keel alias). Next review: **2026-10-05 UTC**, or any
source/main-head change. Parent task: `01a0fe84-0716-7474-b2b7-709e4359b0fb`;
agent task: `pdftextract/stewardship/2026-10-04/busybody-bob`.
[Maintenance epic #225](https://github.com/benpshore/pdftextract/issues/225) owns
this work; [preparation/authorization receipt](https://github.com/benpshore/pdftextract/issues/225#issuecomment-5981124716).
Ben authorized combination on this dedicated branch at 14:25:21 UTC. **Promotion
to main remains held.** Main merges automatically release; this branch is a
review/test artifact, not release authority or a deployment.

## Base, order and preserved sources

Branch: [`integration/security-hardening`](https://github.com/benpshore/pdftextract/tree/integration/security-hardening).
Created from and independently read back at exact current main
`72890e6f9d23b1c10ee2cd9d368c7c0817319c10`. The draft PR and external #225 receipt
record the final head; a file cannot contain its own final commit hash.

| Order | Focused source | Integration operation |
| --- | --- | --- |
| 1 | #222 `6fc3266e0860de85b0bf5bd39a364d783a1a2796` | Exact seven-file tree `10b1fd92c1743c72ffb1dc9a039106d23a33d31e`; new attributed commit `f1074856590f8c5c949a0fbce85c30ceb01b3b68` |
| 2 | #223 `7385a3bcd4592a2071cd5eb93d566fe4ec472723`, contribution base `3b73e37101e0ad1a22372601152f81403b273c54` | Only its 16-file patch, applied cleanly to current main; commit `bb0f15c015fa4b8719381391735518381a1a38db`, tree `f1a608c29cdc3737c6601f6cb89540a4c646c0b8`; unrelated main Cargo/error content preserved |
| 3 | #229 `b752bae170ae628c07b12800a44a0a37b235db28`, contribution base `e15e0e081fdaf8d5904b6154065b16dcd0658f5b` | Adapted compatible local-only web guards/parser/client/composer/tests; keep #222 contract/provider flag; exclude old scholarly/reader/storage parent-stack features |

Individually named archives were created and independently read back at those
exact heads: `archive/steward-20261004/pr-222-6fc3266e0860`,
`archive/steward-20261004/pr-223-7385a3bcd459`, and
`archive/steward-20261004/pr-229-b752bae170ae`. Originals remain open/draft and
unchanged. No reparenting, closure, branch deletion, force push, arbitrary old
stack import, or change to Ben's four user merges occurred here.
[Machine-readable provenance](stewardship/security-integration-20261004/provenance.json)
records exact patch SHA256 values and operations.
[Bounded live open-PR inventory](stewardship/security-integration-20261004/inventory.json)
records other work as separate/not staged without guessing readiness or agent
identity. In particular #175/#192/#228 and earlier native parent stacks are not
part of this tree; older resource/permission/transport PRs need their own focused
review. #226's accountability implementation remains a separate draft.

## Behavior and interaction resolutions

Native CLI/library HTTP adapters require explicit `network`; GROBID requires
`grobid,network`. Default local extraction and cached corpus data remain usable.
Resolver feature unification is explicitly tested; runtime settings cannot add
transport excluded from a build. This is a capability boundary, not an OS-wide
egress sandbox or a qualification of optional adapters/FFI.

Web production source acquisition is fixed off before DNS/HTTP. Capture, remote
assets and client calls reject; the composer preserves a disabled URL draft and
does not register the remote capture tool. Local file/folder/text, authenticated
storage, stored owner assets, reading and downloads retain current-main flows.
The original web adapter's scholarly service/resolver does not exist on main and
was not imported. [Web boundary details](../web/docs/HARDENING_INTEGRATION.md).

Two interactions required explicit resolution:

1. #222's enabled-transport test conflicts with #229's production hold. The
   unmodified production module is tested for early refusal first. A separately
   bundled virtual capability stub then exercises retained transport fixtures.
   It is test-only: no production setting enables acquisition. Generated
   public-only Workers flags and an isolated workerd connection boundary remain
   tested; this does not qualify re-enablement, deployed own-zone routing, or
   live authoritative-DNS rebinding.
2. Removing resource-bearing HTML elements dropped canonical/feed provenance.
   A new focused assertion reproduced null canonical metadata. The adaptation
   reads canonical/feed URLs from an inert template as data before resource
   removal. The assertion and actual Chromium local-upload checks now pass,
   retaining text, caption, source links and scholarly meta tags without loading
   external images/styles/frames or executing source scripts.

#222's source comment/provider flag and current-main asset-route implementation
are retained around the adapted guards. UI work is limited to communicating the
hold and retaining drafts; the later Alpha V2 redesign remains queued.

## Combined engineering evidence and limits

- **14 web commands pass**: TypeScript, network capability, HTML/feeds/Unicode,
  import/upload/Office/node compatibility, real component reader/save behavior,
  MCP and Workers D1/R2 storage, production build and source-fetch fixtures.
  [Command results](stewardship/security-integration-20261004/web-commands.json).
- **5 actual Chromium 151.0.7922.173 checks pass** against the actual local app,
  disposable migrated D1/R2 and local sign-in, without API interception:
  signed-out/spoofed capture refusal; disabled authenticated capture and retained
  URL draft; exact saved local HTML originals/metadata and disabled assets;
  paste/save/read/reload; zero browser external-resource requests/runtime errors.
  [Browser report](stewardship/security-integration-20261004/browser.json).
- Ruff and **210 Python + 28 portable-source tests pass**. The **159-file** source
  manifest verifies. [Native/local command results](stewardship/security-integration-20261004/native-commands.json).
  Local OSV audit failed at its proxy tunnel; Cargo, Swift and CMake are absent,
  so native compilation/test results must come from exact-head hosted CI.
- [Tested-file hashes](stewardship/security-integration-20261004/tested-files.json)
  make the tested code/config/artifacts inspectable. Hashes do not certify code
  behavior. Hosted CI is a separate exact-head gate; source PR green checks do
  not establish combined qualification. The PR/#225 receipts record final runs.

These are synthetic engineering interaction checks, not a formal security
clearance, parser-safety or deployed-identity proof, live registry/GROBID test,
mobile-device test, general extraction accuracy score or 50 GB capacity test.
The native **209 Partial** quality gate remains. #215 remains open and untouched.
Production/site state, package/lock/toolchain pins, versions/tags, protections,
security settings, permissions and credentials were not changed. CI coverage
was retained and augmented with the source contribution's explicit capability
matrix and web guard/transport tests; no gate was weakened.

The formal [security-diff skill](skill://Plugin_1e648473be9c8191a91ac3947151af55/security-diff-scan/SKILL.md)
requires shared config preflight before its workflow. The shared preflight
resource and callable scan/preflight/context tooling remain unavailable. The
[patch-risk skill](skill://Plugin_1e648473be9c8191a91ac3947151af55/assess-patch-risk/SKILL.md)
requires shared artifact-storage instructions/schema; those remain unavailable,
although its leaf rubric/validator are readable. No formal goal/scan, validated
risk rating or audit was created. Authorized engineering tests do not substitute
for those blocked resources. No permission/security-policy workaround was used.

## Accountability and completion criteria

Responsible integration fingerprint:
`sha256:2b6c731a6eed3711be7c08910d82c6a5fd60b74c96b9cceec0464400a53d1e6f`;
configuration: `sha256:4c95bc39e9e3ba047139418455b3f6bdc3e61174472c9f614205a7aa124b2cb3`.
The stable [identity registry](https://github.com/benpshore/pdftextract/blob/aa51e4343853a4c41f723b6706889ea2fca820d5/docs/stewardship/agents.json)
and validator in #226 were used for the new commit trailers and
[8-record integration action segment](stewardship/security-integration-20261004/actions.jsonl).
[Independent checkpoint](stewardship/security-integration-20261004/checkpoint.json):
`65de5e4a22d33bc3c3e0cdaf6e0389243a9b63b8b9d52075f2f0df96c68b50be`.
This segment does not rewrite #226's earlier journal. SHA256 establishes registry
lookup/content/order integrity, not authenticated signing, authorship or
correctness. Ben's human commits are exempt; a GitHub account is transport, not
agent attribution. Original contributors with uncertain metadata remain unknown;
no past identities were invented. All three new commit messages validate against
that registry. Provider usage/credit receipts remain unavailable; no child
agents, purchases or speed/spend expansion occurred.

Done for the staging task: verify final base/head/tree/order, publish a clearly
agent-created draft PR with source links and combined evidence, preserve source
PRs/archives, and report exact-head hosted checks or blockers honestly. Main
promotion, deployment, security audit, native accuracy and parent-stack landing
remain separate decisions.

Rusty Rick's parent-supplied source-linked read-only report is retained as a
later #227/#232 input. Its external four-line task digest, report/manifest hashes
and truncated-copy limitation are recorded in provenance. It is not yet a
validated canonical registry identity or implemented/compiled architecture fix.
Observations, historical measurements and proposals remain distinct; the later
queue does not expand this integration's scope.
