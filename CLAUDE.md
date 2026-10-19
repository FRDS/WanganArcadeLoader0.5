# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

WanganArcadeLoader 0.5 — a loader for the arcade game *Wangan Midnight Maximum
Tune 3DX+*. The game ships as a 32-bit Linux ELF (`main`); this crate loads
into that process and replaces the parts that need arcade hardware: the HASP
dongle, the JAMMA I/O board, the card printer, the ADM display layer, OpenAL
and OpenGL.

**One crate, two targets:**

| | Linux | Windows |
|---|---|---|
| Output | `libwal_3dxp.so` (`i586-unknown-linux-gnu`) | `wal_3dxp.dll` (`i686-pc-windows-gnu`) |
| Injection | `LD_PRELOAD`, `#[ctor]` constructor | plugin for [LINE](https://github.com/axylol/line), a Windows ELF loader |
| Symbols / hooks | `dlsym` + `retour` | LINE's function table (`DlSym`, MinHook) |
| libc interception | `#[no_mangle]` interposers | hook LINE's stubs by address |

Fork of `vixen256/WanganArcadeLoader0.5`; the Windows port lives on branch
`claude/nifty-ptolemy-bfx2bw`.

**Read [docs/ROADMAP.md](docs/ROADMAP.md) before starting work.** It has the
phase status (P0–P7), what is proven on hardware versus only in a container,
decisions already taken and not worth relitigating, and the research for the
unstarted phases — custom BGM, the Gemballa unlock byte offsets, networking.
That research exists nowhere else. This file covers only how the code is put
together.

## Build and test

Nothing builds on Windows natively. Both targets are cross-compiled from
**Ubuntu 24.04** — 24.04 specifically, because its i686 mingw uses DWARF
exceptions, which Rust's `i686-pc-windows-gnu` requires; 22.04 will not link.
WSL is fine (`wsl --install -d Ubuntu-24.04`).

```sh
sudo dpkg --add-architecture i386 && sudo apt-get update
sudo apt-get install -y build-essential gcc-multilib g++-multilib cmake \
  gcc-mingw-w64-i686 g++-mingw-w64-i686
sudo apt-get install -y --no-install-recommends \
  libx11-dev:i386 libxcb1-dev:i386 libxrandr-dev:i386 libxinerama-dev:i386 \
  libxcursor-dev:i386 libxi-dev:i386
sudo apt-get install -y --no-install-recommends wine32:i386 wine   # for tests
rustup target add i586-unknown-linux-gnu i686-pc-windows-gnu
rustup component add clippy
```

`cmake` is not optional: `sdl2` is built `bundled` + `static-link`, so the
first build compiles SDL2 from source and takes a few minutes.

```sh
# Linux target
export PKG_CONFIG_PATH=/usr/lib/i386-linux-gnu/pkgconfig/ \
       PKG_CONFIG_SYSROOT_DIR=/ PKG_CONFIG_ALLOW_CROSS=1
cargo build --release --target i586-unknown-linux-gnu
cargo test  --release --target i586-unknown-linux-gnu --lib
```

```sh
# Windows target
export CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER=i686-w64-mingw32-gcc
cargo build --release --target i686-pc-windows-gnu
cargo test  --release --target i686-pc-windows-gnu --lib \
  --config 'target.i686-pc-windows-gnu.runner="wine"'
tests/mockline/run.sh target/i686-pc-windows-gnu/release/wal_3dxp.dll
```

A single test: append the path, e.g.
`cargo test --release --target i586-unknown-linux-gnu --lib poll::tests::maps_axis_values`.

`makefile` has `make` / `make check` / `make dist` shortcuts for the Linux
target only. Clippy runs with a fixed allow-list (see `makefile:check` and
`.github/workflows/build.yml`); match it rather than fixing the allowed lints.

`cargo build` alone (host target) will not work: this is a 32-bit x86 crate.

### tests/mockline — run it after touching `src/platform/` or `call_game`

`tests/mockline/` is a stand-in for LINE: ~250 lines of C that load the real
DLL, hand it LINE's function table and play the game's side of the boot
sequence. Its fake game functions are compiled with
`-mincoming-stack-boundary=4` and use aligned SSE stores, **so they fault
exactly as the real game would if `call_game` ever stopped aligning the
stack.** It checks the dongle, `system()` emulation, OpenAL redirection, the
USB path patch, `cl_main`, custom resolution, main-thread marshalling and the
ADM mode config.

## Architecture

### The platform boundary

`src/platform/` is the only OS-specific code; the rest of the crate calls
through it. Both sides export the same surface (`get_symbol`, `hook`,
`write_memory`, `load_library`, `try_load_library`, `library_symbol`, `log`,
`exit`, `init`, `network_available`, `load_plugins`, `OPENAL_LIBRARY`,
`CG_LIBRARY`), re-exported by
`platform/mod.rs` under `#[cfg]`. Adding an OS-specific capability means
adding it to *both* `linux.rs` and `windows/mod.rs`, not `#[cfg]`-ing at the
call site.

`src/lib.rs::init()` is the single entry point, reached from the `#[ctor]`
constructor on Linux and from `OnPreExecute("main")` on Windows. It loads
`config.toml` and `keyconfig.toml`, then installs every hook in a fixed order.

`windows/` additionally holds: `line.rs` (LINE's 8-entry function table,
signature scanning, `patch_bytes`), `crash.rs` (a vectored exception handler
that logs faults with module, registers and a stack scan), `system.rs`
(emulates the `find`/`cp` shell commands the game runs, since LINE has no
shell), `log.rs` (`wal_log!`, flushed per line so the log survives a crash).

### `call_game` — the stack alignment rule

**Every call from Rust into game code must go through
`platform::call_game(f, (args,))`.** The game was built by Linux gcc, which
assumes a 16-byte aligned stack at each call; Rust on `i686-pc-windows-gnu`
only guarantees 4. `shim/call.c` realigns and marshals arguments as 32-bit
stack words. `call_game::<_, R>` picks the return path from `R` — floats and
doubles come off the x87 stack, everything else from `eax:edx`.

Calling a game function pointer directly will appear to work and then fault
in SSE code somewhere unrelated.

### Hooking

`hook::hook_symbol(name, func)` resolves a mangled symbol and detours it,
returning the trampoline to the original (store it in a `static mut
ORIGINAL_*` and reach it via `call_game`). Two prebuilt stubs are used
everywhere a hook just needs to succeed or fail: `adachi()` returns true,
`undachi()` returns false.

Two modules bypass hooking and patch machine code instead: `al.rs` and
`opengl.rs` overwrite each function's prologue with `mov eax, addr; jmp eax`,
because a direct jump is cheaper than a detour for ~1200 functions.

### OpenGL on Windows: generated cdecl wrappers

The game calls OpenGL cdecl (Linux convention); Windows `opengl32` is
stdcall. `build.rs` parses the `FUNCS` array out of `src/opengl.rs`, looks
each name up in Khronos' `gl.xml` (all `gl`-supported extensions, so vendor
functions are included) and generates a cdecl→stdcall wrapper per function
into `OUT_DIR/gl_wrappers.rs`. Adding a name to `FUNCS` is enough; a function
with no wrapper is logged and skipped rather than redirected. This runs on
Windows targets only.

