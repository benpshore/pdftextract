# Native network capability

The native engine and CLI default to a build without the external HTTP
adapters. `network` is an explicit Cargo feature for corpus downloads and
bibliographic registry requests; GROBID requires both `grobid` and `network`.
No backend feature implicitly enables `network`. Runtime flags, environment
variables, PDF links, and extracted metadata cannot add code excluded from a
build.

Cargo features are unified across dependencies. Another consumer can explicitly
enable `tpe-biblio/network` without enabling the engine's `network` feature.
The engine resolver therefore also sets its client offline whenever the
engine capability is absent. Direct callers of the independently enabled
bibliographic crate can still use that crate's network adapter. Claims about
excluded dependencies must identify the selected package and complete feature
set, rather than infer them from a root feature alone.

Default builds retain local extraction, bibliography scanning, corpus cache
reads, and source/link provenance. A missing corpus source fails instead of
downloading it. `bibliography --resolve` fails before input/output processing.
The bibliographic client refuses requests even if a caller disables its
runtime offline setting. Offline mode also remains available in a network
build. Enabling `network` restores the existing adapters; it does not change
destination or redirect policy or qualify those adapters for deployment.

The library's local pipeline acquires a file snapshot, passes bytes to the
selected parser, interprets metadata and annotations as data, and publishes
local artifacts. The application jobs call those local pipeline and
bibliography entry points. They do not invoke registry resolution, browser
handoffs, AI requests, or corpus download adapters. The CLI's worker process
uses the same compiled feature set as its parent. `tpe-browser` creates
handoff records and does not perform acquisition itself.

This boundary applies to the native engine/CLI, not every program in the
workspace. The separate application AI client and Zotero client retain their
explicit network operations; the optional search ONNX module can download
models. They are not called by local extraction. Full Docling provisioning and
optional native dependencies also have separate build/runtime requirements;
this feature is not an operating-system egress sandbox or a guarantee about
third-party parser/FFI behavior. Existing worker limits, no-clobber publication,
and Partial/provenance rules remain necessary.

The web source-acquisition owner must independently gate the shared external
source fetch adapter before DNS or HTTP activity. Both URL capture and remote
asset ingestion must observe that capability, including assets referenced by
an uploaded local HTML file. Same-origin authenticated storage and access to
already stored originals/assets are separate operations. Uploading a file or
retaining a source URL must not enable external acquisition. No web files,
credentials, deployment settings, or deployed services are changed here.

This is a functional capability change. It is not a completed security
assessment or a statement that local PDFs are immune to parser RCE. Formal
assessment and release qualification remain separate prerequisites.
