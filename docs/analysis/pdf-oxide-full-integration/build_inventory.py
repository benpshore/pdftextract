"""Rebuild the pinned-source index without compiling or importing either crate.

Run with uv and pass the extracted registry roots and locked cargo metadata.
This is a lexical source index, not rustdoc or proof of public reachability.
Conditional code, private-module reexports, and trait implementations are retained
for review. Comments/string literals are masked so documentation examples are not
counted as declarations. No dependency is downloaded by this script.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tomllib
from collections import Counter
from pathlib import Path

RAW_START = re.compile(r'r(#{0,255})"')
CHAR_LITERAL = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def mask_non_code(source: str) -> str:
    """Keep code offsets/newlines while masking Rust comments and literals."""
    chars = list(source)
    i = 0
    n = len(source)
    while i < n:
        end = i
        if source.startswith("//", i):
            end = source.find("\n", i)
            if end == -1:
                end = n
        elif source.startswith("/*", i):
            depth = 1
            end = i + 2
            while end < n and depth:
                if source.startswith("/*", end):
                    depth += 1
                    end += 2
                elif source.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
        elif source[i] == "r" and (match := RAW_START.match(source, i)):
            terminator = '"' + match[1]
            end = source.find(terminator, i + len(match[0]))
            end = n if end == -1 else end + len(terminator)
        elif source[i] == '"':
            end = i + 1
            while end < n:
                if source[end] == "\\":
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        elif source[i] == "'":
            # Lifetimes are code. Only mask a complete character literal.
            match = CHAR_LITERAL.match(source, i)
            if match:
                end = i + len(match[0])
        if end > i:
            for pos in range(i, min(end, n)):
                if chars[pos] != "\n":
                    chars[pos] = " "
            i = end
        else:
            i += 1
    return "".join(chars)


def brace_end(code: str, start: int) -> int:
    depth = 0
    for pos in range(start, len(code)):
        if code[pos] == "{":
            depth += 1
        elif code[pos] == "}":
            depth -= 1
            if depth == 0:
                return pos + 1
    return len(code)


def attributes_before(source: str, offset: int) -> list[str]:
    """Capture contiguous attributes, skipping ordinary/doc comments."""
    lines = source[:offset].splitlines()
    attrs: list[str] = []
    pending: list[str] = []
    for line in reversed(lines):
        s = line.strip()
        if not s or s.startswith("//"):
            continue
        if s.endswith("]") or pending:
            pending.insert(0, s)
            if s.startswith("#["):
                attrs.insert(0, " ".join(pending))
                pending = []
            elif not pending or len(pending) > 12:
                break
            continue
        break
    return attrs


PUBLIC = re.compile(
    r"\bpub\s+(?:(?:async|unsafe|const|extern)\s+)*"
    r"(?P<kind>fn|struct|enum|trait|type|const|static|mod|use)\s+"
    r"(?P<name>[A-Za-z_][A-Za-z_0-9]*)"
)
FIELD = re.compile(r"(?m)^[ \t]*pub\s+(?P<name>[A-Za-z_][A-Za-z_0-9]*)\s*:(?!:)")
TRAIT_MEMBER = re.compile(r"\b(?P<kind>fn|type|const)\s+(?P<name>[A-Za-z_][A-Za-z_0-9]*)")


def declarations(path: Path, root: Path, package: str) -> list[dict]:
    source = path.read_text()
    code = mask_non_code(source)
    relative = path.relative_to(root).as_posix()
    tests = []
    for match in re.finditer(r"\bmod\s+(?:tests|test_[A-Za-z_0-9]+)\s*\{", code):
        attrs = attributes_before(source, match.start())
        if any("cfg(test)" in value or "cfg(all(test" in value for value in attrs):
            tests.append((match.start(), brace_end(code, code.index("{", match.start()))))
    result = []
    matches = [(match, None) for match in list(PUBLIC.finditer(code)) + list(FIELD.finditer(code))]
    # Trait declarations expose methods without a `pub` keyword. Record those
    # separately from implementing methods; do not infer other visibility.
    for trait in PUBLIC.finditer(code):
        if trait["kind"] != "trait":
            continue
        start = code.find("{", trait.end())
        if start == -1:
            continue
        end = brace_end(code, start)
        for member in TRAIT_MEMBER.finditer(code, start + 1, end - 1):
            nesting = code.count("{", start, member.start()) - code.count(
                "}", start, member.start()
            )
            if nesting == 1:
                matches.append((member, trait["name"]))
    for match, trait_name in matches:
        pos = match.start()
        kind = match.groupdict().get("kind") or "field"
        if kind == "use":
            end = code.find(";", match.end()) + 1
        elif kind == "field":
            end = source.find("\n", match.end())
        else:
            ends = [
                offset
                for token in ("{", ";", "\n")
                if (offset := code.find(token, match.end())) != -1
            ]
            # Multiline function/struct headers continue until a body or semicolon.
            if kind in ("fn", "struct", "enum", "trait", "mod"):
                ends = [
                    offset
                    for token in ("{", ";")
                    if (offset := code.find(token, match.end())) != -1
                ]
            end = min(ends) if ends else len(code)
        if end <= pos:
            end = match.end()
        signature = " ".join(source[pos:end].split())
        if len(signature) > 4000:
            signature = signature[:4000]
        item = {
            "package": package,
            "file": relative,
            "line": source.count("\n", 0, pos) + 1,
            "kind": kind,
            "name": match["name"],
            "declaration": signature,
            "attributes": attributes_before(source, pos),
            "test_only": any(start <= pos < end for start, end in tests),
            "binding_source": relative.startswith(("src/python", "src/wasm", "src/ffi")),
            "binary_source": relative.startswith("src/bin/"),
        }
        if trait_name:
            item["kind"] = "trait_" + kind
            item["trait"] = trait_name
        result.append(item)
    return sorted(result, key=lambda value: (value["file"], value["line"], value["kind"]))


def package_record(root: Path, lock: dict, cargo_home: Path) -> tuple[dict, list[dict]]:
    manifest = tomllib.loads((root / "Cargo.toml").read_text())
    package = manifest["package"]
    name, version = package["name"], package["version"]
    locked = next(
        item for item in lock["package"] if item["name"] == name and item["version"] == version
    )
    files = []
    symbols = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        relative = path.relative_to(root).as_posix()
        if relative.startswith("src/") or relative in {
            "Cargo.toml",
            "Cargo.toml.orig",
            ".cargo_vcs_info.json",
            "LICENSE-MIT",
            "LICENSE-APACHE",
            "NOTICE",
            "TRADEMARKS.md",
        }:
            raw = path.read_bytes()
            files.append({"path": relative, "bytes": len(raw), "sha256": digest(raw)})
        if relative.startswith("src/") and path.suffix == ".rs":
            symbols.extend(declarations(path, root, name))
    source_hash = digest("".join(f"{item['path']}\0{item['sha256']}\n" for item in files).encode())
    archives = sorted((cargo_home / "registry/cache").glob(f"*/{name}-{version}.crate"))
    archive_hash = digest(archives[0].read_bytes()) if len(archives) == 1 else None
    if archive_hash != locked["checksum"]:
        raise ValueError(f"{name} archive does not match Cargo.lock: {archive_hash}")
    record = {
        "name": name,
        "version": version,
        "license": package["license"],
        "declared_rust_version": package.get("rust-version"),
        "edition": package.get("edition"),
        "repository": package.get("repository"),
        "vcs": json.loads((root / ".cargo_vcs_info.json").read_text()),
        "crate_archive_sha256": archive_hash,
        "cargo_lock_checksum_matches": True,
        "source_index_sha256": source_hash,
        "source_hash_recipe": (
            "SHA256 of sorted selected file records encoded as relative_path + NUL + "
            "file_sha256 + LF; includes src/**, manifests, VCS metadata, "
            "licenses/NOTICE/trademark file"
        ),
        "root_public_modules": [
            item
            for item in symbols
            if item["file"] == "src/lib.rs" and item["kind"] == "mod" and not item["test_only"]
        ],
        "root_reexports": [
            item
            for item in symbols
            if item["file"] == "src/lib.rs" and item["kind"] == "use" and not item["test_only"]
        ],
        "features": manifest.get("features", {}),
        "direct_dependency_declarations": manifest.get("dependencies", {}),
        "files": files,
        "counts": {
            "indexed_files": len(files),
            "rust_source_files": sum(item["path"].endswith(".rs") for item in files),
            "lexical_public_declarations": len(symbols),
            "declarations_by_kind": dict(sorted(Counter(item["kind"] for item in symbols).items())),
            "explicit_test_module_declarations": sum(item["test_only"] for item in symbols),
        },
    }
    return record, symbols


def lock_closure(metadata: dict, root_names: set[str]) -> dict:
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [identity for identity, package in packages.items() if package["name"] in root_names]
    seen = set()
    pending = list(roots)
    while pending:
        identity = pending.pop()
        if identity in seen:
            continue
        seen.add(identity)
        pending.extend(dep["pkg"] for dep in nodes[identity]["deps"])
    rows = []
    for identity in sorted(seen):
        package, node = packages[identity], nodes[identity]
        rows.append(
            {
                "name": package["name"],
                "version": package["version"],
                "source": package["source"],
                "license": package.get("license"),
                "rust_version": package.get("rust_version"),
                "features": node["features"],
            }
        )
    versions = [row["rust_version"] for row in rows if row["rust_version"]]
    return {
        "provenance": (
            "Existing cargo metadata --locked --format-version 1 --features pdf-oxide "
            "snapshot; follows resolved dependency edges. Cargo feature unification "
            "and target/dev edges in metadata can exceed the runtime dependency set."
        ),
        "root_packages": sorted(root_names),
        "resolved_packages": rows,
        "maximum_declared_rust_version_in_snapshot_closure": max(
            versions, key=lambda s: tuple(int(x) for x in s.split("."))
        ),
        "minimum_toolchain_verified": False,
    }


def finish_matrix(output: Path, roots: dict[str, Path], inventory: dict) -> None:
    spec = json.loads((output / "capability-spec.json").read_text())
    package_files = {
        package["name"]: {item["path"]: item for item in package["files"]}
        for package in inventory["packages"]
    }
    all_symbols = inventory["declarations"]
    unresolved_apis = []
    for capability in spec["capabilities"]:
        refs = []
        for item in capability["sources"]:
            package, relative = item.split(":", 1)
            path = roots[package] / relative
            if not path.is_file():
                path = path.with_suffix("") / "mod.rs"
            if not path.is_file():
                raise ValueError(f"Missing capability source {item}")
            relative = path.relative_to(roots[package]).as_posix()
            refs.append(
                {
                    "package": package,
                    "file": relative,
                    "sha256": package_files[package][relative]["sha256"],
                }
            )
        capability["sources"] = refs
        capability["evidence_state"] = spec["default_evidence_state"]
        capability["implementation_evidence"] = []
        capability["test_evidence"] = []
        capability["api_declarations"] = []
        for name in capability["apis"]:
            matches = [
                item for item in all_symbols if item["name"] == name and not item["test_only"]
            ]
            preferred = [
                item
                for item in matches
                if any(
                    ref["package"] == item["package"] and ref["file"] == item["file"]
                    for ref in refs
                )
            ]
            chosen = preferred or matches
            if not chosen:
                unresolved_apis.append({"capability": capability["id"], "name": name})
            capability["api_declarations"].extend(
                {
                    key: item[key]
                    for key in ("package", "file", "line", "kind", "name", "attributes")
                }
                for item in chosen
            )
    if unresolved_apis:
        raise ValueError(f"Names not present in lexical source index: {unresolved_apis}")
    spec["counts"] = {
        "capability_obligations": len(spec["capabilities"]),
        "by_group": dict(sorted(Counter(item["group"] for item in spec["capabilities"]).items())),
        "by_owner": dict(sorted(Counter(item["owner"] for item in spec["capabilities"]).items())),
    }
    (output / "capability-matrix.json").write_text(
        json.dumps(spec, indent=2, ensure_ascii=False) + "\n"
    )
    profiles = [
        (
            "core",
            [],
            (
                "Mandatory bounded "
                "worker/extraction/create/edit/Office/security/compliance tests; existing "
                "dependency feature profile has default-features=false."
            ),
        ),
        ("barcodes", ["barcodes"], "Local bounded QR/barcode generation; no scanning or network."),
        (
            "offline_signatures",
            ["signatures"],
            "Local structural/cryptographic verification; no TSA/network/private keys/trust claim.",
        ),
        (
            "rendering",
            ["rendering"],
            (
                "Requires reviewed filesystem/font confinement; includes system-fonts, "
                "jpeg2000 and JBIG2 decoder. Bounded page/pixel/output tests."
            ),
        ),
        (
            "icc_qcms",
            ["icc-qcms"],
            "Pure Rust-compatible qcms profile; compare CMYK/ICC output and unsupported profiles.",
        ),
        (
            "icc_lcms2",
            ["icc-lcms2"],
            (
                "Native library dependency must be recorded per platform; feature "
                "declaration is not a successful build."
            ),
        ),
        (
            "ocr_tract",
            ["ocr-tract"],
            (
                "No automatic fetch; approved hash-verified "
                "detector/recognizer/dictionary fixtures required for positive OCR "
                "evidence."
            ),
        ),
        (
            "ocr_ort",
            ["ocr"],
            (
                "ORT native runtime/model dependencies; contains network-capable prefetch "
                "helper which adapter must not call."
            ),
        ),
        (
            "ml",
            ["ml"],
            (
                "Includes ocr-tract and clustering dependencies; distinguish classical "
                "fallback from model inference."
            ),
        ),
        (
            "table_ml",
            ["table-ml"],
            (
                "Requires native PDFium/model path review and proof of actual model "
                "execution; no heuristic-as-ML claim."
            ),
        ),
        (
            "gpu",
            ["gpu"],
            (
                "ORT provider/device availability and memory limits need platform "
                "evidence; CPU fallback must be explicit."
            ),
        ),
        (
            "parallel",
            ["parallel"],
            (
                "Use existing bounded application queue; upstream whole-document/corpus "
                "Vec helpers are not streaming."
            ),
        ),
        (
            "legacy_crypto",
            ["legacy-crypto"],
            (
                "Explicit legacy PDF compatibility profile; incompatible with fips, no "
                "default enablement inferred."
            ),
        ),
        (
            "fips",
            ["fips", "icc"],
            (
                "Cannot combine legacy-crypto. Native AWS-LC/FIPS build and deployment "
                "validation are separate; no settings change authorized."
            ),
        ),
        (
            "fips_conflict_negative",
            ["fips", "legacy-crypto"],
            (
                "Expected compile_error in upstream lib.rs; never cargo --all-features as "
                "a positive acceptance check."
            ),
        ),
        (
            "python_upstream",
            ["python"],
            (
                "Separate Python worker: pulls rendering, parallel, logging, signatures, "
                "tsa-client, barcodes. Standard3.14 and free-threaded evidence must be "
                "distinguished."
            ),
        ),
        (
            "wasm",
            ["wasm"],
            (
                "Separate wasm32 compile/runtime ownership and bounds tests; native "
                "process supervisor does not apply."
            ),
        ),
        (
            "wasm_ocr",
            ["wasm-ocr"],
            (
                "WASM plus tract model-byte API; explicit browser/Node resource "
                "boundaries and models needed."
            ),
        ),
        (
            "tsa_client",
            ["tsa-client"],
            (
                "Network/credential-capable surface remains disabled in this integration; "
                "local DER parsing does not authorize acquisition."
            ),
        ),
    ]
    package = next(item for item in inventory["packages"] if item["name"] == "pdf_oxide")
    matrix = {
        "schema": 1,
        "status": "Proposed verification matrix, no feature build executed by inventory worker.",
        "upstream_features": package["features"],
        "profiles": [
            {
                "id": name,
                "upstream_features": features,
                "tests_and_constraints": note,
                "build_evidence": [],
                "test_evidence": [],
            }
            for name, features, note in profiles
        ],
        "platforms": [
            {
                "target": "Linux x86_64",
                "status": "Selected VM; exact adapter tests belong to integration owner",
                "obligations": [
                    "bounded disposable subprocess",
                    "filesystem/network/process confinement separately verified",
                    "font provenance",
                    "native library availability",
                ],
            },
            {
                "target": "Linux aarch64",
                "status": "Not tested by inventory worker",
                "obligations": [
                    "retain merged ARM64 PDFium fix",
                    "optional runtime/native dependency builds",
                ],
            },
            {
                "target": "macOS",
                "status": "Mac jobs paused; no Mac access authorized",
                "obligations": ["report unverified platform coverage; do not resume paused jobs"],
            },
            {
                "target": "Windows",
                "status": "Not tested",
                "obligations": [
                    "worker/job-object/resource/atomic publication semantics",
                    "font/runtime provenance",
                ],
            },
            {
                "target": "wasm32",
                "status": "Separate feature surface, no execution claimed",
                "obligations": [
                    "no native subprocess assumption",
                    "host API/memory boundary",
                    "explicit model bytes",
                ],
            },
        ],
        "minimum_supported_toolchain": {
            "package_declared": "1.88",
            "snapshot_closure_maximum_declared": inventory["locked_feature_snapshot"][
                "maximum_declared_rust_version_in_snapshot_closure"
            ],
            "compiled_on_minimum": False,
            "obligation": (
                "Qualify each enabled dependency/feature graph on the claimed minimum; "
                "baseline metadata alone does not qualify optional graphs."
            ),
        },
    }
    (output / "feature-build-matrix.json").write_text(
        json.dumps(matrix, indent=2, ensure_ascii=False) + "\n"
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pdf-root", type=Path, required=True)
    parser.add_argument("--office-root", type=Path, required=True)
    parser.add_argument("--cargo-home", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=Path(__file__).parent)
    args = parser.parse_args()
    lock = tomllib.loads(args.lock.read_text())
    packages, symbols = [], []
    for root in (args.pdf_root, args.office_root):
        record, declarations_ = package_record(root, lock, args.cargo_home)
        packages.append(record)
        symbols.extend(declarations_)
    inventory = {
        "inventory_schema": 1,
        "scope": (
            "Pinned registry sources. This declaration index does not claim every "
            "lexical pub item is reachable from a crate root, every macro expansion "
            "is indexed, every capability is implemented, or any test has passed."
        ),
        "method": (
            "All src/**/*.rs files scanned; Rust comments and string/character "
            "literals masked; lexical pub declarations/fields indexed with source "
            "lines and adjacent attributes. Explicit cfg(test) named test modules "
            "marked. All declarations retained, including private-module reexport "
            "candidates, conditional APIs, bindings and support binaries. Root "
            "module/reexport list and capability matrix identify integration "
            "surfaces."
        ),
        "excluded_from_claims": [
            "Compiler-expanded macros",
            "Effective visibility inferred across private modules",
            "Rustdoc type checking",
            "Runtime behavior from declaration names",
            "API/CLI/MCP parity",
            "Security certification",
            "Extraction accuracy",
        ],
        "cargo_lock_sha256": digest(args.lock.read_bytes()),
        "metadata_snapshot_sha256": digest(args.metadata.read_bytes()),
        "packages": packages,
        "locked_feature_snapshot": lock_closure(
            json.loads(args.metadata.read_text()), {"pdf_oxide", "office_oxide"}
        ),
        "declarations": symbols,
    }
    args.output.mkdir(parents=True, exist_ok=True)
    target = args.output / "source-inventory.json"
    target.write_text(json.dumps(inventory, indent=2, ensure_ascii=False) + "\n")
    finish_matrix(
        args.output, {"pdf_oxide": args.pdf_root, "office_oxide": args.office_root}, inventory
    )
    print(
        json.dumps(
            {
                "output": str(target),
                "sha256": digest(target.read_bytes()),
                "packages": [
                    {
                        "name": row["name"],
                        "version": row["version"],
                        "root_modules": len(row["root_public_modules"]),
                        **row["counts"],
                    }
                    for row in packages
                ],
                "msrv_snapshot": inventory["locked_feature_snapshot"][
                    "maximum_declared_rust_version_in_snapshot_closure"
                ],
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
