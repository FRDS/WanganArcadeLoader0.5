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
`write_memory`, `load_library`, `library_symbol`, `log`, `exit`, `init`,
`network_available`, `load_plugins`, `OPENAL_LIBRARY`), re-exported by
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

`input_log = true` in `config.toml` logs every device and every input with
the binding name to use — this is how a user identifies their wheel's axis
numbers.

### The rest

- `adm.rs` — replaces the game's ADM display layer with GLFW + an OpenGL FBO;
  handles fullscreen (borderless, scaled with black bars), the frame limiter,
  and marshals texture/sprite calls onto the game's main thread.
- `res.rs` — custom resolution: intercepts `lua_getglobal` for `SCREEN_XSIZE`
  and friends, and rescales the minimap viewport and perspective FOV.
- `card.rs` — emulates the magnetic card printer against `card.bin`.
- `al.rs` — redirects 69 OpenAL entry points to the system OpenAL.
- `shader.rs` — recompiles `data/shader/*.cg` to portable ARB at startup when
  it finds NVIDIA-only programs, using the Cg compiler inside
  `cg.dll`/`libCg.so`. Runs before the engine reads them. See the gotcha below.
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
- The Windows build has no `plugins/` support, and `ignore_custom_ioctls` is
  still Linux-only. `file_redirect` now works on both (`windows/files.rs`
  hooks LINE's `open`/`open64`/`fopen` stubs). Note LINE already rewrites
  `/tmp/...` to `./tmp/...` itself, so don't add that rewrite again — and
  `fopen64` currently refuses to hook, so a little file activity is invisible.
- **The dump's shaders are NVIDIA-only.** `data/shader/*.fp`/`*.vp` ship
  compiled for `vp40`/`fp40` with `OPTION NV_vertex_program3` and `BB1:`
  labels. Every other driver rejects all of them — `GL_INVALID_OPERATION in
  ProgramStringARB`, "syntax error near 'BB1'" — and because draws with no
  valid program bound fail too, **the 3D world renders black while the HUD
  looks perfect and the game reports nothing.** `shader.rs` fixes this
  automatically now; the symptom is worth recognising because nothing in the
  game's own output points at it.
- **The game's own diagnostics do not exist.** `alchemy.ini`'s
  `printCompiledShaders` and `defaultReportLevel` are marked "Debug only" and
  are compiled out of the `Static/Release` build it ships as, so turning them
  on achieves nothing. The loader has to supply the instrumentation, and it is
  opt-in per subsystem: `cg_log` (shader profile chosen, Cg listings on
  failure), `file_log` (every file opened; failures always logged),
  `gl_debug` (an OpenGL debug context relaying the driver's own messages —
  this is what found the shader bug). Anything logged per-frame or per-draw
  **must be deduplicated**: an early `gl_debug` build without it produced a
  70 MB log in a single run.
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
