# Focused correction of #251

Base: `76fda4fd16529ffd91b0ac289364a82b25cdeb4a`, tree
`b4221ceb0c802069852b63832f84520c0927bb65`. This corrective branch does not
authorize the broader formats/audio feature in the original PR.

## Output contract

Without force, **any existing planned output** (JSON, text or preview, including
a dangling symlink) skips the whole set. This intentionally prevents replacing a
lone JSON/text output and prevents a mixed old-preview/new-result set. With force,
input path, symlink and hard-link aliases are rejected, including other CLI batch
inputs and files in input packages. Creating outputs inside a package is rejected
before creating the output directory. Public writer callers must supply all
additional inputs through `write_outputs_with_inputs`; `write_outputs` protects
the result's recorded sources. Path-based extractors retain exact canonical
filesystem paths privately as well as the existing JSON display strings, so
non-UTF-8 directories cannot bypass the check. Deserialized or manually built
results rely on their recorded display paths and the caller-supplied input list.

The private publisher adapts #262's reviewed pattern without importing its crate:
stage and sync every output, move old forced entries into a private recovery
directory, publish complete files using exclusive hard links, and roll back on
handled errors. Rollback removes only owned new files, restores old entries and
retains/reports recovery files if restoration fails. Publication visibility is
not atomic across names; forced outputs can temporarily be absent. This contract
assumes cooperating writers. It is not a crash transaction or a guarantee against
hostile concurrent directory changes. No Windows/WASM runtime claim is made.

## XLSX coordinate contract

Present row attributes must contain decimal digits representing a positive row;
present cell references must contain ASCII letters followed by a positive decimal
row agreeing with the enclosing row. Absent attributes retain inferred placement.
The coordinate envelope is 1,048,576 rows by 16,384 columns. Checked arithmetic
and fallible reservation precede vector indexing/growth.

Dense output has stricter implementation limits: 100,000 row slots/work records
and 1,000,000 cell slots/work records per **workbook**, shared across sheets.
Duplicate and trimmed records do not refund the budget. Exceeding these limits
returns `FormatsError::Invalid`; no complete result is published for that input,
and the CLI continues to later inputs. These are coordinate-expansion limits,
not total memory bounds for ZIP/XML parsing, shared strings or number rendering.
The prior ZIP/Snappy allocation, format coverage and live audio findings remain
queued. Decoder coverage, iWork interpretation and audio engine behavior are
unchanged by this correction.

## Evidence workflow

`Formats preservation evidence` checks out the exact correction head, verifies
the immutable #251 base/tree separately, and uses separate target directories.
It runs the production extractor/writer/CLI regressions, the existing format
fixtures, strict lint and source cleanliness on Linux and macOS. Linux also runs
the same integration test file against unchanged original production code,
expecting 18 regression failures and three passing controls. Four potentially
expensive coordinate tests run only on the corrected reader; there is no
deliberate original OOM reproduction. A unit fault injection exercises the actual
publication routine after one/two links for both new and forced output sets.

Expected counts are assertions in the workflow, not claims of completed tests.
The corrective PR records observed results and remaining platform limitations.
