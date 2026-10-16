# Native operation API and worker

This checkpoint is experimental. Read [CHECKPOINT.md](CHECKPOINT.md), the source
matrix, and exact run evidence before treating a capability as qualified.

The Rust entry point is `tpe::oxide::execute(operation, &Context, &params)` under
`pdf-oxide`. Context contains immutable source bytes and verified hashes, optional
one-based inclusive page selection, a password, and finite limits. Parsing hostile
input in an ordinary application process is not supported: use the supervised CLI.

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
are read once with size/change checks before the worker seals filesystem access.

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
bytes. With `--out`, artifacts are published without clobbering existing files;
the receipt retains each artifact's name, size and SHA-256 while its inline byte
array is emptied. A publisher failure can report `publication_unknown`; inspect
existing outputs before retrying. Originals are never used as publication paths.

`capabilities` returns dispatch names, compiled features and known unsupported
edges. It is not an assertion of accuracy or that every possible input works.
Family references: [query](query.md), [creation/Office](create-office.md),
[editing/forms](edit-forms.md), [security](security.md), and
[rendering/OCR](render-ocr.md). Additional operations are `convert.text`,
`convert.markdown` and `convert.html`; table reconstruction is disabled, and HTML
keeps raw markup separate from an attribute-free sanitized display projection.

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
