#!/usr/bin/env python3
"""Fix libc's newlib type aliases for aarch64 Horizon (Nintendo Switch).

libc sizes its newlib types for the 3DS (32-bit ARM). devkitA64's newlib uses
16-bit dev_t/ino_t and 64-bit blkcnt_t/blksize_t, which moves every field of
`struct stat` after st_dev. Measured against devkitA64's sys/stat.h:

    field       real   libc assumed
    st_mode        4              8
    st_size       16             32   (lands inside st_atim)
    sizeof       104              -

Applied by setup-switch-toolchain.sh; idempotent. The resulting offsets are
static-asserted at build time in yaobow/shared/src/switch_libc.rs.
"""
import hashlib
import json
import os
import sys

REL = "src/unix/newlib/mod.rs"

BLK_OLD = "pub type blkcnt_t = i32;\npub type blksize_t = i32;"
BLK_NEW = """// switch probe: aarch64 horizon stat layout -- devkitA64 newlib uses long here
cfg_if! {
    if #[cfg(all(target_os = "horizon", target_arch = "aarch64"))] {
        pub type blkcnt_t = c_long;
        pub type blksize_t = c_long;
    } else {
        pub type blkcnt_t = i32;
        pub type blksize_t = i32;
    }
}"""

DEV_OLD = """    } else {
        pub type dev_t = u32;
        pub type ino_t = u32;
        pub type off_t = i64;
    }"""
DEV_NEW = """    } else if #[cfg(all(target_os = "horizon", target_arch = "aarch64"))] {
        // devkitA64's newlib: 16-bit dev_t/ino_t, unlike the 3DS arm above.
        pub type dev_t = c_short;
        pub type ino_t = c_ushort;
        pub type off_t = i64;
    } else {
        pub type dev_t = u32;
        pub type ino_t = u32;
        pub type off_t = i64;
    }"""


def refresh_checksum(vendor_dir):
    """Registry/vendored crates are checksum-verified; re-stamp our edited file
    or cargo restores the pristine copy on next use."""
    cs = os.path.join(vendor_dir, ".cargo-checksum.json")
    try:
        d = json.load(open(cs))
        if d.get("files"):
            d["files"][REL] = hashlib.sha256(
                open(os.path.join(vendor_dir, REL), "rb").read()
            ).hexdigest()
            json.dump(d, open(cs, "w"))
    except Exception as e:
        print("  checksum skip:", e)


def patch(path):
    s = open(path).read()
    before = s
    s = s.replace(BLK_OLD, BLK_NEW, 1)
    s = s.replace(DEV_OLD, DEV_NEW, 1)
    if s == before:
        sys.exit(f"{path}: libc newlib type aliases did not match as expected")
    open(path, "w").write(s)


if __name__ == "__main__":
    if sys.argv[1] == "--checksum":
        refresh_checksum(sys.argv[2])
    else:
        patch(sys.argv[1])
