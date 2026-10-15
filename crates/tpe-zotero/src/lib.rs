//! Zotero integration for the text-processing-engine workbench.
//!
//! * [`client`]: a Zotero Web API v3 client over `ureq` (read items,
//!   children, collections, tags, groups and key permissions; item-type
//!   templates; create items, notes, collections and link attachments with
//!   `Zotero-Write-Token`; versioned `PATCH` / `DELETE`; the three-step
//!   attachment file upload; `Backoff`, `Retry-After` and retries).
//! * [`item`]: Zotero item JSON and the mapping to and from
//!   [`tpe_common::PaperRecord`], including template-based new items.
//! * [`schema`]: parsers for `/itemTypes`, `/itemTypeFields`,
//!   `/itemTypeCreatorTypes`, `/items/new`, `/keys/current`,
//!   `/users/<id>/groups` and `/tags`.
//! * [`upload`]: the file-upload authorisation, storage upload and
//!   registration bodies.
//! * [`retry`]: pure retry and backoff decisions.
//! * [`local`]: read-only access to a local Zotero 7 data directory
//!   (`zotero.sqlite` opened with `immutable=1`, PDFs under `storage/<key>/`).
//! * [`import`]: the JSON body the Zotero plugin posts to the app's local
//!   `POST /zotero/import` endpoint.
//! * [`headers`]: `Link` / version header parsing used for pagination.
//!
//! Nothing here performs network I/O in tests; every parser is a pure
//! function over recorded JSON or header strings.

#![allow(
    clippy::must_use_candidate,
    clippy::module_name_repetitions,
    clippy::missing_errors_doc
)]

pub mod client;
pub mod error;
pub mod headers;
pub mod import;
pub mod item;
pub mod local;
pub mod retry;
pub mod schema;
pub mod upload;

pub use client::{
    ApiKey, ItemQuery, Library, Page, RawResponse, WriteFailure, WriteResult, ZCollection,
    ZoteroClient,
};
pub use error::ZError;
pub use item::{ZCreator, ZItem, ZItemPatch};
pub use local::{LocalAttachment, LocalCollection, LocalItem, LocalLibrary, find_zotero_dir};
pub use retry::RetryPolicy;
pub use schema::{LibraryAccess, NamedEntry, ZGroup, ZKeyInfo, ZTag};
pub use upload::{FileDescriptor, UploadAuthorization, UploadTarget};
