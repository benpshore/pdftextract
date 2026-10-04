#!/usr/bin/env python3
"""Run a finite local CPU GROBID qualification using only an authored fixture.

No user-document argument is accepted. Docker is an explicit prerequisite; this
script does not modify its daemon, networks, credentials, or host settings.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import signal
import subprocess
import time
import uuid
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from pathlib import Path

IMAGE = "grobid/grobid@sha256:223957791ac2bbe48609dcc58a689b16b60baeae13a8734ef440ae6bfb38f4cd"
CONFIG_PATH = "/opt/grobid/grobid-home/config/grobid.yaml"
NAMESPACE = {"tei": "http://www.tei-c.org/ns/1.0"}
JVM_OPTIONS = "-Xms128m -Xmx2g -XX:ActiveProcessorCount=2"


def command(arguments: list[str], *, timeout: float = 60) -> str:
    # Argument arrays only; no shell expansion or inherited terminal interaction.
    return subprocess.run(  # noqa: S603
        arguments, check=True, capture_output=True, text=True, timeout=timeout
    ).stdout


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def bounded_config(original: str) -> str:
    """Change only verified keys in the digest-pinned distribution's YAML."""
    replacements = {
        "concurrency": "1",
        "poolMaxWait": "1",
        "memoryLimitMb": "1024",
        "timeoutSec": None,
        "nbThreads": "1",
        "modelPreload": "false",
    }
    for key, value in replacements.items():
        if value is None:
            continue
        pattern = rf"(?m)^(\s*{key}:) .*$"
        original, count = re.subn(pattern, rf"\g<1> {value}", original)
        if count != 1:
            raise ValueError(f"expected one upstream {key}, found {count}")
    # Only PDFalto's timeout is changed, not unrelated consolidation timeouts.
    original, count = re.subn(r"(?m)^(      timeoutSec:) 120$", r"\g<1> 30", original)
    if count != 1:
        raise ValueError("unexpected upstream PDFalto timeout")
    if re.search(r'^\s+engine: "delft"', original, flags=re.MULTILINE):
        raise ValueError("expected CPU CRF models only")
    return original


def scholarly_pdf() -> bytes:
    """Deterministic, independently authored two-page PDF; no external assets."""
    pages = [
        [
            (18, "Synthetic Study of Reproducible Document Processing"),
            (12, "Alice Example and Ben Sample"),
            (11, "Department of Computer Science, Example University"),
            (11, "October 2026"),
            (14, "Abstract"),
            (11, "We describe a benign synthetic study of reproducible document processing."),
            (11, "This fixture checks retained scholarly structure and page coordinates."),
            (14, "1 Introduction"),
            (11, "Document processing requires explicit provenance and preserved originals."),
            (11, "Example and Sample (2024) describe a controlled validation procedure."),
            (11, "The present study measures a local CPU service with no metadata lookup."),
            (14, "2 Methods"),
            (11, "We generated two pages containing a title, abstract, methods and references."),
            (11, "All paragraphs and reference records were written for this software test."),
            (11, "The original bytes remain unchanged during repeated structured extraction."),
        ],
        [
            (14, "3 Results"),
            (11, "The experiment reports cold and warm service request times separately."),
            (11, "A successful request is not a claim of exhaustive text or citation accuracy."),
            (11, "The CPU runtime returns page surfaces and region coordinates in raw TEI."),
            (14, "4 Discussion"),
            (11, "Learned extraction can omit or misclassify synthetic scholarly fields."),
            (11, "Raw evidence is retained so that downstream projections can be audited."),
            (14, "References"),
            (11, "Example, A. and Sample, B. (2024). Reproducible document processing."),
            (11, "Journal of Synthetic Methods, 12(3), 100-110. doi:10.5555/synthetic.2024."),
            (11, "Reader, C. (2025). Provenance in scholarly extraction. Test Research,"),
            (11, "7(2), 22-30. doi:10.5555/synthetic.2025."),
        ],
    ]
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Count 2 /Kids [4 0 R 6 0 R] >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>",
    ]
    for number, lines in enumerate(pages):
        content_id = 5 + number * 2
        objects.append(
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            b"/Resources << /Font << /F1 3 0 R >> >> /Contents " + f"{content_id} 0 R >>".encode()
        )
        content = []
        y = 730
        for font_size, line in lines:
            escaped = line.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
            content.append(f"BT /F1 {font_size} Tf 1 0 0 1 50 {y} Tm ({escaped}) Tj ET")
            y -= 28 if font_size >= 14 else 21
        stream = "\n".join(content).encode("ascii")
        objects.append(f"<< /Length {len(stream)} >>\nstream\n".encode() + stream + b"\nendstream")
    data = bytearray(b"%PDF-1.4\n")
    offsets = [0]
    for index, obj in enumerate(objects, 1):
        offsets.append(len(data))
        data.extend(f"{index} 0 obj\n".encode() + obj + b"\nendobj\n")
    xref = len(data)
    data.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
    for offset in offsets[1:]:
        data.extend(f"{offset:010d} 00000 n \n".encode())
    data.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return bytes(data)


