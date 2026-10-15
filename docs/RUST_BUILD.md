# Rust build contract

`rust-toolchain.toml` selects Rust **1.98.1** through rustup, with the minimal
profile plus the matching Clippy and rustfmt components. Run Cargo from the
repository or a descendant directory; a global `stable` default does not
override this file. Explicit `cargo +...`, `RUSTUP_TOOLCHAIN`, and rustup
directory overrides can override it and must not be used to publish builds
with a different compiler. `rustup show active-toolchain` and `rustc -Vv`
record what is actually selected.

The pin is the tested development and release compiler. The manifests do not
declare `rust-version`, and a lower minimum supported Rust version (MSRV) has
not been established. Edition 2024 and dependencies' declared Rust versions
are constraints, not evidence that the complete workspace works on their
oldest compiler. Establishing an MSRV requires its own locked build and test
matrix, including optional native features and the macOS app.

`Cargo.lock` is committed for the entire workspace. Production builds and CI
use `--locked`, so dependency resolution cannot silently replace it. Version
ranges in `Cargo.toml` express allowed upgrades; the lockfile selects exact
versions and registry checksums. Do not hand-edit package versions: release
versions come from git tags. Change dependencies deliberately and validate
the resulting lockfile with the pinned compiler before publishing.

`build.rs` embeds `TPE_VERSION` when supplied by the release workflow; the
conventional leading `v` is removed in `tpe --version`. An ordinary checkout
uses `git describe --tags --match 'v[0-9]*' --always --dirty`, so commits after
a tag and tracked local edits remain visible. A checkout without a matching
tag reports `git-<commit>`, and an archive without repository metadata reports
`source-archive`. Package manifest versions are not used as release identity.
Cargo watches the supplied environment value, git HEAD/index/refs (including
worktree metadata), and tracked source content so a reused build cannot retain
a stale tag or clean-tree identity. Release CI checks the actual executable's
version against its supplied tag.

```sh
rustup show active-toolchain
rustc -Vv
cargo -V
cargo clippy -V
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo build --release --locked -p text-processing-engine --bin tpe
```

`[profile.release]` in the root manifest controls the shipped code: fat LTO
and one codegen unit. Production runs use `target/release/tpe`, or
`cargo run --release --locked --bin tpe -- ...`; plain `cargo run` uses the
development profile. Add `--features pdfium` for the PDFium backend, and
provision the native library as described in [NATIVE.md](NATIVE.md). A native
library is an additional artifact outside Cargo.lock; record its hash too.

The macOS bundle script builds `tpe-app` with `--release --locked`. Linux
workspace tests cover engine-facing app code; they do not compile or launch
the macOS GUI. Its production build and bundle launch need the App workflow
on a native macOS runner. Python tools remain optional evaluation tooling,
managed only with uv, and are not needed to build or run the Rust engine.

For a reproducible run record the source commit and tree, dirty status,
toolchain versions and host target, Cargo.lock SHA-256, feature set, release
profile, build command, native artifact hashes, executable SHA-256, and the
input and output hashes. Those identities describe the observed build;
compiler and lockfile pinning alone do not guarantee byte-identical binaries
across different operating systems, linkers, or native libraries.

## Toolchain doctor and dependency report

`sh scripts/toolchain-doctor.sh` checks a host against every pin in the
repository and prints an exact remedy for each gap: rustup and the pinned
toolchain with clippy and rustfmt (and that nothing overrides
`rust-toolchain.toml`), which compiler `cc` and `c++` resolve to (gcc, clang
or Apple clang) and the linker behind `cc`, git, CMake, pkg-config, uv with
the `requires-python` from `pyproject.toml` and whether that uv has `uv audit`,
Node and pnpm against `web/package.json`, Swift, the native artifacts from
`native/manifest.json` and the MuPDF/Poppler provider variables, libclang on
macOS (bindgen for GPUI), and free disk space. It is POSIX sh, installs
nothing, sets `RUSTUP_AUTO_INSTALL=0` so no probe downloads a toolchain, and
exits 1 when a required item is missing. `--engine-only` requires only what
`cargo build --locked` needs; `--strict` also fails on optional items.

`sh scripts/deps-report.sh [--online] [--all-platforms] [--features LIST]`
prints a Markdown report: identity (commit, toolchain, `Cargo.lock` SHA-256),
every direct dependency with its declared requirement and locked version (and
with `--online` the newest crates.io release, classified as current, compatible
update or major behind), duplicate crate versions with their requesters, the
license summary of the resolved graph, the crates that compile native code,
and the uv tree (with `--outdated` and `uv audit` online). It never runs
`cargo update` and writes nothing; its proposals go to
[UPSTREAMS.md](UPSTREAMS.md) and become a reviewed lockfile PR.

