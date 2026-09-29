# WanganArcadeLoader 0.5 — Windows port handoff

Everything from the cloud session that produced the Windows port of
WanganArcadeLoader (WMMT 3DX+). **The work is in `wal-3dxp-windows.bundle`,
not on GitHub** — pushing failed the whole session (see "GitHub access").

Repo: `FRDS/WanganArcadeLoader0.5`, branch `claude/nifty-ptolemy-bfx2bw`,
forked from `vixen256/WanganArcadeLoader0.5`.

## 1. Restore the work

```sh
git clone wal-3dxp-windows.bundle WanganArcadeLoader0.5
cd WanganArcadeLoader0.5          # already on claude/nifty-ptolemy-bfx2bw
git remote set-url origin https://github.com/FRDS/WanganArcadeLoader0.5
git remote add upstream https://github.com/vixen256/WanganArcadeLoader0.5
git fetch origin && git fetch upstream
```

The bundle carries the full 114-commit history. I verified it by cloning it
here: HEAD `6482871`, 80 files, clean tree. If you'd rather apply the work
onto an existing clone, `windows-port-vs-upstream.diff` is the same thing
squashed against `upstream/master` (69 files, +7994/−1966).

Note for anyone regenerating this: the cloud checkout was a **shallow** clone,
so `git bundle create --all` silently produced an unusable bundle. It needed
`git fetch --unshallow https://github.com/vixen256/WanganArcadeLoader0.5 master`
first, and explicit refs rather than `--all` (the repo carries stray tags from
an unrelated repo whose objects are incomplete).

`commits.txt` lists the 6 commits on top of upstream. The two oldest
(`f706d19`, `44deea4`) are the throwaway go/no-go test crate, deleted again in
`c52af67`; they're kept for the record and can be squashed away.

## 2. Where it stands

The loader builds from **one crate** into `libwal_3dxp.so` (Linux, i586) and
`wal_3dxp.dll` (Windows, i686). On Windows the game's Linux `main` runs under
[LINE](https://github.com/axylol/line), a Windows ELF loader, and this DLL is
LINE's plugin.

**Proven on the user's Windows PC** (Export 3DX+ dump): the game boots through
LINE with 47/47 hooks resolved, the engine initialises, and it reaches
`admCreateWindowi`. Stack alignment, LINE's hook table, symbol lookup and Rust
std all work.

| Phase | State |
|---|---|
| P0 sync with upstream (30 commits), review fixes | done |
| P1 platform layer, Windows boot | done, proven on hardware |
| P2 window + OpenGL | code done, **not yet confirmed rendering on hardware** |
| P3 input (keyboard, pads, wheels) | code done, **awaiting hardware test** |
| P4 custom resolution + file/TTY redirects | resolution ported; Windows redirects not written |
| P5 custom BGM | not started |
| P6 Gemballa unlock | not started, fully researched (section 7) |
| P7 network / localhost split-screen | not started |

**The immediate next step is the user testing the last build** (rendering,
keyboard, pad, wheel) and reporting back `wal_3dxp.log` + `line_console.log`.

### Architecture

- `src/platform/` is everything OS-specific. The rest of the crate calls
  through it.
  - `linux.rs`: `#[ctor]` constructor, `dlsym` + `retour` hooking, and the
    `#[no_mangle]` libc interposers (`system`, `fopen`, `open`, `ioctl`,
    `rename`, the two libstdc++ ones). Unchanged from upstream in behaviour.
  - `windows/`: exports LINE's `OnInitialize` / `OnPreExecute` / `OnDlOpen` /
    `OnDlSym`; `line.rs` wraps LINE's 8-entry function table (DlSym, Hook via
    MinHook, ResolveStub, module start/size); `crash.rs` logs faults with
    module, registers and a stack scan; `system.rs` emulates the `find`/`cp`
    shell commands the game runs; `log.rs` writes `wal_3dxp.log` flushed per
    line.
  - `call.rs` + `shim/call.c`: **every call from Rust into game code goes
    through `call_game`**, which aligns the stack to 16 bytes as the game's
    Linux gcc code expects. Rust on i686-windows only guarantees 4.
