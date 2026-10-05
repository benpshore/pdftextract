#!/bin/sh
# Dependency report for the Cargo workspace and the uv project. Read-only: it
# never runs `cargo update` and never writes Cargo.lock or any manifest.
#
#   sh scripts/deps-report.sh [--online] [--all-platforms] [--features LIST]
#
# Prints Markdown to stdout (redirect it, or paste the sections you need into
# docs/UPSTREAMS.md as proposals):
#   1. identity: commit, toolchain, host triple, Cargo.lock SHA-256
#   2. direct dependencies of every workspace crate: declared requirement,
#      locked version and, with --online, the newest crates.io release
#      (`cargo search`), classified as current / compatible / major behind
#   3. duplicate crate versions (`cargo tree -d`) and who requests each
#   4. license summary of the resolved graph (from cargo metadata; no extra
#      cargo subcommand needed) with a review list for non-permissive terms
#   5. crates that compile or link native code (cc, cmake, bindgen, *-sys)
#   6. Python: `uv tree` (plus `--outdated` and `uv audit` with --online)
# Default scope: the host target with default features, like `cargo build`.
# --all-platforms covers every target in Cargo.lock (may need the network to
# fetch manifests that were never built here). python3 (any 3.x) parses the
# metadata; without it sections 2 to 5 fall back to `cargo tree` summaries.
# POSIX sh; exit 1 when cargo metadata fails, 2 on usage error.

set -u
export RUSTUP_AUTO_INSTALL=0
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd) || exit 2
cd "$root" || exit 2

online=0
all_platforms=0
features=''
while [ $# -gt 0 ]; do
  case $1 in
    --online) online=1 ;;
    --all-platforms) all_platforms=1 ;;
    --features) [ $# -ge 2 ] || { printf 'deps-report: --features needs a value\n' >&2; exit 2; }; features=$2; shift ;;
    -h | --help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) printf 'deps-report: unknown option %s\n' "$1" >&2; exit 2 ;;
  esac
  shift
done

