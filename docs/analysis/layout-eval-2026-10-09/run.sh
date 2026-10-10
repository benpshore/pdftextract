#!/bin/sh
# Reproduce the 2026-10-09 layout evaluation.
# usage: run.sh TPE_BINARY PDF_DIR WORK_DIR
#   PDF_DIR holds <id>.pdf for every id in corpus.json (bytes matching its sha256).
#   WORK_DIR receives the engine output, the prototype renderings and the result files.
# Needs: pdftotext (Poppler), qpdf 11, python3 >= 3.10. No Python packages.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tpe=$1
pdfs=$2
work=$3
mkdir -p "$work/tpe" "$work/render"
for pdf in "$pdfs"/*.pdf; do
    # `tpe extract` exits 1 for a document whose status is partial; that is a result, not a failure.
    "$tpe" extract "$pdf" --db "$work/ledger.sqlite" --out "$work/tpe" >>"$work/extract.log" 2>&1 || true
done
python3 "$here/layout_proto.py" "$here/corpus.json" "$pdfs" "$work/tpe" "$work/render" >"$work/layout.json"
python3 "$here/interleave_check.py" "$here/corpus.json" "$pdfs" "$work/tpe" >"$work/interleave.json"
python3 "$here/word_check.py" "$here/corpus.json" "$pdfs" "$work/tpe" >"$work/word-check.json"
python3 "$here/font_widths.py" "$here/corpus.json" "$pdfs" >"$work/font-widths.json"
