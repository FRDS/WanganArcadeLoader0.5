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
- `fps_limit` (default `60`): Linux caps the frame rate with MangoHUD, Windows
  uses this. `0` turns the cap off.
- `fullscreen = true` is borderless at your monitor's resolution; the game's
  `width` × `height` image is scaled to fit.
- Networking stays off unless `local_ip` is set.
- The `plugins/` folder isn't supported on Windows yet.
