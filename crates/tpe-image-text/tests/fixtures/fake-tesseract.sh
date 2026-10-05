#!/bin/sh
# Stand-in for the tesseract CLI used by the integration tests: answers
# `--version` and otherwise prints a fixed TSV for whatever image it is given.
# FAKE_TESSERACT_MODE=sleep makes it hang (timeout test); =fail makes it exit 1;
# =limits makes it print the soft address-space limit instead of TSV.
if [ "$1" = "--version" ]; then
  echo "tesseract 5.9.9-fake"
  echo " leptonica-0.0.0"
  exit 0
fi
case "${FAKE_TESSERACT_MODE:-tsv}" in
  sleep) sleep 30; exit 0 ;;
  fail) echo "Error opening data file ./tessdata/xxx.traineddata" >&2; exit 1 ;;
  limits)
    printf 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
    printf '5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t90\tas=%s\n' "$(ulimit -v)"
    printf '5\t1\t1\t1\t1\t2\t10\t0\t10\t10\t90\tcpu=%s\n' "$(ulimit -t)"
    exit 0 ;;
esac
[ -f "$1" ] || { echo "input image missing: $1" >&2; exit 1; }
echo "Warning: Invalid resolution 0 dpi. Using 70 instead." >&2
printf 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
printf '1\t1\t0\t0\t0\t0\t0\t0\t420\t90\t-1\t\n'
printf '5\t1\t1\t1\t1\t1\t90\t30\t80\t30\t96.5\tHello\n'
printf '5\t1\t1\t1\t1\t2\t180\t30\t70\t30\t91.5\tOCR\n'
printf '5\t1\t1\t1\t1\t3\t260\t30\t70\t30\t88\t42\n'
