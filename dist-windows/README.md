# WanganArcadeLoader for Windows

On Windows the game's Linux executable (`main`) runs under
[LINE](https://github.com/axylol/line), a Windows loader for Linux ELF
programs. WanganArcadeLoader is LINE's plugin, `wal_3dxp.dll`. It handles the
same things as the Linux build: display, input, dongle, card, OpenAL and
custom resolution.

## What you need next to `main`

From this package:

- `wal_3dxp.dll`, `start.bat`, `fix-libso.ps1`
- `config.toml`, `keyconfig.toml`, `gamecontrollerdb.txt`
- `data/config.lua` (replaces the game's own; back that up first)
- `tmp/`

From elsewhere, all **32-bit**:

- `line.exe`, `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll`, built
  from [axylol/line](https://github.com/axylol/line) with 32-bit MSYS2
  (`C:\msys32`). DLLs from 64-bit MSYS2 fail with error `0xc000007b`.
- `cg.dll`, `cgGL.dll` from the NVIDIA Cg Toolkit 3.1, from its `bin` folder
  (not `bin.x64`).
- `soft_oal.dll` from [OpenAL Soft](https://openal-soft.org), `bin\Win32`.

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

## Configuration

`config.toml` works as on Linux. Missing keys use their defaults. Differences
on Windows:

- `fps_limit` (default `60`): Linux caps the frame rate with MangoHUD, Windows
  uses this. `0` turns the cap off.
- `fullscreen = true` is borderless at your monitor's resolution; the game's
  `width` × `height` image is scaled to fit.
- Networking stays off unless `local_ip` is set.
- The `plugins/` folder isn't supported on Windows yet.
