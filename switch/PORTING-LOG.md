# Rust + std on Switch homebrew — feasibility probe

Verdict: **std works.** `std::fs` compiles, links against devkitA64's newlib + libnx,
and packs into a valid `.nro`. The blocker I expected (no std for Switch) is real
upstream but far shallower than the dead 2018-2019 attempts suggest.

Probed 2026-08-25 on macOS/arm64, devkitA64 GCC 15.2.0, libnx 4.12.0,
rustc 1.100.0-nightly (fb6531d55).

## Results

| Stage | What it proves | Result |
|---|---|---|
| `00-c-baseline` | devkitA64 + libnx newlib stdio/fs works at all | 192 KB `.nro` |
| `01-nostd` | rustc emits valid aarch64 for the official target | 12 KB `.nro` |
| `02-std` | **`std::fs` builds + links for Switch** | **393 KB `.nro`** |

Stage 2 artifact verified: `NRO0` magic present, `_start` defined at 0x0 (same as
the C baseline), 571 defined symbols, and the probe's own strings
("probe.txt", "hello from std::fs") plus std's panic runtime embedded.

## What it took

Two things, both small:

1. **6 constants in `libc`.** `std/src/sys/fs/unix.rs` needs `AT_FDCWD`; libc's
   newlib `horizon` module (written for the 3DS) omits the whole `AT_*` family
   while the sibling `vita` module has it. See `libc-horizon-aarch64.patch`.
   Values taken from devkitA64's own `sys/_default_fcntl.h`, not copied from Vita.

2. **Link order.** devkitPro links `-lnx` *before* `-lc` so libnx's overrides
   (notably `fcntl`) win; rustc injects its own `-lc` ahead of any late args, so
   plain ordering can't be expressed in a target spec. Worked around by pulling
   libnx in with `--whole-archive` up front, which defines its symbols before ld
   ever considers newlib's conflicting archive members.

## Reproduce

    export PATH="$HOME/.cargo/bin:$PATH:/opt/devkitpro/devkitA64/bin"
    export DEVKITPRO=/opt/devkitpro
    cd 02-std
    cargo +nightly build --release \
      -Z json-target-spec -Z build-std=std,panic_abort \
      --target ./aarch64-nintendo-switch.json
    elf2nro target/aarch64-nintendo-switch/release/probe02.elf probe02.nro

`aarch64-nintendo-switch.json` = rustc's built-in `aarch64-nintendo-switch-freestanding`
spec (dumped via `--print target-spec-json`) with the 3DS target's std-enabling
options grafted on: `env: newlib`, `target-family: [unix]`, devkitA64 gcc as the
linker driver, libnx's `switch.specs`, and the built-in linker script dropped
(switch.specs supplies its own).

## Not yet verified

- **Never run on hardware.** Builds and links only; `.nro` untested on console.
- `std::thread` / `std::net` untouched — the probe only exercises `std::fs`.
- newlib has **no `*at()` family** (`openat`/`fstatat`/`unlinkat` absent from
  `libc.a`), so std paths reaching for those will fail at link, not compile.
  `AT_FDCWD` compiles because `fchmodat` resolves elsewhere; other call sites may not.
- `--whole-archive` inflates the binary and is a workaround, not the upstream fix.
  Proper fix is teaching rustc to emit devkitPro's library order.
- The libc patch is applied to the **vendored** copy under
  `~/.rustup/toolchains/*/lib/rustlib/src/rust/library/vendor/`, which
  `-Z build-std` uses — patching `~/.cargo/registry` has no effect. It will be
  lost on toolchain update; upstreaming to rust-lang/libc is the real path.

---

# Stage 3 — real OpenPAL3 code (2026-08-25)

Hello-world proves the toolchain; this proves the *codebase*. Both of these now
build for Switch, as aarch64 ELF64 rlibs:

| crate | lines | artifact |
|---|---|---|
| `yaobow/fileformats` | 11,311 | `libfileformats.rlib` 1.0 MB |
| `yaobow/packfs` | 4,826 | `libpackfs.rlib` 1.2 MB |

plus `common`, `ypk`, `mini-fs` and the whole dependency tree: `binrw`, `serde`,
`serde_json`, `dashmap`, `encoding` (incl. the GBK/Big5 index crates), `aes`,
`sha2`, `lzma-rust2`, `zstd`, `zip`, `miniz_oxide`, `half`, `num-traits` — and
`minilzo-rs`, which compiles C through `cc-rs`, so native C interop works too.

Build with `./build-openpal3-switch.sh -p packfs`.

## What actually needed fixing

Four things, all small, and none of them in std:

1. **`getrandom`** has no backend for this target. Its documented escape hatch is
   `--cfg getrandom_backend="custom"`; the `__getrandom_v03_custom` symbol still
   has to be supplied at final link, and libnx's `randomGet` is the natural source.
   *Not yet written* — only the compile-time half is done.
