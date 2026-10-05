#!/bin/sh
# Toolchain doctor: checks this host against the repository's pins and prints
# an exact remedy for everything that is missing or has drifted.
#
#   sh scripts/toolchain-doctor.sh [--strict] [--engine-only]
#
# POSIX sh (dash, bash, macOS /bin/sh); no GNU-only flags. Read-only: it
# installs nothing, changes no file and never lets rustup auto-install.
# Exit status: 1 when a required item is missing, 2 on usage error, else 0.
#   --strict       optional items (web, native artifacts, swift) fail too
#   --engine-only  require only what `cargo build --locked` needs; the uv,
#                  CMake and C++ gates from AGENTS.md become optional
#
# "required" means the AGENTS.md pre-commit gates on this OS: the pinned Rust
# toolchain with clippy and rustfmt, a C compiler and linker, git, disk space,
# uv with the pyproject Python, plus CMake and a C++/Objective-C++ compiler on
# macOS (the CMake targets need Foundation). Swift, Node/pnpm (web alpha) and
# the native backend artifacts are optional and reported with remedies.

set -u
export RUSTUP_AUTO_INSTALL=0

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd) || exit 2
strict=0
engine_only=0
for arg in "$@"; do
  case $arg in
    --strict) strict=1 ;;
    --engine-only) engine_only=1 ;;
    -h | --help) sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) printf 'toolchain-doctor: unknown option %s\n' "$arg" >&2; exit 2 ;;
  esac
done

required_failed=0
optional_failed=0
warnings=0
failed_items=''

pass() { printf '[ok]   %s\n' "$1"; }
note() { printf '[info] %s\n' "$1"; }
fix() { printf '       fix: %s\n' "$1"; }
# fail WHAT REMEDY: a required item is missing or wrong.
fail() {
  required_failed=$((required_failed + 1))
  failed_items="$failed_items
  - $1"
  printf '[FAIL] %s\n' "$1"
  fix "$2"
}
# miss WHAT REMEDY: an optional item is missing (required under --strict).
miss() {
  if [ "$strict" -eq 1 ]; then
    fail "$1" "$2"
    return
  fi
  optional_failed=$((optional_failed + 1))
  printf '[opt]  %s\n' "$1"
  fix "$2"
}
# need WHAT REMEDY [REQUIRED]: fail when REQUIRED is 1 (default), else miss.
need() {
  if [ "${3:-1}" -eq 1 ]; then fail "$1" "$2"; else miss "$1" "$2"; fi
}
warn() {
  warnings=$((warnings + 1))
  printf '[warn] %s\n' "$1"
  if [ $# -ge 2 ]; then fix "$2"; fi
}
have() { command -v "$1" >/dev/null 2>&1; }
first_line() { "$@" 2>&1 | head -n 1; }

# version_ge A B: dotted version A >= B. A leading non-digit prefix (v…) and
# any suffix from the first character that is not a digit or a dot (rc2, -1)
# are ignored, so 3.14.0rc2 counts as 3.14.0.
version_ge() {
  vg_a=$(printf '%s' "$1" | sed 's/^[^0-9]*//; s/[^0-9.].*$//').0.0.0
  vg_b=$(printf '%s' "$2" | sed 's/^[^0-9]*//; s/[^0-9.].*$//').0.0.0
  vg_i=1
  while [ "$vg_i" -le 4 ]; do
    vg_x=$(printf '%s' "$vg_a" | cut -d. -f"$vg_i")
    vg_y=$(printf '%s' "$vg_b" | cut -d. -f"$vg_i")
    [ -n "$vg_x" ] || vg_x=0
    [ -n "$vg_y" ] || vg_y=0
    if [ "$vg_x" -gt "$vg_y" ]; then return 0; fi
    if [ "$vg_x" -lt "$vg_y" ]; then return 1; fi
    vg_i=$((vg_i + 1))
  done
  return 0
}
# free_gb DIR: whole gigabytes available on DIR's file system (POSIX df -Pk).
free_gb() {
  df -Pk "$1" 2>/dev/null | awk 'NR == 2 { printf "%d\n", $4 / 1048576 }'
}
# compiler_kind COMMAND: gcc, clang, "Apple clang" or unknown, from `-v`.
compiler_kind() {
  ck_v=$("$1" -v 2>&1)
  case $ck_v in
    *"Apple clang version"*) printf 'Apple clang\n' ;;
    *"clang version"*) printf 'clang\n' ;;
    *"gcc version"*) printf 'gcc\n' ;;
    *) printf 'unknown\n' ;;
  esac
}

