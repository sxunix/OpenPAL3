//! libc functions devkitA64's newlib does not provide.
//!
//! Three call sites need these at final link:
//!   * `std::sys::random::hashmap_random_keys` -> `getrandom`
//!   * `std::sys::thread::unix::Thread::new`   -> `sysconf`
//!   * Lua 5.0's `liolib.c` (`io.popen`)       -> `popen` / `pclose`
//!
//! Constant values are taken from devkitA64's `sys/unistd.h`, not guessed.

use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};

unsafe extern "C" {
    fn randomGet(buf: *mut c_void, len: usize);

}

// From devkitA64 sys/unistd.h.
const SC_PAGESIZE: c_int = 8;
const SC_NPROCESSORS_ONLN: c_int = 10;

/// Switch page size, and the count of cores homebrew is allowed to schedule on
/// (the OS reserves the fourth core of the Tegra X1 for itself).
const PAGE_SIZE: c_long = 0x1000;
const USABLE_CORES: c_long = 3;

/// Linux-style `getrandom(2)`. libnx's `randomGet` is backed by the system
/// CSPRNG and cannot fail or short-read, so the whole buffer is always filled.
#[unsafe(no_mangle)]
unsafe extern "C" fn getrandom(buf: *mut c_void, buflen: usize, _flags: c_uint) -> isize {
    if buf.is_null() {
        return -1;
    }
    unsafe { randomGet(buf, buflen) };
    buflen as isize
}

/// Only the two queries that actually get asked are answered; anything else
/// returns -1, which is the documented "no determinate limit" reply and what
/// callers already have to handle.
#[unsafe(no_mangle)]
unsafe extern "C" fn sysconf(name: c_int) -> c_long {
    match name {
        SC_PAGESIZE => PAGE_SIZE,
        SC_NPROCESSORS_ONLN => USABLE_CORES,
        _ => -1,
    }
}

/// Horizon has no fork/exec available to homebrew, so `io.popen` cannot work.
/// Returning null is the documented failure mode and Lua turns it into a normal
/// script-level error rather than a crash.
#[unsafe(no_mangle)]
unsafe extern "C" fn popen(_command: *const c_char, _mode: *const c_char) -> *mut c_void {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
unsafe extern "C" fn pclose(_stream: *mut c_void) -> c_int {
    -1
}

// Note on thread-locals: the target spec sets `has-thread-local: false`, because
// native ELF TLS does not work here -- rustc emits a bare `mrs xN, tpidr_el0`
// plus a static offset, while libnx points `tpidr_el0` at its own 0x200-byte
// ThreadVars block and reaches the real TLS image indirectly (what devkitPro's
// `-mtp=soft` / `__aarch64_read_tp` exists for, and LLVM has no equivalent of).
// std therefore falls back to POSIX keys, which devkitPro's libsysbase already
// implements on top of libnx TLS slots -- nothing to provide here.

// The layout `libc::stat` must have on this target, measured against
// devkitA64's own `sys/stat.h` -- not guessed, and not the layout libc ships
// by default (its newlib types are sized for the 32-bit 3DS, which puts
// st_mode at 8 and st_size at 32, the latter landing inside st_atim).
//
// setup-switch-toolchain.sh corrects the four type aliases that cause this.
// These assertions exist so that a libc version whose layout moves again
// fails the build loudly instead of silently handing std a garbage file size
// -- which is how it first showed up: a 52-byte config read reserving 1.7 GB
// and dying with ErrorKind::OutOfMemory.
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<libc::stat>() == 104);
    assert!(offset_of!(libc::stat, st_dev) == 0);
    assert!(offset_of!(libc::stat, st_ino) == 2);
    assert!(offset_of!(libc::stat, st_mode) == 4);
    assert!(offset_of!(libc::stat, st_nlink) == 8);
    assert!(offset_of!(libc::stat, st_uid) == 10);
    assert!(offset_of!(libc::stat, st_gid) == 12);
    assert!(offset_of!(libc::stat, st_rdev) == 14);
    assert!(offset_of!(libc::stat, st_size) == 16);
    assert!(offset_of!(libc::stat, st_atim) == 24);
    assert!(offset_of!(libc::stat, st_blksize) == 72);
    assert!(offset_of!(libc::stat, st_blocks) == 80);
};
