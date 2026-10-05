# Native dependency compatibility

PR #235 was checked at `dcdd469753899937698e85d11dd5a8d0f405e6fe`
on October 5, 2026. Retain the four independent dependency updates; defer
PDFium 0.9.4 and Office Oxide 0.1.12 rather than expanding the FFI boundary.

## PDFium 0.9.4

The published crate (SHA-256
`8948a803616a9e936b15a6637af2cd48c5fb8ae0fcdeb3c32eac3a540e255a19`,
source commit `6cee8b9a3951832ac0ff62ce4c32800278001cb8`) declares
`PdfiumLibraryBindingsAccessor` **pub(crate)** in `src/pdfium.rs`, despite
the README claiming public access since 0.9.2. Importing that trait cannot
restore `Pdfium::bindings()` for consumers. The text wrapper still lacks
the Unicode-map-error query required by our completeness contract.

It also retains the first library binding in a process-global `OnceCell`,
rejects subsequent `bind_to_library` calls, and locks per FPDF call. Docling's
0.8.37 dependency initializes/destroys the same native library with a separate
mutex. A future migration must provide supported raw access, preserve trusted
library identity, and resolve the shared-library lifecycle and synchronization
before mixing these versions. Changing the page index or adding unsafe blocks
alone does not meet those requirements. Keep the existing mapping document's
borrowed bytes, null checks, page bounds, RAII close order, and Partial evidence.

## Office Oxide 0.1.12

The published crate (SHA-256
`5a94198217c7e922d348097c044a8f8ad6b6d27d789a65de0ebb859617e23f35`)
adds `DocumentIR.defined_names`. With `pdf_oxide` 0.3.78, a locked
`cargo check --features pdf-oxide` fails with E0063 at
`pdf_oxide/src/converters/pdf_to_ir.rs:194` (`DocumentIR { metadata, sections }`).
Keep 0.1.9 until the upstream constructor is compatible.

Native repair checks now compile and test `pdf-oxide`, `mupdf`, `poppler`, and
`grobid` alongside `pdfium` on Linux x64, Linux ARM64, and macOS ARM64. The
licensed-provider runtime tests remain explicitly ignored without their
external libraries. This exercises the optional dependency APIs too.
The expanded Linux ARM64 check exposed an existing signed-`c_char` assumption
in provider identity/error strings. Decode the native one-byte representation
on either signedness; the regression checks UTF-8, NUL termination, and invalid
bytes without changing pointer bounds or library ownership.
GROBID remains non-resolving with `allow_dtd: false`;
no changed path connects these pins to #215 or #216.