### Input

`src/poll.rs` is the whole input layer. Keyboard comes from **GLFW on both
platforms** (reading the game window created in `adm.rs`); SDL2 handles pads
as game controllers *and* opens every joystick raw, for wheels and pedals
that SDL has no pad mapping for. Raw bindings are `JOY[<device>]_BUTTON<n>`,
`JOY[<device>]_AXIS<n>{+,-,_FULL,_FULL_INV}` and
`JOY[<device>]_HAT<n>_{UP,RIGHT,DOWN,LEFT}`; the device prefix is optional and
`JOY<d>` slots are stable across hot-plug. New binding kinds plug into
`binding_is_down`/`was_down`/`is_tapped`, so `jamma.rs` needs no changes.

`src/jamma.rs` turns those bindings into the JAMMA bit field and the analog
values the game reads (`n2jvio` wheel/gas/brake words plus floats in the
input struct).

Each device the loader opens is named in the log with its axis, button and
hat counts, so `JOY0` can be told from `JOY1`. The numbers within a device
come from the OS, not from here — there is no per-input tracing any more.

### The rest

- `adm.rs` — replaces the game's ADM display layer with GLFW + an OpenGL FBO;
  handles fullscreen (borderless, scaled with black bars), the frame limiter,
  and marshals texture/sprite calls onto the game's main thread.
