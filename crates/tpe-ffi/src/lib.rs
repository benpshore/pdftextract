//! `tpe-ffi`: every `unsafe` block in the workspace lives here.
//!
//! The workspace forbids `unsafe_code` (root `Cargo.toml`,
//! `[workspace.lints.rust]`); this crate is the single exemption. Each module
//! wraps one foreign boundary behind a safe API and documents the invariant
//! every `unsafe` block relies on in a `// SAFETY:` comment:
//!
//! - [`alloc`]: the native worker's global allocator, which aborts the
//!   disposable worker on a null allocation once enforcement is switched on.
//! - [`mem`]: an anonymous-mapping probe used by the worker-limit tests.
//! - [`process`] (macOS): the process's virtual size through `proc_pidinfo`.
//! - [`provider`] (feature `provider`): the C ABI loader for optional `MuPDF` and
//!   Poppler providers (`native/provider.h`).
//! - [`av_speech`] (macOS, feature `speech`): `AVSpeechSynthesizer` captured to
//!   PCM and `AVAudioPlayer` playback.
//! - [`finder_services`] (macOS, feature `finder-services`): the Objective-C
//!   `NSServices` provider behind Finder's context menu.
//!
//! Nothing here parses a PDF, touches the network, or decides policy: callers
//! own fingerprinting, limits, and what to do with the data that comes back.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

pub mod alloc;
#[cfg(all(target_os = "macos", feature = "speech"))]
pub mod av_speech;
#[cfg(all(target_os = "macos", feature = "finder-services"))]
pub mod finder_services;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod mem;
#[cfg(target_os = "macos")]
pub mod process;
#[cfg(feature = "provider")]
pub mod provider;
