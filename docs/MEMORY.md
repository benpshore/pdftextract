# Memory model evidence

Measured 2026-10-08 at `1bc92cc` on x86_64 Linux (rustc 1.97.0, release
profile, the default `lopdf` backend). Evidence labels follow
[EXTRACTION_ROUTING_REVIEW.md](EXTRACTION_ROUTING_REVIEW.md): **M** measured
here, **C** read in code, **U** upstream statement, **P** inference. Nothing
below is a wasm measurement; no wasm32 target was available on the host.

## Invariants the engine must be designed around

- **C:** core WebAssembly has `memory.grow` and no instruction that returns
  pages to the host. Rust `dealloc` inside a wasm instance returns memory to
  the allocator for reuse within the instance; the instance's footprint is its
  high-water mark until the instance is destroyed. The `memory-control`
  proposal (`memory.discard`) is listed as unimplemented on Wasmtime's
  proposal-status page; no evidence of a shipped browser implementation.
- **C:** on stable Rust an allocation failure is fatal. `alloc::alloc::
  handle_alloc_error` aborts (its doc comment); `std::alloc::set_alloc_error_hook`
  is `#[unstable(feature = "alloc_error_hook")]`; the wasm32 targets use
  `panic = "abort"` (`rustc_target/src/spec/base/wasm.rs`). An instance that
  attempts one allocation past its maximum traps. Infallible collections
  (`Vec::push`, `String::push_str`, `BTreeMap::insert`) cannot be made to fail
  gracefully from outside; only `try_reserve` paths and pre-allocation checks
  can.
- **C:** `lopdf` parses every xref entry into `Document::objects` at load
  (`Reader::read` → `load_objects_raw`, lopdf 0.44/0.45 `src/reader.rs`).
  Memory retained after open is a function of the whole file, not of the
  pages requested.
- **C:** `hayro-syntax` 0.8 resolves objects by xref offset on demand over the
  input slice, keeps no object cache, and does not cache decoded streams
  (`Stream::decoded` doc comment). `Page::page_stream` *does* cache the
  decoded content stream in a `OnceLock` for the life of the `Page`; reading
  `Contents` through `Page::raw()` avoids that. With default features its
  `unsafe` surface is one `transmute` in `src/page.rs` plus `smallvec`; the
  `unsafe` feature (flate2, memchr, SIMD) is off by default.
- **C:** `web/public/pdf-worker.js` creates one Worker per document, calls
  `doc.free()` and `self.close()`. The lifecycle is already instance-per-document.

## Whole-process peak RSS, native `tpe extract` (**M**)

`scripts/measure_rss.py` runs `tpe extract FILE --db … --json --jobs 1` and
reports `getrusage(RUSAGE_CHILDREN).ru_maxrss` after the child exits, so the
number covers the controller and its disposable worker. Fixtures are public
files from the pdf.js test suite (not committed; SHA-256 in the table).

| file | bytes | pages | peak RSS | wall | status |
| --- | ---: | ---: | ---: | ---: | --- |
| tracemonkey.pdf `3662ff51…` | 1,016,315 | 14 | 32.4 MB | 0.23 s | partial |
| TAMReview.pdf `a6db6d2f…` | 674,629 | 23 | 26.1 MB | 0.19 s | partial |
| issue1905.pdf `a75f3220…` | 918,339 | 1 | 19.3 MB | 0.08 s | complete |
| issue5549.pdf `f12be657…` | 830,412 | 1 | 11.2 MB | 0.04 s | partial |
| cmykjpeg.pdf `659d6b19…` | 374,080 | 1 | 11.1 MB | 0.04 s | complete |
| issue15367.pdf `8d8f4827…` | 42,112 | 3 | 15.5 MB | 0.07 s | complete |
| alphatrans.pdf `234c0ab8…` | 16,910 | 1 | 16.1 MB | 0.06 s | complete |

Process baseline on the smallest inputs is about 11 MB. Peak grows with file
size, not page count. These are controller+worker numbers on a hosted x86_64
container; they say nothing about the M1 target or about a wasm build, and
they do not establish the 256 MiB bound (see NATIVE_EVIDENCE.md).

## Heap retained by the object source alone (**M**)

`scripts/memproto` is a standalone measurement crate (excluded from the
workspace; it contains a counting `GlobalAlloc`, which is `unsafe` and is
not product code). It opens each file with `lopdf` 0.45 (no default features)
and with `hayro-syntax` 0.8 (default features), then for every page decodes
the content stream and every embedded font program (`FontFile`, `FontFile2`,
`FontFile3`, following `DescendantFonts`), and records heap use from the
counting allocator. The input `Vec<u8>` is allocated before measurement and is
excluded from every column. Values in MiB.