2. **`filetime`** (via `mini-fs`'s `tar` feature) wants `futimes`/`lutimes`, which
   devkitA64's newlib lacks. Dropped the `tar` feature; `zip` is kept because
   `packfs` needs `ZipFs`. A real port either forks filetime (the author already
   forks it for Vita) or moves it to `utimensat`.
3. **`memmap`** — no mmap under newlib. Extended the existing
   `cfg(not(target_os = "vita"))` exclusion to cover horizon, and routed
   `create_reader` down the plain-`File` path Vita already uses.
4. **Two `cfg` branches had no arm matching Switch**, so a binding simply didn't
   exist (`cpk_fs.rs`'s `entry`). Added `switch` to the existing alias set.

Changes to the OpenPAL3 clone are captured in `openpal3-switch-cfg.patch`
(54 lines, 5 files) and left applied in the working tree.

## Sizing the rest

The whole repo has only **14** `target_os = "vita"` sites across **9** files, each
crate carrying a `features.rs` that declares platform aliases via `cfg_aliases`.
Adding Switch is that same edit repeated — the Vita port already cut this trail.

## Still not done

- **Nothing has run on hardware.** Everything here is compile + link only.
- `radiance` (the engine, 26,340 lines) was not attempted. Its Cargo.toml gates
  desktop deps on `cfg(not(target_os = "vita"))`, so Switch currently inherits
  winit/ash/Vulkan and will fail until added to those exclusions.
- No rendering backend exists for Switch; `vitagl` (779 lines) is the template.
- The `libc` `AT_*` patch must be reapplied after any toolchain update, and it
  must go in **every** libc copy — `-Z build-std` picks the one the workspace
  lockfile unifies on, and changing the crates.io mirror creates new copies under
  a different registry source id. Stale build-std artifacts also survive the
  patch; `rm -rf target/<target-name>` when the constants seem to be missing.

---

# Stage 4 — the engine (`radiance`)

Added a `switch` alias and extended the Vita dependency exclusions to horizon,
plus `CXX_aarch64_nintendo_switch` for the C++ deps (`imgui-sys`). Result:

**Every platform-independent part of the engine compiled.** All 10 remaining
errors are the platform-abstraction slots that have no Switch arm — nothing else.

Built clean for Switch (~21,700 lines of radiance, plus deps):

| module | lines |
|---|---|
| `rendering` (backend-independent part) | 12,238 |
| `components` | 3,364 |
| `scene` | 2,020 |
| `imgui` | 1,204 |
| `math` / `audio` / `asset` / `utils` / `video` | 2,850 |

...and the dependency tree with it: `alto` (OpenAL), `symphonia`, `lewton`,
`ogg`, `hound`, `image`, `imgui` + `imgui-sys` (C++), `crosscom`, `ypk`,
`radiance-assets`, `ttf-parser`, `toml`, `backtrace`.

## The complete remaining inventory

Each slot, with the size of the Vita implementation Switch would mirror:

| slot | Vita impl | notes |
|---|---|---|
| `rendering::RenderingEngine` | **779** (`vitagl/`, 7 files) | the real work; needs GL bindings first |
| `application::Platform` | **270** (`application/vita/`) | app lifecycle → libnx `appletMainLoop` |
| `input::gamepad::GamepadInput` | **71** | → libnx `padUpdate` |
| `imgui::platform::ImguiPlatform` | **25** | |
| `input::mouse::MouseInput` | **7** (`nop.rs`) | reusable as-is |
| `rendering::platform::Window` | **2** (`dummy.rs`) | reusable as-is |
| `input::keyboard::KeyboardInput` | **1** (`nop.rs`) | reusable as-is |
| `imgui/clipboard.rs` (`arboard`) | — | needs cfg-gating out |

Three of these (keyboard, mouse, Window) are `nop`/`dummy` stubs Vita already
uses — Switch can point at the same files. So the genuinely new code is roughly
**780 lines of render backend + ~370 lines of platform glue**, on top of GL
bindings.

## Still not verified

- **Nothing has run on hardware.** Compile + link only, throughout.
- No Switch render backend was written; the table above is an estimate from the
  Vita sources, not from an implementation.
- `yaobow/shared` (51,078 lines — the actual game logic) was never attempted.
- The `getrandom` custom backend still needs its `__getrandom_v03_custom` symbol.

---

# Stage 5 — a Switch rendering backend (`switchgl`)

`radiance` now **builds clean for Switch** — 0 errors, 0 warnings from the new
code — producing a 26 MB aarch64 `libradiance.rlib` whose objects genuinely
reference the EGL/GLES2/nwindow symbols (`eglCreateWindowSurface`,
`glDrawElements`, `nwindowGetDefault`, …). Mesa's own Switch winsys gets pulled
in with it, which is visible as extra `nwindow*` references from inside `libEGL`.

## What was written

1,483 lines, structured as a sibling of `vitagl`:

| file | lines | |
|---|---|---|
| `rendering/switchgl/gles.rs` | 239 | hand-written GLES2 + EGL + libnx FFI |
| `rendering/switchgl/shader.rs` | 232 | program build, with real compile/link error checks |
| `rendering/switchgl/render_object.rs` | 152 | VBO/IBO |
| `rendering/switchgl/factory.rs` | 105 | `ComponentFactory` |
| `rendering/switchgl/texture.rs` | 68 | |
| `rendering/switchgl/material.rs` | 59 | |
| `rendering/switchgl/mod.rs` | 18 | |
| `rendering/switchgl/switchgl_engine.rs` | 288 | EGL context + draw loop |
| `rendering/switchgl/shaders/*` | 98 | 6 GLSL ES 1.00 programs |
| `application/switch.rs` | 79 | `Platform` over `appletMainLoop` |
| `input/gamepad/switch.rs` | 114 | `GamepadInput` over libnx `padUpdate` |
| `imgui/platform/switch.rs` | 27 | |

Keyboard, mouse and `Window` reuse the `nop`/`dummy` files Vita already uses, as
predicted. Plumbing changes (cfg aliases, typed-handle arms, link flags) are in
`openpal3-switch-cfg.patch` — 295 lines across 14 files.

## Choices worth knowing about

- **The Vita shaders could not be reused.** They are Cg (`float4x4`, `mul`,
  `tex2D`), because vitaGL feeds SceGxm's compiler. Mesa wants GLSL ES, so the
  six programs were rewritten. Only `simple_triangle`, `lightmap_texture` and
  `gradient_y` are real; the rest fall back to the unlit program exactly as the
  Vita backend does.
- **`GLsizeiptr` is pointer-width.** The Vita backend passes `i32` to
  `glBufferData` and gets away with it on 32-bit ARM. On aarch64 that would
  silently truncate, so the bindings use `isize`.
- **Sampler uniforms are set with the program bound.** vitaGL defers uniform
  writes; real GLES2 does not, so `glUseProgram` has to precede `glUniform1i`.
- **A/B and X/Y are mapped by physical position**, not letter — Nintendo's
  layout is mirrored relative to the engine's south/east/west/north.
- **Attribute 2 is explicitly disabled** when a buffer carries no second UV set,
  rather than left dangling from the previous draw.

## Not done — and this is the important part

- **Still nothing has run on hardware.** Every claim above is compile-and-link.
  The EGL setup, the draw loop, the button mapping and the stick orientation are
  all *unverified against a real console*. First boot is where they get tested.
- **imgui is not rendered.** `render()` takes `ImguiFrame` and drops it. There is
  no GLES2 imgui renderer here, so no debug UI and no in-game menus.
- **No offscreen render targets** — `render_scene_to_target` is `unimplemented!()`,
  same as the Vita backend.
- No blending state is set up; only the alpha-cutout path the shaders implement.
- `yaobow/shared` (51,078 lines of game logic) and the `yaobow` binary have still
  never been compiled for Switch, so there is no `.nro` for the game itself —
  only the engine library.
- The `getrandom` custom backend still lacks its `__getrandom_v03_custom` symbol,
  which will surface at final link of the binary, not of these rlibs.

---

# Stage 6 — it links: `yaobow.nro`

**A Switch homebrew binary now builds end-to-end.**

    out/yaobow.nro    8.9 MB    NRO0 magic, from a 12.9 MB aarch64 ELF64 PIE
                                `_start` at 0x0, **0 undefined symbols**

Verified to actually contain the Switch work, not just to have linked:
`projectionMatrix` (our GLSL ES uniform), `eglCreateWindowSurface failed: 0x`
(our switchgl error path), `sdmc:/switch/yaobow` (our paths), and `MESA_DEBUG`
from the linked-in Mesa.

Everything compiled: `shared` (51,078 lines of game logic), the `yaobow` binary,
`radiance_scripting`, `p7`, `crosscom-protosept`, `lua50-32-sys`, `libffi`,
`agent_server`, `radiance_editor`.

## What stage 6 needed

| problem | fix |
|---|---|
| `getrandom` 0.2 **and** 0.3 both in the graph, neither with a backend | `switch_random.rs` (34 lines) answers both hooks from libnx `randomGet`; 0.2 also needs its `custom` feature turned on |
| `libffi-sys` ran `configure --host=aarch64-nintendo-switch`; config.sub rejects it | one arm in libffi-sys's existing remap table → `aarch64-none-elf` (the ABI really is bare-metal aarch64) |
| `filetime` wants `futimes`/`lutimes`; newlib has **no** utime family at all | added a `horizon` arm to the author's filetime fork returning `Unsupported` |
| ffmpeg is not cross-compiled for Switch | `video/mod.rs` registers no decoders there — **movies will not play** |
| `ydirs` had no Switch arm | `sdmc:/switch/yaobow` |
| `crate-type = ["lib", "cdylib"]` — a cdylib cannot satisfy libnx's `--require-defined=main` | dropped `cdylib`; **this breaks the Android build** and needs a real fix |
| newlib lacks `getrandom`, `sysconf`, `popen`, `pclose` (wanted by std's thread + hashmap seed, and Lua's `io.popen`) | `switch_libc.rs` (59 lines); `_SC_` values read from devkitA64's `sys/unistd.h`, not guessed |

Cumulative source changes: `openpal3-switch-cfg.patch`, 487 lines across 22 files,
plus 1,576 lines of new Switch-specific code.

Three patches live **outside** the repo and will vanish on any toolchain or
registry refresh — they are the fragile part of this setup:
`libc` (`AT_*` constants), `libffi-sys` (host remap), `filetime` (horizon arm).

## The honest status

**Nothing here has ever run on a Switch.** Not one line. Everything above is
"the compiler and linker accepted it", which is a real milestone and is *not*
the same as working. Specifically unverified:

- EGL init, the draw loop, and whether anything renders at all
- button mapping and stick orientation
- whether `run_title_selection()` is even the right entry point for a homebrew
  launch, or whether the boot path needs game data present first
- memory footprint against the ~3.2 GB homebrew budget

Known-missing, by construction:

- **no movies** (ffmpeg not built), **no imgui/debug UI or menus** (no GLES2
  imgui renderer), **no offscreen render targets**, **no audio verification**
  (`alto`/OpenAL compiled but was never exercised)
- only 3 of 9 shader programs are real; the rest fall back to unlit textured
- the Android build is currently broken by the `cdylib` removal

Next step is a console and an SD card, not more compiling.

---

# Stage 8 — imgui renderer, and the bug that invalidated stage 6 (2026-08-26)

Started as "write the GLES2 imgui renderer"; verification then exposed that the
previous binaries could never have worked, and fixing that dragged the real
audio/input dependencies into the light.

## The invalidating find

`Application::run()` had arms for linux/macos/android and windows only. On the
switch cfg **both compiled out**, so `run()` fell straight through to
`shutdown()`: the engine was never constructed, and every switchgl/game path was
dead code the linker GC'd. The stage-6/7 `.nro`s would have exited silently even
without the TLS crash. My stage-6 "verified to contain the Switch work" claim
rested on string presence — worthless under `--gc-sections`, where mergeable
rodata survives in any pulled archive member. Verification is now symbol-level.

Fixed with a `#[cfg(switch)]` arm mirroring Windows' inline bootstrap.

## What making the engine *live* then surfaced

- **alto's 35 `al*`/`alc*` symbols became real undefined references** (audio was
  "0 AL symbols" before only because the engine was dead). pkg.devkitpro.org is
  Cloudflare-blocked from here and no CN mirror carries it, so openal-soft
  1.21.1 was **cross-compiled from source** with devkitPro's own recipe (SDL2
  backend; one upstream `mValue` typo patched for GCC 15). Local install under
  `portlibs-local/`, linked via target-spec `late-link-args` together with SDL2
  and its chain.
