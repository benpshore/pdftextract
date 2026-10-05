//! Loopback-only HTTP service that runs scanned PDFs and still images through
//! the engine's docling backends for the browser alpha
//! (`web/lib/scan-service.ts`). See `docs/DOCLING.md`.
//!
//! Security model, in short:
//! - binds a loopback address only and refuses anything else;
//! - every request needs the per-launch bearer token that the binary prints
//!   once at start-up;
//! - `Host` must name the bound loopback address and `Origin`, when a browser
//!   sends one, must be one of the configured web-app origins;
//! - request size, page count and concurrency are bounded;
//! - each scan runs in a disposable worker process of this same binary with a
//!   hard address-space limit and a deadline, so a hung or crashed conversion
//!   is killed and never takes the service down;
//! - the service opens no outbound connections and keeps no state on disk.

#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

pub mod auth;
pub mod capabilities;
pub mod http;
pub mod image_pdf;
pub mod models;
pub mod multipart;
pub mod scan;
pub mod server;
pub mod worker;

/// Identity reported by `GET /capabilities`.
pub const SERVICE_NAME: &str = "tpe-scan-service";
/// Crate version (the workspace version; releases are tagged).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The pinned docling.rs release the engine's backends are built against.
pub const DOCLING_VERSION: &str = "1.69.2";
