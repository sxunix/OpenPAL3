# Nintendo Switch (homebrew) port

Builds `yaobow` as a Switch homebrew `.nro` using devkitPro's devkitA64 +
libnx and a custom std-enabled Rust target (derived from the upstream
`aarch64-nintendo-switch-freestanding` spec with the 3DS target's
newlib/unix options grafted on).

The Switch build boots straight into PAL3 (the way the Vita build boots
straight into PAL4); the scripted title selector is not used.

## Requirements

* devkitPro with `switch-dev` + `switch-sdl2` + `switch-mesa` portlibs
  (`pkg.devkitpro.org` blocks many cloud/VPN egress IPs with a Cloudflare
  403 -- if `dkp-pacman` cannot reach it, a copy of `/opt/devkitpro` from
  a machine that can is the fastest way out)
* Rust nightly with `rust-src`
* host clang (for bindgen), cmake, python3

## Build

    ./switch/build-native-deps.sh     # once: openal-soft + bink-only ffmpeg
    ./switch/build.sh --bin yaobow    # -> switch/out/yaobow.nro

`build.sh` runs `cargo fetch` and then self-heals four patches that live
in cargo's caches (libc `AT_*` constants and `struct stat` layout for
newlib/horizon on aarch64, a libffi-sys configure-triple remap, a
filetime fallback, rand's fork handler) -- see `setup-switch-toolchain.sh`
for what and why. The fetch has to come first: the patches edit crate
sources, which do not exist on a fresh machine until cargo downloads
them. The target spec is generated from `aarch64-nintendo-switch.json.in`
at build time because rustc does not expand variables inside spec files.

If a build starts failing with `AT_FDCWD` / `config.sub` / `futimes` /
`pthread_atfork` errors after a toolchain or registry refresh, run
`setup-switch-toolchain.sh` directly to see which patch did not apply,
and delete `target/aarch64-nintendo-switch` -- stale build-std artifacts
survive a re-patch.

## Install

Copy `switch/out/yaobow.nro` to `sdmc:/switch/`, create
`sdmc:/switch/yaobow/` (logs land there as `yaobow.log`), and put the
game data under `sdmc:/switch/yaobow/PAL3/`. That path is the default;
to use another, write `sdmc:/switch/yaobow/yaobow.toml`:

    [game.pal3]
    asset_path = "sdmc:/switch/yaobow/PAL3"

The engine mounts every `.cpk` under that directory recursively; at
minimum `basedata/basedata.cpk` must be there.

Before a console run, check the data set on the host with the same
mounting code the game uses:

    cargo run -p packfs --example pal3_probe -- /path/to/PAL3

## Status / known limits

Verified in the Ryujinx emulator: boot, imgui + CJK atlas, GLES2
shaders, audio init, gamepad and touch input, config loading. Not yet
verified: package mounting on the console, any 3D scene, audio output,
movie playback, real hardware. `switch/PORTING-LOG.md` has the full
record, including what each run actually showed.

TerrainSplat renders base layer only; TexturedDynamicLit uses the PAL3
Gouraud model; lit shaders see the first 4 scene lights. Upstreaming
candidates for the cache patches: rust-lang/libc (`AT_*` constants and
the aarch64 newlib type widths), tov/libffi-rs (triple remap), the
filetime fork (horizon arm), rust-random/rand (no fork on horizon).