- **The gamepad FFI was wrong**: `padInitializeAny`/`padGetButtons`/
  `padGetStickPos` are NX_INLINE/NX_CONSTEXPR header helpers with no symbol —
  stage-6 linked only because the module was dead. Rewritten against the real
  exports (`padConfigureInput`/`padInitializeWithMask`/`padUpdate`) with
  `PadState` mirrored field-for-field from pad.h.

## The renderer itself

`switchgl/imgui_renderer.rs` (~290 lines): same frame contract as the Vulkan
renderer (`igRender` after `frame_begun`, marker's later `igEndFrame` is a
no-op), font atlas built via `ImFontAtlas_GetTexDataAsRGBA32` with the GL
texture name as `TexID` (the factory's existing convention), scissor-clipped
draw-list walk, u16 indices, `vtx_offset` unused (backend flag never set).
Engine overrides `update_imgui_font_atlas` for the game-font atlas rebuild path.

## Verified (symbol-level, this time)

56 switchgl symbols, 3 imgui_renderer, SwitchGamepadInput, 167 al* +
`SDL_OpenAudioDevice` defined, 0 undefined, `_start` present, 0 bare
`tpidr_el0` reads, all shader literals present. ELF 46.5 MB → `yaobow.nro`
**42.5 MB** (openal+SDL2 account for the growth).

## Still unverified / known limits

- **Hardware: still nothing has run.** The engine now *exists* at boot — that is
  all that changed on paper.
- Audio path is compiled and linked, but SDL2's audio on Switch inside a
  non-SDL app (we init EGL ourselves, never SDL_Init) is untested; openal-soft's
  SDL2 backend calls `SDL_InitSubSystem(AUDIO)` itself, which should work
  standalone — should.
- imgui renderer correctness (scissor flip, color attribute normalization) is
  desk-checked only.
- Vita upstream appears to have the same missing-run-arm problem — not our
  concern, but noted.

---

# Stage 9 — real shaders, metadata, regressions closed (2026-08-26)

## Android regression properly fixed
`crate-type = ["lib", "cdylib"]` is restored. The Switch link failure it caused
came from libnx's `switch.specs` demanding `--require-defined=main`, which a
cdylib cannot satisfy; the build now uses a local `switch-rust.specs` copy with
that guard removed (the bin defines `main` regardless, so nothing is lost).

## NRO metadata
`elf2nro --nacp --icon`: hbmenu now shows "OpenPAL3 (yaobow)" with an original
bow-and-arrow icon (no SoftStar art). The build script emits the finished nro.

## All 9 shader programs now real
New GLSL ES pairs `pal3_lit_common` (per-vertex 2-nearest-light Gouraud +
tint/alpha-ref/fog, faithful to the Vulkan pal3_actor/pal3_geom port of the
original vs_1_1 `skin/geom_*.gbf` model) and `grass` (time-driven wind sway,
texcoord2 tip/coverage weights). Engine now snapshots `scene.lighting()` per
frame (ambient + first 4 point lights + fog), uploads env/material uniforms
per draw (locations are -1 no-ops on programs without them), feeds normals as
attribute 3, and un-gates texcoord2 from texture count.

Honest approximations, in the code comments too: the shader sees only the
first 4 lights (Vulkan uploads 16, both pick 2 per vertex); TerrainSplat
renders its base layer without the splat blend; TexturedDynamicLit borrows the
Gouraud model instead of actor_lit's per-pixel one; grass strength/speed ride
the uv_xform slot exactly as the Vulkan port does.

## Still unverified
No hardware run yet. GLSL ES compiles only on-device (no local validator on
this Mac); syntax was kept to conservative ES 100 (no array constructors,
constant loop bounds, vertex-stage fog depth). If a shader fails to compile
on the console, `SwitchGLShader::new` logs the driver's error to
`sdmc:/switch/yaobow/yaobow.log`.

---

# Stage 10 — the fragile patches are now self-healing (2026-08-26)

`setup-switch-toolchain.sh` re-applies all three out-of-repo patches
idempotently and verifies the toolchain pieces:

| patch | lives in | why it evaporates |
|---|---|---|
| libc `AT_*` constants | every registry + rust-src vendored copy (checksums refreshed) | rustup update, cargo re-extract, mirror switch |
| libffi-sys host remap | registry copy (+ purges its stale build-script cache) | cargo re-extract |
| filetime `horizon` arm | the fork's git checkout (writes `unsupported.rs` too) | `cargo update` moving the rev |

Tested destructively: reverted a libc copy and the filetime checkout to
pristine, ran the script, verified both restored; a full rebuild then produced
a byte-identical `yaobow.nro` (`b9938199…`). `build-openpal3-switch.sh` now
runs it first on every build, so cache refreshes fix themselves.

Remaining caveat: a *new* rustup toolchain (different nightly date) or a new
libc minor version changes paths — the globs cover that — but a future libc
that restructures `src/unix/newlib` would need the patch text revisited. The
real fix is still upstreaming the constants to rust-lang/libc.

---

# Stage 11 — nav input, git history, FBO targets (2026-08-26)

## The gap that would have blanked the hardware test
The Switch `ImguiPlatform::new_frame()` was empty: **no input reached dear
imgui at all**. The title menu would have rendered and been un-operable (no
mouse/keyboard exists; imgui only navigates by gamepad when the backend
submits gamepad events). Vita never hit this because its `main()` boots
straight into a game, skipping the menu. Now: pad state → edge-triggered
`add_key_event`s (Nintendo A=activate, B=cancel, matching platform
convention) + analog left stick, `NAV_ENABLE_GAMEPAD | HAS_GAMEPAD` set.
`ImGuiIO_AddKeyEvent` verified linked.

## Work is now version-controlled
Branch `switch-port` on the OpenPAL3 clone, 7 thematic commits (packfs /
platform slots / switchgl backend / shared shims / entry point / target
spec / nav+FBO), working tree clean. Not pushed anywhere — gh credentials
on this machine are invalid, and pushing is the user's call.

## Offscreen render targets implemented
`radiance_scripting` exposes `ScriptedRenderTarget` to scripts and the title
page is script-driven, so the previous `unimplemented!()` was a live panic
risk. Now a plain GLES2 FBO: color texture (= imgui TexID by the backend's
convention) + DEPTH_COMPONENT16 renderbuffer, `as_switchgl_mut` trait arm
mirroring Vulkan's, scene pass shared with the swapchain path. Caveat: a
target with a different aspect renders with the main camera's projection.

Current nro: `60aac84f…`, 42.6 MB.

## Remaining (from the standing list)
ffmpeg cross-compile (movies), upstreaming prep/PR, ci-switch.yml,
touch-screen → pointer, and everything hardware-gated.

---

# Stage 12 — movies, touch, CI (2026-08-27, from the commit log)

Stage 11's "remaining" list was worked through before this log caught up;
recorded here from the commits so the log stays complete:

- `4e68a27` Bink movie playback via a bink-only ffmpeg cross-build
  (`build-native-deps.sh`, `--enable-decoder=binkvideo,binkaudio_*`).
- `e567f85` Touch screen → imgui pointer (Ryujinx forwards host mouse clicks
  as touch, so this doubles as an emulator pointer).
- `6c39f71` Build tooling moved into `switch/`, `ci-switch.yml` added.

---

# Stage 13 — first runs (2026-08-27/28)

Reproduced the build on a second, clean machine, then ran it. The theory the
project had been carrying — "gamepad input is not reaching the title prompt"
— was wrong, and the first log said so in two lines.

## The clean-machine build: six things that only break away from the author

| what | fix |
|---|---|
| `switch/portlibs-local` was a committed absolute symlink into the author's home; `mkdir -p` fails on the dangling link | removed; `.gitignore` pattern had a trailing slash that skips symlinks |
| `switch/out/` is gitignored and never created; `elf2nro` fails *after* the full build with a bare "Failed to open output file!" | `mkdir -p` |
| ffmpeg source download died 144 KB short; `fetch()` then treated the short file as cached on every re-run | integrity check by container type before trusting and after downloading; `-C -` + `--retry-all-errors` |
| `setup-switch-toolchain.sh` runs before cargo has downloaded the crates it patches, so a fresh machine builds unpatched libc and dies on `AT_FDCWD`. **Every CI run on the branch failed this way.** A plain `cargo fetch` was not enough: build-std's libc (0.2.189) is a different version from the workspace's (0.2.186) and comes from rust-src's own `library/Cargo.lock`, so it was still downloaded mid-build, pristine | `cargo fetch` for the workspace **and** for `$(rustc --print sysroot)/lib/rustlib/src/rust/library` before patching |
| stale build-std artifacts outlive a re-patch | `rm -rf target/aarch64-nintendo-switch` (documented in README) |
| cargo treats registry sources as immutable: an edited crate keeps its fingerprint and the **stale rlib is relinked** — the first rand rebuild still had `bl <pthread_atfork>` in it | patch steps purge `target/.../build/<crate>` |
| ffmpeg-sys's bindgen fails on the first newlib header with `'stddef.h' file not found` when libclang cannot locate its resource directory (the CI container's cannot; macOS's can) | `-isystem <devkitA64 GCC builtin include dir>` in `BINDGEN_EXTRA_CLANG_ARGS` |

`pkg.devkitpro.org` returns a Cloudflare 403 from Shanghai Telecom, Azure
Tokyo and AWS San Jose alike — it blocks cloud egress ranges, not a country.
No CN mirror carries devkitPro (checked TUNA's full list and eight mirrors'
`/devkitpro/`). A tarball of `/opt/devkitpro` from a machine that can reach
it was the way through.

## Run 1: the crash behind the "input" theory

    pad buttons: 0x0000000000000001     <- A pressed. Input works.
    pad buttons: 0x0000000000000000
    Radiance panicked at rand-0.8.6/src/rngs/adapter/reseeding.rs:319:17:
    libc::pthread_atfork failed with code 88

A was reaching HID → our FFI → the edge detector → imgui. The prompt then
advanced, the title script called `RandomService`, rand's `ReseedingRng`
registered a fork handler because the target spec says `target-family =
["unix"]`, devkitPro's libsysbase returned ENOSYS (88), and rand
`panic!`ed. The frozen frame looked like "no reaction to A". The diagnostic
overlay was blank for the same reason — the render loop was dead.

Fix: widen rand's `cfg` so its existing no-op `fork` module is selected on
horizon. Overriding `pthread_atfork` is not an option — libsysbase defines
it in the same object as `pthread_create` and the whole pthread API.
Verified by disassembly, not strings: 0 `bl <pthread_atfork>`, no
`register_fork_handler` symbol. (Also reached from `dlv-list`'s
`VecList::new`, so avoiding it in our own code would not have sufficed.)

Same run, one line earlier:

    failed to read sdmc:/switch/yaobow/yaobow.toml: out of memory

## Run 2: the file-size that was 1.7 GB

With the config read routed through `Take` (opts out of the
metadata-len reservation) and a diagnostic added:

    yaobow.toml: metadata len 1787820992 but read 52 bytes

libc's `newlib` types are sized for the 3DS (32-bit ARM). devkitA64's newlib
uses 16-bit `dev_t`/`ino_t` and 64-bit `blkcnt_t`/`blksize_t`. Measured
with `_Static_assert` against devkitA64's own `sys/stat.h`:

| field | real | libc assumed |
|---|---|---|
| st_mode | 4 | 8 |
| st_size | 16 | 32 (inside `st_atim`) |
| sizeof | 104 | — |

`st_mode` matters more than `st_size`: `is_dir()` is the first line of
`mount_packages_recursive`. Fixed by correcting the four type aliases
(gated on aarch64), and the offsets are now `const`-asserted in
`switch_libc.rs` — broken one on purpose to confirm the build stops (E0080).

Run 2 also showed a full menu-navigation trace (A, D-pad ×10, B) with no
crash, and the Ryujinx side clean apart from 19,038 copies of one benign
`GetThreadName` warning.

## Run 3 (not yet done)

The current binary boots straight into PAL3 (`resolve_asset_path` gained a
switch arm reading the same `yaobow.toml`; default
`sdmc:/switch/yaobow/PAL3`) and logs the package count at info level. The
host probe (`cargo run -p packfs --example pal3_probe`) mounts this exact
data set fine — 45 CPKs, `init.sce`/`Q01.scn`/`PI01.mp3` read back with the
right magic — so a mount failure on the console would be platform, not data.

## Verified (in Ryujinx, symbol- or log-level)

Boot; imgui title page with CJK atlas; gamepad A/B/D-pad and touch reach
imgui; title prompt advances; config file loads with the correct size;
no panic through menu navigation; audio renderer registers; `.nro` has 0
undefined symbols, 63 switchgl / 167 OpenAL / 15 bink symbols.

## Not verified

- The direct-boot build has not been run.
- Package mounting on the console; any `.cpk` read on the console.
- Any 3D scene, any game-font atlas rebuild, any in-game UI.
- Audio *output* (init only), movie playback, save/load.
- Real hardware. Everything above is Ryujinx.

## CI

Green as of `75d0e90` — the first passing run on the branch. Three clean-
machine failures in a row got it there: the patch pass ran before cargo had
unpacked anything; then before it had unpacked *std's* libc; then bindgen
could not find `stddef.h`. The artifact is a valid `NRO0` built on
x86_64 Linux from the same commit that builds here on macOS.

---

# Stage 14 — PAL3 reaches its first playable scene (2026-08-28)

Booting straight into PAL3 (`auto_new_game`) with movies gated off, the
game reached its opening scene on the Switch target for the first time.
From `sdmc:/switch/yaobow/yaobow.log`:

    mounted 45 package(s) from "sdmc:/switch/yaobow/PAL3"
    sce: LoadScene { name: "q01", sub_name: "yn09a" }
    PAL3: loading scene q01/yn09a (/scene/q01/yn09a.scn)
    PAL3 .lgt loaded from /scene/q01/yn09/1/yn09.lgt: 7 lights, ambient [0.1,0.1,0.1]
    loading shader TexturedLightmap / Pal3Prop / Pal3Geom / TexturedNoLight / Pal3Actor
    switchgl: first scene frame with 76 objects (21 visible entities, frame 1)
    sce: SceCommandDlg text: "景天：\n什么声音？……有贼？！"
    switchgl: frame 3600, 76 objects, 21 visible entities   (steady, no crash)

So on the console target, first time for all of it: CPK mounting, .scn
scene load, the .lgt lighting, the five GLES2 shader programs compiling
and linking on-device, the switchgl draw loop, the SCE script VM running
the opening script, and the dialog system rendering the first line.

Verified this run: everything above, plus input (A / D-pad / B reach imgui
and the game), config loading, 45 CPKs mounted, steady 30 fps-ish frame
cadence held for thousands of frames with no panic.

## The one thing that does not work: movies

The intro `pal3op.bik` crashes the run. The bink demuxer and decoders are
present and correct (verified with ffprobe on the exact bytes the VFS
serves), the container opens, both streams are identified, all decoder
objects build -- then, as the three ffmpeg player threads come up, the
**main thread** faults:

    Unhandled exception: Ryujinx.Memory.InvalidMemoryRegionException
      at ARMeilleure.Instructions.NativeInterface.ReadUInt64

The player threads are created, given priority 0x3B and a 3-core affinity
mask, and started (confirmed in Ryujinx's svc trace); the fault is a guest
read of an unmapped address on the main thread around the same moment.
This is an unsolved std::thread + ffmpeg interaction on horizon, not a
codec or data problem. Gated off via `enable_movies = false` for now
(decoder simply not registered → script skips the movie); `init()` is also
bounded by a 10 s first-frame timeout so a movie can never wedge the game
even with decoders on. Cutscenes were already on the known-missing list.

## Still not verified

- **Actual pixels.** Everything here is log- and svc-trace-level. The run
  was headless on a locked desktop; no frame has been looked at, so
  rendering *correctness* (geometry, textures, lighting, the CJK dialog
  font) is unconfirmed even though the draw loop runs.
- Audio output (sounds are triggered in the log; not heard).
- Movement, battles, save/load, scene transitions beyond the opener.
- Real hardware. Still Ryujinx throughout.

---

# Stage 15 — invisible text: three bugs in a trench coat (2026-08-31)

Dialog text was invisible while the dialog frame, game sprites and the 3D
scene all drew. Every plausible single cause was eliminated in turn — and
the answer was three independent bugs stacked:

1. **The native window was 1280x720 while everything assumed 1920x1080.**
   libnx's default NWindow picks its own buffer size. The imgui renderer
   flips scissor rects against the assumed 1080-high framebuffer, so a
   window at y=9..87 was scissored to y=993 — outside the real 720-line
   buffer. Every window-clipped draw (all text) was culled; full-screen
   clipped draws (sprites, images) survived. It also made every frame
   render 1.5x too large with the right/bottom cropped, visible in every
   screenshot as a consistent stretch. `nwindowSetDimensions(1920,1080)`
   before surface creation fixed text *positions*; glyphs then rendered.

2. **The font atlas was 4096x16384 — 256 MB, exactly at
   GL_MAX_TEXTURE_SIZE.** Full-CJK ranges at two sizes with imgui's
   default 3x horizontal oversampling. Sampling it returned black. On
   Switch the CJK faces now use common-simplified ranges, no
   oversampling: 2048x4096, verified legible by rendering the atlas
   itself on screen at 1:1.

3. **PAL3 ships no font file** (the original used the Windows system
   font), and the dialog script guards its text pass with
   `if game_font_size_small() <= 0 { return }` — no game font, no text,
   silently. The size queries now fall back to the bundled faces, and on
   Switch the bundled CJK face registers through the real game-font path
   so the dialog keeps its tuned metrics.

False leads worth remembering: the "garbled atlas" was a correct glyph
sheet downscaled 8x (checked by drawing it at 1:1 — crisp glyphs); the
"init-time uploads are lost" theory died when a texture-size ladder
(512^2..2048x4096 checkerboards) rendered pixel-perfect at every size;
a chunked glTexSubImage2D upload path really is garbled on this stack
and was reverted to single-shot.

## The rendering reference

The desktop build (same tree, Vulkan) now runs on the same machine
against the same install: brew MoltenVK + vulkan-loader + ffmpeg@7 +
shaderc, `VK_ICD_FILENAMES` set. It renders the opening correctly —
characters, lamp-lit room — which is what turned "the Switch picture is
wrong" from an opinion into a diff. Found and fixed along the way:
registering the bundled font on desktop pushed its atlas to 4096x32768
(base faces already fill 16384 there), so the fallback is Switch-only.

Also landed, mirroring the Vulkan pipeline: per-material blend
(premultiplied ONE/1-SRC_ALPHA), DepthMode, CullMode, a depth-mask
restore before clear, and the 16-light table (was capped at 4 — the
opening room's .lgt has 7, and the lamp was among the dropped).

## Verified / still open

Verified on screen (self-captured via `screencapture -l <window>` — the
agent can now see both windows without user relay): dialog text with
correct size and padding, dialog frame, avatar art, cutscene script
progression, 60 FPS.

Open: the Switch scene image still does not match the desktop reference
(perspective looks over-wide/warped, characters not identified on
screen, overall darker). Actor meshes are confirmed *in* the draw list
(per-shader draw telemetry: Pal3Actor x1 from frame 1, x2 the moment the
script spawns the second). A/B captures at matched cutscene moments are
the next step; note the two runs' dialog sequences genuinely diverge
(input-less runs take different branches), so matching is by stable
end-state, not by line.

---

# Stage 16 — the scene was transposed (2026-08-31)

With camera commands proven byte-identical in both runs' sce logs and
the desktop build rendering them correctly, the fault had to be in the
switchgl vertex path. It was the matrices: radiance's Mat44 stores
column-vector matrices row-major; GLES2 forbids transpose=TRUE; the raw
upload therefore fed the shaders transposes, and `P*V*M*v` computed
(M*V*P)^T v. Stage 5's note arguing this away assumed a row-vector
convention the engine does not use.

The first fix -- transpose on the CPU -- changed nothing on screen,
which turned out to be the second bug: **Mat44::transposed swapped
every off-diagonal pair twice and was a no-op**. Dormant upstream, as
nothing on the Vulkan path calls it.

With both fixed, the Switch build renders the opening correctly:
both characters, the room, the lamp glow, the proper cutscene camera,
60 FPS -- verified by self-captured screenshots against the desktop
reference. Remaining visible deltas: somewhat darker than the desktop
image (the documented lighting approximations), dialogs self-advance
without input on Switch (desktop waits for a key -- an input-edge bug
to chase), and movies stay disabled.

---

# Stage 17 — combat is reachable from scripts (2026-09-02)

The rules layer (`openpal3/combat/{state,actions,formulas,ai,settlement}`)
had been transcribed from the binary but nothing called it. Now the
SCE opcodes 73 `Combat`, 80 `CombatBoss`, 82 `CombatMaxRound`, 83
`CombatMustFail`, 131 `CombatNotGameOver` are real commands and 132
`GetCombat` returns the real result. `SceCommandCombat` builds the
encounter the way `0x437280`/`0x4373d0` do (monster draw
`FUN_018c59e0`, floor type from nav tile byte +5, `combatScn.txt`
lookup, battle BGM from `music.txt [combat]`), drives a
`CombatSession` per frame, and settles like `FUN_004055d0` case 3
(HP/SP/MP write-back, experience, skill use counts, money, loot;
`GetCombat = (result == 2)`, i.e. win, round-out or a successful
escape).

The fight itself is a placeholder overlay: imgui text, HP bars, a
four-entry menu (攻击/仙术/防御/逃跑) on keyboard and gamepad. No
battle scene, no animation. Losing without `CombatNotGameOver` shows
"游戏结束" and exits the process — the original returns to the title
via `SwitchMode(0xf)`, and the engine has no path back to the title
yet. `fast_forward` auto-plays a whole battle so script tests need no
input.

While wiring this up, several notes in `re/notes/combat_flow.md` were
corrected against the binary: `+0x48` is MustFail and `+0x50` is
禁止逃跑 (not the other way round); `result` 2 = win, 3 = lose; the
player's 逃跑 entry *does* roll (`FUN_00500d50`, formula in
`FUN_004f98b0` case 3) — only the AI path is unconditional.

`cargo test -p shared`: 266 lib tests pass. Not yet run on hardware;
the overlay needs a visual check.

---

# Stage 18 — every shipped SCE opcode now decodes (2026-09-03)

The vm panics on an opcode it has no arm for, so the question worth
answering first was which opcodes the shipped scripts actually use.
Reading the dispatch table out of the exe (0xd8c464, 16 bytes an entry:
a dword, the handler, then up to eight parameter type bytes) gives every
opcode's signature; walking all 2810 procs in a real install with it
found **fifteen opcodes that occur in scripts and had no arm at all** —
each a crash waiting on a particular scene — and one arm whose arity was
wrong (251 read five parameters where the table has one, desyncing the
rest of the proc). That walk is now `pal3_sce_opcode_census`, and it
fails if a script uses anything the vm cannot decode.

Twenty-eight stubs became real commands: the script-variable and
story-flag family (14, 41, 130, 145, 149, 109/110/111/138, 42, 88, 205,
206, 150), the scene-object state store behind 126/158/159, notes and
full-screen captions (68, 86), and the VFX position pair (142, 144). The
subsystems that genuinely are not here — the shop and smelting menus,
the pawnshop game and its five queries, swat-a-fly, the cave, the guide
bee — consume their parameters, say so through the note widget, and where
a script would branch on the answer, return a defined one. 107 skips the
ski game but still runs its end-of-game script, which is what the
original does on the way out of ski mode.

Reverse-engineering notes went to `re/notes/sce_opcodes.md`, including
four things worth knowing before touching the camera: parameters are
stored last-first; opcode 33 is orbit and 35 is rotate, and yaobow has
the rotate implementation attached to 33; opcode 36's (x, y, z) is the
**look-at point** and its third scalar the **distance**, not a camera
position and an unknown; and 117/128 behave opposite to their names —
128 rotates the world into the given angles and leaves it there, 117
snaps to them and eases back to identity.

Not yet started: the camera family (30/31/32/35/38/39/155/250/251), which
needs the original's polar camera state (yaw, pitch, roll, distance,
look-at, plus a state stack) since radiance's Transform has no notion of
a distance or a look-at point; the party-follow commands; and anything
needing CVD animation, per-role alpha, world rotation or a VFX system.

---

# Stage 19 — where things stand (2026-09-13)

This log stopped at stage 18; the commit history carries the detail since.
In short, between 2026-09-03 and 2026-09-13:

- **Scripts.** Every opcode the shipped scripts use now has a real command
  except 154 (the guide bee, a pure hint, skipped). The camera family
  (30–39, 155, 250, 251) runs on a polar camera state with a push/pop stack.
  `re/notes/sce_opcodes.md` is back in sync with `vm.rs`.
- **Combat.** Stage actors play the original action tables, walk out and
  back, and settle on the efflinker timings; the HUD is the original CBUI
  layout; the camera reads `EditorCam*` and plays flexcam sequences; all 15
  flexeff classes draw; the result screen, defeat screen and combo skills
  (合击技) follow the original timelines.
- **Menus and mini-games.** Shop, smelting, pawnshop, inn, big map, dialog
  choice, notes and captions follow the original pixel layouts; the status
  menu's lineup page has the 3D 五灵盘 with click-to-swap; the 侠义榜 shows
  its book and monsters in 3D.
- **Rendering fixes.** CVD playback follows the original per-type rules
  (including type-17 idle replay, bobbing and spinning); texture row order
  matches gbengine for CVD, single-texture POL and lightmapped POL — the
  last also fixed `.tga` diffuse maps that used to fail to load at all.

Still missing for a full playthrough, in the order they are being done:
the combat ring's 特技 / 阵 / AI / 援 / 魔 buttons, the level-up
"learned" messages, dodge / near-death / special-skill actions, the
pawnshop's 陈列 / 休息, and the remaining combat camera moves. Visual gaps
that do not block play: the ski slope and swat-a-fly art, the inn's
original layout, the screen trail effect. And the Switch build still needs
someone to look at the off-screen render targets on real hardware.

# Stage 20 — playing the main story forward (2026-10-13)

`tools/pal3_drive/explore.py` now plays the game forward on its own: it
walks the player (through the engine, one tile per frame, vines, layer
portals and maze jumps included) to whichever trigger, NPC or exit it has
not tried in the current plot state, pushes dialogs and choices, fights
the battles, and saves when the plot variable (-32768) moves. With
`--level 15` it went from 10200 (chapter 1, 永安当) to 30500 (chapter 3,
back in 渝州). What it turned up, each fixed against the original:

- walk-on triggers judged by layer / active / times (`scene.cpp:702`);
- no `ObjLockInput` — re-interacting while a triggered script ran
  restarted it (the kitchen-basement ladder);
- script run mode was one global flag instead of per script instance
  (`scasmdecoder.cpp:256`) — a dialog menu loop froze the game;
- `Sprite` never returned its imgui texture — the Vulkan descriptor pool
  ran dry on long sessions;
- story battles drawn under the pre-battle `FadeOut` (the original never
  draws screen effects in `GAME_COMBAT`);
- layer changes now use the nav file's portal pairs (`Role::UpdateLayer`);
- talking picks the target like `GetSelRole_RoleCtrl`, party members with
  a script-assigned talk proc included (the boat on the way to 蓬莱);
- the combat scheduler no longer makes a unit wait for someone else's
  action to settle before choosing its own (`ProcControl`), which had let
  a full party starve the enemies of turns.

The large gap it exposed: most interactive scene objects (chests, item
pickups, save points, push boxes, character-skill gates, switches, vines,
lifts…) have no examine behaviour yet — the original's
`SceneObj::InterActRole` / `Update` (~2500 lines) is ported only for four
types. The main story has not been blocked by it so far.

It did block at 31400 (chapter 3, 九顶山 M08). Story proc 1002 sits in
the part of M08/1b that is only reached through M08/4, and M08/4 only
through a vine in M08/2 that the block's entry script (1501) shows when
the switch in M08/3 (object 13) is on. Three things were wrong:

- upstream treated node type 23 (switch) as an "item trigger": examine ran
  its proc (500, which lowers the vine in M08/3) but never flipped the
  switch flag that opcode 126 `GetSwitch` reads, so the M08/2 vine never
  appeared;
- vines and ladders were usable while their object was inactive (the
  original selects only active objects), so the hidden M08/2 vine could
  in fact be climbed — the agent just never tried it;
- nothing picked an object for the examine key.

Ported now, from the exe (`FUN_00424c20` / `FUN_00426eb0` cases 0xc and
0x17, `FUN_00420cd0`, `FUN_0042c810`, action 0x13 in `FUN_0042ebe0`):
object selection for the examine key, the switch (walk over, flip, play
the 调查 action and the switch model, run its proc, link), the
character-skill gate (only for the hero named in parameter 0; plays 技能,
then the gate is cleared and goes inactive), remaining-uses counting with
the count kept in the save, and object-side `ObjLockInput`. One quirk is
kept as the exe has it: selection compares the *last* in-view object's
distance, not the nearest one's, so examining with a farther object
straight behind the target finds nothing. Not done: the 神树 flower
master switch, the world-rotating switch (state 11), a switch driving a
moving platform.

A fourth problem turned up on the way: loading a save never applied the
saved object state to the scene it was loaded into (only script scene
changes did), so a lowered bridge, a sunk turtle, a thrown switch or an
object a script had hidden all came back as built from the `.scn` after
a load. Loading now restores them before the entry script runs, in the
same order as a script scene change.

From the pre-switch save the explorer then found the whole route by
itself — switch in M08/3, vine in M08/2, M08/4, the far side of M08/1b,
1002 and its boss (at level 36; at 26 the party loses and the boss is a
game-over one) — and left 九顶山 for 德阳: plot 40300, chapter 4.

古藤林 (M10/1, 40900) is a teleport maze of 食人花 (node type 32): step
into a flower's tile rect and it swallows you and spits you out next to
its partner. Ported from `FUN_00424c20` / `FUN_00426eb0` case 0x20 and
`Role::RotateFly` (`FUN_00414e50`, 0.5 s cosine ease): fly into the
flower, the flower plays its model while the player is hidden, then the
player is placed on the tile in parameters 0/1 and flies out from the
partner flower (parameter 2). The fly's tilt, scale and crouch pose are
not drawn yet. Plot 41100.

From 41100 to 71800 (蜀山, 雷州, into 神魔之井) the explorer stopped at
these, each ported from the exe as it came up:

- 蜀山故道 M11/1 pressure plates (node type 33, case 0x21: walk to the
  centre, sink 5 over 2 s, run the object's proc, call the linked object,
  one use) — upstream had treated them as item triggers;
- M11/2 rams (node type 13, case 0xd, `scene/ram.rs`): the swinging
  `<name>A.pol` part, its box against the player's, knock-back flight
  and landing on the tile in parameters 0/1 of layer 2, 5 % HP off the
  whole party with the 1.2 s hurt delay. The gauntlet is a timing test;
  the agent walks at a steady pace and was teleported to the exit;
- Tab / pad L cycles the controlled hero in mazes (needed for the
  per-hero skill gates), and the save now keeps the controlled hero, the
  layer and the real position;
- `OpenDoor` finishes the next frame (the original `IsWaiting` is
  `state == 0`, exe 0x42bee0); waiting for the door animation could leave
  input locked for good when a parallel script changed scene meanwhile;
- opcode 124 on a type 23 object posts object event 2 (a gate with
  parameter 0 = 1 follows its switch);
- each of the two scene slots keeps its own last-touched trigger, as
  `Scene::m_lastobj` does, so returning to a STORYB scene does not refire
  the trigger the player is standing on;
- `RoleEndAction` / `RoleShowAction` do not wait on an inactive role
  (`Role::IsWaiting` is false when `!m_active`); 「遇到苍萤」 (plot 70500)
  turns role 11 off and then waits on it.

神魔之井 M15/2 has two-level elevators (node type 39, case 0x27): no
event check; the elevator is placed on the centre tile of its trigger
rect, the player walks there, `wc014` plays, the elevator carries the
player for 8 s to the floor height of the layer in parameter 0, the
player changes layer (the elevator does not) and walks 80 forward in z;
input is held by `ObjLockInput`. The agent's flood treats an elevator as
a layer link. The route: floor switch #8 opens gate #7, elevator up,
plate #1 to M15/3, switch (proc 502) there, back in M15/2 the boulder
(object 15) is gone, elevator down, west exit to M15/c.

From 71800 to 100400 (神树, 锁妖塔, 蜀山 after 雪见's death):

- 神树 M16 flowers (node type 0x25) and the master switch: the flowers
  follow one global open/closed flag kept in the save; opcode 159
  (`InitFlower`) sets it once; the master switch's own `.scn` node type
  decides which objects follow it. The closed model (`…2.pol`) is not
  swapped in yet;
- opcode 158's skip list is cleared on every scene load (FUN_00538d60),
  and objects on it drop their persistent records, so a gate rebuilt
  from the `.scn` is not left open by an old save entry;
- a switch flag with no save record reads the `.scn` initial value
  (锁妖塔 floor 7 plates start on);
- 锁妖塔 M17 moving plates (node type 5): a polyline path from the `.scn`
  (+0x84 count, +0x88 points), speed 100, the player steps off 96 past
  the end; a plate can only be boarded at the end it is parked at; its
  position is saved on arrival and restored on load;
- 锁妖塔 lifts (node type 0x16): event 2 toggles up/down at 30/s with
  `Wg005`, the raised lift is a hole on layer 0 and a bridge on layer 1,
  the depend scene/id at +0x14a/+0x16a selects the switch that powers it;
- the space key asks for a role before an object (`rolectrl.cpp:405`):
  a bed's box was taking the key from 龙葵 standing beside it;
- combat: damage / heal / MISS popups over the hit role
  (FUN_004f2c70 always posts one while the stage role is alive; colours
  from the DisplayChain table), the selected role's HP text over the
  target marker (`???/???` for bosses), and a party attack whose
  targets are all down is dropped instead of walking to an empty spot;
- combat: the outcome is judged every frame even while the control panel
  is open (FUN_004f7910 from FUN_004ecb00; ProcControl force-releases
  the panel). The last enemy fleeing during target selection had left
  the battle on a menu with nothing to pick (M11/2).

- 灵山仙人洞 M19/1 rotating beams (node types 7 and 8): only event 2
  from a linked switch, turn +90 degrees at 30 per second
  (FUN_00424c20 / FUN_00426eb0 case 7), tiles cleared while turning and
  registered at the new angle, a 2 s hold; the angle is saved and the
  tiles re-registered on load (`SceneObj::OnLoad` case 7).

- 草海 M18 cave falling obstacles (node type 0x24): walking into the
  trigger drops the block (gravity 490, to the tile floor − 4), clears
  the trigger rect and blocks the tiles — the caves become one-way;
- the 御剑 page's 御剑飞行 button (`yjfx`, 0xa00571), shown once opcode
  138 sets `DAT_00de625a`: refused in mazes (AssisLang 373) and in Q14
  (108), otherwise the main UI goes to page 8, which opens the world
  map (exe 0x4ae65e). From chapter 11 this is the only way out of 蜀山
  to 酆都; without it the story stopped at 120100.

- 黄泉路 M21 push boxes (node type 0xb) and lift gates (0x1e): examine
  key only; four push points around the box, the nearest one within 600;
  one push moves 50 at WalkSpeed; CanGoTo checks the box's four corner
  tiles and OBB overlap against blocking objects and other boxes (a
  standard separating-axis test stands in for the unreversed
  gbOBBox::IsColOBB); the three-state box (parameter 0 = 2) only moves
  along z and opens the gate in parameter 1 or 2. Upstream treated node
  type 11 as an item trigger and ran its proc on examine; that is gone;
- 冰风谷 M22 slides (0x23): stepping on carries the player along the
  node's polyline at 600 per second with a cosine ease, then runs the
  proc and waits for it; opcode 124 on a lift (type 22) now posts the
  object's event 2 instead of moving it directly, so the snowballs land
  and become walkable bridges.

Story 100102 → 140300 (chapters 10–14). The agent gained `combat_win`
(debug) for the 20-round 邪剑仙 fight in M22/5 that the attack-only
explorer cannot survive.

Not done yet: the closed-flower and plate model swaps, the type 21
eyeball's look, the lift and beam camera locks, the spell-side MISS case
in FUN_0050a700.
