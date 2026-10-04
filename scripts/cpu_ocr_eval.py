"""Run a small real CPU OCR evaluation into a NEW, self-contained SQLite database.

Run manually on a Codex Cloud executor; ordinary tests never run this workload.
The database contains originals, truth, raw output and provenance, not just scores.
"""

import argparse
import hashlib
import json
import os
import platform
import random
import re
import shutil
import signal
import sqlite3
import struct
import subprocess
import sys
import time
import unicodedata
import zlib
from collections import Counter
from datetime import UTC, datetime
from pathlib import Path
from xml.sax.saxutils import escape

HERE = Path(__file__).resolve().parent
SCHEMA = HERE / "cpu_ocr_eval_schema.sql"
LINES = (
    "A Small Study of Garden Robots",
    "Abstract",
    "Three robots counted blue flowers in a garden.",
    "Methods",
    "Each robot visited twelve plots on a clear day.",
    "Results",
    "The observed counts were 12, 24, and 36 flowers.",
    "References",
    "Green A. Garden counting methods. Test Journal.",
    "2024; 3: 12-18. Synthetic example for OCR testing.",
)
TRUTH = "\n".join(LINES)
DPI = 150
HARNESS_VERSION = "2"
NORMALIZATION = "NFKC + casefold + whitespace collapse; word tokens are Unicode word sequences"
PHASE_DEFINITION = (
    "cold = first invocation for each fixture; warm = repeat invocation in the same executor. "
    "Every invocation starts fresh Python/Tesseract processes. OS caches are NOT flushed; "
    "these are first/repeated process timings, not machine-cold or resident-model timings."
)


def now():
    return datetime.now(UTC).isoformat()


def digest(content):
    return hashlib.sha256(content).hexdigest()


