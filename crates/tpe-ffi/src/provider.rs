//! Loader for the optional C ABI PDF providers (`native/provider.h`, ABI 1).
//!
//! Provider and runtime files are trusted deployment code, not PDF inputs. The
//! caller fingerprints both files before and after symbol resolution through
//! the `verify` callback; this module only loads them from the absolute paths
//! it is given, checks the ABI, engine and runtime anchor, and marshals bytes
//! across the boundary. The provider contract forbids exceptions crossing the
//! ABI and requires thread-safe independent sessions; native faults are
//! contained by the enclosing disposable worker, never by this crate.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;
use std::sync::Arc;

use libloading::Library;

/// The provider ABI this loader speaks (`tpe_pdf_provider_abi_version`).
pub const ABI: u32 = 1;

type Open = unsafe extern "C" fn(
    *const u8,
    usize,
    *const c_char,
    *mut *mut c_void,
    *mut u32,
    *mut c_char,
    usize,
) -> c_int;
type Page = unsafe extern "C" fn(
    *mut c_void,
    u32,
    usize,
    *mut *mut u8,
    *mut usize,
    *mut c_char,
    usize,
) -> c_int;
type Close = unsafe extern "C" fn(*mut c_void);
type Free = unsafe extern "C" fn(*mut u8);

/// Why a provider refused to open a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Provider status 2.
    PasswordRequired,
    /// Provider status 3.
    WrongPassword,
    /// Any other non-zero status, with the provider's message.
    Failed(String),
}

/// A loaded provider/runtime pair with its resolved entry points.
#[derive(Debug)]
pub struct ProviderLibrary {
    // Function pointers are used only while both libraries remain loaded.
    _provider: Library,
    _runtime: Library,
    open: Open,
    page: Page,
    close: Close,
    free: Free,
    engine: String,
    version: String,
}