os=$(uname -s 2>/dev/null || printf unknown)
arch=$(uname -m 2>/dev/null || printf unknown)
is_macos=0
[ "$os" = Darwin ] && is_macos=1
full_gates=1
[ "$engine_only" -eq 1 ] && full_gates=0
printf 'toolchain doctor for %s (%s %s)\n' "$root" "$os" "$arch"
printf 'pins: rust-toolchain.toml, pyproject.toml, web/package.json, native/manifest.json\n\n'

# ---------------------------------------------------------------- Rust
printf -- '-- Rust toolchain\n'
channel=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$root/rust-toolchain.toml" 2>/dev/null | head -n 1)
if [ -z "$channel" ]; then
  fail "rust-toolchain.toml has no [toolchain] channel" "git -C $root checkout -- rust-toolchain.toml"
fi
toolchain_ok=0
if have rustup; then
  pass "rustup $(rustup --version 2>/dev/null | head -n 1 | sed 's/^rustup //') at $(command -v rustup)"
  if [ -n "$channel" ] && rustup toolchain list 2>/dev/null | grep "^$channel-" >/dev/null; then
    toolchain_ok=1
    pass "toolchain $channel is installed"
    components=$(rustup component list --installed --toolchain "$channel" 2>/dev/null)
    for component in clippy rustfmt; do
      if printf '%s\n' "$components" | grep "^$component-" >/dev/null; then
        pass "component $component ($channel)"
      else
        toolchain_ok=0
        fail "component $component is missing from toolchain $channel" \
          "rustup component add $component --toolchain $channel"
      fi
    done
  elif [ -n "$channel" ]; then
    fail "toolchain $channel is not installed (rust-toolchain.toml pins it)" \
      "cd $root && rustup toolchain install   # reads rust-toolchain.toml (profile minimal, clippy, rustfmt); older rustup: rustup toolchain install $channel --profile minimal --component clippy --component rustfmt"
  fi
  if [ "$toolchain_ok" -eq 1 ]; then
    active=$(cd "$root" && rustup show active-toolchain 2>&1 | head -n 1)
    case $active in
      "$channel-"*) pass "active toolchain in the repository: $active" ;;
      *) fail "active toolchain in the repository is '$active', not $channel" \
        "unset RUSTUP_TOOLCHAIN; rustup override unset --path $root   # rust-toolchain.toml must win; do not publish builds from another compiler" ;;
    esac
    for probe in "rustc -V" "cargo -V" "cargo clippy -V" "cargo fmt --version"; do
      # shellcheck disable=SC2086 # the probe is a fixed word list
      line=$(cd "$root" && $probe 2>&1 | head -n 1)
      case $line in
        rustc\ "$channel"* | cargo\ "$channel"* | clippy\ * | rustfmt\ *) pass "$probe -> $line" ;;
        *) fail "$probe -> $line" "run from $root with the pinned toolchain: cd $root && rustup toolchain install" ;;
      esac
    done
    host_triple=$(cd "$root" && rustc -vV 2>/dev/null | sed -n 's/^host: //p')
    note "rustc host triple: ${host_triple:-unknown}"
  fi
  if [ -n "${RUSTUP_TOOLCHAIN:-}" ]; then
    warn "RUSTUP_TOOLCHAIN=$RUSTUP_TOOLCHAIN overrides rust-toolchain.toml for every cargo call" "unset RUSTUP_TOOLCHAIN"
  fi
  if rustup override list 2>/dev/null | grep -F "$root" >/dev/null; then
    warn "a rustup directory override is set for $root" "rustup override unset --path $root"
  fi
else
  fail "rustup is not installed (the pinned toolchain is managed by rustup)" \
    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- --no-modify-path --default-toolchain none; then: cd $root && rustup toolchain install"
fi
if [ -f "$root/.cargo/config.toml" ]; then
  if grep -E '^\s*(linker|rustflags)\s*=' "$root/.cargo/config.toml" >/dev/null 2>&1; then
    warn ".cargo/config.toml sets linker or rustflags; CI does not" "keep per-host linker choices in ~/.cargo/config.toml, not in the repository"
  else
    pass ".cargo/config.toml forces no linker and no rustflags (CI parity)"
  fi
fi
if [ -n "${CARGO_TARGET_DIR:-}" ]; then
  note "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
