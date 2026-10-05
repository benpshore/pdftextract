//! Anonymous-mapping probe: proves (in a disposable process) that an installed
//! address-space limit is really enforced by the kernel.

use rustix::io::Errno;
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous, munmap};

/// Map `length` bytes of fresh private read/write memory, optionally write to
/// its first and last byte, and unmap it. `Err` carries the `mmap` errno
/// (`ENOMEM` once the mapping would exceed `RLIMIT_AS`).
pub fn anonymous_mapping_probe(length: usize, touch: bool) -> Result<(), Errno> {
    if length == 0 {
        return Err(Errno::INVAL);
    }
    // SAFETY: null requests a fresh kernel-chosen address. No references
    // alias this private mapping; the returned region is always unmapped.
    let pointer = unsafe {
        mmap_anonymous(
            std::ptr::null_mut(),
            length,
            ProtFlags::READ | ProtFlags::WRITE,
            MapFlags::PRIVATE,
        )
    }?;
    if touch {
        // SAFETY: these two bytes are inside the fresh nonempty writable
        // mapping. Volatile writes prove usable pages.
        unsafe {
            pointer.cast::<u8>().write_volatile(1);
            pointer.cast::<u8>().add(length - 1).write_volatile(1);
        }
    }
    // SAFETY: this is the untouched base and length from mmap; no references
    // to the mapping exist when it is released.
    unsafe { munmap(pointer, length) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_mapping_round_trips() {
        anonymous_mapping_probe(4096, true).unwrap();
        anonymous_mapping_probe(4096, false).unwrap();
        assert_eq!(anonymous_mapping_probe(0, true), Err(Errno::INVAL));
    }
}
