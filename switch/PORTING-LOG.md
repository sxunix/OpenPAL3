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