fi

# ---------------------------------------------------------- C and C++
printf -- '\n-- C/C++ compilers and linker (every build compiles SQLite and ring through the cc crate)\n'
cc_cmd=${CC:-cc}
cc_kind=unknown
if have "$cc_cmd"; then
  cc_kind=$(compiler_kind "$cc_cmd")
  pass "C compiler: $cc_cmd -> $(command -v "$cc_cmd") [$cc_kind] $(first_line "$cc_cmd" --version)"
  if [ "$cc_kind" = unknown ]; then
    warn "could not classify $cc_cmd as gcc or clang; builds still work, but record it with the build" "$cc_cmd -v"
  fi
else
  fail "no C compiler ('$cc_cmd' not found); libsqlite3-sys and ring need one in every build" \
    "Debian/Ubuntu: sudo apt-get install build-essential | Fedora: sudo dnf install gcc gcc-c++ | macOS: xcode-select --install"
fi
if [ -n "${CC:-}" ] && have cc && have "$CC"; then
  driver_kind=$(compiler_kind cc)
  if [ "$driver_kind" != "$cc_kind" ]; then
    warn "CC=$CC [$cc_kind] compiles C objects but rustc links through cc [$driver_kind] (mixed drivers)" \
      "either unset CC, or export CARGO_TARGET_$(printf '%s' "${host_triple:-HOST_TRIPLE}" | tr 'a-z-' 'A-Z_')_LINKER=$CC so one driver compiles and links"
  fi
fi
cxx_cmd=${CXX:-c++}
cxx_required=0
[ "$is_macos" -eq 1 ] && [ "$full_gates" -eq 1 ] && cxx_required=1
if have "$cxx_cmd"; then
  pass "C++ compiler: $cxx_cmd -> $(command -v "$cxx_cmd") [$(compiler_kind "$cxx_cmd")] $(first_line "$cxx_cmd" --version)"
else
  need "no C++ compiler ('$cxx_cmd'); needed by the root CMake targets (macOS), tpe-search --features usearch (cxx), the Poppler provider and aws-lc-sys" \
    "Debian/Ubuntu: sudo apt-get install g++ (or clang) | Fedora: sudo dnf install gcc-c++ | macOS: xcode-select --install" "$cxx_required"
fi
if have "$cc_cmd"; then
  ld_path=$("$cc_cmd" -print-prog-name=ld 2>/dev/null | head -n 1)
  case $ld_path in
    /*) ;;
    *) ld_path=$(command -v "${ld_path:-ld}" 2>/dev/null || printf '') ;;
  esac
  if [ -n "$ld_path" ] && [ -x "$ld_path" ]; then
    if [ "$is_macos" -eq 1 ]; then
      ld_desc=$("$ld_path" -v 2>&1 | head -n 1)
    else
      ld_desc=$("$ld_path" --version 2>&1 | head -n 1)
    fi
    pass "linker behind $cc_cmd: $ld_path ($ld_desc)"
  else
    fail "$cc_cmd cannot find its linker (ld)" "Debian/Ubuntu: sudo apt-get install binutils | macOS: xcode-select --install"
  fi
fi
for alt in ld.lld lld mold ld.gold; do
  if have "$alt"; then
    note "alternative linker available: $alt -> $(command -v "$alt") (not used unless you configure it; CI uses the default ld)"
  fi
done
build_env=$(env 2>/dev/null | grep -E '^(CC|CXX|CFLAGS|CXXFLAGS|LDFLAGS|AR|TARGET_CC|HOST_CC|CRATE_CC_NO_DEFAULTS|RUSTFLAGS|CARGO_BUILD_RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_TARGET|CARGO_TARGET_[A-Z0-9_]+_(LINKER|RUSTFLAGS)|LIBCLANG_PATH|MACOSX_DEPLOYMENT_TARGET|SDKROOT|PDFIUM_[A-Z_]+|DOCLING_[A-Z_]+|TPE_[A-Z_]+_PROVIDER_PATH|MUPDF_[A-Z_]+|POPPLER_[A-Z_]+)=' | sort)
if [ -n "$build_env" ]; then
  note "environment that changes builds or runtime loading:"
  printf '%s\n' "$build_env" | sed 's/^/         /'
else
  pass "no build-altering environment variables set (CC, CXX, CFLAGS, RUSTFLAGS, *_LINKER, LIBCLANG_PATH, ...)"
fi

# --------------------------------------------- build tools and services
printf -- '\n-- Build tools\n'
if have git; then
  pass "git $(git --version 2>/dev/null | sed 's/^git version //') (build.rs derives the version identity from it)"
else
  fail "git is not installed; build.rs needs it for the version identity" "Debian/Ubuntu: sudo apt-get install git | macOS: xcode-select --install"
fi
cmake_min=$(sed -n 's/^cmake_minimum_required(VERSION \([0-9.]*\)).*/\1/p' "$root/CMakeLists.txt" 2>/dev/null | head -n 1)
[ -n "$cmake_min" ] || cmake_min=3.25
cmake_required=0
[ "$is_macos" -eq 1 ] && [ "$full_gates" -eq 1 ] && cmake_required=1
if have cmake; then
  cmake_version=$(cmake --version 2>/dev/null | head -n 1 | sed 's/^cmake version //')
  if version_ge "$cmake_version" "$cmake_min"; then
    pass "cmake $cmake_version (root CMakeLists.txt needs >= $cmake_min; the Poppler provider >= 3.20, building Poppler itself >= 3.28)"
  else
    need "cmake $cmake_version is older than the $cmake_min the root CMakeLists.txt requires" "install CMake >= $cmake_min (brew install cmake | sudo apt-get install cmake | https://cmake.org/download/)" "$cmake_required"
  fi