def request(
    url: str, *, body: bytes | None = None, content_type: str | None = None, timeout: float = 90
) -> bytes:
    # curl's --max-time covers the entire transfer, unlike a socket-read timeout.
    # The supervising subprocess deadline is an independent finite wall clock.
    arguments = [
        "curl",
        "--disable",
        "--fail",
        "--silent",
        "--show-error",
        "--max-time",
        str(timeout),
        "--connect-timeout",
        str(min(timeout, 2)),
        "--noproxy",
        "*",
        "--proto",
        "=http",
        "--write-out",
        "\n%{http_code}",
    ]
    if content_type:
        arguments.extend(["--header", f"Content-Type: {content_type}"])
    if body is not None:
        arguments.extend(["--data-binary", "@-"])
    arguments.append(url)
    # No --location: redirects cannot carry a document to another endpoint.
    result = subprocess.run(  # noqa: S603
        arguments, input=body, check=True, capture_output=True, timeout=timeout + 1
    ).stdout
    content, status = result.rsplit(b"\n", 1)
    if status != b"200":
        raise RuntimeError(f"unexpected HTTP status: {status.decode('ascii')}")
    return content


def multipart(pdf: bytes) -> tuple[bytes, str]:
    boundary = "tpe-synthetic-grobid-fixture"
    fields = [
        ("consolidateHeader", "0"),
        ("consolidateCitations", "0"),
        ("consolidateFunders", "0"),
        ("includeRawCitations", "1"),
        ("includeRawAffiliations", "1"),
        ("includeRawCopyrights", "1"),
        ("generateIDs", "1"),
    ] + [
        ("teiCoordinates", value)
        for value in (
            "head",
            "p",
            "s",
            "ref",
            "biblStruct",
            "figure",
            "formula",
            "title",
            "persName",
            "affiliation",
            "note",
        )
    ]
    body = bytearray()
    for name, value in fields:
        body.extend(
            f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n'
            f"{value}\r\n".encode()
        )
    body.extend(
        f'--{boundary}\r\nContent-Disposition: form-data; name="input"; '
        'filename="synthetic-scholarly.pdf"\r\nContent-Type: application/pdf\r\n\r\n'.encode()
    )
    body.extend(pdf)
    body.extend(f"\r\n--{boundary}--\r\n".encode())
    return bytes(body), f"multipart/form-data; boundary={boundary}"


def summarize_tei(tei: bytes) -> dict:
    # Only the digest-pinned local service processes the authored fixture here.
    root = ET.fromstring(tei)  # noqa: S314
    text = " ".join(root.itertext())
    surfaces = [dict(node.attrib) for node in root.findall(".//tei:surface", NAMESPACE)]
    coordinates = [
        node.attrib["coords"] for node in root.iter() if node.attrib.get("coords", "").strip()
    ]
    empty_coordinate_attributes = sum(
        1 for node in root.iter() if "coords" in node.attrib and not node.attrib["coords"].strip()
    )
    normalized_text = " ".join(text.split())
    return {
        "sha256": sha256(tei),
        "bytes": len(tei),
        "surfaces": surfaces,
        "coordinate_regions": len(coordinates),
        "empty_coordinate_attributes": empty_coordinate_attributes,
        "coordinate_pages": sorted(
            {int(box.split(",")[0]) for c in coordinates for box in c.split(";")}
        ),
        "references": len(root.findall(".//tei:listBibl/tei:biblStruct", NAMESPACE)),
        "title_retained": "Synthetic Study of Reproducible Document Processing" in text,
        "first_doi_retained": "10.5555/synthetic.2024" in text,
        "second_doi_retained": "10.5555/synthetic.2025" in text,
        "second_abstract_sentence_retained": (
            "This fixture checks retained scholarly structure and page coordinates."
            in normalized_text
        ),
        "introduction_classified_as_heading": any(
            "Introduction" in "".join(node.itertext())
            for node in root.findall(".//tei:head", NAMESPACE)
        ),
        "warning": "Semantic model projection; not OCR or exhaustive PDF text coverage.",
    }


