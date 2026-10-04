"""Real local raster OCR plus supervised failure/cancellation contracts."""

import hashlib
import importlib.util
import json
import os
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
import zlib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "cpu_ocr.py"
SPEC = importlib.util.spec_from_file_location("cpu_ocr", SCRIPT)
ocr = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ocr)


def native_binary(name):
    candidate = shutil.which(name)
    if not candidate:
        pytest.skip(f"real CPU OCR prerequisite missing: {name}")
    candidate = Path(candidate).resolve()
    if candidate.read_bytes()[:4] != b"\x7fELF":
        # The Codex image's explicit Poppler launcher target, never a guessed runtime hash.
        actual = candidate.parent / "../../native/poppler/poppler/bin" / name
        if actual.is_file():
            candidate = actual.resolve()
    if candidate.read_bytes()[:4] != b"\x7fELF":
        pytest.skip(f"supply a real native binary for OCR fixture tool {name}")
    return str(candidate)


@pytest.fixture(scope="module")
def runtime():
    models = Path("/usr/share/tesseract-ocr/5/tessdata")
    if not (models / "eng.traineddata").is_file():
        pytest.skip("real English Tesseract model is not provisioned")
    return {
        "tesseract": native_binary("tesseract"),
        "pdftoppm": native_binary("pdftoppm"),
        "pdfinfo": native_binary("pdfinfo"),
        "models": str(models),
    }


def pdf_bytes(objects):
    result = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for index, data in enumerate(objects, 1):
        offsets.append(len(result))
        result.extend(f"{index} 0 obj\n".encode() + data + b"\nendobj\n")
    xref = len(result)
    result.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        result.extend(f"{offset:010d} 00000 n \n".encode())
    result.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(result)


def stream(data, extra=b""):
    return (
        b"<< /Length " + str(len(data)).encode() + extra + b" >>\nstream\n" + data + b"\nendstream"
    )


@pytest.fixture(scope="module")
def scanned(tmp_path_factory, runtime):
    directory = tmp_path_factory.mktemp("cpu-ocr-synthetic")
    text_pdf = directory / "render-source.pdf"
    text_pdf.write_bytes(
        pdf_bytes(
            [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 160] "
                b"/Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
                stream(
                    b"BT /F1 28 Tf 25 100 Td (LOCAL CPU OCR) Tj "
                    b"0 -40 Td (Synthetic study 123) Tj ET"
                ),
            ]
        )
    )
    subprocess.run(  # noqa: S603
        [
            runtime["pdftoppm"],
            "-r",
            "144",
            "-singlefile",
            "-gray",
            str(text_pdf),
            str(directory / "scan"),
        ],
        check=True,
        capture_output=True,
    )
    image = directory / "scan.pgm"
    with image.open("rb") as handle:
        assert handle.readline() == b"P5\n"
        width, height = map(int, handle.readline().split())
        assert handle.readline() == b"255\n"
        pixels = handle.read()
    page = (
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 160] "
        b"/Resources << /XObject << /Im0 5 0 R >> >> /Contents 6 0 R >>"
    )
    image_pdf = directory / "two-scanned-pages.pdf"
    image_pdf.write_bytes(
        pdf_bytes(
            [
                b"<< /Type /Catalog /Pages 2 0 R >>",
                b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
                page,
                page,
                stream(
                    zlib.compress(pixels),
                    (
                        f" /Type /XObject /Subtype /Image /Width {width} "
                        f"/Height {height} /ColorSpace /DeviceGray /BitsPerComponent 8 "
                        "/Filter /FlateDecode"
                    ).encode(),
                ),
                stream(b"q 400 0 0 160 0 0 cm /Im0 Do Q"),
            ]
        )
    )
    return image, image_pdf


def cli(path, runtime, *extra):
    command = [sys.executable, str(SCRIPT), str(path), "--tessdata-dir", runtime["models"]]
    for name in ("tesseract", "pdftoppm", "pdfinfo"):
        command += ["--" + name, runtime[name]]
    result = subprocess.run(command + list(extra), capture_output=True, timeout=30, check=False)  # noqa: S603
    records = [json.loads(line) for line in result.stdout.splitlines()]
    return result, records


