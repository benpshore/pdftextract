"""Count words in the engine's own page text that Poppler never printed for that page.

A word is a run of four or more ASCII letters after NFKC. A word in the
engine's `pages[].text` counts as unknown when it does not occur, ignoring
case, in `pdftotext` (default mode) output for the same page, with or without
Poppler's end-of-line hyphens joined.

This is a floor on garbling, not an error rate: it cannot see wrong order,
and it also counts harmless differences in how the two tools join hyphens.

Usage: word_check.py CORPUS_JSON PDF_DIR TPE_JSON_DIR > word-check.json
"""

import json
import re
import sys
from collections import Counter
from pathlib import Path

from common import engine_doc, load_corpus, norm, run, split_pages

WORD_RE = re.compile(r"[A-Za-z]{4,}")


def words(text: str) -> list[str]:
    return WORD_RE.findall(norm(text))


def main() -> None:
    corpus, pdf_dir, json_dir = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    report = {}
    for entry in load_corpus(corpus):
        name = entry["id"]
        doc = engine_doc(json_dir, entry["sha256"])
        poppler = split_pages(run(["pdftotext", str(pdf_dir / f"{name}.pdf"), "-"]))
        if len(poppler) != len(doc["pages"]):
            raise SystemExit(f"{name}: page count differs between the engine and Poppler")
        unknown: Counter = Counter()
        total = 0
        for page, poppler_page in zip(doc["pages"], poppler, strict=True):
            vocabulary = {word.lower() for word in words(poppler_page)}
            vocabulary |= {word.lower() for word in words(poppler_page.replace("-\n", ""))}
            for word in words(page["text"]):
                total += 1
                if word.lower() not in vocabulary:
                    unknown[word] += 1
        report[name] = {
            "engine_status": doc.get("status"),
            "engine_words": total,
            "unknown_words": sum(unknown.values()),
            "most_common_unknown": [word for word, _ in unknown.most_common(12)],
        }
    json.dump(report, sys.stdout, indent=1, sort_keys=True)
    print()


if __name__ == "__main__":
    main()