have() { command -v "$1" >/dev/null 2>&1; }
sha256_of() {
  if have sha256sum; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

host=$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')
commit=$(git rev-parse --short HEAD 2>/dev/null || printf unknown)
describe=$(git describe --tags --match 'v[0-9]*' --always --dirty 2>/dev/null || printf unknown)
stamp=$(date -u +%Y-%m-%dT%H:%M:%SZ)

printf '# Dependency report\n\n'
printf -- '- generated: %s (read-only; no `cargo update`, no lockfile or manifest change)\n' "$stamp"
printf -- '- source: %s (%s)\n' "$commit" "$describe"
printf -- '- toolchain: %s; %s; host %s\n' "$(rustc -V 2>/dev/null)" "$(cargo -V 2>/dev/null)" "${host:-unknown}"
printf -- '- Cargo.lock sha256: %s\n' "$(sha256_of Cargo.lock)"
if [ "$all_platforms" -eq 1 ]; then scope='every target in Cargo.lock'; else scope="host target ${host:-unknown}"; fi
if [ -n "$features" ]; then scope="$scope, features $features"; else scope="$scope, default features"; fi
printf -- '- scope: %s (normal, build and dev dependencies)\n' "$scope"
printf -- '- network: %s\n\n' "$([ "$online" -eq 1 ] && printf 'online (crates.io latest versions, uv outdated, uv audit)' || printf 'offline')"

meta=$(mktemp "${TMPDIR:-/tmp}/deps-report.XXXXXX") || exit 1
trap 'rm -f "$meta"' EXIT
set -- --format-version 1 --locked
[ "$all_platforms" -eq 1 ] || set -- "$@" --offline --filter-platform "${host:-x86_64-unknown-linux-gnu}"
[ -z "$features" ] || set -- "$@" --features "$features"
if ! cargo metadata "$@" >"$meta" 2>"$meta.err"; then
  printf 'cargo metadata failed:\n' >&2
  cat "$meta.err" >&2
  rm -f "$meta.err"
  exit 1
fi
rm -f "$meta.err"

py=''
if have python3; then py=python3; elif have uv && uv python find >/dev/null 2>&1; then py="uv run --no-project python"; fi

if [ -n "$py" ]; then
  # shellcheck disable=SC2086 # $py may be a word list (uv run ... python)
  $py - "$meta" "$online" <<'PY'
import json, re, subprocess, sys
from collections import defaultdict

meta = json.load(open(sys.argv[1]))
online = sys.argv[2] == "1"
packages = {p["id"]: p for p in meta["packages"]}
members = set(meta["workspace_members"])
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
by_name = defaultdict(set)
dependents = defaultdict(set)
for node in nodes.values():
    by_name[packages[node["id"]]["name"]].add(packages[node["id"]]["version"])
    for dep in node["deps"]:
        dependents[dep["pkg"]].add(node["id"])

def label(pid):
    p = packages[pid]
    return f"{p['name']} {p['version']}"

def parse(v):
    core = re.split(r"[-+]", v)[0]
    parts = [int(x) if x.isdigit() else 0 for x in core.split(".")]
    while len(parts) < 3:
        parts.append(0)
    return tuple(parts[:3]), "-" in v

def classify(locked, latest):
    if latest is None:
        return "lookup failed"
    l, _ = parse(locked)
    n, pre = parse(latest)
    if n == l:
        return "current"
    if n < l:
        return "locked is newer than the search result"
    tag = " (prerelease)" if pre else ""
    if l[0] == 0:
        compatible = n[0] == 0 and n[1] == l[1]
    else:
        compatible = n[0] == l[0]
    return ("compatible update" if compatible else "major behind") + tag

latest_cache = {}
def latest(name):
    if not online:
        return ""
    if name not in latest_cache:
        try:
            out = subprocess.run(["cargo", "search", name, "--limit", "1"], capture_output=True, text=True, timeout=60).stdout
            m = re.search(r'^' + re.escape(name) + r' = "([^"]+)"', out, re.M)
            latest_cache[name] = m.group(1) if m else None
        except Exception:
            latest_cache[name] = None
    return latest_cache[name]

# 2. direct dependencies
rows = {}
for mid in sorted(members, key=lambda i: packages[i]["name"]):
    member = packages[mid]
    node = nodes.get(mid)
    locked_by_name = {}
    if node:
        for dep in node["deps"]:
            locked_by_name.setdefault(packages[dep["pkg"]]["name"], set()).add(packages[dep["pkg"]]["version"])
    for d in member["dependencies"]:
        if d.get("path"):
            continue
        key = (d["name"], d["req"])
        row = rows.setdefault(key, {"members": [], "kinds": set(), "locked": set(), "optional": d.get("optional", False), "target": d.get("target")})
        row["members"].append(member["name"])
        row["kinds"].add(d.get("kind") or "normal")
        row["locked"] |= locked_by_name.get(d["name"], set())
print("## Direct dependencies (declared requirement vs Cargo.lock)\n")
print("Rows are one per crate and requirement; `not in scope` means the dependency is optional, dev-only or for another target in this scope. "
      "`compatible update` means `cargo update -p <crate>` would take it under the same requirement; `major behind` needs a manifest change: both are proposals for a lockfile PR, never applied here.\n")
header = "| crate | requirement | locked | workspace crates (kind) | " + ("newest on crates.io | status |" if online else "note |")
print(header)
print("| --- | --- | --- | --- | " + ("--- | --- |" if online else "--- |"))
behind = []
for (name, req), row in sorted(rows.items()):
    locked = ", ".join(sorted(row["locked"])) or "not in scope"
    who = ", ".join(f"{m}" for m in sorted(set(row["members"]))) + " (" + "/".join(sorted(row["kinds"])) + ")"
    note = []
    if row["optional"]:
        note.append("optional")
    if row["target"]:
        note.append(f"target `{row['target']}`")
    if online:
        new = latest(name)
        status = classify(sorted(row["locked"])[-1], new) if row["locked"] else "not in scope"
        if status.startswith(("compatible", "major")):
            behind.append((name, req, locked, new, status))
        print(f"| {name} | `{req}` | {locked} | {who} | {new or 'lookup failed'} | {status}{'; ' + ', '.join(note) if note else ''} |")
    else:
        print(f"| {name} | `{req}` | {locked} | {who} | {', '.join(note) or ''} |")
if online:
    print("\n### Update proposals\n")
    if behind:
        for name, req, locked, new, status in behind:
            print(f"- {name}: requirement `{req}`, locked {locked}, newest {new}: {status}")
    else:
        print("- none: every direct dependency in scope is at its newest crates.io release")

# 3. duplicates
print("\n## Duplicate crate versions (`cargo tree -d`)\n")
dups = {n: vs for n, vs in by_name.items() if len(vs) > 1}
if not dups:
    print("- none in this scope")
for name in sorted(dups):
    print(f"- {name}: " + ", ".join(sorted(dups[name], key=lambda v: parse(v)[0])))
    for pid in sorted(p for p in nodes if packages[p]["name"] == name):
        who = sorted(label(d) for d in dependents.get(pid, []))
        print(f"  - {packages[pid]['version']} requested by: " + (", ".join(who) if who else "(workspace)"))

# 4. licenses
print("\n## License summary (declared `license` fields of the resolved graph)\n")
permissive = re.compile(r"^(MIT|Apache-2\.0|BSD-[0-3]-Clause|ISC|Zlib|Unlicense|0BSD|BSL-1\.0|CC0-1\.0|Unicode-3\.0|Unicode-DFS-2016|CDLA-Permissive-2\.0|MPL-2\.0|OpenSSL|NCSA|Apache-2\.0 WITH LLVM-exception)$")
counts = defaultdict(list)
review = []
for pid in nodes:
    p = packages[pid]
    if pid in members:
        continue
    lic = p.get("license") or ("license-file" if p.get("license_file") else "(none declared)")
    counts[lic].append(p["name"])
    terms = [t.strip() for t in re.split(r"\bOR\b|\bAND\b|/", lic)]
    if lic in ("(none declared)", "license-file") or not all(permissive.match(t.strip("() ")) for t in terms):
        review.append((p["name"], p["version"], lic))
print("| license | crates |")
print("| --- | ---: |")
for lic, names in sorted(counts.items(), key=lambda kv: (-len(kv[1]), kv[0])):
    print(f"| {lic} | {len(names)} |")
print("\nReview (not plainly permissive, or no SPDX expression declared):\n")
if review:
    for name, version, lic in sorted(review):
        print(f"- {name} {version}: {lic}")
else:
    print("- none")

# 5. native build crates
print("\n## Crates that compile or link native code\n")
native = re.compile(r"^(cc|cmake|bindgen|clang-sys|cxx|cxx-build|pkg-config|vcpkg|nasm-rs|ort-sys|libloading|.*-sys)$")
found = [pid for pid in nodes if native.match(packages[pid]["name"]) and pid not in members]
if not found:
    print("- none in this scope")
for pid in sorted(found, key=lambda i: packages[i]["name"]):
    who = sorted(label(d) for d in dependents.get(pid, []))
    print(f"- {label(pid)}: requested by " + (", ".join(who) if who else "(workspace)"))
PY
else
  printf '## Direct dependencies (locked versions; python3 not found, so no requirement column)\n\n```\n'
  cargo tree --locked --offline --workspace --depth 1 -e normal,build,dev --prefix none --format '{p}' 2>/dev/null | grep -v '(/' | sort -u
  printf '```\n\n## Duplicate crate versions (`cargo tree -d`)\n\n```\n'
  cargo tree --locked --offline --workspace -d --depth 0 -e normal,build --prefix none --format '{p}' 2>/dev/null | grep -v '^$' | sort -u
  printf '```\n\n## License summary (`cargo tree --format {l}`)\n\n```\n'
  cargo tree --locked --offline --workspace -e normal,build --prefix none --format '{l}' 2>/dev/null | sed 's/ (\*)$//' | sort | uniq -c | sort -rn
  printf '```\n'
fi

printf '\n## Python (uv)\n\n```\n'
if [ "$online" -eq 1 ]; then
  uv tree --outdated --depth 1 2>&1
  printf '\n$ uv audit --preview-features audit-command\n'
  if uv audit --help >/dev/null 2>&1; then
    uv audit --preview-features audit-command 2>&1
  else
    printf 'uv %s has no audit subcommand; see scripts/toolchain-doctor.sh for the remedy\n' "$(uv --version 2>/dev/null | sed 's/^uv //')"
  fi
else
  uv tree --offline --depth 1 2>&1
fi
printf '```\n'
