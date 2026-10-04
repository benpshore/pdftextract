# Frozen corpus Partial classification — 2026-10-04

All 240 backend-paper outcomes from [run 37171610964](https://github.com/benpshore/pdftextract/actions/runs/37171610964) are classified below. This is an offline evidence replay, not new extraction, acceptance, binary attestation or a status change. Tracking: [#190](https://github.com/benpshore/pdftextract/issues/190), [#181](https://github.com/benpshore/pdftextract/issues/181), [#153](https://github.com/benpshore/pdftextract/issues/153).

Source commit `eb5f711b866b188b60af95d4330c9fc34a095a82`, tree `e4af4c57a29c848729ad85ec4433d210a3d1390d`. Artifact `11291642808`, ZIP SHA-256 `4d0631b62d89db40232bcc1b6756c62dcfd6179dc3bbcd6c18fcb15aff55fe16`. [Machine-readable rows](native-partials-2026-10-04.json) retain PDF/source pins, report/provenance/dump hashes, ordered diagnostic hashes, category incidence/pages and first exact diagnostic. Original reports/dumps remain in the artifact.

## Replayed integrity and gate

The connected GitHub artifact download returned the existing ZIP. Its bytes match the published digest. For every backend, recorded commit and five source/input hashes match the clean frozen checkout using `source_provenance` and `verify_current_provenance` in `native/validate_eval.py`. All 60 PDF/source pin tuples match the unchanged dev manifest. `validate_dumps` passes for all 240 rows; historical policy loading retains its six page/reference baselines and grants zero exceptions. Offline `validate` is explicitly bound to the recorded `linux aarch64` host rather than the x64 analysis machine. Its only errors are the 209 recorded Partials below. No gate or historical policy was changed.

| Backend | Complete | Partial / strict errors | Truth / extracted / matched references |
|---|---:|---:|---:|
| lopdf | 13 | 47 | 3830 / 3830 / 3829 |
| pdfium | 10 | 50 | 3830 / 3743 / 3741 |
| docling-text | 8 | 52 | 3830 / 3635 / 3618 |
| docling | 0 | 60 | 3830 / 3790 / 3757 |

Every backend retains 60 papers and 1,747 pages. There are zero zero-match papers. Matching measures the existing evaluator's reference identity/coverage; it does not establish correct fields, body transcription, geometry, or completeness. Complete rows also remain in the table and JSON.

## Diagnostic families

- **U**: unmapped source codes, 45 papers in lopdf and both Docling modes.
- **D**: undecodable font TT1, `2305.13843`, four diagnostics on page 2.
- **C**: optional CFF helper disabled, `2501.17300`, 32 diagnostics across 22 pages. This paper also has U diagnostics; enabling a feature cannot by itself clear its Partial status.
- **F**: unverified fallback encoding, `2510.26824`, 24 diagnostics on all 12 pages.
- **M**: PDFium mapping uncertainty, 49 papers with native mapping flags and one with unresolved mapping summary.
- **S**: Docling Text scalar coverage disagreement, all 52 Partials; 47 overlap native mapping-related Partials. The five without native mapping warnings are `2511.13979`, `2505.23862`, `2506.08311`, `2511.15503`, and `2602.01390`.
- **R**: shorter Docling Text candidate rejected, 44 papers. Retained native text does not resolve the scalar disagreement.
- **V**: full Docling reconstruction coverage unverified, every paper and every one of the 1,747 pages.
- **Q**: omitted formula regions, 48 full Docling papers.
- **G**: full Docling annotation geometry unverified, `2410.17124`, pages 37–51; URI targets remain without inferred rectangles.

Categories overlap. Counts use page diagnostics once; their repeated document summaries are hashed separately. No resource-limit or page-extraction failure occurs in this artifact, and no Partial lacks a classified diagnostic. Diagnostic incidence establishes neither recoverability nor independent visible-text truth.

## All per-paper outcomes

Cells show status, diagnostic families, and matched/truth references. `Complete` records the actual report status; `P` means Partial.

| Paper | lopdf | PDFium | Docling Text | Full Docling |
|---|---|---|---|---|
| 2108.04588 | P U 31/31 | P M 31/31 | P R/S/U 31/31 | P Q/U/V 31/31 |
| 2502.00857 | Complete  29/29 | P M 29/29 | Complete  29/29 | P V 28/29 |
| 2412.06210 | P U 44/44 | P M 44/44 | P R/S/U 44/44 | P U/V 44/44 |
| 2505.16990 | P U 56/56 | P M 56/56 | P R/S/U 56/56 | P Q/U/V 53/56 |
| 2509.10402 | Complete  61/61 | P M 61/61 | Complete  61/61 | P V 61/61 |
| 2603.21379 | P U 35/35 | P M 35/35 | P R/S/U 35/35 | P Q/U/V 35/35 |
| 2603.04447 | P U 23/23 | P M 23/23 | P R/S/U 23/23 | P Q/U/V 23/23 |
| 2306.11313 | P U 63/63 | P M 63/63 | P R/S/U 63/63 | P Q/U/V 63/63 |
| 2503.15734 | P U 33/33 | P M 33/33 | P R/S/U 33/33 | P Q/U/V 33/33 |
| 2505.01811 | P U 33/33 | P M 33/33 | P S/U 33/33 | P Q/U/V 33/33 |
| 2604.03540 | P U 44/44 | P M 44/44 | P R/S/U 44/44 | P Q/U/V 43/44 |
| 2511.13979 | Complete  59/59 | Complete  59/59 | P R/S 59/59 | P Q/V 54/59 |
| 2305.13843 | P D 198/198 | P M 198/198 | P D/R/S 198/198 | P D/Q/V 198/198 |
| 2507.08599 | P U 16/16 | P M 16/16 | P R/S/U 16/16 | P Q/U/V 13/16 |
| 2504.09409 | P U 58/58 | P M 58/58 | P R/S/U 58/58 | P Q/U/V 58/58 |
| 2608.28714 | P U 299/299 | P M 299/299 | P R/S/U 299/299 | P Q/U/V 298/299 |
| 2608.03351 | P U 22/22 | P M 22/22 | P R/S/U 22/22 | P Q/U/V 22/22 |
| 2509.08395 | P U 46/46 | Complete  46/46 | P S/U 46/46 | P Q/U/V 45/46 |
| 2601.13206 | P U 24/24 | P M 24/24 | P S/U 24/24 | P Q/U/V 24/24 |
| 2501.17300 | P C 54/54 | P M 54/54 | P C/S 54/54 | P C/Q/V 54/54 |
| 2309.10334 | P U 36/36 | P M 36/36 | P R/S/U 36/36 | P Q/U/V 36/36 |
| 2401.15719 | P U 54/54 | P M 54/54 | P R/S/U 54/54 | P Q/U/V 52/54 |
| 2406.19204 | Complete  59/59 | P M 59/59 | Complete  59/59 | P Q/V 57/59 |
| 2410.17124 | P U 94/94 | Complete  94/94 | P R/S/U 94/94 | P G/U/V 94/94 |
| 2410.19245 | P U 53/53 | P M 53/53 | P R/S/U 53/53 | P Q/U/V 53/53 |
| 2412.11061 | P U 26/27 | Complete  27/27 | P S/U 27/27 | P Q/U/V 27/27 |
| 2503.00030 | P U 70/70 | P M 70/70 | P R/S/U 70/70 | P Q/U/V 70/70 |
| 2503.04404 | P U 56/56 | P M 56/56 | P R/S/U 56/56 | P U/V 56/56 |
| 2503.13415 | P U 341/341 | P M 341/341 | P R/S/U 341/341 | P U/V 341/341 |
| 2504.10389 | P U 40/40 | P M 40/40 | P R/S/U 40/40 | P Q/U/V 40/40 |
| 2505.22973 | P U 117/117 | P M 117/117 | P R/S/U 3/117 | P Q/U/V 94/117 |
| 2505.23862 | Complete  45/45 | Complete  45/45 | P R/S 45/45 | P Q/V 45/45 |
| 2506.03828 | P U 60/60 | P M 60/60 | P R/S/U 59/60 | P Q/U/V 60/60 |
| 2506.08311 | Complete  62/62 | Complete  62/62 | P R/S 62/62 | P V 62/62 |
| 2506.23487 | P U 28/28 | P M 28/28 | P R/S/U 28/28 | P Q/U/V 24/28 |
| 2507.14211 | P U 54/54 | P M 54/54 | P S/U 54/54 | P Q/U/V 54/54 |
| 2507.14212 | P U 35/35 | P M 35/35 | P R/S/U 35/35 | P Q/U/V 35/35 |
| 2508.02208 | P U 41/41 | P M 41/41 | P R/S/U 41/41 | P Q/U/V 41/41 |
| 2508.19485 | P U 56/56 | P M 56/56 | P R/S/U 56/56 | P Q/U/V 55/56 |
| 2509.04183 | P U 59/59 | P M 59/59 | P S/U 59/59 | P Q/U/V 55/59 |
| 2509.12458 | Complete  44/44 | Complete  44/44 | Complete  44/44 | P Q/V 40/44 |
| 2509.17930 | Complete  33/33 | P M 33/33 | Complete  33/33 | P Q/V 33/33 |
| 2509.24852 | P U 65/65 | P M 65/65 | P R/S/U 65/65 | P Q/U/V 65/65 |
| 2510.07065 | P U 20/20 | P M 20/20 | P R/S/U 20/20 | P Q/U/V 20/20 |
| 2510.26824 | P F/U 181/181 | P M 92/181 | P F/R/S/U 90/181 | P F/U/V 181/181 |
| 2511.15503 | Complete  139/139 | P M 139/139 | P R/S 139/139 | P V 139/139 |
| 2511.22707 | P U 43/43 | P M 43/43 | P R/S/U 43/43 | P Q/U/V 38/43 |
| 2512.10223 | P U 31/31 | P M 31/31 | P R/S/U 31/31 | P Q/U/V 31/31 |
| 2601.09974 | P U 35/35 | P M 35/35 | P R/S/U 35/35 | P Q/U/V 35/35 |
| 2601.12491 | Complete  64/64 | P M 64/64 | Complete  64/64 | P V 61/64 |
| 2602.00685 | P U 59/59 | P M 59/59 | P R/S/U 59/59 | P Q/U/V 59/59 |
| 2602.01390 | Complete  70/70 | Complete  70/70 | P S 70/70 | P V 70/70 |
| 2602.02748 | Complete  40/40 | Complete  40/40 | Complete  40/40 | P Q/V 39/40 |
| 2602.16061 | P U 54/54 | P M 54/54 | P R/S/U 54/54 | P Q/U/V 54/54 |
| 2602.17690 | P U 57/57 | P M 57/57 | P R/S/U 57/57 | P Q/U/V 57/57 |
| 2603.03010 | P U 51/51 | P M 51/51 | P R/S/U 51/51 | P Q/U/V 51/51 |
| 2603.04445 | Complete  113/113 | Complete  113/113 | Complete  107/113 | P V 104/113 |
| 2603.05575 | P U 34/34 | P M 34/34 | P R/S/U 34/34 | P Q/U/V 34/34 |
| 2603.12824 | P U 39/39 | P M 39/39 | P R/S/U 39/39 | P Q/U/V 39/39 |
| 2603.19305 | P U 43/43 | P M 43/43 | P R/S/U 43/43 | P Q/U/V 43/43 |

## Focused next experiment and release boundary

The smallest source-backed candidate is an A/B examination of the already implemented optional CFF helper on `2501.17300`, using the same pinned PDF and frozen source. First retain the selected font objects/program hashes and verify the helper's restricted applicability; then compare feature-disabled and `pdf-extract` builds while keeping raw codes, geometry, all mapping warnings and source pins. Measure collateral text/reference changes and retain Partial wherever source codes remain unresolved. No recovery or status improvement is claimed here. This avoids a speculative broad decoder repair based only on warning counts.

The source-region experiment handed over under #196 remains separately based on PR208 (`cd95a8b…`) and is not applied to these corpus rows. Its source CMaps and embedded font cmap share PDF-author control; concordance does not independently verify visible glyph meaning. Its held-out labels remain sealed pending policy freeze and harness qualification.

Existing exact-source hosted evidence already includes successful optimized `docling,pdfium` compilation and all four runtime evaluations. The artifact contains reports and dumps, not a portable executable. The stopped local x64 optimized cache/binary and runtime-test logs are absent from this replacement environment. Consequently no fresh local production release-mode containment/startup result is claimed. No completed build or rotation CI was rerun.

Rotation PR209 run37172901050 is completed/success at `78ff9429f9e0231562396e58b3a51bd81e7fdbd1`; required `ci` and all six jobs passed. OCR/GROBID runtime evidence remains in PR206/209 `docs/CPU_RUNTIME_HANDOFF.md`. All OCR remains inferred/Partial and absent GROBID geometry remains unavailable.

Local report verification: 240 dump checks and four current-provenance/pin checks passed; strict replay produced exactly 47/50/52/60 status errors; 210 Python tests, Ruff and Rust formatting passed. Local OSV audit is network-tunnel blocked; Swift/CMake/CTest are unavailable. Rust compilation checks on the unchanged production source require packages absent from the offline replacement cache; existing exact-head hosted results remain separate evidence. Formal security scanning is explicitly deferred, with no security clearance claim. The corpus gate remains unmet; no merge, release or deployment occurred.