- `build.rs` generates ~1000 cdecl→stdcall OpenGL wrappers from Khronos'
  `gl.xml` (Windows only); the game calls GL cdecl, `opengl32` is stdcall.
- `vendor/retour/` is the upstream `retour` fork with one change: its `win64`
  function-pointer impls are now `#[cfg(target_arch = "x86_64")]`, because
  current rustc rejects that ABI on 32-bit. Without this **nothing builds**.
  Patched in via `[patch]` in `Cargo.toml`.
- Input: keyboard comes from GLFW on both platforms (`x11` and `device_query`
  are gone); SDL2 0.38 handles pads and raw joysticks. 0.36 can't
  cross-compile for windows-gnu.

## 3. Build and test

Toolchain (Ubuntu 24.04; 24.04 matters — its i686 mingw uses DWARF exceptions,
which Rust's `i686-pc-windows-gnu` needs):

```sh
sudo dpkg --add-architecture i386 && sudo apt-get update
sudo apt-get install -y gcc-multilib g++-multilib cmake \
  gcc-mingw-w64-i686 g++-mingw-w64-i686
sudo apt-get install -y --no-install-recommends \
  libx11-dev:i386 libxcb1-dev:i386 libxrandr-dev:i386 libxinerama-dev:i386 \
  libxcursor-dev:i386 libxi-dev:i386
sudo apt-get install -y --no-install-recommends wine32:i386 wine   # for tests
rustup target add i586-unknown-linux-gnu i686-pc-windows-gnu
```

```sh
# Linux
export PKG_CONFIG_PATH=/usr/lib/i386-linux-gnu/pkgconfig/ \
       PKG_CONFIG_SYSROOT_DIR=/ PKG_CONFIG_ALLOW_CROSS=1
cargo build --release --target i586-unknown-linux-gnu
cargo test  --release --target i586-unknown-linux-gnu --lib

# Windows
export CARGO_TARGET_I686_PC_WINDOWS_GNU_LINKER=i686-w64-mingw32-gcc
cargo build --release --target i686-pc-windows-gnu
cargo test  --release --target i686-pc-windows-gnu --lib \
  --config 'target.i686-pc-windows-gnu.runner="wine"'
tests/mockline/run.sh target/i686-pc-windows-gnu/release/wal_3dxp.dll
```

`tests/mockline/` is a **stand-in for LINE**: ~250 lines of C that load the
real DLL, hand it LINE's function table, and play the game's side of the boot
sequence. Its fake game functions are compiled with
`-mincoming-stack-boundary=4` and use aligned SSE stores, so they fault exactly
as the real game would if `call_game` ever stopped aligning the stack. It
checks the dongle, `system()` emulation, OpenAL redirection, the USB path
patch, `cl_main`, custom resolution, main-thread marshalling and the ADM mode
config. **Run it after touching anything in `src/platform/` or `call_game`.**

CI (`.github/workflows/build.yml`) has a `linux` job (unchanged) and a
`windows` job that cross-builds, checks the DLL's exports and that it imports
only DLLs every Windows install has, lints `start.bat`, and runs both test
suites under Wine. **CI has never actually run** — nothing is pushed.

## 4. Testing on Windows

The game folder is `C:\Wangan Midnight Maximum Tune 3DX+ (Export) (2010)`
(spaces and a `)` — `start.bat` is written to survive that; the CI lint
enforces it).

Needs, next to `main`, all **32-bit**:
- `line.exe` + `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll`, built
  from 32-bit MSYS2 (`C:\msys32`). 64-bit MSYS2 DLLs give `0xc000007b`; the
  giveaway is `msys-gcc_s-1.dll not found` (64-bit ships `msys-gcc_s-seh-1.dll`).
  The last 32-bit MSYS2 installer is at https://repo.msys2.org/distrib/i686/.
- `cg.dll`, `cgGL.dll` — NVIDIA Cg Toolkit 3.1, `bin` not `bin.x64`.
- `soft_oal.dll` — OpenAL Soft, `bin\Win32`.

Then `start.bat`. It creates `tmp/`, copies `sys_04.wav`, runs
`fix-libso.ps1`, recompiles shaders if `cgc` is on PATH, and launches LINE.
Logs: `wal_3dxp.log` (loader) and `line_console.log` (LINE + game).

**The dump's broken symlinks bit us hard and will bite others.** Copying the
dump from Linux turned every symlink into a **0-byte file** —
`libstdc++.so.6`, the five `libboost_*-1_36.so.1.36.0`, `libz.so.1`. LINE
can't parse them, every C++ import resolves to 0, and the game dies at
`eip=00000000` during static constructors. `fix-libso.ps1` now copies the real
library over each empty name automatically. **`librt.so.1` is deliberately
left empty** — LINE supplies its functions, and loading glibc's real `librt`
risks a fresh crash. Upstream's `start.sh` only repairs the *text* and
`IntxLNK` symlink forms, not this empty-file one.

## 5. What is and isn't verified

Verified on real Windows hardware: LINE loads the DLL, 47/47 hooks, engine
boots to window creation, with and without the stack-alignment shim (so the
boot path doesn't depend on alignment — but keep the shim; rendering and
gameplay code may well use aligned SSE).

Verified only in the container: both builds, clippy, 7 unit tests (native and
under Wine), the mock LINE host, `fix-libso.ps1` against a synthetic dump
layout, and the CI shell steps run by hand.

**Never verified:** actual rendering, real input devices, audio, a full race,
and anything on Linux since the input rewrite (Linux is compile-only per the
user's decision — the GLFW keyboard switch and SDL 0.38 bump are untested
there).

## 6. Decisions already made

- **Rust, one codebase**, not a C++ fork. The archived C++ port
  (`axylol/line-wal0.5`) is the reference for LINE mechanics only.
- **Windows-only testing**; Linux gets CI compile checks.
- **GLFW keyboard on both platforms** (drops `x11`, `device_query`).
- **Widescreen deferred.** It lives inside tattoohanz's closed fork of the
  loader DLL (`msys-line_wal_3dxp.dll`), which has dozens of HUD/sprite/anm
  hooks plus force feedback. LINE loads one plugin, so it can't run alongside
  ours. Getting it means asking tattoohanz for source (it's GPLv3).
- **Gemballa patches applied in memory**, never to `main` on disk.
- **ARM is pointless**: the game is 32-bit x86 and our code rewrites its
  machine code, so an ARM build could never load into it. On ARM you run the
  x86 build under emulation (Windows Prism, or Box64/Box32 on Android).

## 7. Remaining work, with research already done

### P4 — file/TTY redirects on Windows
Upstream's `file_redirect` and `ignore_custom_ioctls` work on Linux via the
`open`/`ioctl` interposers. On Windows hook LINE's stubs via
`line::resolve_stub("open")` / `("ioctl")`, same as `system` already does in
`platform/windows/mod.rs`. A 3-argument cdecl `open` is enough (no variadics).

### P5 — custom BGM (built in, from `vixen256/audio_3dxp`, GPLv3)
~800 lines of Rust, hooks ~15 game symbols plus a few version-gated absolute
NOPs; reads `plugins/audio.toml` and `plugins/bgms/*`. Port into `src/bgm.rs`
behind a config flag, keep its file layout so existing packs work, and verify
expected bytes before writing the NOPs. Credit vixen256.

### P6 — Gemballa unlock (from tattoohanz's Gemballa-Unlocker)
Extracted from the PyInstaller bundle the user supplied. Two halves:

**(a) `main` byte edits** — apply in memory after verifying the expected bytes.
File offset + `0x8048000` = virtual address. **3DX+ Export v386**
(md5 `6f02c8bc3c6c1711eb2bb97218328ff6`, 24925524 bytes):
- `0x7A8641`: `83 be b0 02 00 00 01` → `...00` — maker-select lower bound,
  lets index 0 (GEMBALLA) be selected. *This is the region lock.*
- `0x7A8209`, `0x7A86AB`, `0x7A87A8`, `0x7A88B3`:
  `c7 44 24 08 01 00 00 00` → `c7 44 24 08 47 1c 00 00` — the four
  `setImageReplace(gemballa, NULL)` calls become identity.
- `0x6895A8`: `83 c8 02` → `90 90 90` — `clV386CardData::assign()` stops
  OR-ing error flag 2 for Gemballa cards, which otherwise makes the card
  invalid.
- `0x12ED6B0`: dword `277` → `247` — story chara 30 (Masaki) drives the
  Gemballa again.
- Optional card-conversion NOPs at `0x6825F4`, `0x683CA7`, `0x684FD5`,
  `0x68666E` (only matter for v337/v363 cards upgrading to v386).
- 24 frame-table immediates at `0x7A8028`–`0x7A8130` (`c7 45 xx` stack
  stores). **Default: leave alone.** They only need changing if you also
  replace the `.anm` timelines with JPN-derived ones.
- A separate 3DX profile exists (`DX_SIZE` 24288148, maker index at
  `[esi+0x494]`, `clV363CardData`, one byte edit at `0x1390E90`: `35`→`34`).

**(b) Asset porting** — ~150 sprites, three car folders (`gem_964_38RS`,
`gem_964_RSR`, `gem_997_AVA`), `.anm` files and `car.lua`, copied from a **JPN
dump**. Cannot be replaced by code; document running tattoohanz's
`copy_gemballa_assets` and keep it an offline step.

**Optional dialogue fix** (from tattoohanz's *widescreen* installer manifest,
same 3DX+ Export target): write 3 UTF-32 strings into free space at
`0x1388971`, `0x1388999`, `0x139CD48` and repoint 3 pointers at `0x58EA0C`,
`0x58EA66`, `0x5900E6`.

### P7 — network
Currently the network boot thread is stubbed out unless `local_ip` is set.
Start by hooking `clNet::getAddress` to return `local_ip` (default
`127.0.0.1`) with a /24 mask, as the C++ port does, and try two instances on
loopback for split-screen. `clNet::setInterfaceAddress` is already stubbed on
Windows. If the game's LAN session needs broadcast, loopback won't carry it —
say so and move to real IPs.

## 8. GitHub access — blocked all session

Every push failed:

```
remote: Claude doesn't have GitHub access to FRDS/WanganArcadeLoader0.5 ...
fatal: ... The requested URL returned error: 403
```

Fix by reconnecting GitHub at https://claude.ai/connect-github and installing
the Claude GitHub App on the repo (an org owner may need to). From a local
session with your own credentials this presumably just works — push the
restored branch and CI will run for the first time.

## 9. Builds already sent to the user

- `wal_line-windows-test.zip`, `…-2.zip` — the go/no-go test plugin (obsolete).
- `wal_3dxp-windows.zip` — first real port build.
- `wal_3dxp-windows-2.zip` — **current**: adds wheel/pedal support, keyboard
  driving keys and `input_log`.

To rebuild that bundle: build the DLL, then copy it plus `dist/*` (minus
`start.sh`) and `dist-windows/{start.bat,fix-libso.ps1,README.md}` into one
folder. The `windows` CI job's `Package` step does exactly this.

## 10. Controls (defaults, `dist/keyconfig.toml`)

| Action | Keyboard | Pad |
|---|---|---|
| Test / Service | F1 / F2 | |
| Card insert | C | Start |
| Gas / Brake | W / S | R/L trigger, or A / B |
| Steer | A / D | Left stick |
| Gear up / down | E / Q | R/L bumper |
| Gear N, 1–6 | 0–6 | |
| View / Intrude | V / Space | Y / X |

Wheels and pedals use raw joystick bindings, since SDL has no pad mapping for
them: `JOY_BUTTON<n>`, `JOY_AXIS<n>+`/`-`, `JOY_AXIS<n>_FULL`,
`JOY_AXIS<n>_FULL_INV` (pedals resting at +1 — most of them),
`JOY_HAT<n>_UP|DOWN|LEFT|RIGHT`, optionally device-scoped as
`JOY1_AXIS2_FULL_INV`. Set `input_log = true` in `config.toml` and the loader
logs each device and every input with the binding name to use.
