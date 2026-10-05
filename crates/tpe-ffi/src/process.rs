//! Darwin process accounting through `proc_pidinfo`.

/// The current process's virtual size as XNU reports it (`pti_virtual_size`).
///
/// XNU reports `vm_map_adjusted_size`. Exotic/transformed processes may have
/// reserved regions subtracted while `RLIMIT_AS` checks the full map size, so
/// a caller installing a limit must fail closed rather than widen its cap.
/// <https://github.com/apple-oss-distributions/xnu/blob/xnu-11417.140.69/osfmk/kern/bsd_kern.c#L1010>
pub fn virtual_bytes() -> Result<u64, String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_taskinfo>::uninit();
    let size = i32::try_from(std::mem::size_of::<libc::proc_taskinfo>())
        .map_err(|_| "proc_taskinfo size does not fit an i32".to_string())?;
    let pid =
        i32::try_from(std::process::id()).map_err(|_| "pid does not fit an i32".to_string())?;
    // SAFETY: the buffer is correctly aligned and exactly `size` bytes long;
    // proc_pidinfo receives its exclusive writable pointer and retains none.
    // Fields are read only after the exact full-structure return is confirmed.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTASKINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return Err(format!(
            "worker virtual-memory accounting returned {written} bytes, expected {size}"
        ));
    }
    // SAFETY: the successful exact-size call initialized the full structure.
    let info = unsafe { info.assume_init() };
    Ok(info.pti_virtual_size)
}
