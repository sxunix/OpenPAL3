#!/bin/sh
# Cross-build the native C/C++ dependencies the Switch port links against:
#   * openal-soft 1.21.1  (SDL2 backend; devkitPro's own recipe + one GCC15 fix)
#   * ffmpeg 7.1          (devkitPro horizon patch, trimmed to Bink playback)
#
# Idempotent: each build is skipped when its installed library already exists.
# pkg.devkitpro.org is Cloudflare-blocked from some networks, which is why
# these are built from source instead of `dkp-pacman -S`.
set -eu

PROBE="$(cd "$(dirname "$0")" && pwd)"
LOCAL="$PROBE/portlibs-local"
DKP="${DEVKITPRO:-/opt/devkitpro}"
export PATH="$DKP/portlibs/switch/bin:$DKP/devkitA64/bin:$DKP/tools/bin:$PATH"
export DEVKITPRO="$DKP"
JOBS="${JOBS:-8}"

fetch() { # url out
  # A previously-truncated file must not count as cached: codeload serves the
  # source tarballs chunked (no Content-Length), so a dropped connection
  # leaves a short file behind that curl cannot detect as incomplete and the
  # old `[ -f ] && return` then treated as done -- the build failed on every
  # re-run with an unfixable `tar: unexpected end of file`.
  if [ -f "$2" ]; then
    verify_download "$2" && return 0
    echo "  ${2##*/}: 缓存损坏,重新下载"
    rm -f "$2"
  fi
  # -C - resumes a partial file; --retry-all-errors also retries the
  # connection resets that produced the truncation in the first place.
  curl -L --fail --retry 8 --retry-all-errors --retry-delay 3 \
       --connect-timeout 30 -C - -o "$2" "$1" 2>/dev/null || {
    rm -f "$2"; echo "下载失败: $1" >&2; return 1
  }
  verify_download "$2" || { rm -f "$2"; echo "下载校验失败: $1" >&2; return 1; }
}

# Integrity check by container type; anything else is accepted as-is.
verify_download() {
  case "$1" in
    *.tar.gz|*.tgz) gzip -t "$1" 2>/dev/null ;;
    *.tar.xz)       xz -t "$1" 2>/dev/null ;;
    *)              [ -s "$1" ] ;;
  esac
}

# --- openal-soft --------------------------------------------------------------
OPENAL_LIB="$LOCAL/install/opt/devkitpro/portlibs/switch/lib/libopenal.a"
if [ -f "$OPENAL_LIB" ]; then
  echo "openal: 已就绪 ($OPENAL_LIB)"
else
  echo "openal: 构建中..."
  mkdir -p "$LOCAL" && cd "$LOCAL"
  fetch "https://raw.githubusercontent.com/devkitPro/pacman-packages/master/switch/openal-soft/avoid-readlink.patch" avoid-readlink.patch
  fetch "https://codeload.github.com/kcat/openal-soft/tar.gz/refs/tags/1.21.1" openal-soft.tar.gz
  rm -rf openal-soft-1.21.1 && tar -xzf openal-soft.tar.gz
  cd openal-soft-1.21.1
  patch -Np1 -i ../avoid-readlink.patch
  sed -i.bak 's#SDL2/##' alc/backends/sdl2.cpp
  # GCC 15 rejects a pre-existing typo lazily accepted by older compilers.
  sed -i.bak 's|const T& operator\*() const& { return this->mValue; }|const T\& operator*() const\& { return mStore.mValue; }|' common/aloptional.h
  mkdir -p build && cd build
  PORTLIBS_PREFIX="$DKP/portlibs/switch" aarch64-none-elf-cmake -G"Unix Makefiles" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DALSOFT_UTILS=OFF -DLIBTYPE=STATIC -DALSOFT_EXAMPLES=OFF \
    -DALSOFT_REQUIRE_SDL2=ON -DALSOFT_BACKEND_SDL2=ON \
    -DSDL2_INCLUDE_DIR="$DKP/portlibs/switch/include" \
    -DCMAKE_INSTALL_PREFIX="$DKP/portlibs/switch" \
    ../
  make -j"$JOBS"
  make install DESTDIR="$LOCAL/install"
  echo "openal: 完成"
fi

# --- ffmpeg (bink-only) -------------------------------------------------------
FFMPEG_LIB="$LOCAL/ffmpeg-install/lib/libavcodec.a"
if [ -f "$FFMPEG_LIB" ]; then
  echo "ffmpeg: 已就绪 ($FFMPEG_LIB)"
else
  echo "ffmpeg: 构建中..."
  mkdir -p "$LOCAL/ffmpeg" && cd "$LOCAL/ffmpeg"
  fetch "https://raw.githubusercontent.com/devkitPro/pacman-packages/master/switch/ffmpeg/PKGBUILD" PKGBUILD
  fetch "https://raw.githubusercontent.com/devkitPro/pacman-packages/master/switch/ffmpeg/ffmpeg-7.1.patch" ffmpeg-7.1.patch
  fetch "https://codeload.github.com/FFmpeg/FFmpeg/tar.gz/refs/tags/n7.1" n71.tar.gz
  rm -rf ffmpeg-7.1 FFmpeg-n7.1 && tar -xzf n71.tar.gz && mv FFmpeg-n7.1 ffmpeg-7.1
  cd ffmpeg-7.1
  patch -Np1 -i ../ffmpeg-7.1.patch
  ./configure --prefix="$LOCAL/ffmpeg-install" --enable-gpl --disable-shared --enable-static \
    --cross-prefix=aarch64-none-elf- --enable-cross-compile \
    --arch=aarch64 --cpu=cortex-a57 --target-os=horizon --enable-pic \
    --extra-cflags="-D__SWITCH__ -D_GNU_SOURCE -O2 -march=armv8-a -mtune=cortex-a57 -mtp=soft -fPIC -ftls-model=local-exec -I$DKP/libnx/include" \
    --extra-ldflags="-fPIE -L$DKP/libnx/lib" \
    --disable-runtime-cpudetect --disable-programs --disable-debug --disable-doc --disable-autodetect \
    --enable-asm --enable-neon \
    --disable-avdevice --disable-postproc --disable-avfilter --disable-network \
    --disable-encoders --disable-muxers --disable-parsers --disable-bsfs \
    --disable-protocols --enable-protocol=file \
    --disable-demuxers --enable-demuxer=bink \
    --disable-decoders --enable-decoder='binkvideo,binkaudio_dct,binkaudio_rdft' \
    --enable-swscale --enable-swresample \
    --disable-zlib --disable-bzlib --disable-iconv
  make -j"$JOBS"
  make install
  echo "ffmpeg: 完成"
fi

echo "== 原生依赖全部就绪 =="