impl ProviderLibrary {
    /// Load `runtime` then `provider`, check the ABI version, that the
    /// provider names `expected_engine`, and that it links against the loaded
    /// runtime (its anchor symbol address matches). `verify` runs before the
    /// libraries are opened and again after every symbol is resolved so a
    /// file replaced in between is rejected.
    pub fn load(
        provider: &Path,
        runtime: &Path,
        expected_engine: &str,
        verify: &dyn Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        verify()?;
        // SAFETY: explicit, fingerprinted trusted deployment files supplied by
        // the caller. Loading runs their initializers; the provider contract
        // requires them to be side-effect free and exception free.
        let runtime = unsafe { Library::new(runtime) }.map_err(|e| e.to_string())?;
        // SAFETY: as above, for the provider library.
        let provider = unsafe { Library::new(provider) }.map_err(|e| e.to_string())?;
        // SAFETY: the symbol has this exact signature in `native/provider.h`.
        let abi = unsafe {
            provider.get::<unsafe extern "C" fn() -> u32>(b"tpe_pdf_provider_abi_version\0")
        }
        .map_err(|e| e.to_string())?;
        // SAFETY: a pure query function of the loaded, still-live provider.
        if unsafe { abi() } != ABI {
            return Err("unsupported native provider ABI".to_string());
        }
        let string = |name: &[u8]| -> Result<String, String> {
            // SAFETY: the symbol has this exact signature in `native/provider.h`.
            let function = unsafe { provider.get::<unsafe extern "C" fn() -> *const c_char>(name) }
                .map_err(|e| e.to_string())?;
            // SAFETY: the provider returns a static NUL-terminated string that
            // outlives the library; `bounded_string` reads at most 256 bytes.
            unsafe { bounded_string(function()) }
        };
        let engine = string(b"tpe_pdf_provider_engine\0")?;
        let version = string(b"tpe_pdf_provider_version\0")?;
        if engine != expected_engine
            || version.is_empty()
            || !version.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("native provider engine/version mismatch".to_string());
        }
        let anchor_symbol = std::ffi::CString::new(string(b"tpe_pdf_provider_runtime_symbol\0")?)
            .map_err(|e| e.to_string())?;
        // SAFETY: the anchor is read as a data symbol address only; it is never
        // called or dereferenced.
        let runtime_anchor =
            unsafe { runtime.get::<*const c_void>(anchor_symbol.as_bytes_with_nul()) }
                .map_err(|e| e.to_string())?;
        // SAFETY: the symbol has this exact signature in `native/provider.h`.
        let anchor = unsafe {
            provider.get::<unsafe extern "C" fn() -> usize>(b"tpe_pdf_provider_runtime_anchor\0")
        }
        .map_err(|e| e.to_string())?;
        // SAFETY: a pure query function of the loaded, still-live provider.
        if unsafe { anchor() } != *runtime_anchor as usize {
            return Err("provider does not use the fingerprinted native runtime".to_string());
        }
        // SAFETY: each symbol has the matching signature in `native/provider.h`;
        // the raw pointers are copied out while the library is live and are only
        // called while `Self` keeps both libraries loaded.
        let open = *unsafe { provider.get::<Open>(b"tpe_pdf_provider_open\0") }
            .map_err(|e| e.to_string())?;
        // SAFETY: as above.
        let page = *unsafe { provider.get::<Page>(b"tpe_pdf_provider_page\0") }
            .map_err(|e| e.to_string())?;
        // SAFETY: as above.
        let close = *unsafe { provider.get::<Close>(b"tpe_pdf_provider_close\0") }
            .map_err(|e| e.to_string())?;
        // SAFETY: as above.
        let free = *unsafe { provider.get::<Free>(b"tpe_pdf_provider_free\0") }
            .map_err(|e| e.to_string())?;
        verify()?;
        Ok(Self {
            _provider: provider,
            _runtime: runtime,
            open,
            page,
            close,
            free,
            engine,
            version,
        })
    }

    /// The engine name the provider reported (`tpe_pdf_provider_engine`).
    pub fn engine(&self) -> &str {
        &self.engine
    }

    /// The version string the provider reported (`tpe_pdf_provider_version`).
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Open `bytes` (optionally with a password) and keep them alive for the
    /// document's lifetime, as the provider contract requires.
    pub fn open(
        self: &Arc<Self>,
        bytes: Box<[u8]>,
        password: Option<&CStr>,
    ) -> Result<ProviderDocument, OpenError> {
        let mut handle = std::ptr::null_mut();
        let mut pages = 0;
        let mut error = [0 as c_char; 512];
        // SAFETY: ABI-checked trusted provider; `bytes` and this library outlive
        // the returned handle, and `error` is a writable buffer of the stated
        // length.
        let status = unsafe {
            (self.open)(
                bytes.as_ptr(),
                bytes.len(),
                password.map_or(std::ptr::null(), CStr::as_ptr),
                &raw mut handle,
                &raw mut pages,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        if status != 0 || handle.is_null() || pages == 0 {
            if !handle.is_null() {
                // SAFETY: a handle the provider just returned and nothing else owns.
                unsafe { (self.close)(handle) };
            }
            return Err(match status {
                2 => OpenError::PasswordRequired,
                3 => OpenError::WrongPassword,
                _ => OpenError::Failed(error_message(&error)),
            });
        }
        Ok(ProviderDocument {
            api: Arc::clone(self),
            handle,
            _bytes: bytes,
            pages,
        })
    }
}

/// An open provider document; closed on drop.
pub struct ProviderDocument {
    api: Arc<ProviderLibrary>,
    handle: *mut c_void,
    _bytes: Box<[u8]>,
    pages: u32,
}

impl ProviderDocument {
    pub fn page_count(&self) -> u32 {
        self.pages
    }

    /// The library this document was opened with.
    pub fn library(&self) -> &ProviderLibrary {
        &self.api
    }

    /// The provider's JSON for 1-based `page`, copied into Rust memory and
    /// released on the provider side before returning. `max_output` is the
    /// byte cap the provider must honour.
    pub fn page(&self, page: u32, max_output: usize) -> Result<Vec<u8>, String> {
        let mut pointer = std::ptr::null_mut();
        let mut length = 0;
        let mut error = [0 as c_char; 512];
        // SAFETY: live handle and bounded output ABI; provider memory is freed
        // on every path below, before the libraries or document can be dropped.
        let status = unsafe {
            (self.api.page)(
                self.handle,
                page,
                max_output,
                &raw mut pointer,
                &raw mut length,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        let output = Output {
            api: &self.api,
            pointer,
        };
        if status != 0 {
            let mut message = error_message(&error);
            if !output.pointer.is_null() {
                message.push_str(" (provider returned output on failure)");
            }
            return Err(message);
        }
        if length == 0 || length > max_output || output.pointer.is_null() {
            return Err("provider returned invalid or oversized page output".to_string());
        }
        // SAFETY: the provider reports `length` readable bytes at `pointer`,
        // which `output` keeps alive until this function returns.
        let bytes = unsafe { std::slice::from_raw_parts(output.pointer, length) }.to_vec();
        Ok(bytes)
    }
}

impl Drop for ProviderDocument {
    fn drop(&mut self) {
        // SAFETY: a handle this document owns; closed exactly once, while the
        // libraries are still loaded (`api` is dropped after this field).
        unsafe { (self.api.close)(self.handle) };
    }
}

struct Output<'a> {
    api: &'a ProviderLibrary,
    pointer: *mut u8,
}

impl Drop for Output<'_> {
    fn drop(&mut self) {
        if !self.pointer.is_null() {
            // SAFETY: provider-allocated output is released once through the
            // provider's own free function.
            unsafe { (self.api.free)(self.pointer) };
        }
    }
}

/// Read a provider identity string.
///
/// # Safety
/// `pointer` is null or readable through its NUL terminator, which must occur
/// within 256 bytes.
unsafe fn bounded_string(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("provider returned a null identity string".to_string());
    }
    let mut bytes = Vec::new();
    for offset in 0..256 {
        // SAFETY: the caller guarantees readability up to the NUL, which this
        // loop stops at; `offset` never exceeds 255.
        let byte = unsafe { *pointer.add(offset) }.cast_unsigned();
        if byte == 0 {
            return String::from_utf8(bytes).map_err(|e| e.to_string());
        }
        bytes.push(byte);
    }
    Err("provider identity string exceeds 255 bytes".to_string())
}