- `res.rs` — custom resolution: intercepts `lua_getglobal` for `SCREEN_XSIZE`
  and friends, rescales the minimap viewport and perspective FOV, and widens
  the 2D layer's orthographic box so a render that is not 4:3 doesn't stretch
  it (`widen`, unit-tested as a pure function; `widescreen_hud` turns it off).
  That hook has to decide what *not* to touch —
  `igMatrix44f::makeOrthographicProjection` is a general matrix utility, so the
  same call builds world-space projections — and it does so from the shape of
  the box, the way `set_viewport` keys on `88x82` for the minimap. `res_log`
  reports every projection the engine asks for, deduplicated; it is how the
  2D layer's box was measured in the first place.
- `hud.rs` — widescreen **placement**: moves each HUD widget out to the edge
  it was drawn against, behind `widescreen_anchor` (off by default) and only
  while a race is on screen. Everything goes through
  `clSpriteManager::entry`, where `applyUser` is forced — an entry the engine
  finds clean carries stale vertices — and the four finished x floats at
  `+0x08`/`+0x14`/`+0x20`/`+0x2c` are offset by the bar. A *widget* is
  whatever brackets the quads: an anime by its `.anm` length and frame count,
  a string by its draw call, and a console's raw entries by runs of quads
  within 8 units of each other. `side` decides from the union over every
  frame, never shrinking, and is pure and unit-tested over every widget real
  races produced. **Menus are excluded deliberately** — they are layouts whose
  pieces only mean anything side by side, and per-widget decisions tear them
  apart. Also the measurement, behind `res_log`: per-widget extents, entry
  classes, and what brackets what. The per-class offsets it reads came from
  the dump's own 20-byte setters — `clSpriteAnime` keeps x at `+0x4C`,
  `clSpriteFont` at `+0x20`, and the two layouts differ.
- `card.rs` — emulates the magnetic card printer against `card.bin`, which is
  207 bytes: three 69-byte tracks and a CRC-16/CCITT-FALSE at `0xCD` over the
  rest. Most of it is the status bytes rather than the data:
  `clCardPrinter::startSequence()` zeroes s1/s2/s3 before every command and
  `0x00` is not the `0x30` that means "normal", so **every command has to
  answer with all three**, and the status has to be written before the command
  word that tells the game to read it. `respond` is a pure function over the
  command, its payload and the card, so the thirteen commands and the three
  partial-write selectors are unit-tested; `exec` only marshals. Also answers
  `clCardDeviceGameAutoService::getResult`, which otherwise latches a card
  error after enough idle attract polls. `card_log` traces every command.
- `elfsym.rs` — reads the game ELF's `.dynsym` so `crash.rs` can name the
  function an address is in. `main` is `ET_EXEC` and not
  position-independent, so a link address is a runtime address; there is no
  `.symtab` in the Export dump, and 47,937 of its 61,074 `.dynsym` entries are
  functions with an address. Bounded by `st_size`, so an address outside every
  function is reported as the module and offset it already was.
- `report.rs` — `Reported::say`, which logs a line once and then goes quiet
  after 64 distinct ones. Anything on a per-frame or per-draw path goes
  through one of these; see the dedup gotcha below.
- `al.rs` — redirects 69 OpenAL entry points to the system OpenAL.
- `shader.rs` — makes `data/shader` suit the machine before the engine reads
  it. Each compiled program declares what it needs in its own `OPTION NV_*;`
  directives, so the required extension is *derived* (`OPTION
  NV_fragment_program2;` → `GL_NV_fragment_program2`) and put to the driver
  through a throwaway invisible GLFW window — GLFW is reference counted, so
  this leaves `adm.rs` free to initialise its own later. Only if the driver
  is missing something does it keep the originals as `*.orig` and recompile
  the `.cg` sources with the compiler inside `cg.dll`/`libCg.so`. On an NVIDIA
  GPU the shipped programs are left alone, since `fp40`/`vp40` are the better
  ones; the `.orig` copies are even put back if a folder recompiled elsewhere
  later runs on one. **Every uncertain path recompiles, and to portable ARB**
  — `arbvp1`/`arbfp1` works everywhere, so an inconclusive probe must never
  risk the black world. The target pair is only ever `vp40`/`fp40` when the
  programs in the folder declare those extensions *and* the driver confirms
  it has them, which is the one case where recompiling portable would throw
  away the better programs; that pass rebuilds only what is missing, and
  falls back to portable in the same run if Cg cannot produce it. `decide()`
  takes the probe as a parameter so the whole table is unit-tested, and
  `answer_from()` refuses to answer about an extension the driver was never
  asked about; `shader_mode` in `config.toml` overrides the lot. See the
  gotcha below.