## C/C++ toolchain: clang and gcc

The engine is Rust, but every build compiles C: `libsqlite3-sys` (bundled
SQLite, through `rusqlite`) and `ring` (through `ureq`/`rustls`) use the `cc`
crate. Optional paths add more: `tpe-search --features usearch` compiles C++
through `cxx`; `liteparse-layout` and `tpe-search --features onnx` resolve
`aws-lc-sys` (C, with CMake as its fallback builder); the `docling`, `onnx` and
speech features let `ort-sys` download a prebuilt ONNX Runtime at build time;
`crates/tpe-app` on macOS builds GPUI, whose `bindgen` needs libclang; and
`crates/tpe-browser` builds `cef-dll-sys` with CMake. The default Linux and
macOS workspace build needs only a C compiler and its linker.

How the compiler is chosen, and why clang and gcc hosts can differ:

- The `cc` crate uses `$CC` (or `CC_<triple>`, `TARGET_CC`), else `cc`; it
  adds `$CFLAGS` and, on macOS, `-mmacosx-version-min` from
  `MACOSX_DEPLOYMENT_TARGET` when set. `c++`/`$CXX` is used the same way for
  C++. `CRATE_CC_NO_DEFAULTS=1` drops its default flags.
- rustc links through `cc` (the driver on `PATH`), not through `$CC`. A host
  with `CC=clang` and a gcc `cc` therefore compiles C objects with clang and
  links with gcc. That works, but it is a mixed configuration: set
  `CARGO_TARGET_<TRIPLE>_LINKER=clang` (or `-C linker=`) when one driver
  should do both. The doctor warns when it sees the mix.
- CI uses the runner defaults: the image's gcc with GNU ld on `ubuntu-latest`
  and `ubuntu-24.04-arm`, Apple clang with ld64 on `macos-15` (the exact
  versions are in each job's log, next to `rustc --version --verbose`). Nothing in the
  repository forces a compiler or a linker, and `.cargo/config.toml` sets no
  `linker`, `rustflags` or `-C link-arg`, so a local build resolves exactly as
  CI does. Faster linkers (mold, lld) are a per-host choice for
  `~/.cargo/config.toml`; the config file shows the inactive examples.
- The user-built providers follow the same rule: `native/mupdf/build-provider.sh`
  uses `${CC:-cc}` with `-std=c11 -Wall -Wextra -Werror`, and
  `native/poppler/CMakeLists.txt` requests C++20 with `-Wpedantic -Werror`.
  gcc and clang warn differently under `-Werror`, so build a provider with the
  compiler you will ship it with and record which one in the build record.
- Object code from gcc and from clang is not byte-identical; the reproducible
  build record above must name the C compiler and linker (`cc -v`,
  `cc -print-prog-name=ld`) next to the Rust toolchain. The doctor prints both.

## Pins that must agree

| pin | where | checked by |
| --- | --- | --- |
| Rust 1.98.1, clippy, rustfmt | `rust-toolchain.toml`; the `ci`, App, Build binaries and Native workflows run `rustup show active-toolchain \|\| rustup toolchain install` from it, the diagnostic workflows rely on rustup's auto-install | doctor; CI cache keys hash the file |
| `Cargo.lock` | every CI cargo command uses `--locked` | `cargo build --locked` fails on drift |
| Action SHAs | one SHA per action across `.github/workflows`, tag in the trailing comment | `git ls-remote --tags https://github.com/<owner>/<action>.git refs/tags/<tag>` must print that SHA |
| Swift image | `swift:6.4.0-noble@sha256:...` in `ci.yml` | doctor reports the local Swift and names the image |
| Node >= 22.13.0, pnpm 11.25.0 | `web/package.json` (`engines`, `packageManager`); `web.yml` installs Node 22 and that pnpm | doctor |
| Python >= 3.14 | `pyproject.toml` `requires-python`; CI installs the latest uv | doctor (`uv python find`) |
| PDFium `chromium/8066`, docling `models-v1` | `native/manifest.json` with SHA-256 pins | `native/fetch.sh`; doctor reports presence |

The workflows pin the installer step, not a second copy of the version: the
toolchain file is the single source, so bumping it is one change plus a
`Cargo.lock` validation with the new compiler.