/// The NUL-terminated prefix of a provider error buffer, lossily decoded.
pub fn error_message(error: &[c_char]) -> String {
    String::from_utf8_lossy(
        &error
            .iter()
            .take_while(|&&b| b != 0)
            .map(|b| b.cast_unsigned())
            .collect::<Vec<_>>(),
    )
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_strings_are_bounded() {
        let text = c"mupdf";
        // SAFETY: a static NUL-terminated string literal.
        assert_eq!(unsafe { bounded_string(text.as_ptr()) }.unwrap(), "mupdf");
        // SAFETY: null is explicitly accepted.
        assert!(unsafe { bounded_string(std::ptr::null()) }.is_err());
        let long = vec![b'x'; 300];
        let long = std::ffi::CString::new(long).unwrap();
        // SAFETY: a NUL-terminated string longer than the bound.
        assert!(unsafe { bounded_string(long.as_ptr()) }.is_err());
    }

    #[test]
    fn error_messages_stop_at_nul() {
        let mut buffer = [0 as c_char; 8];
        for (slot, byte) in buffer.iter_mut().zip(b"abc\0def") {
            *slot = *byte as c_char;
        }
        assert_eq!(error_message(&buffer), "abc");
    }

    #[test]
    fn missing_libraries_fail_before_any_symbol_is_resolved() {
        let error = ProviderLibrary::load(
            Path::new("/nonexistent/provider.so"),
            Path::new("/nonexistent/runtime.so"),
            "mupdf",
            &|| Ok(()),
        )
        .unwrap_err();
        assert!(!error.is_empty());
        let refused = ProviderLibrary::load(
            Path::new("/nonexistent/provider.so"),
            Path::new("/nonexistent/runtime.so"),
            "mupdf",
            &|| Err("fingerprint changed".to_string()),
        )
        .unwrap_err();
        assert_eq!(refused, "fingerprint changed");
    }
}
