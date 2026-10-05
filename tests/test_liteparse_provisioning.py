"""Execute provisioning with synthetic pinned bytes, never real native libraries.

The macOS fixture models a refusing sha256sum alias beside working shasum on Linux.
It proves option compatibility and fail-closed checks, not execution on a Mac.
"""

import hashlib
import io
import json
import os
import shutil
import subprocess
import tarfile
from pathlib import Path

import pytest


@pytest.mark.parametrize("checksum", ["gnu", "mac-wrapper", "shasum"])
@pytest.mark.parametrize("damage", [None, "archive", "library"])
def test_reviewed_archive_and_library_remain_required(tmp_path, checksum, damage):
    root = tmp_path / "fixture"
    native = root / "native"
    native.mkdir(parents=True)
    script = native / "fetch-liteparse.sh"
    shutil.copyfile(Path(__file__).parents[1] / "native/fetch-liteparse.sh", script)

    # These bytes are intentionally not executable. Nothing loads the fixture
    # library: the real shell script verifies, extracts and copies its bytes.
    library = b"synthetic reviewed library\n"
    member = "lib/libpdfium.dylib" if checksum == "mac-wrapper" else "lib/libpdfium.so"
    archive = root / "fixture.tgz"
    with tarfile.open(archive, "w:gz") as bundle:
        for name, content in [(member, library), ("include/fpdfview.h", b"fixture header\n")]:
            info = tarfile.TarInfo(name)
            info.size = len(content)
            bundle.addfile(info, io.BytesIO(content))
    row = {
        "kind": "pdfium",
        "platform": "mac-arm64" if checksum == "mac-wrapper" else "linux-x64",
        "url": "https://fixture.invalid/never-downloaded.tgz",
        "member": member,
        "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
        "sha256": hashlib.sha256(library).hexdigest(),
    }
    if damage == "library":
        row["sha256"] = "0" * 64
    # The production manifest's portable parser intentionally uses one JSON row
    # per line. Preserve that contract without modifying production pins.
    (native / "manifest.json").write_text(json.dumps(row) + "\n")
    if damage == "archive":
        archive.write_bytes(archive.read_bytes() + b"unreviewed mutation")

    # A curated PATH makes the checksum fallback deterministic. Required tools
    # are resolved from the trusted test environment, never from source input.
    tools = tmp_path / "tools"
    tools.mkdir()
    for command in ["dirname", "sed", "mktemp", "cp", "rm", "mkdir", "tar", "gzip"]:
        executable = shutil.which(command)
        assert executable, f"required fixture utility missing: {command}"
        (tools / command).symlink_to(executable)
    uname = tools / "uname"
    uname.write_text(
        '#!/bin/sh\ncase "$1" in\n'
        + (
            "-s) echo Darwin ;;\n-m) echo arm64 ;;\n"
            if checksum == "mac-wrapper"
            else "-s) echo Linux ;;\n-m) echo x86_64 ;;\n"
        )
        + "*) exit 2 ;;\nesac\n"
    )
    uname.chmod(0o755)
    if checksum in {"shasum", "mac-wrapper"}:
        executable = shutil.which("shasum")
        assert executable, "shasum required for preferred-interface qualification"
        (tools / "shasum").symlink_to(executable)
    if checksum == "gnu":
        # macOS has a non-GNU alias with this same name. Its mere presence must
        # not pass mismatch cases through option refusal or pretend to exercise
        # the GNU fallback. Linux jobs run these cases; macOS still runs all real
        # shasum/pin/mismatch cases and native provisioning on its actual runner.
        executable = shutil.which("gsha256sum") or shutil.which("sha256sum")
        if not executable:
            pytest.skip("GNU checksum fixture requires GNU sha256sum")
        version = subprocess.run(  # noqa: S603 - trusted host utility, fixed argument
            [executable, "--version"], capture_output=True, text=True, check=False, timeout=5
        )
        if version.returncode != 0 or "GNU coreutils" not in version.stdout:
            pytest.skip("GNU checksum fixture unavailable; shasum cases remain required")
        (tools / "sha256sum").symlink_to(executable)
    elif checksum == "mac-wrapper":
        # Hosted evidence disproved the earlier accepting-short-option model.
        # Both --check and -c failed there. A refusing alias must not preempt the
        # known-working shasum interface, whose real hash gate is exercised here.
        wrapper = tools / "sha256sum"
        wrapper.write_text("#!/bin/sh\necho 'usage: sha256sum [-bctwz] [files ...]' >&2\nexit 64\n")
        wrapper.chmod(0o755)
    environment = {**os.environ, "PATH": str(tools), "TMPDIR": str(tmp_path)}
    result = subprocess.run(  # noqa: S603 - repository script, synthetic trusted fixture paths
        ["/bin/sh", str(script), "--archive", str(archive)],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        check=False,
        timeout=10,
    )
    if damage:
        assert result.returncode != 0
        assert "FAILED" in result.stdout, "a pin mismatch must reach real checksum verification"
        assert "usage:" not in result.stdout + result.stderr
        assert not (root / ".pdfium").exists(), "mismatched bytes must never publish"
    else:
        assert result.returncode == 0, result.stderr
        assert (root / ".pdfium" / member).read_bytes() == library
        assert (root / ".pdfium/include/fpdfview.h").read_bytes() == b"fixture header\n"
    assert not list(tmp_path.glob("tpe-liteparse.*")), "trap must remove temporary extraction"
