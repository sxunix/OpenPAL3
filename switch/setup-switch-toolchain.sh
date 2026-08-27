#!/bin/sh
# Re-apply the three out-of-repo patches the OpenPAL3 Switch build depends on.
#
# They live in cargo's registry/vendor/git caches, so ANY of these wipes them:
#   * `rustup update` / toolchain reinstall   (rust-src vendored libc)
#   * cargo re-extracting a registry crate    (libc, libffi-sys)
#   * `cargo update` moving the filetime rev  (git checkout path changes)
#
# Idempotent: safe to run any number of times. Run it whenever the build
# starts failing with AT_FDCWD / config.sub / futimes errors.
set -eu

note() { printf '%s\n' "$*"; }

# Where the patch targets are expected. Printed up front so a run that
# patches nothing shows why (typically: the registry has not been unpacked
# yet on this machine).
note "registry src: $(ls -d "$HOME"/.cargo/registry/src/*/ 2>/dev/null | tr '\n' ' ')"
note "libc copies:  $(ls -d "$HOME"/.cargo/registry/src/*/libc-*/ "$HOME"/.rustup/toolchains/*/lib/rustlib/src/rust/library/vendor/libc-*/ 2>/dev/null | wc -l | tr -d ' ')"

# --- 1. libc: AT_* constants for the newlib `horizon` module -----------------
# std/src/sys/fs/unix.rs needs AT_FDCWD; libc's horizon module (written for
# the 3DS) lacks the whole AT_* family. Values from devkitA64's
# sys/_default_fcntl.h. Patch EVERY copy: -Z build-std uses the toolchain's
# vendored copy for std, while the workspace's own libc dep resolves from the
# registry (and switching crates.io mirrors creates additional copies).
PATCH_LIBC='
// --- switch probe patch: values from devkitA64 newlib sys/_default_fcntl.h ---
pub const AT_FDCWD: crate::c_int = -2;
pub const AT_EACCESS: crate::c_int = 0x0001;
pub const AT_SYMLINK_NOFOLLOW: crate::c_int = 0x0002;
pub const AT_SYMLINK_FOLLOW: crate::c_int = 0x0004;
pub const AT_REMOVEDIR: crate::c_int = 0x0008;
pub const AT_EMPTY_PATH: crate::c_int = 0x0010;'

for L in "$HOME"/.cargo/registry/src/*/libc-*/ \
         "$HOME"/.rustup/toolchains/*/lib/rustlib/src/rust/library/vendor/libc-*/; do
  F="$L/src/unix/newlib/horizon/mod.rs"
  [ -f "$F" ] || continue
  if grep -q "AT_FDCWD" "$F"; then
    note "libc  已打: $L"
  else
    printf '%s\n' "$PATCH_LIBC" >> "$F"
    note "libc  打上: $L"
  fi
  # Registry/vendored copies are checksum-verified; refresh our file's entry
  # or cargo restores the pristine copy on next use.
  CS="$L/.cargo-checksum.json"
  [ -f "$CS" ] && python3 - "$L" <<'PY'
import hashlib, json, sys, os
v = sys.argv[1]; cs = os.path.join(v, '.cargo-checksum.json')
try:
    d = json.load(open(cs)); rel = 'src/unix/newlib/horizon/mod.rs'
    if d.get('files'):
        d['files'][rel] = hashlib.sha256(
            open(os.path.join(v, rel), 'rb').read()).hexdigest()
        json.dump(d, open(cs, 'w'))
except Exception as e:
    print('  checksum skip:', e)
PY
done