| mode | file | pages | peak during open | retained after open | largest per-page rise | retained after all pages |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| lopdf | tracemonkey.pdf | 14 | 3.11 | 2.97 | 0.12 | 2.97 |
| hayro (uncached) | tracemonkey.pdf | 14 | 0.08 | 0.08 | 0.11 | 0.08 |
| hayro (`page_stream` cache) | tracemonkey.pdf | 14 | 0.08 | 0.08 | 0.13 | 0.28 |
| lopdf | TAMReview.pdf | 23 | 1.79 | 1.73 | 0.16 | 1.73 |
| hayro (uncached) | TAMReview.pdf | 23 | 0.07 | 0.07 | 0.11 | 0.07 |
| hayro (`page_stream` cache) | TAMReview.pdf | 23 | 0.07 | 0.07 | 0.13 | 0.10 |
| lopdf | issue1905.pdf | 1 | 1.05 | 1.03 | 0.21 | 1.03 |
| hayro (uncached) | issue1905.pdf | 1 | 0.01 | 0.01 | 0.13 | 0.01 |
| lopdf | issue5549.pdf | 1 | 0.82 | 0.82 | 0.00 | 0.82 |
| hayro (uncached) | issue5549.pdf | 1 | 0.01 | 0.00 | 0.00 | 0.00 |
| lopdf | cmykjpeg.pdf | 1 | 0.42 | 0.41 | 0.07 | 0.41 |
| hayro (uncached) | cmykjpeg.pdf | 1 | 0.01 | 0.01 | 0.00 | 0.01 |
| lopdf | issue15367.pdf | 3 | 0.14 | 0.13 | 0.24 | 0.13 |
| hayro (uncached) | issue15367.pdf | 3 | 0.01 | 0.01 | 0.22 | 0.01 |

Decoded embedded font bytes were identical between the two object sources on
every file (0.88 MiB tracemonkey, 0.21 MiB TAMReview, 0.27 MiB issue15367),
so both read the same font programs.

What the two tables say together: on these files the object table that
`lopdf` retains is 1–3× the file size and is page-independent; the on-demand
source retains tens of kilobytes after open and returns to that level after
every page. The whole-process RSS (first table) is an order of magnitude above
either, so most of the engine's footprint is outside the object source
(worker baseline, spans, ledger); that remainder has not been attributed yet.

## Sustained extraction in one process (**M**)

`scripts/sustain` runs the full pipeline (`tpe::pipeline::run_job`, `lopdf`
backend, after `warm_up()`) or one object source over a file list for N
passes in a single process under the counting allocator, and prints heap
retained after every document and the running peak. Ten pdf.js files
(the seven above plus issue2074, issue4650, issue7891_bc1), 20 passes,
200 extractions per mode:

| mode | retained after pass 1 | retained after pass 20 | delta per pass | peak above baseline |
| --- | ---: | ---: | ---: | ---: |
| full pipeline, lopdf backend | 7.496 MiB | 7.496 MiB | 0.0000 MiB | 20.46 MiB |
| lopdf object source only | 0.019 MiB | 0.019 MiB | 0.0000 MiB | 3.21 MiB |
| hayro-syntax object source only | 0.001 MiB | 0.001 MiB | 0.0000 MiB | 1.12 MiB |

The pipeline's retained heap rises to 7.5 MiB during the first pass (one-time
state after warm-up; 7.28 MiB after the first document, 7.50 after the ninth)
and is then flat to the byte for the remaining 190 extractions. The counting
allocator records bytes requested, not allocator fragmentation, so these are
lower bounds for what a wasm instance's linear memory would hold.

## What this does and does not establish

Established (**M**): on these inputs, natively, the engine does not retain
memory across documents, and its heap high-water mark is about 20 MiB above
process baseline. Not established: any browser bound. A wasm instance's
linear memory is grow-only, its allocator fragments differently, and its
reclaim depends on the browser; none of that is measured here. The
measurement that establishes a browser bound is linear-memory size under
sustained extraction in Safari (macOS and iOS) for both lifecycles
(one instance reused; one instance per document), which needs a wasm32 build
on the target and is the next step, not a consequence of this document.

## Consequences (**P**, hypotheses for the Safari measurement to confirm or reject)

1. A fixed-footprint browser build needs an on-demand object source. With
   the eager source, the retained set is file-size-bound before the first
   page is read; with the on-demand source it is the input bytes plus the
   current page.
2. Budget checks must precede allocation. Because an allocation past the
   instance maximum aborts the instance, the existing charge accounting has to
   gate work before the allocation is attempted, and the host must already
   hold every per-page result the instance has emitted, so that an abort
   loses one page rather than a document.
3. The per-page working set for content and font decoding is a few hundred
   kilobytes on these files. The engine's own per-page structures (spans,
   geometry, ordering) are not included in that number and need the same
   measurement before a budget is set.
4. Any page-level cache that lives as long as the document (hayro's
   `page_stream`, or an equivalent in the engine) turns per-page cost into
   per-document cost and must not be used in the fixed-footprint build.

## Not measured

- Any wasm32 build (no target available on the measurement host), so no linear-memory growth, allocator fragmentation, or browser reclaim behaviour.
- Image-heavy files, files above 1 MB, and scanned documents.
- Peak heap of the full engine per page (only the object source was isolated).
- Browser reclaim timing after `Worker.terminate()` / `self.close()`.
- The M1 host; see NATIVE_EVIDENCE.md for why hosted numbers do not transfer.

## Reproduce

```sh
cargo build --release --bin tpe
uv run python scripts/measure_rss.py target/release/tpe FILE.pdf [FILE.pdf …]
cd scripts/memproto && cargo build --release
./target/release/memproto lopdf FILE.pdf
./target/release/memproto hayro FILE.pdf
./target/release/memproto hayro-cached FILE.pdf
cd ../sustain && cargo build --release
./target/release/sustain tpe 20 FILE.pdf [FILE.pdf …]
./target/release/sustain lopdf 20 FILE.pdf [FILE.pdf …]
./target/release/sustain hayro 20 FILE.pdf [FILE.pdf …]
```

Fixture sources: `https://raw.githubusercontent.com/mozilla/pdf.js/master/test/pdfs/<name>`
(pdf.js test corpus; check each file's licence before adding it to a manifest).
