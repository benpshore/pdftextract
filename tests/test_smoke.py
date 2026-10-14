"""The Python launcher must preserve the native CLI's process boundary."""

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import text_processing_engine


def test_missing_engine_is_actionable_failure(monkeypatch, capsys):
    monkeypatch.setattr(text_processing_engine.shutil, "which", lambda name: None)
    with pytest.raises(SystemExit) as error:
        text_processing_engine.main()
    assert error.value.code == 127
    captured = capsys.readouterr()
    assert captured.out == ""
    assert "native tpe executable not found on PATH" in captured.err


def test_arguments_are_forwarded_without_shell_or_capture(monkeypatch):
    arguments = ["extract", "a ; $(echo nope).pdf", "--timeout-ms", "42", "--", "-file.pdf"]
    monkeypatch.setattr(text_processing_engine.sys, "argv", ["text-processing-engine", *arguments])
    monkeypatch.setattr(text_processing_engine.shutil, "which", lambda name: "/installed/tpe")
    calls = []
    monkeypatch.setattr(text_processing_engine.os, "execv", lambda *args: calls.append(args))
    text_processing_engine.main()
    assert calls == [("/installed/tpe", ["/installed/tpe", *arguments])]


def test_exec_failure_has_nonzero_status(monkeypatch, capsys):
    monkeypatch.setattr(text_processing_engine.shutil, "which", lambda name: "/installed/tpe")

    def fail(*args):
        raise PermissionError("execution denied")

    monkeypatch.setattr(text_processing_engine.os, "execv", fail)
    with pytest.raises(SystemExit) as error:
        text_processing_engine.main()
    assert error.value.code == 126
    assert "execution denied" in capsys.readouterr().err


@pytest.mark.skipif(os.name != "posix", reason="POSIX process replacement contract")
@pytest.mark.parametrize("entrypoint", ["module", "console"])
def test_real_launcher_preserves_process_io_and_status(tmp_path, entrypoint):
    engine = tmp_path / "tpe"
    engine.write_text(
        f"#!{sys.executable}\n"
        "import json, os, sys\n"
        "print(json.dumps([os.getpid(), sys.argv[1:], sys.stdin.read()]))\n"
        "print('native diagnostic', file=sys.stderr)\n"
        "sys.exit(23)\n"
    )
    engine.chmod(0o700)
    command = (
        [sys.executable, "-m", "text_processing_engine"]
        if entrypoint == "module"
        else [str(Path(sys.executable).parent / "text-processing-engine")]
    )
    arguments = ["extract", "spaces ; $(echo nope).pdf", "--timeout-ms", "42"]
    with subprocess.Popen(  # noqa: S603
        [*command, *arguments],
        env={**os.environ, "PATH": str(tmp_path)},
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    ) as process:
        stdout, stderr = process.communicate("input stream", timeout=10)
        assert process.returncode == 23
        assert json.loads(stdout) == [process.pid, arguments, "input stream"]
        assert stderr == "native diagnostic\n"
