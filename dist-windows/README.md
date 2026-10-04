# WanganArcadeLoader for Windows

On Windows the game's Linux executable (`main`) runs under
[LINE](https://github.com/axylol/line), a Windows loader for Linux ELF
programs. WanganArcadeLoader is LINE's plugin, `wal_3dxp.dll`. It handles the
same things as the Linux build: display, input, dongle, card, OpenAL and
custom resolution.

## What you need next to `main`

From this package:

- `wal_3dxp.dll`, `start.bat`, `fix-libso.ps1`
- `soft_oal.dll` — OpenAL Soft 1.23.1, cross-compiled by our CI and
  self-contained; see `licenses/openal-soft/`
- `config.toml`, `keyconfig.toml`, `gamecontrollerdb.txt`
- `data/config.lua` (replaces the game's own; back that up first)
- `tmp/`

Also in the `dist-windows-bundle` download:

- `line.exe`, `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll`, built
  by our CI from [axylol/line](https://github.com/axylol/line) at the commit
  recorded in `licenses/line/SOURCE.txt`.

  If you took the plain `dist-windows` artifact instead, or built from source,
  these four are not here — see "Building LINE yourself" below. They are all
  **32-bit**, and if you substitute your own, all four must come from the
  *same* 32-bit MSYS2 install: 64-bit ones fail with `0xc000007b`, and the
  giveaway is `msys-gcc_s-seh-1.dll` in place of `msys-gcc_s-1.dll`.

From elsewhere — the only thing left to fetch yourself, both **32-bit**:

- `cg.dll`, `cgGL.dll` from the NVIDIA Cg Toolkit 3.1, from its `bin` folder
  (not `bin.x64`). These are NVIDIA-proprietary and can't be redistributed,
  so they stay a manual step. The game needs Cg at run time, and `shader.rs`
  uses the compiler inside `cg.dll` to fix the dump's NVIDIA-only shaders.

## Running

Run `start.bat` from the game folder. Each time, it:

- creates the `tmp\data\...` folders and copies `sys_04.wav`, like `start.sh`;
- repairs `libso/`: dumps copied from Linux often turn library symlinks into
  empty files, and `fix-libso.ps1` copies the real library over each one;
- recompiles the shaders for non-NVIDIA GPUs the first time, if `cgc` is on
  `PATH`;
- runs LINE with the right settings.

The loader writes its messages to `wal_3dxp.log`; the game and LINE write to
`line_console.log`. If something goes wrong, send both, plus
`line.exe.stackdump` if it exists.

## Controls

Defaults from `keyconfig.toml`:

| Action | Keyboard | Pad (Xbox / PlayStation) |
|---|---|---|
| Test / Service | F1 / F2 | |
| Insert card (`card.bin`) | C | Start |
| Gas / Brake | W / S | Right / left trigger, or A / B |
| Steer | A / D | Left stick |
| Gear up / down | E / Q | Right / left bumper |
| Gear N, 1–6 | 0–6 | |
| View / Intrude | V / Space | Y / X |
| Debug menu | Arrow keys | |

Pads work if SDL recognises them (`gamecontrollerdb.txt` adds more). Keys
only register while the game window has focus.

### Steering wheels and pedals

Wheels usually aren't in SDL's pad database, so they use raw joystick
bindings (`JOY_AXIS…`, `JOY_BUTTON…`; the list is at the bottom of
`keyconfig.toml`):

1. Set `input_log = true` in `config.toml` and start the game.
2. Turn the wheel both ways, press each pedal and the buttons you want.
3. `wal_3dxp.log` lists your devices (`JOY0 = "..."`) and each input with the
   name to use, for example
   `JOY0_AXIS2 = -0.98 (a pedal resting at +1? use JOY0_AXIS2_FULL_INV)`.
4. Put those names in `keyconfig.toml`, for example
   `GAS = ["JOY0_AXIS2_FULL_INV"]`, then set `input_log` back to `false`.

## Configuration

`config.toml` works as on Linux. Missing keys use their defaults. Differences
on Windows:

- `input_log` (default `false`): logs controllers and inputs; see above.
- `cg_log` (default `false`): logs the game's Cg shader calls — which profile
  the engine asks for, which one Cg picks for your GPU, and the compiler
  listing for any shader that fails. Turn this on if the world renders black or
  untextured while the HUD looks right. The engine's own shader reporting in
  `alchemy.ini` (`printCompiledShaders`, `defaultReportLevel`) is marked "Debug
  only" and is compiled out of the release build the game ships as, so it
  cannot tell you any of this — the loader has to.
- `fps_limit` (default `60`): Linux caps the frame rate with MangoHUD, Windows
  uses this. `0` turns the cap off.
- `fullscreen = true` is borderless at your monitor's resolution; the game's
  `width` × `height` image is scaled to fit.
- Networking stays off unless `local_ip` is set.
- The `plugins/` folder isn't supported on Windows yet.

## Building LINE yourself

Only needed if you want to change LINE or check our build. LINE is a Cygwin
program, so it can't be cross-compiled — it needs **32-bit** MSYS2 on Windows
(`C:\msys32`; MSYS2 stopped shipping i686 installers in 2020, so use a
`msys2-base-i686-*` archive). Clone
[axylol/line](https://github.com/axylol/line), check out the commit named in
`licenses/line/SOURCE.txt`, then from an MSYS shell:

```sh
pacman --noconfirm --needed -S base-devel msys2-devel cmake ninja wget
rebaseall -p
./scripts/ci.sh
```

That leaves `line.exe` and the three `msys-*.dll` files in `dist/`. Copy all
four — mixing them with DLLs from another MSYS2 install is what produces
`0xc000007b`.

## Licences

This loader and LINE are both GPL-3.0. `LICENSE.txt` is the loader's licence;
`licenses/line/` holds LINE's, its third-party licence texts, the exact
upstream commit we built, and the MSYS2 package list that built it. The
complete corresponding source for the bundled `line.exe` is the `line-source`
artifact of the CI run that produced the bundle, and is also on GitHub at the
commit in `SOURCE.txt`.

`soft_oal.dll` is OpenAL Soft, LGPL v2 with parts under BSD-3-Clause;
`licenses/openal-soft/` holds both texts plus the tag and build flags we used.
It is unmodified, and the loader opens it by filename at run time, so you can
substitute your own build of it without rebuilding anything else.

`cg.dll` and `cgGL.dll` are NVIDIA's and are not included.
