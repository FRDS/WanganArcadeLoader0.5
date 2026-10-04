# WanganArcadeLoader 0.5 — Windows port roadmap

Where the Windows port stands, what was decided, and the research for the
phases not yet started. For how the crate is put together, see
[CLAUDE.md](../CLAUDE.md); for building and running, see that file and
[dist-windows/README.md](../dist-windows/README.md).

The loader builds from **one crate** into `libwal_3dxp.so` (Linux, i586) and
`wal_3dxp.dll` (Windows, i686). On Windows the game's Linux `main` runs under
[LINE](https://github.com/axylol/line), a Windows ELF loader, and this DLL is
LINE's plugin.

## Status

| Phase | State |
|---|---|
| P0 sync with upstream (30 commits), review fixes | done |
| P1 platform layer, Windows boot | done, proven on hardware |
| P2 window + OpenGL | **done, rendering confirmed on hardware** |
| P3 input (keyboard, pads, wheels) | keyboard and pad confirmed; wheels/pedals still untested |
| P4 custom resolution + file/TTY redirects | resolution ported; `file_redirect` done on Windows, `ignore_custom_ioctls` still Linux-only |
| P5 custom BGM | not started |
| P6 Gemballa unlock | not started, fully researched below |
| P7 network / localhost split-screen | not started |

Proven on hardware (Export 3DX+ dump): boots through LINE with 47/47 hooks
resolved, reaches `admCreateWindowi`, renders menus and a lit 3D world, and
plays a race. Stack alignment, LINE's hook table, symbol lookup and Rust std
all work. Confirmed on **Intel Iris Xe** and **Intel UHD**, the latter on a
hybrid laptop beside an RTX 2070 Max-Q.

Also proven, 2026-10-04/05:

- The bundle runs with nothing fetched by hand — our own `line.exe`,
  `soft_oal.dll` and the Cg runtime.
- The shader capability probe works under LINE. It creates a GL context at
  `init()`, before the game has one, and leaves GLFW for `adm.rs` to
  initialise afterwards.
- **The Cg compiler inside `cg.dll` is equivalent to standalone `cgc`.** All
  38 programs of a clean dump compile byte-identically to cgc 3.1.0013's
  output (615 instruction lines each side); only the `# command line args`
  comment differs. So `shader.rs` needs no toolkit install and gives up
  nothing by not having one.
- **The probe sees the GPU the game renders with, including on a hybrid.**
  The OpenGL ICD is loaded once per process, so the game inherits whatever
  card the probe's context brought in. Confirmed on the Intel UHD + RTX 2070
  Max-Q laptop both ways: with no preference set both reported Intel, and
  with Windows' per-application preference pointed at the discrete card the
  same run logged `driver probe: has all of GL_NV_vertex_program3,
  GL_NV_fragment_program2` beside `GL renderer: NVIDIA GeForce RTX 2070`.
  That is why restoring `*.orig` is automatic again rather than advisory.
  Note the `NvOptimusEnablement` export from our DLL did **not** achieve
  this — the per-application setting did.

Still unverified: steering wheels and pedals, audio, a full race to
completion, and anything on Linux since the input rewrite (Linux is
compile-only by decision — the GLFW keyboard switch and the SDL 0.38 bump are
untested there).

## Decisions already made

- **Rust, one codebase**, not a C++ fork. The archived C++ port
  (`axylol/line-wal0.5`) is the reference for LINE mechanics only.
- **Windows-only testing**; Linux gets CI compile checks.
- **GLFW keyboard on both platforms** (drops `x11`, `device_query`).
- **Widescreen was deferred**, on the grounds that it lives inside tattoohanz's
  closed fork of the loader DLL (`msys-line_wal_3dxp.dll`), which has dozens of
  HUD/sprite/anm hooks plus force feedback; LINE loads one plugin, so it can't
  run alongside ours, and getting it would mean asking tattoohanz for source
  (it's GPLv3). **This is being revisited** — the game exposes
  `igMatrix44f::makeOrthographicProjection`, the direct counterpart to the
  `makePerspectiveProjectionRadians` hook `res.rs` already installs, so the 2D
  layer may be correctable from one hook rather than dozens. Note also that an
  earlier HUD rescale (`clSpriteAnime::draw`, scale at `0x40`, x position at
  `0x4C`) existed upstream and was removed in `af420be` in favour of
  letterboxing; recover it with `git show 25c5d33:src/res.rs`.
- **Gemballa patches applied in memory**, never to `main` on disk.
- **ARM is pointless**: the game is 32-bit x86 and our code rewrites its
  machine code, so an ARM build could never load into it. On ARM you run the
  x86 build under emulation (Windows Prism, or Box64/Box32 on Android).

## Windows runtime strategy

Running the game on Windows takes more than our DLL, and the pieces differ in
how far we can go:

| Piece | Status |
|---|---|
| `wal_3dxp.dll` | cross-built from Ubuntu in ~15 s, imports only stock Windows DLLs |
| `line.exe` + `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll` | built by the `line` CI job, bundled |
| `soft_oal.dll` | cross-built with mingw-w64 in the `windows` job, bundled |
| `cg.dll`, `cgGL.dll` | extracted from the Cg Toolkit 3.1 installer, bundled unmodified |

**A release is now self-contained**: a user supplies only their own dump.

Two things decided along the way, so they are not relitigated:

- **Precompiled "universal" shaders cannot replace Cg.** The game `dlopen`s
  `libCg.so` *and* `libCgGL.so` and calls `cgCreateProgram`,
  `cgGLLoadProgram` and `cgGLGetLatestProfile` itself; `cgGL` contains no
  compiler and our loader never loads it. The runtime is needed to load a
  shader in *any* format. Separately, shipping converted `.fp`/`.vp` would
  mean redistributing derivatives of the game's own `.cg` sources, which is
  the line this project holds everywhere else.
- **Cg may be redistributed.** An earlier note here claimed otherwise; that
  was wrong. NVIDIA's licence: "The SOFTWARE may be redistributed providing
  that distributed Cg compiler and runtime binaries are unmodified, except
  for decompression and compression." Debian ships it in `non-free` on the
  same basis. Consequence to keep in mind: the release is no longer
  all-free-software, and the Cg binaries must never be stripped.

OpenAL Soft was likewise not a new dependency when the Windows build picked it
up:
upstream's `build` job has compiled it from source and shipped it as
`libopenal.so` since before this fork, and the archived C++ LINE port already
opened `soft_oal.dll` by that name (`al.cpp`, `dlopen`). Windows was simply
missing the build step. Note the licence: OpenAL Soft is **LGPL v2** (parts
BSD-3-Clause), so the binary ships with `licenses/openal-soft/` — upstream's
Linux artifact still has no licence text for it.

**LINE cannot be cross-compiled.** It is a Cygwin program: `pei-i386`, imports
`msys-2.0.dll`, calls `cygwin_attach_dll`/`cygwin_internal`/`cygwin_premain`.
mingw-w64 cannot produce that, so it needs 32-bit MSYS2 on a Windows host —
which is why the `line` job is the only thing in this repo on a Windows
runner, and why `msys2/setup-msys2` is no help (MSYS2 dropped i686 in 2020).
Everything else we ship cross-builds from Linux.

That job exists because making users build LINE themselves was our single
biggest setup failure: a 64-bit MSYS2 build yields `0xc000007b` with
`msys-gcc_s-1.dll` vs `msys-gcc_s-seh-1.dll` as the only tell.

It also unblocks **patching LINE**, which is the larger prize. Three of its
limits are already in our way and none can be touched while LINE is a binary
someone else built: `fopen64` that MinHook refuses to hook (see P4 below), the
whitelist confining the game to its own directory, and `fixPathIfNeeded`
handling only `/tmp`. Building it ourselves from a pinned commit is the
precondition; forking it is not decided and should not be done casually —
the seam at `platform/windows/line.rs` is what keeps LINE replaceable.

## Remaining work, with research already done

### P4 — TTY redirects on Windows

`file_redirect` now works on both platforms (`platform/windows/files.rs` hooks
LINE's `open`/`open64`/`fopen` stubs). `ignore_custom_ioctls` is still
Linux-only; on Windows hook LINE's stub via `line::resolve_stub("ioctl")`, the
same way `system` and the file stubs already do.

Two known gaps: `fopen64` refuses to hook (MinHook declines the stub at
`0x40ef00`), so a little file activity is invisible; and LINE already rewrites
`/tmp/...` to `./tmp/...` itself in `fixPathIfNeeded`, so don't add that
rewrite again.

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

## Fixed: `dist/start.sh`'s destructive recompile

The Linux launcher used to write its `.recompiled` marker *before* doing any
work and delete each `.fp`/`.vp` with no backup, so one run without `cgc` left
the game with no shader files at all and then permanently skipped the retry.
That block is gone: `shader.rs` does the job with the compiler inside
`libCg.so`, which now ships beside the launcher, so `cgc` need not be
installed. `shader.rs` also recovers the folders the old block wrecked — it
rebuilds when the compiled programs are *missing*, not only when they carry
`OPTION NV_`, which is the case it could not previously see.
