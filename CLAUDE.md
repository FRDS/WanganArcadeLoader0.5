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

**Read [docs/HANDOFF.md](docs/HANDOFF.md) before starting work.** It is the
full record of the Windows port: what is proven on hardware versus only in a
container, the phase plan (P0–P7), decisions already taken and not worth
relitigating, and the research for the unstarted phases (custom BGM, the
Gemballa unlock byte offsets, networking). This file covers only how the code
is put together.

## Build and test

Nothing builds on Windows natively. Both targets are cross-compiled from
**Ubuntu 24.04** — 24.04 specifically, because its i686 mingw uses DWARF
exceptions, which Rust's `i686-pc-windows-gnu` requires. See docs/HANDOFF.md
§3 for the full `apt` list.

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
  `msys-gcc_s-seh-1.dll` instead).
- Formatting is **hard tabs** (`rustfmt.toml`), matching upstream.
- The Windows build has no `plugins/` support, and `ignore_custom_ioctls` is
  still Linux-only. `file_redirect` now works on both (`windows/files.rs`
  hooks LINE's `open`/`open64`/`fopen` stubs). Note LINE already rewrites
  `/tmp/...` to `./tmp/...` itself, so don't add that rewrite again — and
  `fopen64` currently refuses to hook, so a little file activity is invisible.
- **Diagnostic logging is opt-in per subsystem**, because the game is a black
  box with its own reporting compiled out: `cg_log` (shader profile chosen and
  Cg compiler listings), `file_log` (every file opened; failures are always
  logged), `gl_debug` (an OpenGL debug context, relaying the driver's own
  messages). Anything logged per-frame or per-draw **must be deduplicated** —
  an early `gl_debug` build without it produced a 70 MB log in one run.
- **ARM is not a target.** The game is 32-bit x86 and this code rewrites its
  machine code; run the x86 build under emulation instead.

## Testing on the Windows machine

The game folder is `C:\Wangan Midnight Maximum Tune 3DX+ (Export) (2010)`.
Deploy by copying the built DLL plus `dist/*` (minus `start.sh`) and
`dist-windows/{start.bat,fix-libso.ps1,README.md}` into it — exactly what the
`windows` CI job's `Package` step produces. Then run `start.bat` from that
folder.

Logs to collect afterwards: `wal_3dxp.log` (the loader), `line_console.log`
(LINE and the game), and `line.exe.stackdump` if it exists.

## Repository state

CI (`.github/workflows/build.yml`) has never run — nothing has been pushed.
Pushing failed all of the previous session (see docs/HANDOFF.md §8); from a
local session with the user's own credentials it should work.
