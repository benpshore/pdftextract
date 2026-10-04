# Native bibliography correctness handoff

Source freeze: `eb5f711b866b188b60af95d4330c9fc34a095a82`, tree `e4af4c57a29c848729ad85ec4433d210a3d1390d`.

## Published draft stack

| PR | Exact head | Purpose |
|---|---|---|
| [191](https://github.com/benpshore/pdftextract/pull/191) | e9e41ceb308356c98c810f489cd4a0f1dfd11da9 | Integrate preserved native repair stack with PR175 |
| [194](https://github.com/benpshore/pdftextract/pull/194) | 0ece13395f8372a6e9b32aa93f16b612a850b0fe | Capture fixed nine-case raw geometry and scoring evidence |
| [203](https://github.com/benpshore/pdftextract/pull/203) | 5cd375e6b2c46792cd5f70cf9ab58e990ebbe5d0 | Keep two aligned column headings with their respective text |
| [204](https://github.com/benpshore/pdftextract/pull/204) | 490e39f9320c1b9e2766f136c27f0dccf9ca78e0 | Repair qualified split Docling bibliography opening |
| [205](https://github.com/benpshore/pdftextract/pull/205) | eb5f711b866b188b60af95d4330c9fc34a095a82 | Separate current source provenance from historical Partial eligibility |

Each PR targets the previous branch; PR191 targets PR175. All are draft and bug-labeled. Tracking [190](https://github.com/benpshore/pdftextract/issues/190), engine epic [181](https://github.com/benpshore/pdftextract/issues/181), release verification [153](https://github.com/benpshore/pdftextract/issues/153). Original 171/172/174 branches remain intact. Web branches remain separately owned.

## Reproduced and repaired cases

The original eight failures all existed on PR174. On intended PR175/191/194, six Docling Text failures were already fixed: 2503.15734=33/33 matches,2309.10334=36/36,2503.13415=341/341,2506.23487=28/28,2507.14211=54/54,2602.16061=54/54. All remain Partial. This work does not claim to implement those six prior fixes.

Two original failures remained, plus an additional current-tree Docling Text failure:

| Backend/input | Truth | Before extracted/matched | Repair |
|---|---:|---:|---|
| PDFium 2502.00857v2 |29|4/0|29/29 confirmed in fresh hosted203 extraction; Partial retained |
| Full Docling 2309.10334 |36|1/0|36/36 confirmed in fresh hosted204 extraction; Partial retained |
| Docling Text 2502.00857v2 |29|4/0|29/29 confirmed in fresh hosted203 extraction; Complete retained |

Raw spans show column headings detached as page furniture in 2502. The shared fix is geometric, with running-head/folio negatives. Full Docling's upstream column order interleaves appendix prose inside its first reference; the adapter uses typed list identity, upright geometry, a clear bottom-band gap, bibliographic signatures and consecutive next-page entries. Ambiguous layouts remain unchanged. No citation regex broadening, OCR hypothesis, source-span rewriting or truth relabeling.

Focused denominator: original 8 backend/input cases plus 1 current case, across 7 pinned PDFs. Full corpus: 60 unique papers and 1,747 pages per backend. Matching is reference coverage/identity under the existing evaluator, not perfect field-level accuracy.

## Provenance correction

Exact combined-tree failures191/run 37169841972 and194/run 37170137003 correctly record current lock 00ff969d; historical policy correctly pins ae0367f2. Other four pins match. PR175 adds 73 package versions and changes PDFium-render's libloading edge 0.8.9→0.9.0. All six historical diagnostic signatures also changed; none qualifies for its old exception.

PR205 checks recorded source commit and five input hashes against a clean source checkout, then separately evaluates historical eligibility. A verified different lock grants zero historical Partial exceptions. Original policy bytes/pins, same-input page/reference baselines and all real status/count/dump failures remain enforced. It does not attest executable binaries/features. Actual archived191 revalidation still fails all four backends with 47/50/52/60 Partial rows and three zero-match rows visible.

## Checks and review

Final local checks: 835 Rust tests passed/8 ignored; 210 Python tests passed; strict Clippy, Ruff and formatting passed. Native geometry integrations and seven Docling helper tests passed. Local audit network access and Swift/CMake availability were blocked. Final 205 hosted Linux x64, Linux ARM64 and macOS ARM64 repair jobs all passed, including Python 210 and the audit, strict Rust/Clippy/public regressions, and macOS Swift plus CTest 2/2. Exact-head artifacts were independently hash/source/input verified; see [platform verification](https://github.com/benpshore/pdftextract/pull/205#issuecomment-5975904158).

Independent review comments: [203](https://github.com/benpshore/pdftextract/pull/203#issuecomment-5975783492), [204](https://github.com/benpshore/pdftextract/pull/204#issuecomment-5975807306), [205](https://github.com/benpshore/pdftextract/pull/205#issuecomment-5975846358). The reviewer found a malformed historical-lock validation gap; it was fixed and tested before publication.

Fresh 203 diagnostic artifact 11291123545, SHA256 `f2359a51755b7206e3f9b9f68c012a8b00a65622aaffa12a487e51215fdc01fa`, verifies both 2502 fixes and all 546 control matches. Only its not-yet-stacked full Docling case remains zero. Fresh combined 204 diagnostic run 37171317217 is successful: all 9 cases observed, 0 execution failures, 0 zero-match cases, all 640 backend-case reference matches. Artifact 11291592897 SHA256 `2b71cbc00139f713ef8da834cfcb183f52c2b41824b9f93200cf8fc927f4126d` matches exact 490e39f/source tree and all input pins. All 8 Partial case statuses and 1 Complete remain unchanged. Combined 204 full corpus run 37171317246 is verified: all four backends retain 60 papers / 1,747 pages / 3,830 truth references, with zero zero-match papers. Extracted/matched counts are lopdf 3,830/3,829; PDFium 3,743/3,741; Docling Text 3,635/3,618; full Docling 3,790/3,757. Partial counts remain 47/50/52/60. Only the three intended bibliography cases changed against 191, plus one matched lopdf body word; no reference/status regressions or other non-timing differences. Artifact 11291397802 SHA256 `92529735d427397f8b65bf8cfb9e321b5d93bdc867937657af3b1e29571a6014` is source/input verified. Its old validator remains red on the historical lock classifier. Final 205 full-corpus run 37171610964 is terminal and verified. Its build/Clippy/tests and all four evaluations pass; validation fails only 209 genuine Partial outcomes. The historical lock mismatch is gone, zero exceptions apply, and every non-timing per-paper result exactly equals 204 across 240 rows. Artifact 11291642808 SHA256 `4d0631b62d89db40232bcc1b6756c62dcfd6179dc3bbcd6c18fcb15aff55fe16` matches exact eb5f711/tree e4af4c5, all source hashes and paper pins. CI and all three repair platforms passed. The live tracker is `hosted-repair-tracking.json`.

## Broader requirements and release boundary

The independently audited scholarly-route plan is [recorded under 195](https://github.com/benpshore/pdftextract/issues/195#issuecomment-5975721771), with 181/182 links. Metadata-led identity/naming, real GROBID deployment and owner-bound service integration remain explicit future work. Current 175 fallback is whole-pass; region-level fusion is a proposed contract, not implemented or qualified behavior. PDFium/FFI/security architecture remains separately owned. Oxide defects are scoped under 193; the settled comparison was not repeated.

Conditional merge/public-release authorization exists. The full-corpus gate and separately blocked formal security clearance still block release; no main merge, release, Site deployment or production qualification occurred. No corpus policy waiver or historical repin was used. All original Partial statuses and denominators remain visible.