- `vendor/retour/` — the upstream `retour` fork with one change: its `win64`
  function-pointer impls are now `#[cfg(target_arch = "x86_64")]`, because
  current rustc rejects that ABI on 32-bit. Patched in via `[patch]` in
  `Cargo.toml`. **Without this nothing builds** — don't drop the vendored copy.

## Things that will bite you

- **`dist-windows/start.bat`**: must keep CRLF endings (enforced by
  `.gitattributes` and a CI lint). The game folder name contains spaces and a
  `)`, so the lint also rejects `%CD%`, any second use of `%~dp0`, and
  multi-line `( )` blocks — `)` inside a block ends it early and the script
  breaks on the user's real path.
- **Broken symlinks in game dumps**: copying a dump off Linux turns
  `libstdc++.so.6`, the `libboost_*` libraries and `libz.so.1` into 0-byte
  files. LINE can't parse them, every C++ import resolves to 0 and the game
  dies at `eip=00000000` in static constructors. `fix-libso.ps1` repairs this.
  **`librt.so.1` is deliberately left empty** — LINE supplies its functions.
- **32-bit MSYS2 only** for `line.exe` and its `msys-*.dll`s. 64-bit ones give
  `0xc000007b`; the giveaway is `msys-gcc_s-1.dll not found` (64-bit ships
  `msys-gcc_s-seh-1.dll` instead). CI now builds these four itself (the `line`
  job) and the `dist-windows-bundle` artifact contains them, so this only
  matters if you hand-assemble a folder — but it is still the most common
  setup failure, and the checks in that job exist to keep it out of releases.
