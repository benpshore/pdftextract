//! Finder's context menu: the two `NSServices` entries declared in the app
//! bundle's Info.plist send `getText:userData:error:` and
//! `getBibliography:userData:error:` to the application's services provider.
//! That provider is an Objective-C class declared here at run time; each
//! message reads the file URLs off the pasteboard and hands the paths to the
//! handler the application registered with [`install`]. macOS calls these on
//! the main thread.

// The `objc` 0.2 macros expand `#[cfg(feature = "cargo-clippy")]` into this
// crate, where no such feature exists.
#![allow(unexpected_cfgs)]

use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;
use std::sync::OnceLock;

use objc::declare::ClassDecl;
use objc::runtime::{BOOL, Class, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};

type Id = *mut Object;

/// Receives the file paths a Service was invoked on.
pub type Handler = fn(Vec<PathBuf>);

/// The handlers for `getText:` and `getBibliography:`, set once by [`install`].
static HANDLERS: OnceLock<(Handler, Handler)> = OnceLock::new();

static PROVIDER_CLASS: OnceLock<&'static Class> = OnceLock::new();

/// The `TPEServiceProvider` class, declared once.
fn provider_class() -> &'static Class {
    PROVIDER_CLASS.get_or_init(|| {
        let mut decl =
            ClassDecl::new("TPEServiceProvider", class!(NSObject)).expect("class name is unused");
        // SAFETY: both selectors are declared with the `(id, SEL, id, id,
        // NSString **)` signature `NSServices` sends, matching the `extern "C"`
        // functions registered here.
        unsafe {
            decl.add_method(
                sel!(getText:userData:error:),
                get_text as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
            );
            decl.add_method(
                sel!(getBibliography:userData:error:),
                get_bibliography as extern "C" fn(&Object, Sel, Id, Id, *mut c_void),
            );
        }
        decl.register()
    })
}

extern "C" fn get_text(_: &Object, _: Sel, pasteboard: Id, _: Id, _: *mut c_void) {
    if let Some((text, _)) = HANDLERS.get() {
        // SAFETY: `pasteboard` is the live NSPasteboard macOS passed for this message.
        text(unsafe { file_paths(pasteboard) });
    }
}

extern "C" fn get_bibliography(_: &Object, _: Sel, pasteboard: Id, _: Id, _: *mut c_void) {
    if let Some((_, bibliography)) = HANDLERS.get() {
        // SAFETY: `pasteboard` is the live NSPasteboard macOS passed for this message.
        bibliography(unsafe { file_paths(pasteboard) });
    }
}

/// The file URLs on the pasteboard as paths (`NSSendFileTypes` puts them there).
///
/// # Safety
/// `pasteboard` must be a live `NSPasteboard`.
unsafe fn file_paths(pasteboard: Id) -> Vec<PathBuf> {
    // SAFETY: every message below is sent to a live object with the documented
    // selector and argument types; returned ids are checked for null before use,
    // and `UTF8String` is valid for the life of the autoreleased `path`.
    unsafe {
        let url_class: Id = std::ptr::from_ref::<Class>(class!(NSURL)).cast_mut().cast();
        let classes: Id = msg_send![class!(NSArray), arrayWithObject: url_class];
        let options: Id = msg_send![class!(NSDictionary), dictionary];
        let urls: Id = msg_send![pasteboard, readObjectsForClasses: classes options: options];
        if urls.is_null() {
            return Vec::new();
        }
        let count: usize = msg_send![urls, count];
        (0..count)
            .filter_map(|i| {
                let url: Id = msg_send![urls, objectAtIndex: i];
                let is_file: BOOL = msg_send![url, isFileURL];
                if is_file != YES {
                    return None;
                }
                let path: Id = msg_send![url, path];
                if path.is_null() {
                    return None;
                }
                let utf8: *const c_char = msg_send![path, UTF8String];
                if utf8.is_null() {
                    return None;
                }
                Some(PathBuf::from(
                    CStr::from_ptr(utf8).to_string_lossy().into_owned(),
                ))
            })
            .collect()
    }
}

/// Make the shared `NSApplication` answer the two Services with a fresh
/// provider that forwards to `on_text` and `on_bibliography`. Call once the
/// application exists (inside the GPUI `Application::run`). A second call
/// keeps the first handlers and returns `false`.
pub fn install(on_text: Handler, on_bibliography: Handler) -> bool {
    let installed = HANDLERS.set((on_text, on_bibliography)).is_ok();
    // SAFETY: `sharedApplication` exists once the application is running;
    // `setServicesProvider:` retains the fresh provider object.
    unsafe {
        let app: Id = msg_send![class!(NSApplication), sharedApplication];
        let provider: Id = msg_send![provider_class(), new];
        let _: () = msg_send![app, setServicesProvider: provider];
    }
    installed
}

#[cfg(test)]
mod tests {
    use super::provider_class;
    use objc::runtime::{BOOL, YES};
    use objc::{msg_send, sel, sel_impl};

    #[test]
    fn provider_answers_both_service_messages() {
        let class = provider_class();
        // SAFETY: `new` on a registered NSObject subclass; `respondsToSelector:`
        // is a pure query on that live object.
        let (text, biblio, other): (BOOL, BOOL, BOOL) = unsafe {
            let provider: *mut objc::runtime::Object = msg_send![class, new];
            (
                msg_send![provider, respondsToSelector: sel!(getText:userData:error:)],
                msg_send![provider, respondsToSelector: sel!(getBibliography:userData:error:)],
                msg_send![provider, respondsToSelector: sel!(getSomethingElse:)],
            )
        };
        assert_eq!(text, YES);
        assert_eq!(biblio, YES);
        assert_ne!(other, YES);
    }
}
