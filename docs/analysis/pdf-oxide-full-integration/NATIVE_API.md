# Native operation API and worker

This checkpoint is experimental. Read [CHECKPOINT.md](CHECKPOINT.md), the source
matrix, and exact run evidence before treating a capability as qualified.

The Rust entry point is `tpe::oxide::execute(operation, &Context, &params)` under
`pdf-oxide`. Context contains immutable source bytes and verified hashes, optional
one-based inclusive page selection, a password, and finite limits. Parsing hostile
input in an ordinary application process is not supported: use `tpe oxide` or
`tpe extract --backend pdf-oxide`. The containment evidence here covers these
worker routes; benchmark, bibliography and library callers do not inherit it.

```json
{
  "operation": "query.text",
  "inputs": ["/workspace/approved-fixture.pdf"],
  "pages": [1, 2],
  "params": {}
}
```

Save a request as a private local file, then run:

```sh
tpe oxide --request request.json --out derivatives \
  --timeout-ms 60000 --max-memory-growth-mib 1024 --max-output-bytes 67108864
```

Requests deny unknown fields. `password_file` is an optional path to at most
4,096 UTF-8 bytes, including an intentionally empty password; whitespace is
literal. New encryption passwords belong in the private request parameters and
are omitted from response provenance. No password is needed in argv. Sources
are read once with size/change checks in a supervised acquisition worker and
copied into controller-owned private staging. The operation worker verifies those
staged bytes before sealing filesystem access. The same deadline, cancellation
and capture limits cover both workers; the controller does not parse input PDFs.

`security.redact_text` requires bounded, nonempty `verify_absent` target strings
in the private request. Values are omitted from receipts and errors; receipts
retain only their count. The narrow adapter verifies supported decoded content
and independent extraction before and after rewriting. Retained targets,
uncertain decoding/encoding, marked content, saved text state and unsupported
carriers cause an error with no derivative. This is target-specific evidence,
not a universal secure-erasure claim. See the exact scope in [security](security.md).

The worker installs a Linux syscall policy, resource limits and the existing
controller-death lease. Its x86_64 implementation has bounded runtime evidence;
the aarch64 implementation has not been executed in this VM. Failure to install the policy is
an error. The policy denies path IO, sockets, exec and process creation. Threads
remain allowed; there is no separate cgroup/PID/thread-count guarantee. Ordinary
application-library callers cannot opt into filesystem-dependent features by
setting a JSON flag. The controller and atomic publisher retain their intended
file access; parser-worker evidence does not describe those processes.

Outputs include exact source hashes, public request options, dependency/build
identity, resource and containment evidence, diagnostics, and owned artifact
bytes. `producer.build_version` preserves the build's git/tag identity, including
the dirty marker when present; it is not a signed attestation. With `--out`, artifacts are published without clobbering existing files;
the receipt retains each artifact's name, size and SHA-256 while its inline byte
array is emptied. A publisher failure can report `publication_unknown`; inspect
existing outputs before retrying. Originals are never used as publication paths.

Each response includes a fresh opaque `request_id`. Before creating output files,
the publisher checks the controller-held operation, ordered source hashes/sizes,
public request options, current producer/backend, unused publication fields and
the exact captured-response digest. Full private-request/password hashes stay in
private transport only. These checks reject swapped/replayed transport messages
and altered captures; they cannot establish that a parser's artifact is a correct
semantic transformation of the source.

`capabilities` returns dispatch names, compiled features and known unsupported
edges. It is not an assertion of accuracy or that every possible input works.
Family references: [query](query.md), [creation/Office](create-office.md),
[editing/forms](edit-forms.md), [security](security.md), and
[rendering/OCR](render-ocr.md). Additional operations are `convert.text`,
`convert.markdown` and `convert.html`; table reconstruction is disabled, and HTML
keeps raw markup separate from an attribute-free sanitized display projection.
Query/conversion outcomes remain `partial`; `truncated: null` means unknown.
An observed native operator-cap warning gives `truncated: true` with its original
category and page value. Separate operation-page context does not restamp a
missing native page. Known silent text-cap paths are rejected by a conservative
bounded work preflight. Text-consuming adapters also reject Form subtype shapes
that can make the pinned reader omit a whole Form; priming its bounded cache is
not enabled as a correctness repair.

Feature combinations are explicit:

| Feature | Additional behavior |
| --- | --- |
| `pdf-oxide` | Queries, conversion, creation, editing/forms, Office and security adapters |
| `pdf-oxide-rendering` | Raster/separation/flatten paths in a confined worker |
| `pdf-oxide-barcodes` | Bounded QR and one-dimensional barcode generation |
| `pdf-oxide-signatures` | Offline signature verification; no signing, TSA or online trust claim |
| `pdf-oxide-ocr` | Offline tract inference using supplied model/dictionary bytes |

The existing `tpe extract --backend pdf-oxide` path also seals its parser worker
after input acquisition. Optional `--pdf-oxide-cleanup-preview`,
`--pdf-oxide-html` and `--pdf-oxide-images` add separate, bounded projections.
Cleanup can remove meaningful headings/citations and is never committed back to
the source or baseline spans. Tables are raw candidates with explicit validation
errors, not validated interpretations. Supervised `routed` extraction returns an
explicit unsupported error until lazy PDFium loading has a qualified filesystem
policy. Its existing library implementation remains preserved for parent review;
it does not silently fall through to unconfined parsing in this CLI checkpoint.

No database/queue service, model download, persistent MCP registration, remote
write capability or release is introduced. Swift, platform-specific native
dependencies, cross-binding parity and OCR/rendering accuracy require separate
evidence; successful local tests alone do not supply it.

Flattening has a further conservative page-carrier admission rule. A selected
page with affected annotations/widgets must have supported explicit /Contents
and its own /Resources dictionary (direct or one indirect reference). Pages that
rely on inherited resources reject because the pinned writer shadows their fonts
and images; pages lacking Contents reject because it removes appearances without
attaching painting. Explicit empty content arrays with local resources remain
available. The check walks the bounded raw page tree, since the normal page API
materializes inheritance. These guards do not establish general visual fidelity.

forms.flatten also rejects an indirect /AP /N state dictionary. The pinned writer
resolves that reference as a final appearance and can silently regenerate generic
paint instead of using the valid selected /AS stream. Direct state dictionaries
and references to ordinary normal appearance streams remain available within the
existing guards. The positive controls preserve selected custom stream bytes;
they do not establish general form appearance fidelity.
