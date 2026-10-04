"""Synthetic publisher-contract fixtures; no corpus bytes or native execution."""

import hashlib
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

spec = importlib.util.spec_from_file_location(
    "visual_audit", Path(__file__).parents[1] / "scripts/export_visual_audit.py"
)
assert spec and spec.loader
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


@pytest.mark.parametrize(
    ("status", "exit_code", "damage"),
    [
        ("complete", 0, None),
        ("partial", 1, None),
        ("partial", 0, None),
        ("complete", 1, None),
        ("failed", 1, None),
        ("partial", 137, None),
        ("partial", 1, "missing"),
        ("partial", 1, "text"),
        ("partial", 1, "json"),
        ("partial", 1, "hash"),
        ("partial", 1, "pages"),
        ("partial", 1, "symlink"),
    ],
)
def test_diagnostic_publication_requires_consistent_outputs(
    tmp_path, monkeypatch, status, exit_code, damage
):
    pdf = tmp_path / "input.pdf"
    pdf.write_bytes(b"synthetic PDF fixture; never parsed")
    extracted, output = tmp_path / "extracted", tmp_path / "audit"
    extracted.mkdir()
    result = {
        "document": {
            "hash": hashlib.sha256(pdf.read_bytes()).hexdigest(),
            "size": pdf.stat().st_size,
            "pages": 2,
        },
        "status": status,
        "pages": [{"page": 1, "text": "Page one \u03b1"}, {"page": 2, "text": "Page two"}],
    }
    if damage == "hash":
        result["document"]["hash"] = "0" * 64
    if damage == "pages":
        result["pages"].reverse()
    json_path, text_path = extracted / "published.json", extracted / "published.txt"
    json_path.write_text(json.dumps(result))
    text_path.write_bytes("\f".join(p["text"] for p in result["pages"]).encode())
    if damage == "missing":
        text_path.unlink()
    elif damage == "text":
        text_path.write_text("unrelated stale output")
    elif damage == "json":
        json_path.write_text("{}")
    elif damage == "symlink":
        sibling = extracted / "other.txt"
        text_path.rename(sibling)
        text_path.symlink_to(sibling)
    published = {
        **result,
        "output_paths": [str(json_path), str(text_path)],
        "worker_limits": {"fixture": True},
    }

    def run(argv, **options):
        # Model only the documented stdout/exit contract. Assert explicit argv,
        # bounded execution and no shell; actual Rust execution is a hosted gate.
        assert argv[1:3] == ["extract", str(pdf.resolve())]
        assert argv[-1] == "--json"
        assert options["timeout"] == 120
        assert "shell" not in options
        return SimpleNamespace(stdout=json.dumps(published), returncode=exit_code)

    monkeypatch.setattr(audit.subprocess, "run", run)
    accepted = not damage and (status, exit_code) in [("complete", 0), ("partial", 1)]
    if accepted:
        receipt = audit.export_paper(pdf, tmp_path / "tpe", extracted, output, "arxiv_fixture")
        assert receipt["status"] == status
        assert receipt["quality_accepted"] is (status == "complete")
        assert (output / "arxiv_fixture.pdf").read_bytes() == pdf.read_bytes()
        assert (output / "arxiv_fixture.tpe.txt").read_bytes() == text_path.read_bytes()
    else:
        with pytest.raises(ValueError):
            audit.export_paper(pdf, tmp_path / "tpe", extracted, output, "arxiv_fixture")
        assert not list(output.glob("*.pdf")), "invalid publication must not export input bytes"


@pytest.mark.parametrize("status", ["complete", "partial"])
def test_export_finishes_all_diagnostics_then_preserves_quality_exit(tmp_path, monkeypatch, status):
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps({"items": [{"id": i, "split": "dev"} for i in audit.EXPORT_IDS]})
    )
    calls = []

    def export(*args):
        calls.append(args)
        return {"status": status, "quality_accepted": status == "complete"}

    monkeypatch.setattr(audit, "export_paper", export)
    output = tmp_path / "audit"
    code = audit.main(
        [
            str(p)
            for p in [
                manifest,
                tmp_path / "cache",
                tmp_path / "tpe",
                tmp_path / "extracted",
                output,
            ]
        ]
    )
    assert len(calls) == 3
    assert len(json.loads((output / "status.json").read_text())) == 3
    assert code == (1 if status == "partial" else 0)


def test_heldout_split_refused_before_any_export(tmp_path, monkeypatch):
    manifest = tmp_path / "manifest.json"
    manifest.write_text(
        json.dumps({"items": [{"id": i, "split": "heldout"} for i in audit.EXPORT_IDS]})
    )

    def forbidden(*args):
        pytest.fail("heldout bytes must not be opened or exported")

    monkeypatch.setattr(audit, "export_paper", forbidden)
    with pytest.raises(ValueError, match="development"):
        audit.main(
            [
                str(p)
                for p in [
                    manifest,
                    tmp_path / "cache",
                    tmp_path / "tpe",
                    tmp_path / "extracted",
                    tmp_path / "audit",
                ]
            ]
        )