else
  need "cmake is not installed (root CMake targets on macOS; Poppler provider; aws-lc-sys fallback)" \
    "macOS: brew install cmake | Debian/Ubuntu: sudo apt-get install cmake" "$cmake_required"
fi
if have pkg-config; then
  pass "pkg-config $(pkg-config --version 2>/dev/null | head -n 1) (probed by libsqlite3-sys; needed to build Poppler)"
else
  miss "pkg-config is not installed (optional: rusqlite is bundled; Poppler and system-library builds need it)" \
    "macOS: brew install pkg-config | Debian/Ubuntu: sudo apt-get install pkg-config"
fi
if have make; then pass "make $(first_line make --version | sed 's/^GNU Make //')"; else note "make not found (only CMake's Unix Makefiles generator needs it; ninja also works)"; fi
if have ninja; then note "ninja $(first_line ninja --version)"; fi

# -------------------------------------------------- Python through uv
printf -- '\n-- Python (uv only; never pip)\n'
py_req=$(sed -n 's/^requires-python *= *">=\([0-9.]*\)".*/\1/p' "$root/pyproject.toml" 2>/dev/null | head -n 1)
[ -n "$py_req" ] || py_req=3.14
if have uv; then
  uv_version=$(uv --version 2>/dev/null | head -n 1 | sed 's/^uv //')
  pass "uv $uv_version at $(command -v uv)"
  if uv audit --help >/dev/null 2>&1; then
    pass "uv audit is available (AGENTS.md: uv audit --preview-features audit-command)"
  else
    warn "uv $uv_version has no 'audit' subcommand; the AGENTS.md audit gate cannot run with it (CI installs the latest uv)" \
      "update uv: uv self update (standalone install) | brew upgrade uv | pipx upgrade uv; or run it once without installing: uvx --from 'uv>=0.9' uv audit --preview-features audit-command"
  fi
  py_found=$(cd "$root" && uv python find ">=$py_req" 2>/dev/null | head -n 1)
  if [ -n "$py_found" ]; then
    pass "Python >= $py_req for uv: $py_found ($("$py_found" --version 2>&1 | head -n 1))"
  else
    miss "no Python >= $py_req is known to uv (pyproject requires-python); uv sync downloads one when the network allows" \
      "uv python install $py_req"
  fi
else
  need "uv is not installed (AGENTS.md: Python only through uv; the required ci check runs ruff, pytest and uv audit)" \
    "macOS: brew install uv | any: curl -LsSf https://astral.sh/uv/install.sh | sh (then restart the shell)" "$full_gates"
fi

