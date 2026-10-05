//! Stop disposable workers on allocation failure, including fallible allocation.
//!
//! A decoder can suppress an error from `Vec::try_reserve`/`Read::read_to_end`
//! and return a successful prefix. Once the native worker's OS limits and core
//! policy are installed, a null Rust allocation must end that worker instead.
//! Controllers and library users keep their normal allocator behavior. This is
//! not an additional memory limit and does not intercept native malloc/mmap.
//!
//! The binary that wants this policy declares the allocator itself:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOCATOR: tpe_ffi::alloc::WorkerAllocator = tpe_ffi::alloc::WorkerAllocator;
//! ```
//!
//! and calls [`enforce`] after installing its worker limits.

use std::alloc::{GlobalAlloc, Layout, LayoutError, System};
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// [`System`] with a one-way abort-on-null policy for disposable workers.
pub struct WorkerAllocator;

fn checked(pointer: *mut u8) -> *mut u8 {
    if pointer.is_null() && ENABLED.load(Ordering::SeqCst) {
        allocation_failed();
    }
    pointer
}

#[cold]
fn allocation_failed() -> ! {
    // No formatting, locks, allocation or unwinding inside GlobalAlloc. In
    // particular, handle_alloc_error is permitted to unwind under some policies.
    // Worker stderr is a controller-owned capture file; failure to write must
    // still terminate the process.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let _ = rustix::io::write(
        rustix::stdio::stderr(),
        b"memory allocation failed in native worker\n",
    );
    std::process::abort()
}

// SAFETY: every allocation/deallocation uses System with its original layout;
// failure can abort, but none of these methods may unwind. Activation never
// changes allocator ownership, including for storage allocated before activation.
unsafe impl GlobalAlloc for WorkerAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        checked(unsafe { System.alloc(layout) })
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        checked(unsafe { System.alloc_zeroed(layout) })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: the pointer and layout came from this same System allocator.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, bytes: usize) -> *mut u8 {
        // SAFETY: the caller supplies the original allocation and valid new size.
        checked(unsafe { System.realloc(pointer, layout, bytes) })
    }
}

/// One-way process policy; call only after installing disposable-worker limits.
pub fn enforce() {
    ENABLED.store(true, Ordering::SeqCst);
}

/// Whether [`enforce`] has been called in this process.
pub fn enforced() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

/// Request `size` zeroed bytes from the global allocator and release them
/// without touching the mapping. `Ok(true)` when the allocation succeeded,
/// `Ok(false)` when the allocator returned null (or, under [`enforce`], the
/// process aborted before returning).
pub fn zeroed_allocation_succeeds(size: usize, align: usize) -> Result<bool, LayoutError> {
    let layout = Layout::from_size_align(size, align)?;
    // SAFETY: the layout is non-zero and valid. An unexpected successful
    // allocation is freed with the exact pointer/layout it was returned with.
    let pointer =
        std::hint::black_box(unsafe { std::alloc::alloc_zeroed(std::hint::black_box(layout)) });
    if pointer.is_null() {
        return Ok(false);
    }
    // SAFETY: exact pointer/layout returned above, still live, freed once.
    unsafe { std::alloc::dealloc(pointer, layout) };
    Ok(true)
}

/// Allocate `size` zeroed bytes, check that the first byte reads as zero, and
/// free them. `Ok(true)` only when every step held.
pub fn zeroed_small_control(size: usize, align: usize) -> Result<bool, LayoutError> {
    let layout = Layout::from_size_align(size.max(1), align)?;
    // SAFETY: a nonzero valid layout; its returned allocation is freed once.
    let pointer =
        std::hint::black_box(unsafe { std::alloc::alloc_zeroed(std::hint::black_box(layout)) });
    if pointer.is_null() {
        return Ok(false);
    }
    // SAFETY: pointer is a live, non-null allocation of at least one byte.
    let first = unsafe { *pointer };
    // SAFETY: exact layout and pointer from alloc_zeroed above.
    unsafe { std::alloc::dealloc(pointer, layout) };
    Ok(first == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_control_round_trips_through_system() {
        assert!(zeroed_small_control(4096, 8).unwrap());
        assert!(zeroed_allocation_succeeds(64, 8).unwrap());
        assert!(Layout::from_size_align(8, 3).is_err());
        assert!(zeroed_small_control(8, 3).is_err());
    }

    #[test]
    fn enforcement_is_off_in_this_process() {
        assert!(!enforced());
    }
}