# --- 1b. libc: struct stat layout for aarch64 horizon -------------------------
# libc's newlib types were sized for the 3DS (32-bit ARM). devkitA64's newlib
# uses 16-bit dev_t/ino_t and 64-bit blkcnt_t/blksize_t, so every field of
# `struct stat` after st_dev sits at the wrong offset on aarch64 -- measured
# against devkitA64's own sys/stat.h, st_mode is at 4 (libc said 8) and
# st_size at 16 (libc said 32, which lands inside st_atim).
#
# The visible damage: reading a 52-byte yaobow.toml reserved a buffer from a
# 1.7 GB "size" and failed with ErrorKind::OutOfMemory, and st_mode being
# wrong makes is_file/is_dir report nonsense -- which is what the recursive
# .cpk mount walks on.
#
# Fixing the four type aliases fixes the layout; the struct itself is left
# alone. Gated on aarch64 so the 3DS arm is untouched. The build additionally
# static-asserts the resulting offsets (shared/src/switch_libc.rs), so if a
# future libc moves them the build fails instead of silently reading garbage.
# Upstreaming candidate, like the AT_* constants.
for L in "$HOME"/.cargo/registry/src/*/libc-*/ \
         "$HOME"/.rustup/toolchains/*/lib/rustlib/src/rust/library/vendor/libc-*/; do
  F="$L/src/unix/newlib/mod.rs"
  [ -f "$F" ] || continue
  if grep -q 'switch probe: aarch64 horizon stat layout' "$F"; then
    note "stat  已打: $L"
  else
    python3 "$(dirname "$0")/patch-libc-stat.py" "$F" || exit 1
    note "stat  打上: $L"
    rm -rf "$(dirname "$0")/../target/aarch64-nintendo-switch/release/build/libc"
  fi
  CS="$L/.cargo-checksum.json"
  [ -f "$CS" ] && python3 "$(dirname "$0")/patch-libc-stat.py" --checksum "$L"
done

# --- 2. libffi-sys: config.sub host remap ------------------------------------
# libffi's autotools configure rejects --host=aarch64-nintendo-switch. The
# build script already remaps a handful of triples; add ours -> aarch64-none-elf
# (the ABI genuinely is bare-metal aarch64). A stale build-script binary keeps
# the pristine behaviour, so purge its cache too.
for L in "$HOME"/.cargo/registry/src/*/libffi-sys-*/; do
  F="$L/build/not_msvc.rs"
  [ -f "$F" ] || continue
  if grep -q "aarch64-nintendo-switch" "$F"; then
    note "libffi 已打: $L"
  else
    python3 - "$F" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
s = s.replace('''            // Everything else should be fine to pass straight through
            other => other,''','''            // config.sub has no notion of the Switch; the ABI is plain
            // bare-metal aarch64, which it does recognize.
            "aarch64-nintendo-switch" => "aarch64-none-elf",
            // Everything else should be fine to pass straight through
            other => other,''')
open(p, 'w').write(s)
PY
    note "libffi 打上: $L"
    rm -rf "$(dirname "$0")/../target/release/build/libffi-sys" \
           "$(dirname "$0")/../OpenPAL3/target/aarch64-nintendo-switch/release/build/libffi-sys"
  fi
done

# --- 3. filetime fork: horizon fallback --------------------------------------
# devkitA64's newlib exports no utime-family functions at all, so setting file
# times is honestly unsupported. Adds an `unsupported.rs` arm to the author's
# fork checkout (git checkouts carry no checksums).
for D in "$HOME"/.cargo/git/checkouts/filetime-*/*/; do
  M="$D/src/unix/mod.rs"
  [ -f "$M" ] || continue
  if grep -q "horizon" "$M"; then
    note "filetime 已打: $D"
    continue
  fi
  cat > "$D/src/unix/unsupported.rs" <<'RS'
//! Fallback for targets whose libc has no `utime` family at all.
//!
//! devkitA64's newlib exports none of `utimes`/`futimes`/`lutimes`/`utimensat`,
//! so timestamps simply cannot be set. Reporting `Unsupported` is honest and
//! lets callers that treat it as non-fatal (archive extraction, mostly) carry on.

use crate::FileTime;
use std::fs;
use std::io;
use std::path::Path;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "setting file times is not supported on this target",
    )
}

pub fn set_file_times(_p: &Path, _atime: FileTime, _mtime: FileTime) -> io::Result<()> {
    Err(unsupported())
}

pub fn set_file_mtime(_p: &Path, _mtime: FileTime) -> io::Result<()> {
    Err(unsupported())
}

pub fn set_file_atime(_p: &Path, _atime: FileTime) -> io::Result<()> {
    Err(unsupported())
}

pub fn set_file_handle_times(
    _f: &fs::File,
    _atime: Option<FileTime>,
    _mtime: Option<FileTime>,
) -> io::Result<()> {
    Err(unsupported())
}

pub fn set_symlink_file_times(_p: &Path, _atime: FileTime, _mtime: FileTime) -> io::Result<()> {
    Err(unsupported())
}
RS
  python3 - "$M" <<'PY'
import sys
p = sys.argv[1]; s = open(p).read()
s = s.replace('''    } else {
        mod utimes;
        pub use self::utimes::*;
    }''','''    } else if #[cfg(target_os = "horizon")] {
        mod unsupported;
        pub use self::unsupported::*;
    } else {
        mod utimes;
        pub use self::utimes::*;
    }''')
open(p, 'w').write(s)
PY
  note "filetime 打上: $D"
done

# --- 4. rand: no fork handler on horizon --------------------------------------
# rand's ReseedingRng registers a pthread_atfork handler on every `unix` target
# and panics if the call returns non-zero. Our target spec sets
# target-family = ["unix"], but Horizon has no fork at all and devkitPro's
# libsysbase pthread_atfork returns ENOSYS (88) -- so `thread_rng()` panicked
# the moment anything touched it (first hit: the title page, via the scripted
# RandomService and via dlv-list's VecList::new).
#
# rand already ships a no-op `fork` module for platforms without fork; this
# just widens its cfg to select it here. Overriding pthread_atfork instead is
# not possible: libsysbase defines it in the same object as pthread_create and
# the whole pthread API, so a second definition is a duplicate symbol.
#
# Upstreaming candidate alongside the libc AT_* constants.
for L in "$HOME"/.cargo/registry/src/*/rand-0.8.*/; do
  F="$L/src/rngs/adapter/reseeding.rs"
  [ -f "$F" ] || continue
  if grep -q 'target_os = "horizon"' "$F"; then
    note "rand  已打: $L"
  else
    python3 - "$F" <<'PY_RAND'
