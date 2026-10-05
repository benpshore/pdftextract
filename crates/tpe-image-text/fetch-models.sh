#!/bin/sh
# Provision the ocrs model files pinned in crates/tpe-image-text/models/manifest.json
# into .models/ocrs/ (relative to the repository root), mirroring native/fetch.sh.
# See docs/IMAGE_TEXT.md.
#
#   sh crates/tpe-image-text/fetch-models.sh                 fetch what is missing, verify, print a table
#   sh crates/tpe-image-text/fetch-models.sh --force         re-download everything
#   sh crates/tpe-image-text/fetch-models.sh --print-hashes  no network: print file, bytes, sha256 of what is on disk
#   sh crates/tpe-image-text/fetch-models.sh --dir DIR       use DIR instead of .models/ocrs
#
# Idempotent: a file already at its destination is not downloaded again, but it
# is always re-hashed and checked against the manifest. A size or sha256 that
# does not match fails the run and a freshly downloaded mismatch is deleted; the
# engine itself repeats the same check before loading a file, so a stale or
# tampered model is refused twice. Every artifact has exactly one URL; there are
# no fallback mirrors, because a mutable fallback would defeat the pin.
set -eu

usage() {
  echo "usage: sh crates/tpe-image-text/fetch-models.sh [--force] [--print-hashes] [--dir DIR]" >&2
}

FORCE=false
PRINT_ONLY=false
DEST_DIR=.models/ocrs
while [ $# -gt 0 ]; do
  case "$1" in
    --force) FORCE=true ;;
    --print-hashes) PRINT_ONLY=true ;;
    --dir)
      shift
      [ $# -gt 0 ] || { usage; exit 2; }
      DEST_DIR=$1
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"
MANIFEST=crates/tpe-image-text/models/manifest.json
if [ ! -f "$MANIFEST" ]; then
  echo "error: $MANIFEST not found under $ROOT" >&2
  exit 1
fi

sha256_of() { # <file> -> hex digest on stdout
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    echo "error: neither sha256sum nor shasum is available" >&2
    exit 1
  fi
}

bytes_of() { # <file> -> size in bytes (macOS wc pads with spaces)
  wc -c <"$1" | tr -d ' '
}

WORK=$(mktemp -d "${TMPDIR:-/tmp}/tpe-ocrs-models.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
trap 'exit 130' INT TERM
ENTRIES="$WORK/entries"
ROWS="$WORK/rows"
: >"$ROWS"

# Flatten the manifest to one `|`-separated record per entry: name|url|dest|bytes|sha256
if command -v python3 >/dev/null 2>&1; then
  python3 - "$MANIFEST" >"$ENTRIES" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as f:
    manifest = json.load(f)
for entry in manifest["entries"]:
    values = [str(entry[k]) for k in ("name", "url", "dest", "bytes", "sha256")]
    for v in values:
        if "|" in v:
            sys.exit(f"manifest value contains '|': {v}")
    print("|".join(values))
PY
else
  # Line-oriented fallback: the manifest keeps each entry object on one line.
  field() { # <line> <key> -> value
    printf '%s\n' "$1" | sed -n "s/.*\"$2\": *\"\\{0,1\\}\\([^\",}]*\\)\"\\{0,1\\}.*/\\1/p"
  }
  grep '"name"' "$MANIFEST" | while IFS= read -r line; do
    printf '%s|%s|%s|%s|%s\n' "$(field "$line" name)" "$(field "$line" url)" \
      "$(field "$line" dest)" "$(field "$line" bytes)" "$(field "$line" sha256)"
  done >"$ENTRIES"
fi

if [ "$PRINT_ONLY" = false ] && ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required" >&2
  exit 1
fi

CURL_FLAGS="-fsSL --connect-timeout 30 --speed-limit 1024 --speed-time 60 --retry 8"

FAILED=0
while IFS='|' read -r name url dest_name pin_bytes pin; do
  dest="$DEST_DIR/$dest_name"
  state=cached
  if [ "$PRINT_ONLY" = true ]; then
    if [ ! -f "$dest" ]; then
      printf '%s|%s|%s|%s\n' missing "$dest" - - >>"$ROWS"
      continue
    fi
    state=present
  elif [ "$FORCE" = true ] || [ ! -f "$dest" ]; then
    mkdir -p "$DEST_DIR"
    echo "  > $url" >&2
    # shellcheck disable=SC2086 # CURL_FLAGS is a flag list, splitting intended
    if ! curl $CURL_FLAGS -o "$dest.download" "$url"; then
      rm -f "$dest.download"
      echo "error: download failed: $url" >&2
      FAILED=1
      printf '%s|%s|%s|%s\n' FAILED "$dest" - - >>"$ROWS"
      continue
    fi
    mv "$dest.download" "$dest"
    state=downloaded
  fi

  hash=$(sha256_of "$dest")
  size=$(bytes_of "$dest")
  if [ "$PRINT_ONLY" = false ] && { [ "$size" != "$pin_bytes" ] || [ "$pin" != "$hash" ]; }; then
    echo "error: $dest ($name): $size bytes sha256 $hash; manifest pins $pin_bytes bytes sha256 $pin" >&2
    echo "       (delete it or rerun with --force if the file on disk is stale)" >&2
    FAILED=1
    # Never leave a freshly downloaded, unverified file where it would be used.
    case "$state" in downloaded*) rm -f "$dest" ;; esac
    state=MISMATCH
  elif [ "$state" != present ]; then
    state="$state,verified"
  fi
  printf '%s|%s|%s|%s\n' "$state" "$dest" "$size" "$hash" >>"$ROWS"
done <"$ENTRIES"

echo
echo "ocrs models ($ROOT/$DEST_DIR)"
printf '%-22s %-40s %11s  %s\n' state file bytes sha256
while IFS='|' read -r state dest size hash; do
  printf '%-22s %-40s %11s  %s\n' "$state" "$dest" "$size" "$hash"
done <"$ROWS"

if [ "$FAILED" != 0 ]; then
  echo "error: ocrs model provisioning failed" >&2
  exit 1
fi
