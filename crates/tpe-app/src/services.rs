//! Finder's context menu: the two `NSServices` entries declared in the
//! bundle's Info.plist (`bundle/Info.plist`) send `getText:userData:error:`
//! and `getBibliography:userData:error:` to the app's services provider.
//! The Objective-C provider lives in `tpe_ffi::finder_services`; this module
//! only connects its two messages to `gui::intake`.

use std::path::PathBuf;

use tpe_app::jobs::Action;

fn text(paths: Vec<PathBuf>) {
    crate::gui::intake(Action::Text, paths);
}

fn bibliography(paths: Vec<PathBuf>) {
    crate::gui::intake(Action::Bibliography, paths);
}

/// Make the shared `NSApplication` answer the two Services with a fresh
/// provider. Call once the application exists (inside `Application::run`).
pub fn install() {
    tpe_ffi::finder_services::install(text, bibliography);
}
