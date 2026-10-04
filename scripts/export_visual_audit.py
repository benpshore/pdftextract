"""Export only existing development diagnostics; Partial still fails the lane.

The CLI intentionally exits 1 after publishing a Partial result. Diagnostic
export may preserve that evidence, but must not turn it into Complete or accept
a failed/cancelled/missing publication. No held-out input is eligible here.
"""

import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path

EXPORT_IDS = ("arxiv:2309.10334", "arxiv:2401.15719", "arxiv:2509.12458")


def export_paper(pdf: Path, tpe: Path, extracted: Path, output: Path, stem: str) -> dict:
    """Run one bounded extraction, validate its committed outputs, then copy.

    The structured stdout includes the publisher's actual no-clobber paths. Use
    those paths instead of guessing a hash filename or selecting an old output.
    The disk JSON and page text must agree with that exact publication before
    diagnostic bytes are copied. These checks are integrity/coverage evidence,
    not verification that the extracted text is correct or complete.
    """
    extracted.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    process = subprocess.run(  # noqa: S603 - explicit local executable/argv, no shell
        [
            str(tpe.resolve()),
            "extract",
            str(pdf.resolve()),
            "--db",
            str(extracted / "audit.sqlite"),
            "--out",
            str(extracted),
            "--json",
        ],
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    result = json.loads(process.stdout)
    status = result.get("status")
    expected_exit = {"complete": 0, "partial": 1}.get(status)
    if expected_exit is None or process.returncode != expected_exit:
        raise ValueError("extraction failed/cancelled or exit disagrees with publication status")
    digest = hashlib.sha256(pdf.read_bytes()).hexdigest()
    document = result["document"]
    pages = result["pages"]
    if document["hash"] != digest or document["size"] != pdf.stat().st_size:
        raise ValueError("published source identity differs from the selected PDF")
    if not pages or [p["page"] for p in pages] != list(range(1, document["pages"] + 1)):
        raise ValueError("publication has missing, duplicate or unordered page outcomes")
    original_paths = [Path(p) for p in result["output_paths"]]
    if any(p.is_symlink() for p in original_paths):
        raise ValueError("committed output must not be a symlink")
    paths = [p.resolve() for p in original_paths]
    if len(paths) != 2 or any(
        p.parent != extracted.resolve() or not p.is_file() or p.is_symlink() for p in paths
    ):
        raise ValueError("missing or unexpected committed output paths")
    by_suffix = {p.suffix: p for p in paths}
    if set(by_suffix) != {".json", ".txt"}:
        raise ValueError("publication must provide both JSON and page text")
    persisted = json.loads(by_suffix[".json"].read_text())
    if persisted != {k: v for k, v in result.items() if k not in {"output_paths", "worker_limits"}}:
        raise ValueError("disk JSON differs from this controller publication")
    expected_text = "\f".join(p["text"] for p in pages).encode()
    if by_suffix[".txt"].read_bytes() != expected_text:
        raise ValueError("page text differs from the published page outcomes")

    shutil.copyfile(pdf, output / f"{stem}.pdf")
    shutil.copyfile(by_suffix[".txt"], output / f"{stem}.tpe.txt")
    shutil.copyfile(by_suffix[".json"], output / f"{stem}.tpe.json")
    return {
        "id": stem.replace("arxiv_", "arxiv:"),
        "source_sha256": digest,
        "status": status,
        "cli_exit": process.returncode,
        "page_outcomes": len(pages),
        "quality_accepted": status == "complete",
        "scope": "Diagnostic publication, not general extraction accuracy or security clearance",
    }


def main(argv: list[str]) -> int:
    manifest_path, cache, tpe, extracted, output = map(Path, argv)
    items = {item["id"]: item for item in json.loads(manifest_path.read_text())["items"]}
    # Check the whole export scope before processing any input. A future split
    # change must fail visibly rather than silently exposing held-out material.
    if any(items.get(paper_id, {}).get("split") != "dev" for paper_id in EXPORT_IDS):
        raise ValueError("visual export is restricted to the three existing development inputs")
    records = []
    output.mkdir(parents=True, exist_ok=True)
    for paper_id in EXPORT_IDS:
        stem = paper_id.replace(":", "_")
        records.append(export_paper(cache / stem / "paper.pdf", tpe, extracted, output, stem))
        (output / "status.json").write_text(json.dumps(records, indent=2) + "\n")
    partial = sum(record["status"] == "partial" for record in records)
    print(
        f"Exported {len(records)} development diagnostics; "
        f"{partial} Partial results remain unqualified."
    )
    # Finish diagnostic export, then keep the original nonzero quality signal.
    # The workflow's always() artifact upload can preserve failure evidence.
    return int(partial > 0)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
