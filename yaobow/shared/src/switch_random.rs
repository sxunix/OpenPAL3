//! Entropy source for Switch homebrew.
//!
//! `getrandom` has no backend for this target, and two incompatible major
//! versions are in the graph (0.2 via `config`/`rand`, 0.3 via `zip`), each with
//! its own custom-backend hook. Both are answered here from libnx's `randomGet`,
//! which is backed by the system CSPRNG.
//!
//! 0.2 needs its `custom` feature enabled (see this crate's horizon-only
//! dependency) or it refuses to compile at all; 0.3 only needs
//! `--cfg getrandom_backend="custom"` plus this symbol at link time.

use std::os::raw::c_void;

unsafe extern "C" {
    fn randomGet(buf: *mut c_void, len: usize);
}

/// getrandom 0.2's hook. Returns 0 on success; a non-zero value would be
/// interpreted as an `Error` code, and `randomGet` cannot fail.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_custom(dest: *mut u8, len: usize) -> u32 {
    unsafe { randomGet(dest as *mut c_void, len) };
    0
}

/// getrandom 0.3's hook.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    unsafe { randomGet(dest as *mut c_void, len) };
    Ok(())
}
