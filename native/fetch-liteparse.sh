#!/bin/sh
# Provision the existing reviewed PDFium library AND its matching headers for
# LiteParse's optional pure layout dependency. Never use its automatic download.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
case "$(uname -s)" in
  Linux) platform=linux ;;
  Darwin) platform=mac ;;
  *) echo "unsupported LiteParse build platform" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) platform="$platform-x64" ;;
  aarch64 | arm64) platform="$platform-arm64" ;;
  *) echo "unsupported LiteParse build architecture" >&2; exit 1 ;;
esac

# The reviewed manifest intentionally keeps one entry per line (fetch.sh uses
# this same portable fallback). Only the exact current-platform PDFium row is read.
entry=$(sed -n '/"kind": "pdfium", "platform": "'"$platform"'"/p' native/manifest.json)
field() { printf '%s\n' "$entry" | sed -n 's/.*"'"$1"'": *"\([^"]*\)".*/\1/p'; }
url=$(field url)
member=$(field member)
archive_pin=$(field archive_sha256)
library_pin=$(field sha256)
for pin in "$archive_pin" "$library_pin"; do
  case "$pin" in '' | *[!0-9a-f]*) echo "missing reviewed PDFium hash" >&2; exit 1 ;; esac
  [ "${#pin}" -eq 64 ] || { echo "invalid PDFium hash" >&2; exit 1; }
done
case "$member" in lib/libpdfium.so | lib/libpdfium.dylib) ;; *) echo "invalid PDFium member" >&2; exit 1 ;; esac

work=$(mktemp -d "${TMPDIR:-/tmp}/tpe-liteparse.XXXXXX")
trap 'rm -rf "$work"' EXIT
trap 'exit 130' INT TERM
archive="$work/pdfium.tgz"
case "$#" in
  0) curl -fsSL --connect-timeout 30 --max-time 120 --max-filesize 67108864 --retry 3 "$url" -o "$archive" ;;
  2) [ "$1" = --archive ] && [ -f "$2" ] || { echo "usage: $0 [--archive FILE]" >&2; exit 2; }; cp "$2" "$archive" ;;
  *) echo "usage: $0 [--archive FILE]" >&2; exit 2 ;;
esac

# Compare digests as strings: macOS ships a BSD `sha256sum` whose flags differ
# from GNU's (`--check` is rejected with a usage error), while both tools and
# `shasum -a 256` agree on the `<hex>  <file>` output format (as in fetch.sh).
digest_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    echo "error: neither sha256sum nor shasum is available" >&2
    exit 1
  fi
}
verify() { # <pinned hex> <file>
  actual=$(digest_of "$2")
  if [ "$actual" != "$1" ]; then
    echo "error: $2: sha256 $actual does not match the pinned $1" >&2
    exit 1
  fi
}
verify "$archive_pin" "$archive"
mkdir "$work/extracted"
# Archive identity is verified before any member is extracted. Do not inherit
# release-builder ownership when provisioning inside a rootless container.
tar --no-same-owner -xzf "$archive" -C "$work/extracted" "$member" include
verify "$library_pin" "$work/extracted/$member"
test -f "$work/extracted/include/fpdfview.h"
mkdir -p .pdfium/lib .pdfium/include
cp "$work/extracted/$member" ".pdfium/$member"
cp -R "$work/extracted/include/." .pdfium/include/
printf 'Verified PDFium library and headers provisioned for %s\n' "$platform"