def test_real_image_ocr_keeps_original_and_tsv(scanned, runtime, tmp_path):
    source = scanned[0]
    before = source.read_bytes()
    artifacts = tmp_path / "raw"
    result, records = cli(source, runtime, "--artifacts-dir", str(artifacts))
    assert result.returncode == 0, result.stderr
    start, page, summary = records
    assert start["source"]["sha256"] == hashlib.sha256(before).hexdigest()
    assert source.read_bytes() == before
    assert "LOCALCPUOCR" in "".join(page["text"].split())
    assert "Synthetic study 123" in page["text"]
    assert page["status"] == "partial" and page["page"] == 1
    assert page["raster"]["pdf_transform"] is None
    assert page["raster"]["coordinate_space"] == "raster_pixels_top_left"
    assert all(0 <= word["confidence"] <= 100 for word in page["words"])
    raw = Path(page["artifacts"]["tsv"]["path"]).read_bytes()
    assert hashlib.sha256(raw).hexdigest() == page["artifacts"]["tsv"]["sha256"]
    assert "LOCAL" in raw.decode()
    assert summary["completed_pages"] == 1 and summary["outcome"] == "success"
    assert page["timing_seconds"]["ocr"] > 0
    assert page["resources"]["child_max_rss_kib"] > 0
    assert (
        start["runtime"]["models"][0]["sha256"]
        == hashlib.sha256((Path(runtime["models"]) / "eng.traineddata").read_bytes()).hexdigest()
    )


def test_real_image_only_pdf_two_workers_keeps_page_provenance(scanned, runtime):
    source = scanned[1]
    before = source.read_bytes()
    assert b"BT " not in before
    result, records = cli(source, runtime, "--workers", "2", "--dpi", "144")
    assert result.returncode == 0, result.stderr
    pages = [record for record in records if record["event"] == "page"]
    assert sorted(page["page"] for page in pages) == [1, 2]
    assert all("LOCALCPUOCR" in "".join(page["text"].split()) for page in pages)
    assert all(page["raster"]["pdf_transform"] is None for page in pages)
    assert all("pdf_coordinate_transform_unverified" in page["warnings"] for page in pages)
    assert all(page["timing_seconds"]["rasterize"] > 0 for page in pages)
    assert records[-1]["completed_pages"] == 2 and records[-1]["interrupted_pages"] == 0
    assert source.read_bytes() == before


def test_blank_is_partial_not_claimed_complete(runtime, tmp_path):
    blank = tmp_path / "blank.pgm"
    blank.write_bytes(b"P5\n100 100\n255\n" + b"\xff" * 10000)
    result, records = cli(blank, runtime)
    assert result.returncode == 0
    assert records[1]["text"] == "" and records[1]["status"] == "partial"
    assert "no_text_recognized_does_not_prove_blank" in records[1]["warnings"]


@pytest.mark.parametrize("frames", [1, 2])
def test_real_tiff_single_frame_and_multiframe_refusal(scanned, runtime, tmp_path, frames):
    with scanned[0].open("rb") as handle:
        assert handle.readline() == b"P5\n"
        width, height = map(int, handle.readline().split())
        assert handle.readline() == b"255\n"
        pixels = handle.read()
    ifd_size = 2 + 9 * 12 + 4
    pixel_offset = 8 + frames * ifd_size
    tags = [
        (256, 4, width),
        (257, 4, height),
        (258, 3, 8),
        (259, 3, 1),
        (262, 3, 1),
        (273, 4, pixel_offset),
        (277, 3, 1),
        (278, 4, height),
        (279, 4, len(pixels)),
    ]
    data = bytearray(b"II*\0" + struct.pack("<I", 8))
    for frame in range(frames):
        data += struct.pack("<H", len(tags))
        for tag, kind, value in tags:
            data += struct.pack("<HHII", tag, kind, 1, value)
        data += struct.pack("<I", 8 + ifd_size if frame + 1 < frames else 0)
    data += pixels
    source = tmp_path / f"frames-{frames}.tif"
    source.write_bytes(data)
    result, records = cli(source, runtime)
    if frames == 1:
        assert result.returncode == 0, result.stderr
        assert "LOCALCPUOCR" in "".join(records[1]["text"].split())
    else:
        assert result.returncode == 1
        assert records[1]["event"] == "page_error"
        assert records[1]["error"] == "multiple_image_frames_unsupported"
        assert records[-1]["failed_pages"] == 1
    assert source.read_bytes() == data


@pytest.mark.parametrize(
    "contents", [b"%PDF-1.4\ninvalid", b"\x89PNG\r\n\x1a\ninvalid", b"II*\0tiff"]
)
def test_bad_or_unsupported_input_is_failure_without_fake_text(runtime, tmp_path, contents):
    source = tmp_path / "bad"
    source.write_bytes(contents)
    result, records = cli(source, runtime)
    assert result.returncode != 0
    assert not any(record["event"] == "page" for record in records)
    assert source.read_bytes() == contents