import sys
p = sys.argv[1]; s = open(p).read()
before = s
s = s.replace(
    '#[cfg(all(unix, not(target_os = "emscripten")))]\nmod fork {',
    '#[cfg(all(unix, not(target_os = "emscripten"), not(target_os = "horizon")))]\nmod fork {')
s = s.replace(
    '#[cfg(not(all(unix, not(target_os = "emscripten"))))]\nmod fork {',
    '#[cfg(not(all(unix, not(target_os = "emscripten"), not(target_os = "horizon"))))]\nmod fork {')
if s == before:
    sys.exit("rand reseeding.rs: cfg 行未按预期匹配")
open(p, 'w').write(s)
PY_RAND
    note "rand  打上: $L"
    # cargo treats registry sources as immutable, so editing one does not
    # invalidate its fingerprint -- the stale rlib is relinked and the patch
    # never reaches the binary. Purge rand's build directory so it recompiles.
    rm -rf "$(dirname "$0")/../target/aarch64-nintendo-switch/release/build/rand"
  fi
  CS="$L/.cargo-checksum.json"
  [ -f "$CS" ] && python3 - "$L" <<'PY_CS'
import hashlib, json, sys, os
v = sys.argv[1]; cs = os.path.join(v, '.cargo-checksum.json')
try:
    d = json.load(open(cs)); rel = 'src/rngs/adapter/reseeding.rs'
    if d.get('files'):
        d['files'][rel] = hashlib.sha256(
            open(os.path.join(v, rel), 'rb').read()).hexdigest()
        json.dump(d, open(cs, 'w'))
except Exception as e:
    print('  checksum skip:', e)
PY_CS
done

# --- 5. sanity: the pieces the build script expects --------------------------
fail=0
for f in "$(dirname "$0")/switch-rust.specs" \
         "$(dirname "$0")/aarch64-nintendo-switch.json.in" \
         "$(dirname "$0")/portlibs-local/install/opt/devkitpro/portlibs/switch/lib/libopenal.a" \
         /opt/devkitpro/devkitA64/bin/aarch64-none-elf-gcc \
         /opt/devkitpro/libnx/lib/libnx.a; do
  if [ -e "$f" ]; then note "OK    $f"; else note "缺失!  $f"; fail=1; fi
done
if ! rustup component list --installed --toolchain nightly 2>/dev/null | grep -q rust-src; then
  note "缺失!  nightly rust-src (装法: RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static rustup component add rust-src --toolchain nightly)"
  fail=1
fi
[ "$fail" = 0 ] && note "== 全部就绪 ==" || note "== 有缺失项, 见上 =="