def json_text(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def insert(db, table, **values):
    # Table and column identifiers come only from this module, never CLI/input documents.
    columns = ",".join(values)
    placeholders = ",".join("?" for _ in values)
    return db.execute(
        f"INSERT INTO {table} ({columns}) VALUES ({placeholders})",  # noqa: S608
        tuple(values.values()),
    ).lastrowid


def artifact(db, content, media_type="application/octet-stream"):
    sha = digest(content)
    row = db.execute("SELECT id FROM artifacts WHERE sha256 = ?", (sha,)).fetchone()
    return (
        row[0]
        if row
        else insert(
            db,
            "artifacts",
            sha256=sha,
            byte_count=len(content),
            media_type=media_type,
            content=content,
        )
    )


def create_database(path):
    """Exclusively create: an existing evidence database can never be overwritten."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("xb"):
        pass
    db = sqlite3.connect(path)
    db.row_factory = sqlite3.Row
    db.executescript(SCHEMA.read_text())
    return db


def read_optional(path):
    try:
        return Path(path).read_text().strip()
    except OSError:
        return None


def command_output(command):
    try:
        result = subprocess.run(  # noqa: S603
            command, capture_output=True, text=True, timeout=15, check=False
        )
        return {
            "command": command,
            "exit_code": result.returncode,
            "output": (result.stdout + result.stderr).strip(),
            "stdout": result.stdout.strip(),
            "stderr": result.stderr.strip(),
        }
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"command": command, "error": str(exc)}


def environment_snapshot():
    cpu_max = read_optional("/sys/fs/cgroup/cpu.max")
    quota = None
    if cpu_max and cpu_max.split()[0] != "max":
        numerator, denominator = map(int, cpu_max.split())
        quota = numerator / denominator
    memory_max = read_optional("/sys/fs/cgroup/memory.max")
    affinity = sorted(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else None
    return {
        "captured_at": now(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "os_cpu_count": os.cpu_count(),
        "affinity_cpu_ids": affinity,
        "affinity_cpu_count": len(affinity) if affinity is not None else None,
        "cgroup_cpu_max": cpu_max,
        "cgroup_cpu_stat": read_optional("/sys/fs/cgroup/cpu.stat"),
        "cpu_quota_cores": quota,
        "cgroup_cpuset": read_optional("/sys/fs/cgroup/cpuset.cpus.effective"),
        "memory_limit_bytes": int(memory_max) if memory_max and memory_max != "max" else None,
        "cgroup_memory_current_bytes": read_optional("/sys/fs/cgroup/memory.current"),
        "cgroup_memory_peak_bytes": read_optional("/sys/fs/cgroup/memory.peak"),
        "cgroup_memory_note": "Cgroup values include other work in the executor, not OCR alone.",
        "proc_meminfo": read_optional("/proc/meminfo"),
        "cpu_model": next(
            (
                line.split(":", 1)[1].strip()
                for line in (read_optional("/proc/cpuinfo") or "").splitlines()
                if line.startswith("model name")
            ),
            None,
        ),
        "git": command_output(["git", "rev-parse", "HEAD"]),
        "git_status": command_output(["git", "status", "--porcelain"]),
        "python": sys.version,
        "runtime_environment": {
            key: os.environ.get(key)
            for key in (
                "FONTCONFIG_FILE",
                "POPPLER_DATADIR",
                "OMP_NUM_THREADS",
                "OMP_THREAD_LIMIT",
                "UV_CACHE_DIR",
                "XDG_CACHE_HOME",
            )
        },
        "toolchain": {
            name: command_output(command)
            for name, command in {
                "uv": ["uv", "--version"],
                "rustc": ["rustc", "--version"],
                "cargo": ["cargo", "--version"],
                "clippy": ["cargo", "clippy", "--version"],
                "rustup": ["rustup", "--version"],
            }.items()
        },
    }


def pdf_document(pages, raster=None):
    """Minimal deterministic PDF writer; image-only pages contain no text operators."""
    objects = [b"", b"", b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    image_id = None
    if raster is not None:
        width, height, pixels = raster
        compressed = zlib.compress(pixels)
        image_id = len(objects) + 1
        objects.append(
            f"<< /Type /XObject /Subtype /Image /Width {width} /Height {height} "
            f"/ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode "
            f"/Length {len(compressed)} >>\nstream\n".encode()
            + compressed
            + b"\nendstream"
        )
    page_ids = []
    for kind in pages:
        page_id, stream_id = len(objects) + 1, len(objects) + 2
        page_ids.append(page_id)
        if kind == "scan":
            content = b"q 612 0 0 792 0 0 cm /Scan Do Q"
            resources = f"<< /XObject << /Scan {image_id} 0 R >> >>"
        else:
            rows = []
            for index, line in enumerate(LINES):
                escaped = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
                rows.append(f"BT /F1 20 Tf 45 {745 - index * 34} Td ({escaped}) Tj ET")
            content = "\n".join(rows).encode()
            resources = "<< /Font << /F1 3 0 R >> >>"
        objects.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            f"/Resources {resources} /Contents {stream_id} 0 R >>".encode()
        )
        objects.append(
            f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream"
        )
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>"
    kids = " ".join(f"{page_id} 0 R" for page_id in page_ids)
    objects[1] = f"<< /Type /Pages /Kids [{kids}] /Count {len(page_ids)} >>".encode()
    result = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for index, obj in enumerate(objects, start=1):
        offsets.append(len(result))
        result.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        result.extend(f"{offset:010d} 00000 n \n".encode())
    result.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(result)


def read_pgm(path):
    data = path.read_bytes()
    header = re.match(rb"P5\s+(\d+)\s+(\d+)\s+255[\r\n ]", data)
    if not header:
        raise ValueError("Expected 8-bit binary PGM from pdftoppm")
    width, height = int(header[1]), int(header[2])
    pixels = data[header.end() :]
    if len(pixels) != width * height:
        raise ValueError("PGM pixel count mismatch")
    return width, height, pixels


def png(width, height, pixels):
    def chunk(name, payload):
        return (
            struct.pack(">I", len(payload))
            + name
            + payload
            + struct.pack(">I", zlib.crc32(name + payload))
        )

    scanlines = b"".join(b"\0" + pixels[row * width : (row + 1) * width] for row in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(scanlines))
        + chunk(b"IEND", b"")
    )


def generate_fixtures(directory, pdftoppm):
    """No downloads: render our own synthetic scholarly text, then remove the text layer."""
    directory.mkdir(parents=True, exist_ok=False)
    base = directory / "generation-source.pdf"
    base.write_bytes(pdf_document(["text"]))
    command = [
        pdftoppm,
        "-r",
        str(DPI),
        "-gray",
        "-singlefile",
        str(base),
        str(directory / "generation-source"),
    ]
    subprocess.run(command, check=True, capture_output=True, timeout=30)  # noqa: S603
    width, height, pixels = read_pgm(directory / "generation-source.pgm")
    rotated = bytes(
        pixels[(height - 1 - x) * width + y] for y in range(width) for x in range(height)
    )
    rng = random.Random(1729)  # noqa: S311 -- deterministic synthetic noise, no security use.
    degraded = bytes(
        max(0, min(255, int(100 + value * 0.5) + rng.randint(-35, 35))) for value in pixels
    )
    definitions = [
        ("clean_image", "png", png(width, height, pixels), [(width, height)], "identity"),
        (
            "clean_scan",
            "pdf",
            pdf_document(["scan"], (width, height, pixels)),
            [(width, height)],
            "image-only PDF, no text operators",
        ),
        (
            "rotated_image",
            "png",
            png(height, width, rotated),
            [(height, width)],
            "90 degrees clockwise; OCR is not given orientation truth",
        ),
        (
            "degraded_scan",
            "pdf",
            pdf_document(["scan"], (width, height, degraded)),
            [(width, height)],
            "contrast 0.5, offset 100, seeded uniform noise +/-35",
        ),
        (
            "mixed_document",
            "pdf",
            pdf_document(["text", "scan"], (width, height, pixels)),
            [(width, height), (width, height)],
            "page 1 native text; page 2 image-only scan",
        ),
        ("invalid_pdf", "pdf", b"%PDF-1.7\n%%EOF\n", [], "intentionally malformed PDF"),
    ]
    fixtures = []
    for name, extension, content, sizes, transform in definitions:
        path = directory / f"{name}.{extension}"
        path.write_bytes(content)
        fixtures.append(
            {
                "name": name,
                "path": path,
                "content": content,
                "sizes": sizes,
                "kind": extension,
                "transform": transform,
                "expected_outcome": "success" if sizes else "failed",
                "generation": {
                    "generator_version": HARNESS_VERSION,
                    "seed": 1729,
                    "dpi": DPI,
                    "transform": transform,
                    "source_pdf_sha256": digest(base.read_bytes()),
                    "command": command,
                    "generator_sha256": digest(Path(__file__).read_bytes()),
                },
                "source_assets": [
                    {
                        "path": str(base),
                        "role": "fixture_generator_original",
                        "uri": "synthetic://cpu-ocr-eval/v1/generation-source.pdf",
                    }
                ],
            }
        )
    return fixtures


def load_public_fixtures(manifest_path):
    """Consume an explicitly acquired, license-checked corpus manifest; never download."""
    manifest = json.loads(manifest_path.read_text())
    dataset = manifest["dataset"]
    for field in (
        "repository_url",
        "commit",
        "repository_checksum",
        "license",
        "license_url",
        "retrieved_at",
    ):
        if not dataset.get(field):
            raise ValueError(f"Public corpus manifest requires dataset.{field}")
    fixtures = []
    for item in manifest["fixtures"]:
        path = (manifest_path.parent / item["path"]).resolve()
        content = path.read_bytes()
        if digest(content) != item["sha256"]:
            raise ValueError(f"Public fixture checksum mismatch: {item['name']}")
        pages = item["ground_truth"]
        if not pages or any(not page["text"].strip() for page in pages):
            raise ValueError("Public corpus requires nonempty independently supplied ground truth")
        assets = [
            {"path": str(manifest_path), "role": "corpus_manifest", "uri": str(manifest_path)}
        ]
        for source_file in manifest.get("files", []):
            asset_path = (
                manifest_path.parent / source_file.get("local_path", source_file["repository_path"])
            ).resolve()
            if digest(asset_path.read_bytes()) != source_file["sha256"]:
                raise ValueError("Public corpus file checksum mismatch")
            assets.append(
                {"path": str(asset_path), "role": "corpus_file", "uri": source_file["url"]}
            )
        if item.get("ground_truth_path"):
            truth_path = (manifest_path.parent / item["ground_truth_path"]).resolve()
            truth_bytes = truth_path.read_bytes()
            if digest(truth_bytes) != item["ground_truth_sha256"]:
                raise ValueError("Public ground truth checksum mismatch")
            if truth_bytes.decode() != "\f".join(page["text"] for page in pages):
                raise ValueError("Inline ground truth differs from upstream original")
        for asset in item.get("source_assets", []):
            asset = dict(asset, path=str((manifest_path.parent / asset["path"]).resolve()))
            if digest(Path(asset["path"]).read_bytes()) != asset["sha256"]:
                raise ValueError("Public source asset checksum mismatch")
            assets.append(asset)
        fixtures.append(
            {
                "name": item["name"],
                "path": path,
                "content": content,
                "kind": path.suffix.lstrip(".").lower(),
                "expected_outcome": "success",
                "sizes": [(page["width_pixels"], page["height_pixels"]) for page in pages],
                "truth": [page["text"] for page in pages],
                "dpis": [page.get("dpi") for page in pages],
                "generation": {
                    "manifest_sha256": digest(manifest_path.read_bytes()),
                    "transform": item.get("transform", "unaltered upstream input"),
                    "ground_truth_origin": item.get("ground_truth_note"),
                },
                "source_metadata": dict(
                    dataset,
                    source_uri=item["source_uri"],
                    fixture_identifier=item["fixture_identifier"],
                    fixture_commit=item.get("fixture_commit"),
                    fixture_added_at=item.get("fixture_added_at"),
                ),
                "ground_truth_commit": item.get("ground_truth_commit"),
                "ground_truth_added_at": item.get("ground_truth_added_at"),
                "source_assets": assets,
            }
        )
    return fixtures


def prepare_fixture_font(workdir, source):
    """Pin the only available substitute font in this process, without user config edits."""
    directory = workdir / "fixture-font"
    directory.mkdir()
    target = directory / source.name
    target.write_bytes(source.read_bytes())
    config = workdir / "fixture-font.conf"
    config.write_text(
        '<?xml version="1.0"?>\n<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">\n'
        f"<fontconfig><dir>{escape(str(directory))}</dir>"
        f"<cachedir>{escape(str(workdir / 'font-cache'))}</cachedir></fontconfig>\n"
    )
    os.environ["FONTCONFIG_FILE"] = str(config)
    return target


def register_fixture(db, fixture):
    metadata = fixture.get("source_metadata", {})
    source_id = insert(
        db,
        "sources",
        uri=metadata.get("source_uri", f"synthetic://cpu-ocr-eval/v1/{fixture['name']}"),
        license=metadata.get("license", "CC0-1.0"),
        license_url=metadata.get("license_url"),
        description=metadata.get(
            "description", metadata.get("name", "New synthetic text; no user or medical data.")
        ),
        dataset_name=metadata.get("name"),
        dataset_version=metadata.get("version"),
        repository_url=metadata.get("repository_url"),
        repository_commit=metadata.get("commit"),
        repository_tree=metadata.get("repository_tree"),
        repository_checksum=metadata.get("repository_checksum"),
        repository_checksum_method=metadata.get("repository_checksum_method"),
        repository_commit_at=metadata.get("repository_commit_at"),
        fixture_identifier=metadata.get("fixture_identifier", fixture["name"]),
        fixture_commit=metadata.get("fixture_commit"),
        fixture_added_at=metadata.get("fixture_added_at"),
        selection_note=metadata.get("selection"),
        published_at=metadata.get("published_at"),
        retrieved_at=metadata.get("retrieved_at", now()),
        doi=metadata.get("doi"),
        original_artifact_id=artifact(
            db,
            fixture["content"],
            {
                "pdf": "application/pdf",
                "png": "image/png",
                "jpg": "image/jpeg",
                "jpeg": "image/jpeg",
                "tif": "image/tiff",
                "tiff": "image/tiff",
                "pgm": "image/x-portable-graymap",
            }.get(fixture["kind"], "application/octet-stream"),
        ),
    )
    for asset in fixture.get("source_assets", []):
        insert(
            db,
            "source_assets",
            source_id=source_id,
            artifact_id=artifact(db, Path(asset["path"]).read_bytes()),
            role=asset["role"],
            uri=asset["uri"],
        )
    fixture_id = insert(
        db,
        "fixtures",
        name=fixture["name"],
        source_id=source_id,
        kind=fixture["kind"],
        expected_outcome=fixture["expected_outcome"],
        generator_json=json_text(fixture["generation"]),
    )
    for page, (width, height) in enumerate(fixture["sizes"], start=1):
        truth = fixture.get("truth", [TRUTH] * len(fixture["sizes"]))[page - 1]
        page_id = insert(
            db,
            "fixture_pages",
            fixture_id=fixture_id,
            page_number=page,
            ground_truth=truth,
            ground_truth_artifact_id=artifact(db, truth.encode(), "text/plain"),
            ground_truth_commit=fixture.get("ground_truth_commit"),
            ground_truth_added_at=fixture.get("ground_truth_added_at"),
            width_pixels=width,
            height_pixels=height,
            dpi=fixture.get("dpis", [DPI] * len(fixture["sizes"]))[page - 1],
        )
        for ordinal, line in enumerate(LINES if "truth" not in fixture else []):
            x, y, region_width, region_height = (
                45 * DPI / 72,
                (27 + ordinal * 34) * DPI / 72,
                540 * DPI / 72,
                28 * DPI / 72,
            )
            if fixture["name"] == "rotated_image":
                x, y, region_width, region_height = (
                    width - y - region_height,
                    x,
                    region_height,
                    region_width,
                )
            insert(
                db,
                "truth_regions",
                fixture_page_id=page_id,
                ordinal=ordinal,
                text=line,
                x=x,
                y=y,
                width=region_width,
                height=region_height,
                coordinate_space="raster_pixels_top_left",
                precision_note="Nominal fixture line region; not a glyph-tight annotation.",
            )
    return fixture_id


def edit_distance(left, right):
    row = list(range(len(right) + 1))
    for i, item in enumerate(left, start=1):
        new_row = [i]
        for j, other in enumerate(right, start=1):
            new_row.append(min(new_row[-1] + 1, row[j] + 1, row[j - 1] + (item != other)))
        row = new_row
    return row[-1]


def score(truth, output):
    def normalize(text):
        return " ".join(unicodedata.normalize("NFKC", text).casefold().split())

    expected, actual = normalize(truth), normalize(output)
    expected_words, actual_words = re.findall(r"\w+", expected), re.findall(r"\w+", actual)
    chars, words = edit_distance(expected, actual), edit_distance(expected_words, actual_words)
    missing = Counter(expected_words) - Counter(actual_words)
    extra = Counter(actual_words) - Counter(expected_words)
    return {
        "normalization": NORMALIZATION,
        "reference_characters": len(expected),
        "character_edits": chars,
        "cer": chars / max(1, len(expected)),
        "reference_words": len(expected_words),
        "word_edits": words,
        "wer": words / max(1, len(expected_words)),
        "omitted_words": sum(missing.values()),
        "extra_words": sum(extra.values()),
        "missing_words_json": json_text(dict(missing)),
    }


def register_component(db, run_id, role, name, facts):
    path = str(facts.get("path", ""))
    sha = facts.get("sha256")
    if not sha and path and Path(path).is_file():
        with Path(path).open("rb") as stream:
            sha = hashlib.file_digest(stream, "sha256").hexdigest()
    if not sha:
        return
    version = str(facts.get("version", "unversioned; pinned by SHA-256"))
    row = db.execute(
        "SELECT id FROM components WHERE role=? AND name=? AND version=? AND sha256=?",
        (role, name, version, sha),
    ).fetchone()
    component_artifact = (
        artifact(db, Path(path).read_bytes())
        if path and Path(path).is_file() and facts.get("preserve_bytes", True)
        else None
    )
    component_id = (
        row[0]
        if row
        else insert(
            db,
            "components",
            role=role,
            name=name,
            version=version,
            path=path,
            sha256=sha,
            artifact_id=component_artifact,
            metadata_json=json_text(facts),
        )
    )
    db.execute("INSERT OR IGNORE INTO run_components VALUES (?, ?)", (run_id, component_id))


def run_process(command, workdir, timeout):
    """Keep output in files; wait4 reports a process maximum, not summed tree RSS."""
    workdir.mkdir(parents=True, exist_ok=False)
    stdout_path, stderr_path = (workdir / filename for filename in ("stdout.jsonl", "stderr.txt"))
    started_at, start = now(), time.perf_counter()
    interruption = None
    rss_line = re.search(r"^VmRSS:\s+(\d+)", read_optional("/proc/self/status") or "", re.MULTILINE)
    evaluator_rss = int(rss_line[1]) if rss_line else None

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt

    previous_handlers = {}
    with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
        process = subprocess.Popen(  # noqa: S603
            command,
            stdout=out,
            stderr=err,
            start_new_session=True,
        )
        try:
            previous_handlers = {
                sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)
            }
            deadline = start + timeout
            while True:
                try:
                    waited, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if waited:
                        break
                    if time.perf_counter() >= deadline:
                        if interruption:
                            os.killpg(process.pid, signal.SIGKILL)
                        else:
                            interruption = "timeout"
                            os.killpg(process.pid, signal.SIGINT)
                        deadline = time.perf_counter() + 10
                    time.sleep(0.02)
                except KeyboardInterrupt:
                    interruption = "cancelled"
                    for sig in previous_handlers:
                        signal.signal(sig, signal.SIG_IGN)
                    os.killpg(process.pid, signal.SIGINT)
                    deadline = time.perf_counter() + 10
            process.returncode = os.waitstatus_to_exitcode(status)
        finally:
            for sig, handler in previous_handlers.items():
                signal.signal(sig, handler)
    measurements = {
        "process_max_rss_kib": usage.ru_maxrss,
        "user_cpu_seconds": usage.ru_utime,
        "system_cpu_seconds": usage.ru_stime,
    }
    if evaluator_rss is not None:
        measurements["evaluator_rss_before_spawn_kib"] = evaluator_rss
    return {
        "started_at": started_at,
        "elapsed_seconds": time.perf_counter() - start,
        "exit_code": process.returncode,
        "stdout": stdout_path.read_bytes(),
        "stderr": stderr_path.read_bytes(),
        "interruption": interruption,
        "resources": measurements,
        "command": command,
    }


def record_attempt(db, run_id, fixture_id, fixture, result):
    events, parse_errors = [], []
    for line in result["stdout"].decode(errors="replace").splitlines():
        try:
            event = json.loads(line)
            if not isinstance(event, dict):
                raise ValueError("event is not an object")
            events.append(event)
        except (ValueError, TypeError) as exc:
            parse_errors.append(f"{exc}: {line}")
    summary = next((event for event in reversed(events) if event.get("event") == "summary"), {})
    outcome = result["interruption"] or summary.get("outcome", "failed")
    succeeded = outcome == "success" and result["exit_code"] == 0 and not parse_errors
    expected_met = (
        succeeded
        if fixture["expected_outcome"] == "success"
        else (outcome == "failed" and result["exit_code"] != 0)
    )
    attempt_id = insert(
        db,
        "attempts",
        run_id=run_id,
        fixture_id=fixture_id,
        started_at=result["started_at"],
        elapsed_seconds=result["elapsed_seconds"],
        exit_code=result["exit_code"],
        outcome=outcome,
        expected_outcome_met=int(expected_met),
        command_json=json_text(result["command"]),
        stdout_artifact_id=artifact(db, result["stdout"], "application/x-ndjson"),
        stderr_artifact_id=artifact(db, result["stderr"], "text/plain"),
    )

    def record_error(category, message, payload, page=None):
        insert(
            db,
            "errors",
            attempt_id=attempt_id,
            page_number=page,
            category=category,
            message=message,
            payload_json=json_text(payload),
        )
        db.execute("UPDATE attempts SET expected_outcome_met=0 WHERE id=?", (attempt_id,))

    for message in parse_errors:
        insert(
            db,
            "errors",
            attempt_id=attempt_id,
            category="invalid_jsonl",
            message=message,
            payload_json="{}",
        )
    if not summary or result["interruption"]:
        insert(
            db,
            "errors",
            attempt_id=attempt_id,
            category=result["interruption"] or "no_summary",
            message=result["stderr"].decode(errors="replace"),
            payload_json=json_text(result["interruption"]),
        )
    for metric, value in result["resources"].items():
        insert(
            db,
            "resource_measurements",
            attempt_id=attempt_id,
            metric=metric,
            value=value,
            unit="KiB" if metric.endswith("kib") else "seconds",
            method=(
                "/proc/self/status VmRSS before spawn; evaluator current RSS, not OCR"
                if metric == "evaluator_rss_before_spawn_kib"
                else "os.wait4 rusage on Linux; individual process maximum including inherited "
                "spawn footprint, not summed tree RSS"
            ),
        )
    expected_pages = {
        row["page_number"]: row
        for row in db.execute("SELECT * FROM fixture_pages WHERE fixture_id=?", (fixture_id,))
    }
    seen_pages = set()
    for ordinal, event in enumerate(events):
        insert(db, "events", attempt_id=attempt_id, ordinal=ordinal, payload_json=json_text(event))
        kind, page_number = event.get("event"), event.get("page")
        for warning in event.get("warnings", []):
            insert(
                db,
                "warnings",
                attempt_id=attempt_id,
                page_number=page_number,
                payload_json=json_text(warning),
            )
        if kind == "start":
            if event.get("source", {}).get("sha256") != digest(fixture["content"]):
                record_error(
                    "source_hash_mismatch", "Runtime source differs from stored original", event
                )
            for name, facts in event.get("runtime", {}).items():
                if name == "models":
                    for model in facts:
                        register_component(db, run_id, "model", model["language"], model)
                elif isinstance(facts, dict):
                    register_component(db, run_id, "runtime", name, facts)
        if kind in ("page_error", "error"):
            insert(
                db,
                "errors",
                attempt_id=attempt_id,
                page_number=page_number,
                category=str(event.get("code", kind)),
                message=str(event.get("error", event)),
                payload_json=json_text(event),
            )
        if kind != "page":
            continue
        if not isinstance(page_number, int) or page_number < 1 or page_number in seen_pages:
            record_error("invalid_page", "Missing, invalid, or duplicate page number", event)
            continue
        seen_pages.add(page_number)
        tsv_facts = event.get("artifacts", {}).get("tsv", {})
        tsv = None
        try:
            if not tsv_facts.get("path"):
                raise ValueError("Runtime did not retain its raw TSV artifact")
            tsv = Path(tsv_facts["path"]).read_bytes()
            if digest(tsv) != tsv_facts.get("sha256"):
                raise ValueError("Runtime TSV hash mismatch")
        except (OSError, ValueError, TypeError) as exc:
            record_error("tsv_artifact_error", str(exc), tsv_facts, page_number)
        text = event.get("text", "")
        if not isinstance(text, str):
            record_error("invalid_text", "OCR text is not a string", event, page_number)
            text = ""
        expected = expected_pages.get(page_number)
        if expected is None:
            record_error(
                "unexpected_page", "OCR returned a page outside fixture truth", event, page_number
            )
        if event.get("source_sha256", digest(fixture["content"])) != digest(fixture["content"]):
            record_error(
                "page_source_hash_mismatch",
                "Page source differs from stored original",
                event,
                page_number,
            )
        page_id = insert(
            db,
            "page_outputs",
            attempt_id=attempt_id,
            fixture_page_id=expected["id"] if expected else None,
            page_number=page_number,
            status=event.get("status", "unknown"),
            raw_text=text,
            tsv_artifact_id=artifact(db, tsv, "text/tab-separated-values")
            if tsv is not None
            else None,
            raster_json=json_text(event.get("raster", {})),
            timing_json=json_text(event.get("timing_seconds", {})),
        )
        for diagnostic in event.get("artifacts", {}).get("diagnostics", []):
            try:
                content = Path(diagnostic["path"]).read_bytes()
                if digest(content) != diagnostic["sha256"]:
                    raise ValueError("Runtime diagnostic hash mismatch")
                insert(
                    db,
                    "page_artifacts",
                    page_output_id=page_id,
                    role=diagnostic["name"],
                    artifact_id=artifact(db, content, "text/plain"),
                )
            except (KeyError, OSError, TypeError, ValueError, sqlite3.IntegrityError) as exc:
                record_error("diagnostic_artifact_error", str(exc), diagnostic, page_number)
        for region_ordinal, region in enumerate(event.get("words", [])):
            try:
                x, y, width, height = map(float, region["bbox"])
                if not isinstance(region["text"], str):
                    raise ValueError("Region text is not a string")
            except (KeyError, TypeError, ValueError) as exc:
                record_error("invalid_region", str(exc), region, page_number)
                continue
            insert(
                db,
                "ocr_regions",
                page_output_id=page_id,
                ordinal=region_ordinal,
                text=region["text"],
                confidence=region.get("confidence"),
                x=x,
                y=y,
                width=width,
                height=height,
                coordinate_space="raster_pixels_top_left",
                provenance_json=json_text(
                    {
                        key: value
                        for key, value in region.items()
                        if key not in ("text", "confidence", "bbox")
                    }
                ),
            )
        for metric, value in event.get("resources", {}).items():
            if isinstance(value, (int, float)):
                insert(
                    db,
                    "resource_measurements",
                    attempt_id=attempt_id,
                    page_number=page_number,
                    metric=metric,
                    value=value,
                    unit="KiB" if metric.endswith("kib") else "unknown",
                    method="Runtime-reported per-process rusage maximum; not summed tree RSS",
                )
        if expected:
            insert(
                db,
                "accuracy",
                page_output_id=page_id,
                **score(expected["ground_truth"], text),
            )
    for number in expected_pages.keys() - seen_pages:
        expected = expected_pages[number]
        page_id = insert(
            db,
            "page_outputs",
            attempt_id=attempt_id,
            fixture_page_id=expected["id"],
            page_number=number,
            status="missing",
            raw_text="",
            tsv_artifact_id=None,
            raster_json="{}",
            timing_json="{}",
        )
        insert(db, "accuracy", page_output_id=page_id, **score(expected["ground_truth"], ""))
        insert(
            db,
            "errors",
            attempt_id=attempt_id,
            page_number=number,
            category="missing_page",
            message="Expected page has no OCR result; scored as a complete omission.",
            payload_json="{}",
        )
        db.execute("UPDATE attempts SET expected_outcome_met=0 WHERE id=?", (attempt_id,))
    db.commit()
    return outcome


def export_summary(db):
    return {
        "schema_version": db.execute("PRAGMA user_version").fetchone()[0],
        "harness_version": HARNESS_VERSION,
        "phase_definition": PHASE_DEFINITION,
        "accuracy_definition": NORMALIZATION,
        "omission_definition": (
            "Multiset reference token deficit, including substitutions; not alignment deletions."
        ),
        "sources": [
            dict(row)
            for row in db.execute(
                "SELECT s.*, a.sha256 AS original_sha256, a.byte_count FROM sources s "
                "JOIN artifacts a ON a.id=s.original_artifact_id"
            )
        ],
        "environments": [
            json.loads(row[0]) for row in db.execute("SELECT snapshot_json FROM environments")
        ],
        "components": [
            dict(row) for row in db.execute("SELECT role,name,version,path,sha256 FROM components")
        ],
        "runs": [
            dict(row)
            for row in db.execute(
                "SELECT id,phase,repetition,status,started_at,finished_at FROM runs"
            )
        ],
        "attempts": [
            dict(row)
            for row in db.execute(
                "SELECT r.phase, f.name AS fixture, a.outcome, a.expected_outcome_met, "
                "a.elapsed_seconds, "
                "a.exit_code, (SELECT value FROM resource_measurements WHERE attempt_id=a.id AND "
                "metric='process_max_rss_kib' LIMIT 1) AS process_max_rss_kib "
                "FROM attempts a JOIN runs r ON r.id=a.run_id JOIN fixtures f ON f.id=a.fixture_id"
            )
        ],
        "page_scores": [dict(row) for row in db.execute("SELECT * FROM page_scores")],
        "errors": [
            dict(row)
            for row in db.execute("SELECT attempt_id,page_number,category,message FROM errors")
        ],
        "counts": {
            table: db.execute(f"SELECT count(*) FROM {table}").fetchone()[0]  # noqa: S608
            for table in (
                "fixtures",
                "fixture_pages",
                "truth_regions",
                "attempts",
                "page_outputs",
                "ocr_regions",
                "artifacts",
                "warnings",
                "errors",
            )
        },
        "integrity_check": db.execute("PRAGMA integrity_check").fetchone()[0],
        "foreign_key_violations": [list(row) for row in db.execute("PRAGMA foreign_key_check")],
    }


def evaluate(args, summary_output):
    db = create_database(args.database)
    workdir = args.database.parent / (args.database.stem + "-artifacts")
    workdir.mkdir(exist_ok=False)
    fixture_font = (
        prepare_fixture_font(workdir, args.fixture_font_file.resolve())
        if args.fixture_font_file
        else None
    )
    environment = environment_snapshot()
    environment_id = insert(
        db,
        "environments",
        captured_at=environment["captured_at"],
        executor="Codex Cloud ephemeral executor",
        snapshot_json=json_text(environment),
    )
    pdftoppm = shutil.which(args.pdftoppm)
    if not pdftoppm:
        raise RuntimeError("Evaluation requires a native pdftoppm executable")
    generation_start = time.perf_counter()
    fixtures = generate_fixtures(workdir / "fixtures", pdftoppm)
    if args.public_manifest:
        fixtures.extend(load_public_fixtures(args.public_manifest.resolve()))
    fixture_ids = {fixture["name"]: register_fixture(db, fixture) for fixture in fixtures}
    artifact(db, (workdir / "fixtures" / "generation-source.pdf").read_bytes(), "application/pdf")
    config = {
        "workers": args.workers,
        "dpi": DPI,
        "psm": 6,
        "language": "eng",
        "timeout_seconds": 30,
        "memory_mib": 1024,
        "outer_concurrency": 1,
        "fixture_preparation_seconds": time.perf_counter() - generation_start,
        "provisioning_seconds": None,
        "provisioning_note": "Preinstalled offline runtimes/models; provisioning not timed.",
        "execution_note": args.execution_note,
        "fixture_font_file": str(fixture_font) if fixture_font else None,
        "font_config_mode": "private_explicit_font" if fixture_font else "inherited_environment",
        "tessdata_dir": str(args.tessdata_dir),
        "pdftoppm": pdftoppm,
        "pdfinfo": args.pdfinfo,
        "repetitions": args.repetitions,
        "resource_scope": "No aggregate process-tree peak claim; see per-measurement methods.",
    }
    interrupted = False
    for repetition in range(args.repetitions):
        run_id = insert(
            db,
            "runs",
            environment_id=environment_id,
            phase="cold" if repetition == 0 else "warm",
            repetition=repetition,
            phase_definition=PHASE_DEFINITION,
            started_at=now(),
            status="running",
            command_json=json_text([sys.executable, *sys.argv]),
            config_json=json_text(config),
        )
        for path, name, role, version in [
            (Path(sys.executable).resolve(), "python", "toolchain", sys.version),
            (Path(__file__), "cpu_ocr_eval.py", "harness", HARNESS_VERSION),
            (SCHEMA, "cpu_ocr_eval_schema.sql", "schema", "1"),
            (HERE / "cpu_ocr.py", "cpu_ocr.py", "runtime", "source SHA-256"),
        ]:
            register_component(db, run_id, role, name, {"path": str(path), "version": version})
        if shutil.which("uv"):
            register_component(
                db,
                run_id,
                "toolchain",
                "uv",
                {
                    "path": shutil.which("uv"),
                    "version": environment["toolchain"]["uv"].get("output"),
                    "preserve_bytes": False,
                },
            )
        register_component(
            db,
            run_id,
            "configuration",
            "rust-toolchain.toml",
            {
                "path": str(HERE.parent / "rust-toolchain.toml"),
                "version": "source SHA-256",
            },
        )
        if os.environ.get("FONTCONFIG_FILE"):
            register_component(
                db,
                run_id,
                "fixture_configuration",
                "FONTCONFIG_FILE",
                {
                    "path": os.environ["FONTCONFIG_FILE"],
                    "version": "source SHA-256",
                },
            )
        if fixture_font:
            register_component(
                db,
                run_id,
                "fixture_generator",
                "explicit fixture font",
                {
                    "path": str(fixture_font),
                    "original_path": str(args.fixture_font_file.resolve()),
                    "version": "original font bytes pinned by SHA-256",
                },
            )
        if args.fixture_font_probe:
            register_component(
                db,
                run_id,
                "qualification",
                "pre-OCR font-open trace",
                {
                    "path": str(args.fixture_font_probe.resolve()),
                    "version": "strace openat of same native rasterizer and one-font configuration",
                },
            )
        # Preserve actual rasterizer and selected base font.
        register_component(
            db,
            run_id,
            "fixture_generator",
            "pdftoppm",
            {"path": pdftoppm, "version": command_output([pdftoppm, "-v"]).get("output")},
        )
        if shutil.which("fc-match"):
            font = command_output(["fc-match", "Helvetica", "-f", "%{file}"])
            if font.get("exit_code") == 0:
                register_component(
                    db,
                    run_id,
                    "fixture_generator",
                    "Helvetica substitution font",
                    {"path": font["stdout"], "fontconfig_diagnostics": font["stderr"]},
                )
        db.commit()
        for index, fixture in enumerate(fixtures):
            attempt_dir = workdir / f"run-{run_id}" / fixture["name"]
            command = [
                sys.executable,
                str(HERE / "cpu_ocr.py"),
                str(fixture["path"]),
                "--tessdata-dir",
                str(args.tessdata_dir),
                "--pdftoppm",
                pdftoppm,
                "--pdfinfo",
                args.pdfinfo,
                "--artifacts-dir",
                str(attempt_dir / "ocr"),
                "--workers",
                str(args.workers),
                "--dpi",
                str(DPI),
                "--psm",
                "6",
                "--timeout-seconds",
                "30",
                "--memory-mib",
                "1024",
            ]
            result = run_process(command, attempt_dir, timeout=120)
            outcome = record_attempt(db, run_id, fixture_ids[fixture["name"]], fixture, result)
            print(
                json_text(
                    {
                        "run": run_id,
                        "fixture": fixture["name"],
                        "outcome": outcome,
                        "elapsed_seconds": result["elapsed_seconds"],
                    }
                ),
                flush=True,
            )
            if result["interruption"] == "cancelled":
                interrupted = True
                for queued in fixtures[index + 1 :]:
                    skipped = {
                        "started_at": now(),
                        "elapsed_seconds": 0.0,
                        "exit_code": None,
                        "stdout": b"",
                        "stderr": b"Cancelled before this fixture started",
                        "interruption": "not_attempted",
                        "resources": {},
                        "command": [],
                    }
                    record_attempt(db, run_id, fixture_ids[queued["name"]], queued, skipped)
                break
        failed = db.execute(
            "SELECT count(*) FROM attempts WHERE run_id=? AND expected_outcome_met=0", (run_id,)
        ).fetchone()[0]
        db.execute(
            "UPDATE runs SET finished_at=?, status=? WHERE id=?",
            (now(), "cancelled" if interrupted else "failed" if failed else "completed", run_id),
        )
        db.commit()
        if interrupted:
            break
    summary = export_summary(db)
    db.close()
    summary["database"] = {
        "path": str(args.database),
        "sha256": digest(args.database.read_bytes()),
        "bytes": args.database.stat().st_size,
    }
    summary_output.write(json.dumps(summary, indent=2, ensure_ascii=False) + "\n")
    summary_output.flush()
    print(
        json_text(
            {
                "database": str(args.database),
                "summary": str(args.summary),
                "counts": summary["counts"],
            }
        ),
        flush=True,
    )
    return 130 if interrupted else int(any(run["status"] != "completed" for run in summary["runs"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--database", type=Path, required=True, help="NEW SQLite path; never overwritten"
    )
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--tessdata-dir", type=Path, required=True)
    parser.add_argument(
        "--pdftoppm", required=True, help="Path to native executable, not a launcher"
    )
    parser.add_argument(
        "--pdfinfo", required=True, help="Path to native executable, not a launcher"
    )
    parser.add_argument("--public-manifest", type=Path)
    parser.add_argument(
        "--fixture-font-file",
        type=Path,
        help="Use a private Fontconfig directory containing only this font",
    )
    parser.add_argument(
        "--fixture-font-probe",
        type=Path,
        help="Optional pre-OCR native font-open trace to retain as evidence",
    )
    parser.add_argument("--execution-note", default="Other executor workloads not controlled")
    parser.add_argument("--workers", type=int, default=2)
    parser.add_argument("--repetitions", type=int, default=2)
    args = parser.parse_args()
    if args.workers < 1 or args.repetitions < 2:
        parser.error("workers must be positive; at least two repetitions are needed for cold/warm")
    args.database = args.database.resolve()
    args.summary = args.summary.resolve()
    args.tessdata_dir = args.tessdata_dir.resolve()
    if args.summary == args.database:
        parser.error("summary and database must be different paths")
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    # Refuse any existing summary too; it must never clobber a DB, source, or earlier report.
    with args.summary.open("x") as summary_output:
        return evaluate(args, summary_output)


if __name__ == "__main__":
    raise SystemExit(main())