# ------------------------------------------------------ Node and pnpm
printf -- '\n-- Web alpha (optional): Node and pnpm from web/package.json\n'
node_req=$(sed -n 's/.*"node": *">=\([0-9.]*\)".*/\1/p' "$root/web/package.json" 2>/dev/null | head -n 1)
[ -n "$node_req" ] || node_req=22.13.0
pnpm_pin=$(sed -n 's/.*"packageManager": *"pnpm@\([0-9.]*\)".*/\1/p' "$root/web/package.json" 2>/dev/null | head -n 1)
[ -n "$pnpm_pin" ] || pnpm_pin=11.25.0
if have node; then
  node_version=$(node --version 2>/dev/null | head -n 1)
  if version_ge "$node_version" "$node_req"; then
    pass "node $node_version at $(command -v node) (engines: >= $node_req)"
    case $node_version in v22.*) ;; *) warn "CI uses Node 22 (setup-node 22); $node_version differs" "mise use node@22 | nvm use 22" ;; esac
  else
    miss "node $node_version is older than the >= $node_req web/package.json requires" "mise use node@22 | nvm install 22 | https://nodejs.org/"
  fi
else
  miss "node is not installed (web alpha only)" "mise use node@22 | nvm install 22 | https://nodejs.org/"
fi
if have pnpm; then
  pnpm_version=$(pnpm --version 2>/dev/null | head -n 1)
  if [ "$pnpm_version" = "$pnpm_pin" ]; then
    pass "pnpm $pnpm_version matches the packageManager pin"
  else
    miss "pnpm $pnpm_version does not match the packageManager pin pnpm@$pnpm_pin (CI installs exactly that)" \
      "corepack enable && corepack prepare pnpm@$pnpm_pin --activate | npm install --global pnpm@$pnpm_pin"
  fi
else
  miss "pnpm is not installed (web alpha only; pin pnpm@$pnpm_pin)" "corepack enable && corepack prepare pnpm@$pnpm_pin --activate | npm install --global pnpm@$pnpm_pin"
fi

# -------------------------------------------------------------- Swift
printf -- '\n-- Swift (optional locally; CI runs it in a pinned container)\n'
swift_image=$(sed -n 's/.*container: *swift:\([^@ ]*\)@.*/\1/p' "$root/.github/workflows/ci.yml" 2>/dev/null | head -n 1)
if have swift; then
  pass "swift: $(first_line swift --version)"
else
  miss "swift is not installed (swift build && swift test; CI uses the swift:${swift_image:-6.4.0-noble} image)" \
    "macOS: xcode-select --install (or Xcode) | Linux: https://www.swift.org/install/ or docker run --rm -v \$PWD:/src -w /src swift:${swift_image:-6.4.0-noble} swift test"
fi

# ---------------------------------------------- native backend artifacts
printf -- '\n-- Optional native backends (docs/NATIVE.md, native/README.md)\n'
if [ "$is_macos" -eq 1 ]; then pdfium_lib=.pdfium/lib/libpdfium.dylib; else pdfium_lib=.pdfium/lib/libpdfium.so; fi
if [ -f "$root/$pdfium_lib" ]; then
  pass "PDFium library present: $root/$pdfium_lib (features pdfium, docling, liteparse-layout)"
  case ${PDFIUM_DYNAMIC_LIB_PATH:-} in
    "") miss "PDFIUM_DYNAMIC_LIB_PATH is unset; tpe rejects relative and implicit PDFium paths" "export PDFIUM_DYNAMIC_LIB_PATH=\"$root/.pdfium/lib\"" ;;
    /*) if [ -e "$PDFIUM_DYNAMIC_LIB_PATH" ]; then pass "PDFIUM_DYNAMIC_LIB_PATH=$PDFIUM_DYNAMIC_LIB_PATH exists"; else warn "PDFIUM_DYNAMIC_LIB_PATH=$PDFIUM_DYNAMIC_LIB_PATH does not exist" "export PDFIUM_DYNAMIC_LIB_PATH=\"$root/.pdfium/lib\""; fi ;;
    *) warn "PDFIUM_DYNAMIC_LIB_PATH=$PDFIUM_DYNAMIC_LIB_PATH is relative; the runtime rejects it" "export PDFIUM_DYNAMIC_LIB_PATH=\"$root/.pdfium/lib\"" ;;
  esac
else
  miss "PDFium library not provisioned ($pdfium_lib; features pdfium, docling, liteparse-layout)" \
    "cd $root && sh native/fetch.sh --pdfium-only && export PDFIUM_DYNAMIC_LIB_PATH=\"$root/.pdfium/lib\""
fi
if [ -f "$root/.pdfium/include/fpdfview.h" ]; then
  pass "PDFium headers present (.pdfium/include; feature liteparse-layout, see .cargo/config.toml)"
else
  miss "PDFium headers not provisioned (.pdfium/include; only feature liteparse-layout needs them)" "cd $root && sh native/fetch-liteparse.sh"
fi
models_missing=0
models_total=0
for dest in $(sed -n 's/.*"kind": "model".*"dest": *"\([^"]*\)".*/\1/p' "$root/native/manifest.json" 2>/dev/null); do
  models_total=$((models_total + 1))
  [ -f "$root/$dest" ] || models_missing=$((models_missing + 1))
