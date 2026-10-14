"""The Python launcher must preserve the native CLI's process boundary."""

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
