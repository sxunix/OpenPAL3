#!/bin/sh
# Build OpenPAL3 crates for Switch homebrew. See README.md for what this proved.
set -e

# Self-heal: cargo/rustup cache refreshes silently drop the three out-of-repo
# patches this build needs; re-applying is idempotent and near-free.
"$(dirname "$0")/setup-switch-toolchain.sh" >/dev/null || {
  echo "setup-switch-toolchain.sh failed -- run it directly to see why" >&2
  exit 1
}
export PATH="$HOME/.cargo/bin:$PATH:/opt/devkitpro/devkitA64/bin"
export DEVKITPRO=/opt/devkitpro

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
# sysroot or it parses host headers (or nothing).
export BINDGEN_EXTRA_CLANG_ARGS="--target=aarch64-none-elf -D__SWITCH__ -I$FFMPEG_DIR/include -I/opt/devkitpro/devkitA64/aarch64-none-elf/include -I/opt/devkitpro/libnx/include"

# getrandom ships no backend for this target; the custom one needs a
# __getrandom_v03_custom symbol at final link (libnx randomGet would supply it).
export RUSTFLAGS='--cfg getrandom_backend="custom"'

# static.rust-lang.org measures ~18 KB/s here; USTC ~2.1 MB/s.
export RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static
export RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup

# Materialize the target spec from its template — the JSON needs absolute
# paths (rustc does not expand variables in specs), which are machine-local.
SWITCHDIR="$(cd "$(dirname "$0")" && pwd)"
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
  /opt/devkitpro/tools/bin/elf2nro "$E" "$OUT/yaobow.nro" \
    --nacp="$SWITCHDIR/yaobow.nacp" --icon="$SWITCHDIR/yaobow-icon.jpg" >/dev/null
  echo "nro: $OUT/yaobow.nro"
fi