done
if [ "$models_total" -gt 0 ] && [ "$models_missing" -eq 0 ]; then
  pass "docling models present ($models_total files under .models; feature docling)"
else
  miss "docling models: $models_missing of $models_total missing under .models (feature docling; ~82 MB, plus ONNX Runtime downloaded by ort-sys at build time)" \
    "cd $root && sh native/fetch.sh"
fi
for pair in "MuPDF:TPE_MUPDF_PROVIDER_PATH:MUPDF_DYNAMIC_LIB_PATH:native/mupdf/README.md" "Poppler:TPE_POPPLER_PROVIDER_PATH:POPPLER_DYNAMIC_LIB_PATH:native/poppler/README.md"; do
  engine=${pair%%:*}
  rest=${pair#*:}
  provider_var=${rest%%:*}
  rest=${rest#*:}
  runtime_var=${rest%%:*}
  readme=${rest#*:}
  eval "provider_val=\${$provider_var:-}"
  eval "runtime_val=\${$runtime_var:-}"
  if [ -z "$provider_val" ] && [ -z "$runtime_val" ]; then
    note "$engine provider not configured (optional user build; $provider_var and $runtime_var; see $readme)"
    continue
  fi
  for item in "$provider_var=$provider_val" "$runtime_var=$runtime_val"; do
    value=${item#*=}
    case $value in
      "") warn "${item%%=*} is unset while its $engine counterpart is set; the loader needs both absolute files" "see $readme" ;;
      /*) if [ -f "$value" ]; then pass "$item"; else warn "$item does not exist" "see $readme"; fi ;;
      *) warn "$item is not an absolute path; the loader rejects it" "see $readme" ;;
    esac
  done
done
if [ "$is_macos" -eq 1 ]; then
  libclang=''
  if [ -n "${LIBCLANG_PATH:-}" ] && [ -e "$LIBCLANG_PATH" ]; then
    libclang=$LIBCLANG_PATH
  elif have xcode-select; then
    dev_dir=$(xcode-select -p 2>/dev/null)
    for candidate in "$dev_dir/usr/lib/libclang.dylib" "$dev_dir/Toolchains/XcodeDefault.xctoolchain/usr/lib/libclang.dylib"; do
      [ -f "$candidate" ] && { libclang=$candidate; break; }
    done
  fi
  if [ -n "$libclang" ]; then
    pass "libclang for bindgen (GPUI in crates/tpe-app): $libclang"
  else
    miss "libclang not found; cargo build -p tpe-app (GPUI, bindgen) needs it" "xcode-select --install, or export LIBCLANG_PATH=/Library/Developer/CommandLineTools/usr/lib"
  fi
fi

# --------------------------------------------------------------- disk
printf -- '\n-- Disk space (df -Pk)\n'
target_dir=${CARGO_TARGET_DIR:-$root/target}
probe_dir=$target_dir
while [ ! -d "$probe_dir" ] && [ "$probe_dir" != / ]; do probe_dir=$(dirname -- "$probe_dir"); done
for dir in "$root" "$probe_dir"; do
  gb=$(free_gb "$dir")
  if [ -z "$gb" ]; then
    warn "could not read free space for $dir" "df -Pk $dir"
  elif [ "$gb" -lt 4 ]; then
    fail "only ${gb} GB free on $dir; a workspace debug build plus registry needs more" "free disk space, or point CARGO_TARGET_DIR at a larger volume"
  elif [ "$gb" -lt 15 ]; then
    warn "${gb} GB free on $dir (a full workspace target directory can exceed 15 GB)" "free disk space or set CARGO_TARGET_DIR to a larger volume"
  else
    pass "${gb} GB free on $dir"
  fi
  [ "$probe_dir" = "$root" ] && break
done

# ------------------------------------------------------------ summary
printf '\nsummary: %d required missing, %d optional missing, %d warnings\n' "$required_failed" "$optional_failed" "$warnings"
if [ "$required_failed" -gt 0 ]; then
  printf 'required items to fix:%s\n' "$failed_items"
  exit 1
fi
exit 0