- **Never strip or otherwise modify `cg.dll`/`cgGL.dll`/`libCg.so`/
  `libCgGL.so`.** We redistribute them under NVIDIA's licence, which permits
  it *only* for unmodified binaries ("except for decompression and
  compression"). Both CI sites that handle them say so; the `build` job strips
  `libopenal.so` right next door, so the exception is easy to "tidy up" by
  mistake. Doing so would void the redistribution grant.
- **The game needs the Cg runtime, not just a shader compiler.** It `dlopen`s
  `libCg.so` *and* `libCgGL.so` and calls `cgCreateProgram`/`cgGLLoadProgram`/
  `cgGLGetLatestProfile` itself (`OnDlOpen` maps those names to the DLLs).
  Our loader never touches `cgGL` at all. So no amount of shader
  pre-compilation removes the dependency — and shipping converted `.fp`/`.vp`
  would mean redistributing the game's own assets, which this project doesn't
  do.
- Formatting is **hard tabs** (`rustfmt.toml`), matching upstream.
- The Windows build has no `plugins/` support. `file_redirect` and
  `ignore_custom_ioctls` now work on both: `windows/files.rs` hooks LINE's
  `open`/`open64`/`fopen` stubs and `windows/ioctl.rs` its `ioctl` stub. Note
  LINE already rewrites `/tmp/...` to `./tmp/...` itself, so don't add that
  rewrite again — and `fopen64` currently refuses to hook, so a little file
  activity is invisible. Unlike the Linux interposer, the ioctl hook does not
  call the original first: LINE's stub can only print `Untranslated ioctl` and
  fail for the two ops we answer. The game reaches them through
  `/dev/ttyM1`/`ttyM2`, whose `open` fails under LINE, so nothing has been
  seen to call them — it is parity, not an observed fix.
- **The dump's shaders are NVIDIA-only.** `data/shader/*.fp`/`*.vp` ship
  compiled for `vp40`/`fp40` with `OPTION NV_vertex_program3` and `BB1:`
  labels. A driver without the matching `GL_NV_*` extensions rejects all of
  them — `GL_INVALID_OPERATION in ProgramStringARB`, "syntax error near
  'BB1'" — and because draws with no valid program bound fail too, **the 3D
  world renders black while the HUD looks perfect and the game reports
  nothing.** `shader.rs` handles this automatically now, and only when the
  driver actually needs it; the symptom is worth recognising because nothing
  in the game's own output points at it. Note the files carry an `!!ARBfp1.0`
  header *and* an `OPTION NV_` line, so the header tells you nothing — the
  `OPTION` directives are the discriminator.
- **Seven of the dump's 19 `.cg` sources are EUC-JP, not UTF-8.**
  `light_trail`, `light_trail_cutoff`, `palette_sprite`, `radial_blur`,
  `sprite`, `sprite_neg` and `yuv` carry Japanese comments in EUC-JP — in
  files whose own Emacs modeline claims `coding: utf-8-unix`. `read_to_string`
  fails on all seven, and an earlier version of `shader.rs` skipped them with
  a bare `continue`, so they were never recompiled and stayed NVIDIA-only.
  The visible symptom was the attract movie playing with red and blue
  swapped, because `yuv.cg` is the Bink YUV→RGB converter; the HUD and the
  world looked fine, so nothing pointed at shaders. Every non-ASCII byte in
  all seven sits inside a comment, so `shader.rs` reads bytes and converts
  lossily. **Never reach for `read_to_string` on dump assets**, and never
  `continue` past one without logging — "24 programs, 0 failed" looked
  healthy while two of nineteen sources were being dropped every launch.
- **The game's own diagnostics do not exist.** `alchemy.ini`'s
  `printCompiledShaders` and `defaultReportLevel` are marked "Debug only" and
  are compiled out of the `Static/Release` build it ships as, so turning them
  on achieves nothing. The loader has to supply the instrumentation, and it is
  opt-in per subsystem: `card_log` (every printer command and the status it
  answered with), `car_log` (every car the engine resolves, by id and folder
  name) and `res_log` (the screen size, viewports and projections). Four more
  existed and were removed once they had answered: `input_log`, `cg_log`,
  `file_log` and `gl_debug` — the last of which is what found the shader bug,
  so an OpenGL debug context is the thing to rebuild first if the world ever
  comes out wrong again. Failed opens and refused card writes are still
  reported unconditionally; it is the per-event tracing that is gone. Anything
  logged per-frame or per-draw **must be deduplicated**: an early build
  relaying OpenGL debug output without it produced a 70 MB log in a single
  run. Use `report.rs` — a `static Reported` per subsystem, which
  stops after 64 distinct messages, because deduplicating by text alone never
  repeats for a line carrying an address or a size and so never stops.
- **The test menu navigates on the gear shifter, and auto-repeats it.** There
  are no cursor switches in the JAMMA map at all — the switch table at
  `0x93cfc40` pairs raw bits with exactly ten game bits (shifter × 4, gas,
  brake, view, intrude, test, service) — so the menu reads the shifter's
  position. `clInputDevice`'s constructor stores `0xF` as the auto-repeat mask
  at `+0x1c`, which is those four shifter bits and nothing else, and
  `clInputDevice::update()` re-fires a *held* masked bit every 8 frames after
  a 30-frame delay (`+0x18`), on top of the usual edge trigger
  (`trig = now & ~old` at `+0x10`, copied to `+0x14`). A real lever springs
  back to neutral when the operator lets go; ours does not, so **anything that
  leaves a shifter bit asserted gives the menu a cursor key held down for
  ever**. `jamma.rs` forces neutral when test or service is tapped. Note that
  no other stuck bit can do this: gas, brake, view and intrude are outside the
  repeat mask, so they read as one press and stop.
- **Frame rate is game speed.** The engine advances one simulation step per
  frame drawn, so anything that changes the frame rate changes how fast the
  game plays — observed directly: holding the rate below 60 made the game run
  slow in menus as well as in a race. `fps_limit` is therefore a speed
  control, not a performance knob, and 60 is the rate the arcade hardware ran
  at rather than a preference. `vsync` can override it downward, because a
  display that cannot hold 60 paces the swap. This is the context for every
  pacing decision in `adm.rs`, and the reason `limit_fps()` must not be
  weakened: rendering faster is not a free win, it is a bug.
- **The game ends itself, and it is not a crash.** It arms
  `setitimer(ITIMER_REAL)` at 50 Hz and installs no SIGALRM handler at all
  (measured with `signal`/`sigaction` watched from before `main`). The signal
  eventually reaches a thread that does not block it and the default action
  takes the process: Windows exit code `0x0E00`, no `line.exe.stackdump`,
  nothing from `crash.rs`, nothing in the Windows event log, and -- with all
  four of msys's exit functions hooked -- no `exit()` either. `ignore_alarm`
  (default on, `windows/alarm.rs`) sets `SIG_IGN` before the game can arm it.
  **Read the exit code before calling anything a crash:** Cygwin encodes a
  normal exit as `status << 8`, a signal death as `sig << 8`, and adds `0x80`
  when it dumps core, so `0x0E00` is signal 14 and `0x8B00` is SIGSEGV with a
  core. A *second* ending is still open -- SIGSEGV inside `msys-2.0.dll` at
  `pthread_cancel`, with a stackdump; see docs/ROADMAP.md.
- **ARM is not a target.** The game is 32-bit x86 and this code rewrites its
  machine code; run the x86 build under emulation instead.

## Testing on the Windows machine

The game folder is `C:\Wangan Midnight Maximum Tune 3DX+ (Export) (2010)`.
The simplest deploy is to unzip the `dist-windows-bundle` artifact into it:
that is the `windows` job's output plus the `line` job's `line.exe` and three
msys DLLs. **Nothing has to come from elsewhere any more** — the bundle
carries `line.exe`, the msys runtime, `soft_oal.dll` and the Cg runtime. Then
run `start.bat` from that folder.

Deploying a locally built DLL means copying it plus `dist/*` (minus
`start.sh`) and `dist-windows/{start.bat,fix-libso.ps1,README.md}` — exactly
what the `windows` job's `Package` step produces — over a folder that already
has `line.exe` beside it.

Logs to collect afterwards: `wal_3dxp.log` (the loader), `line_console.log`
(LINE and the game), and `line.exe.stackdump` if it exists.

## Repository state

The branch is `claude/nifty-ptolemy-bfx2bw` on `FRDS/WanganArcadeLoader0.5`,
forked from `vixen256/WanganArcadeLoader0.5` (added as `upstream`). Note that
`gh` resolves to `upstream` unless you pass
`--repo FRDS/WanganArcadeLoader0.5`.

CI (`.github/workflows/build.yml`) has four jobs:

- `build` — Linux i586; also compiles openal-soft from source (slow) and
  unpacks the Cg runtime into `dist`.
- `windows` — cross-build, DLL export and import checks, a `start.bat` lint,
  and both test suites under Wine. It also cross-builds OpenAL Soft 1.23.1
  with mingw-w64 and ships it as `soft_oal.dll` (the name `al.rs` opens at
  run time; CMake emits `OpenAL32.dll`), cached on the pinned tag. That step
  checks the DLL imports no mingw runtime and still exports every entry point
  `al.rs` patches — `al.rs` panics at startup if one is missing, so a version
  bump must not drop any. It also extracts `cg.dll`/`cgGL.dll` from the Cg
  Toolkit 3.1 installer with **`innoextract`** — it is Inno Setup 5.3.10, and
  p7zip cannot open it — taking `app/bin/`, never `app/bin.x64/`.
- `line` — the only job on a Windows runner. Builds `line.exe` in 32-bit
  MSYS2 from a pinned `axylol/line` commit, because LINE is a Cygwin program
  and cannot be cross-compiled. Its output is cached on the pinned SHA, so
  after the first run it restores in seconds.
- `bundle` — `needs: [windows, line]`; merges their artifacts into
  `dist-windows-bundle`.

`line` and `bundle` are deliberately additive: nothing above them refers to
them, so reverting the commit that added them restores the previous pipeline
exactly. Keep it that way — don't wire `build` or `windows` into them.

Two traps if you touch the `line` job. LINE's `scripts/ci.sh` has no `set -e`
and wraps its body in `if [ -e ./build/line.exe ]`, so a failed compile exits
**0** with no `dist/` at all; and GitHub's `pwsh` shell propagates only the
last command's exit code. Almost every check in that job exists because of
one of those two, so don't thin them out. Never use `shell: bash` there
either — that is Git Bash, which carries its own Cygwin `msys-2.0.dll`, and
two Cygwin runtimes in one process give "cygheap base mismatch".