def qualify(output_dir: Path, tpe: Path | None = None) -> dict:
    output_dir.mkdir(parents=True, exist_ok=False)
    name = f"tpe-grobid-cpu-{uuid.uuid4().hex[:12]}"
    report = {"started_utc": datetime.now(UTC).isoformat(), "image": IMAGE, "container": name}
    report["runner_sha256"] = sha256(Path(__file__).read_bytes())
    created = False
    caught = None
    try:
        image_info = json.loads(command(["docker", "image", "inspect", IMAGE]))[0]
        report["image_id"] = image_info["Id"]
        report["platform"] = f"{image_info['Os']}/{image_info['Architecture']}"
        original = command(
            [
                "docker",
                "run",
                "--rm",
                "--network",
                "none",
                "--cpus",
                "1",
                "--memory",
                "512m",
                "--entrypoint",
                "cat",
                IMAGE,
                CONFIG_PATH,
            ]
        )
        configuration = bounded_config(original).encode()
        config_file = output_dir / "grobid.yaml"
        config_file.write_bytes(configuration)
        report["configuration_sha256"] = sha256(configuration)
        report["limits"] = {
            "cpus": 2,
            "memory_bytes": 4 * 1024**3,
            "jvm_heap": "2g",
            "pids": 128,
            "concurrency": 1,
            "pdfalto_timeout_seconds": 30,
            "pdfalto_memory_mib": 1024,
            "model_preload": False,
            "consolidation": 0,
        }
        created = True
        command(
            [
                "docker",
                "create",
                "--name",
                name,
                "--cpus",
                "2",
                "--memory",
                "4g",
                "--memory-swap",
                "4g",
                "--pids-limit",
                "128",
                "--ulimit",
                "core=0",
                "--publish",
                "127.0.0.1::8070",
                "--env",
                f"JAVA_OPTS={JVM_OPTIONS}",
                IMAGE,
            ]
        )
        command(["docker", "cp", str(config_file), f"{name}:{CONFIG_PATH}"])
        start = time.monotonic()
        command(["docker", "start", name])
        binding = json.loads(command(["docker", "inspect", name]))[0]["NetworkSettings"]["Ports"][
            "8070/tcp"
        ][0]
        if binding["HostIp"] != "127.0.0.1":
            raise RuntimeError("service binding is not loopback")
        endpoint = f"http://127.0.0.1:{int(binding['HostPort'])}"
        report["endpoint"] = endpoint
        deadline = time.monotonic() + 120
        while True:
            try:
                if request(endpoint + "/api/isalive", timeout=1).strip() == b"true":
                    break
            except subprocess.SubprocessError:
                pass
            if time.monotonic() >= deadline:
                raise TimeoutError("GROBID readiness exceeded 120 seconds")
            time.sleep(0.25)
        report["startup_seconds"] = time.monotonic() - start
        report["reported_version"] = request(endpoint + "/api/version").decode()
        report["runtime_identity"] = command(
            [
                "docker",
                "exec",
                name,
                "sh",
                "-c",
                "java -version 2>&1; sha256sum grobid-home/models/*/model.wapiti "
                "grobid-home/models/*/*/model.wapiti grobid-home/pdfalto/lin-64/pdfalto "
                "grobid-home/lib/lin-64/libwapiti.so",
            ]
        )
        pdf = scholarly_pdf()
        source = output_dir / "synthetic-scholarly.pdf"
        source.write_bytes(pdf)
        report["input_sha256"] = sha256(pdf)
        body, content_type = multipart(pdf)
        report["requests"] = []
        for label in ("cold", "warm-1", "warm-2"):
            start = time.monotonic()
            tei = request(
                endpoint + "/api/processFulltextDocument", body=body, content_type=content_type
            )
            elapsed = time.monotonic() - start
            (output_dir / f"{label}.tei.xml").write_bytes(tei)
            report["requests"].append({"label": label, "seconds": elapsed, **summarize_tei(tei)})
        if tpe is not None:
            report["rust_cli"] = []
            environment = {**os.environ, "TPE_GROBID_URL": endpoint}
            environment.pop("TPE_GROBID_BEARER_TOKEN", None)
            for label in ("rust-client-1", "rust-client-2"):
                start = time.monotonic()
                result = subprocess.run(  # noqa: S603
                    [str(tpe), "grobid", "--consolidation", "0", str(source)],
                    env=environment,
                    capture_output=True,
                    check=False,
                    timeout=65,
                )
                elapsed = time.monotonic() - start
                (output_dir / f"{label}.stderr.log").write_bytes(result.stderr)
                if result.returncode != 0:
                    report["rust_cli_error"] = result.stderr.decode("utf-8", errors="replace")
                    raise RuntimeError(
                        f"Rust GROBID client exited {result.returncode}; see stderr log"
                    )
                (output_dir / f"{label}.json").write_bytes(result.stdout)
                decoded = json.loads(result.stdout)
                report["rust_cli"].append(
                    {
                        "label": label,
                        "seconds": elapsed,
                        "output_sha256": sha256(result.stdout),
                        "output_bytes": len(result.stdout),
                        "input_sha256_matches": decoded.get("source_sha256") == sha256(pdf),
                        "tei_sha256_matches": decoded.get("tei_sha256")
                        == sha256(decoded["raw_tei"].encode()),
                        "semantic_projection": decoded.get("coverage") == "semantic_projection",
                        "pages": len(decoded["pages"]),
                        "citations": len(decoded["citations"]),
                        "warnings": decoded["warnings"],
                        "top_level_keys": sorted(decoded),
                    }
                )
            report["rust_cli_binary_sha256"] = sha256(tpe.read_bytes())
        report["original_unchanged"] = source.read_bytes() == pdf
        report["container_stats_after_requests"] = command(
            ["docker", "stats", "--no-stream", "--format", "{{json .}}", name]
        ).strip()
        report["cgroup_memory_peak_bytes"] = int(
            command(["docker", "exec", name, "cat", "/sys/fs/cgroup/memory.peak"]).strip()
        )
        report["effective_host_limits"] = {
            key: value
            for key, value in json.loads(command(["docker", "inspect", name]))[0][
                "HostConfig"
            ].items()
            if key
            in ("Memory", "MemorySwap", "NanoCpus", "PidsLimit", "PortBindings", "DeviceRequests")
        }
        report["status"] = (
            "passed"
            if all(
                row["title_retained"]
                and row["coordinate_pages"] == [1, 2]
                and len(row["surfaces"]) == 2
                and row["references"] == 2
                and row["first_doi_retained"]
                and row["second_doi_retained"]
                for row in report["requests"]
            )
            and report["original_unchanged"]
            and all(
                row["input_sha256_matches"]
                and row["tei_sha256_matches"]
                and row["semantic_projection"]
                and row["pages"] == 2
                and row["citations"] == 2
                and row["warnings"]
                for row in report.get("rust_cli", [])
            )
            else "failed_acceptance"
        )
    except BaseException as error:
        report["status"] = "interrupted" if isinstance(error, KeyboardInterrupt) else "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        caught = error
    finally:
        source_file = output_dir / "synthetic-scholarly.pdf"
        if "input_sha256" in report:
            try:
                report["original_unchanged"] = (
                    sha256(source_file.read_bytes()) == report["input_sha256"]
                )
            except OSError as error:
                report["original_unchanged"] = False
                report["source_check_error"] = str(error)
        if created:
            try:
                log_command = ["docker", "logs", name]
                logs = subprocess.run(  # noqa: S603
                    log_command,
                    capture_output=True,
                    timeout=20,
                )
                (output_dir / "service.log").write_bytes(logs.stdout + logs.stderr)
            except (OSError, subprocess.SubprocessError) as error:
                report["log_capture_error"] = f"{type(error).__name__}: {error}"
            try:
                command(["docker", "rm", "--force", name], timeout=30)
                report["container_removed"] = True
            except (OSError, subprocess.SubprocessError) as error:
                report["container_removed"] = False
                report["cleanup_error"] = f"{type(error).__name__}: {error}"
                report["status"] = "cleanup_failed"
        report["finished_utc"] = datetime.now(UTC).isoformat()
        (output_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    if caught is not None:
        raise caught
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True, help="New evidence directory")
    parser.add_argument("--tpe", type=Path, help="Built Rust tpe binary with grobid feature")
    args = parser.parse_args()

    def interrupted(signum, frame):
        raise KeyboardInterrupt(f"received signal {signum}")

    signal.signal(signal.SIGTERM, interrupted)
    report = qualify(args.output_dir.resolve(), args.tpe.resolve() if args.tpe else None)
    print(json.dumps(report, indent=2))
    if report["status"] != "passed":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