def test_no_overwrite_or_output_alias(scanned, runtime):
    source = scanned[0]
    before = source.read_bytes()
    result, records = cli(source, runtime, "--output", str(source))
    assert result.returncode == 1 and not records
    assert source.read_bytes() == before


def test_missing_model_fails_closed(scanned, runtime, tmp_path):
    altered = dict(runtime, models=str(tmp_path))
    result, records = cli(scanned[0], altered)
    assert result.returncode == 1
    assert records[-1] == {"event": "error", "error": "FileNotFoundError"}


def test_nonregular_input_rejected_without_blocking(tmp_path):
    fifo = tmp_path / "fifo"
    os.mkfifo(fifo)
    with pytest.raises(ocr.OcrError, match="input_not_regular_file"):
        ocr.snapshot(fifo, tmp_path / "copy", ocr.Supervisor(0.3))


@pytest.mark.parametrize("mode", ["timeout", "cancelled"])
def test_supervisor_kills_process_group(tmp_path, mode):
    ocr.enable_subreaper()
    marker = tmp_path / "pids"
    code = (
        "import os,subprocess,sys,time; "
        "p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(60)']); "
        "open(sys.argv[1],'w').write(str(os.getpid())+' '+str(p.pid)); time.sleep(60)"
    )
    supervisor = ocr.Supervisor(0.6 if mode == "timeout" else 10)
    if mode == "cancelled":
        timer = threading.Timer(0.6, supervisor.cancelled.set)
        timer.start()
    started = time.monotonic()
    with pytest.raises(ocr.OcrError, match=mode):
        supervisor.run([sys.executable, "-c", code, str(marker)], tmp_path, 256)
    assert time.monotonic() - started < 3
    assert marker.exists()
    for pid in map(int, marker.read_text().split()):
        status = Path(f"/proc/{pid}/stat")
        assert not status.exists()
    if mode == "cancelled":
        timer.join()


def test_cli_sigterm_reaps_page_and_native_processes(scanned, runtime, tmp_path):
    # A real high-resolution raster keeps a native worker active while we cancel.
    output = tmp_path / "cancel.jsonl"
    command = [
        sys.executable,
        str(SCRIPT),
        str(scanned[1]),
        "--tessdata-dir",
        runtime["models"],
        "--pdftoppm",
        runtime["pdftoppm"],
        "--pdfinfo",
        runtime["pdfinfo"],
        "--dpi",
        "1500",
        "--output",
        str(output),
    ]
    process = subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)  # noqa: S603
    deadline = time.monotonic() + 5
    descendants = []
    while time.monotonic() < deadline:
        # This VM's procfs omits task/*/children; stat PPIDs cover thread-spawned children.
        parents = {}
        for status in Path("/proc").glob("[0-9]*/stat"):
            try:
                parents[status.parent.name] = status.read_text().rpartition(")")[2].split()[1]
            except OSError:
                continue
        descendants = [pid for pid, parent in parents.items() if parent == str(process.pid)]
        native_children = [pid for pid, parent in parents.items() if parent in descendants]
        if output.exists() and b'"event": "start"' in output.read_bytes() and native_children:
            descendants.extend(native_children)
            break
        time.sleep(0.01)
    assert descendants and native_children, "expected live page worker and native descendant"
    process.send_signal(signal.SIGTERM)
    _, stderr = process.communicate(timeout=5)
    assert process.returncode == 143, stderr
    records = [json.loads(line) for line in output.read_bytes().splitlines()]
    assert records[-1]["outcome"] == "cancelled"
    assert all(not Path(f"/proc/{pid}").exists() for pid in descendants)


def test_native_address_space_limit_is_enforced(tmp_path):
    supervisor = ocr.Supervisor(5)
    with pytest.raises(ocr.OcrError, match="worker_exit"):
        supervisor.run([sys.executable, "-c", "x=bytearray(512*1024*1024)"], tmp_path, 128)


def test_backpressure_honors_deadline():
    read_fd, write_fd = os.pipe()
    try:
        os.set_blocking(write_fd, False)
        with (
            os.fdopen(write_fd, "wb", buffering=0) as output,
            pytest.raises(ocr.OcrError, match="timeout"),
        ):
            ocr.write_output(output, b"x" * (1024 * 1024), ocr.Supervisor(0.1))
    finally:
        os.close(read_fd)


def test_nonfinite_confidence_is_not_valid_geometry(tmp_path):
    tsv = tmp_path / "bad.tsv"
    tsv.write_text(
        "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n"
        "1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n"
        "5\t1\t1\t1\t1\t1\t1\t1\t10\t10\tNaN\tword\n"
    )
    with pytest.raises(ocr.OcrError, match="invalid_word_confidence"):
        ocr.read_tsv(tsv)
