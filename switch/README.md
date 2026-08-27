# Nintendo Switch (homebrew) port

Builds `yaobow` as a Switch homebrew `.nro` using devkitPro's devkitA64 +
libnx and a custom std-enabled Rust target (derived from the upstream
`aarch64-nintendo-switch-freestanding` spec with the 3DS target's
newlib/unix options grafted on).

## Requirements

* devkitPro with `switch-dev` + `switch-sdl2` + `switch-mesa` portlibs
* Rust nightly with `rust-src`
* host clang (for bindgen), cmake, python3

## Build

    ./switch/build-native-deps.sh     # once: openal-soft + bink-only ffmpeg
    ./switch/build.sh --bin yaobow    # -> switch/out/yaobow.nro

`build.sh` self-heals three patches that live in cargo's caches (libc
`AT_*` constants for newlib/horizon, a libffi-sys configure-triple remap,
a filetime fallback) — see `setup-switch-toolchain.sh` for what and why.
The target spec is generated from `aarch64-nintendo-switch.json.in` at
build time because rustc does not expand variables inside spec files.

## Install

Copy `switch/out/yaobow.nro` to `sdmc:/switch/`, create
`sdmc:/switch/yaobow/` (logs land there), put game data under e.g.
`sdmc:/switch/yaobow/PAL3/` and point `sdmc:/switch/yaobow/yaobow.toml`
at it.

## Status / known limits

Verified in the Ryujinx emulator up to the title page (script engine,
imgui + CJK atlas, GLES2 shaders, audio init). Not yet verified on
hardware. TerrainSplat renders base layer only; TexturedDynamicLit uses
the PAL3 Gouraud model; lit shaders see the first 4 scene lights.
Upstreaming candidates for the cache patches: rust-lang/libc (AT_*
constants), tov/libffi-rs (triple remap), the filetime fork (horizon arm).
