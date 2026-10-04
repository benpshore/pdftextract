"""Focused harness regressions; these do not stand in for a real GROBID run."""

import importlib.util
import json
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location(
    "grobid_cpu_runtime", Path(__file__).parents[1] / "scripts" / "grobid_cpu_runtime.py"
)
runtime = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runtime)


def test_fixture_is_stable_and_consolidation_disabled():
    pdf = runtime.scholarly_pdf()
    assert runtime.sha256(pdf) == "cc9f98b7c98aedfb6d48cb12b49354dc38b4bd5683eac131216cea18e42794fa"
    body, content_type = runtime.multipart(pdf)
    assert pdf in body
    assert "multipart/form-data" in content_type
    for name in ("consolidateHeader", "consolidateCitations", "consolidateFunders"):
        assert f'name="{name}"\r\n\r\n0\r\n'.encode() in body


def test_transfer_deadline_and_redirect_refusal(tmp_path, monkeypatch):
    seen = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            seen.append(self.path)
            self.send_response(302 if self.path == "/redirect" else 200)
            self.send_header("Location", "/target")
            self.end_headers()
            try:
                for _ in range(100):
                    self.wfile.write(b"x")
                    self.wfile.flush()
                    time.sleep(0.03)
            except BrokenPipeError, ConnectionResetError:
                pass

    # A user curl configuration must not turn on redirect following.
    (tmp_path / ".curlrc").write_text("location\n")
    monkeypatch.setenv("CURL_HOME", str(tmp_path))
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    endpoint = f"http://127.0.0.1:{server.server_port}"
    try:
        started = time.monotonic()
        with pytest.raises(subprocess.CalledProcessError):
            runtime.request(endpoint + "/slow", timeout=0.15)
        assert time.monotonic() - started < 2
        with pytest.raises(RuntimeError, match="302"):
            runtime.request(endpoint + "/redirect", timeout=5)
        assert seen == ["/slow", "/redirect"]
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def test_cleanup_failure_keeps_original_error_and_report(tmp_path, monkeypatch):
    configuration = """grobid:
  concurrency: 10
  poolMaxWait: 1
      memoryLimitMb: 6096
      timeoutSec: 120
    nbThreads: 0
  modelPreload: true
"""

    def command(args, **kwargs):
        if args[1:3] == ["image", "inspect"]:
            return json.dumps([{"Id": "image", "Os": "linux", "Architecture": "amd64"}])
        if args[1] == "run":
            return configuration
        if args[1] == "create":
            raise RuntimeError("original create failure")
        if args[1] == "rm":
            raise subprocess.TimeoutExpired(args, 30)
        raise AssertionError(args)

    monkeypatch.setattr(runtime, "command", command)
    monkeypatch.setattr(
        runtime.subprocess,
        "run",
        lambda *args, **kwargs: subprocess.CompletedProcess(args, 0, b"", b""),
    )
    output = tmp_path / "qualification"
    with pytest.raises(RuntimeError, match="original create failure"):
        runtime.qualify(output)
    report = json.loads((output / "report.json").read_text())
    assert report["error"] == "RuntimeError: original create failure"
    assert report["status"] == "cleanup_failed"
    assert report["container_removed"] is False
    assert "TimeoutExpired" in report["cleanup_error"]
