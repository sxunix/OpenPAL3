#!/bin/sh
# Build OpenPAL3 crates for Switch homebrew. See README.md for what this proved.
set -e

export PATH="$HOME/.cargo/bin:$PATH:/opt/devkitpro/devkitA64/bin"
export DEVKITPRO=/opt/devkitpro

# The out-of-repo patches are applied to crate sources in cargo's registry,
# so those sources have to exist before the patch pass runs. On a fresh
# machine (or every CI run) cargo only downloads them during the build --
# after setup-switch-toolchain.sh has already looked and found nothing to
# patch -- and the first build then fails on `AT_FDCWD`. Fetch first.
SWITCHDIR="$(cd "$(dirname "$0")" && pwd)"
(cd "$SWITCHDIR/.." && cargo +nightly fetch) || {
  echo "cargo fetch failed" >&2
  exit 1
}
# That only covers the workspace lockfile. -Z build-std resolves std's own
# dependencies from rust-src's library/Cargo.lock, and its libc is a
# different version from the workspace's (0.2.189 vs 0.2.186 at the time of
# writing) -- so it is a second registry copy, downloaded mid-build, after
# the patch pass. Fetch std's graph too so that copy exists to be patched.
STD_SRC="$(rustc +nightly --print sysroot)/lib/rustlib/src/rust/library"
cargo +nightly fetch --manifest-path "$STD_SRC/Cargo.toml" || {
  echo "cargo fetch of the std workspace failed (is rust-src installed?)" >&2
  exit 1
}

# Self-heal: cargo/rustup cache refreshes silently drop the out-of-repo
# patches this build needs; re-applying is idempotent and near-free.
# Output is kept: which copies got patched is the first thing to look at
# when a build fails on AT_FDCWD, and CI logs are the only view into that.
"$SWITCHDIR/setup-switch-toolchain.sh" || {
  echo "setup-switch-toolchain.sh failed, see above" >&2
  exit 1
}

# cc-rs keys native-C cross settings off the triple with dashes -> underscores.
export CC_aarch64_nintendo_switch=/opt/devkitpro/devkitA64/bin/aarch64-none-elf-gcc
export AR_aarch64_nintendo_switch=/opt/devkitpro/devkitA64/bin/aarch64-none-elf-ar
export CXX_aarch64_nintendo_switch=/opt/devkitpro/devkitA64/bin/aarch64-none-elf-g++
SWITCH_CFLAGS="-march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE -D__SWITCH__ -I/opt/devkitpro/libnx/include"
export CFLAGS_aarch64_nintendo_switch="$SWITCH_CFLAGS"
export CXXFLAGS_aarch64_nintendo_switch="$SWITCH_CFLAGS"

# Prebuilt bink-only ffmpeg for the horizon target (see portlibs-local/ffmpeg).
export FFMPEG_DIR="$(cd "$(dirname "$0")" && pwd)/portlibs-local/ffmpeg-install"
# ffmpeg-sys runs bindgen over the ffmpeg headers; point clang at the cross
# sysroot or it parses host headers (or nothing). stddef.h/stdarg.h are
# compiler-provided, not newlib's: a libclang that cannot locate its own
# resource directory (the CI container's) fails on the first newlib header
# with "'stddef.h' file not found", so hand it devkitA64 GCC's builtin
# include directory explicitly -- the same headers the real cross-compile uses.
GCC_INC="$(ls -d "$DEVKITPRO"/devkitA64/lib/gcc/aarch64-none-elf/*/include | head -1)"
export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-none-elf -D__SWITCH__ -I$FFMPEG_DIR/include -I/opt/devkitpro/devkitA64/aarch64-none-elf/include -I/opt/devkitpro/libnx/include -isystem $GCC_INC"

# getrandom ships no backend for this target; the custom one needs a
# __getrandom_v03_custom symbol at final link (libnx randomGet would supply it).
export RUSTFLAGS='--cfg getrandom_backend="custom"'

# static.rust-lang.org measures ~18 KB/s here; USTC ~2.1 MB/s.
export RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
export RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup

# Materialize the target spec from its template — the JSON needs absolute
# paths (rustc does not expand variables in specs), which are machine-local.
sed -e "s|@PROBE@|$SWITCHDIR|g" -e "s|@DEVKITPRO@|${DEVKITPRO:-/opt/devkitpro}|g" \
  "$SWITCHDIR/aarch64-nintendo-switch.json.in" > "$SWITCHDIR/../aarch64-nintendo-switch.json"

cd "$SWITCHDIR/.."
cargo +nightly build --release \
  -Z json-target-spec -Z build-std=std,panic_abort \
  --target ./aarch64-nintendo-switch.json "$@"

# After cargo succeeds, repack the .nro with hbmenu metadata when the bin was built.
E="$(dirname "$0")/../target/aarch64-nintendo-switch/release/yaobow.elf"
OUT="$(dirname "$0")/out"
if [ -f "$E" ]; then
  # out/ is gitignored build output, so it does not exist in a fresh clone and
  # elf2nro fails with a bare "Failed to open output file!" after a full build.
  mkdir -p "$OUT"
  /opt/devkitpro/tools/bin/elf2nro "$E" "$OUT/yaobow.nro" \
    --nacp="$SWITCHDIR/yaobow.nacp" --icon="$SWITCHDIR/yaobow-icon.jpg" >/dev/null
  echo "nro: $OUT/yaobow.nro"
fi
