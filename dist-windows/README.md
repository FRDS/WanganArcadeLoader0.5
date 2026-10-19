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
- `cg.dll`, `cgGL.dll` — the NVIDIA Cg Toolkit 3.1 runtime, redistributed
  unmodified; see `licenses/cg-toolkit/`
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

**Nothing else to fetch.** Everything the loader and the game need beyond
your own dump is in this package.

Why Cg is here: the game `dlopen`s `libCg.so` and `libCgGL.so` itself and
cannot start without them, so no amount of shader work removes the
requirement. `shader.rs` also borrows the Cg compiler that lives inside
`cg.dll` to fix the dump's NVIDIA-only shaders. NVIDIA's licence permits
redistributing the Cg binaries as long as they are unmodified, which is why
they are not stripped. Cg is proprietary — it is the one piece here that
isn't free software.

## Running

Run `start.bat` from the game folder. Each time, it:

- creates the `tmp\data\...` folders and copies `sys_04.wav`, like `start.sh`;
- repairs `libso/`: dumps copied from Linux often turn library symlinks into
  empty files, and `fix-libso.ps1` copies the real library over each one;
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

1. Open "Set up USB game controllers" (run `joy.cpl`), pick your wheel and
   click Properties. Turn the wheel and press each pedal and button, and note
   which numbered axis or button moves.
2. SDL numbers axes, buttons and hats in the same order Windows does, so those
   are the numbers to use. Start the game once and `wal_3dxp.log` names each
   device it opened — `JOY0 = "..." (4 axes, 12 buttons, 1 hats)` — which is
   how to tell `JOY0` from `JOY1` when more than one is plugged in.
3. Put the names in `keyconfig.toml`, for example
   `GAS = ["JOY0_AXIS2_FULL_INV"]`. A pedal that rests at +1 wants
   `_FULL_INV`, one that rests at -1 wants `_FULL`; if a pedal reads backwards
   in game, swap the two.

## Configuration

`config.toml` works as on Linux. Missing keys use their defaults. Differences
on Windows:

- `custom_bgm` (default `false`): adds your own race music. Each folder in
  `plugins/bgms` holding a `mod.toml` and a `mod.png` becomes another set in
  the BGM select screen, next to the three the game ships, and
  `plugins/audio.toml` says which set your card uses — `bgm = "<folder
  name>"`. A `mod.toml` lists the songs:

  ```toml
  [[song]]
  name = "Shown in the list"
  file = "track01.wav"
  ```

  Songs are WAV, the same as the game's own under `data/sound/bgm`. Add
  `stories = ["...", "..."]` to choose what each story episode plays, one
  name per episode; without it story mode picks from the pack at random. The
  set your card carries is masked back to what it really held before anything
  writes it, so a card never records a set a cabinet would read as something
  else. This is a port of vixen256's `audio_3dxp` and reads its file layout,
  so packs made for that plugin work here unchanged.
- `bgm_selector` (default `true`, needs `custom_bgm`): lets the card screen's
  BGM row reach your packs, so you can pick a set in game rather than editing
  `plugins/audio.toml`. On the card check screen, move the cursor to the SOUND
  row with the intrude button and turn the wheel: past the three sets the game
  ships it carries on into your packs and wraps round at either end, showing
  each pack's `mod.png` where the maxi logo sits. Whatever you leave it on is
  written back to `plugins/audio.toml`, so it survives a restart. Normally the
  row is locked unless your card has unlocked it, and it only ever offered
  three sets; this rewrites those limits in memory, checking the bytes it
  expects first and leaving everything alone if they do not match.- `card_log` (default `false`): logs every command the game sends the card
  reader, its payload size and the status the emulation answers with. Turn this
  on if the title screen says the card reader isn't functional, if a save
  doesn't stick, or if anything card-shaped goes wrong at the end of a race —
  it names the command and what it was told, which is the only way to see
  inside that handshake. The checksum of `card.bin` is reported at startup
  too.
- `fps_limit` (default `60`): Linux caps the frame rate with MangoHUD, Windows
  uses this. `0` turns the cap off.
- `width` × `height` is the resolution the game renders at. **Windowed, it has
  to fit inside the window's client area.** A window can't be as tall as your
  display once it has a title bar, so 1080 on a 1080p screen will never fit,
  and whatever doesn't fit is clipped before the game can draw it — maximising
  is the usual way to trip this. What's left is shown undistorted and filling
  the window rather than padded out, so the result is a quietly cropped
  picture rather than an obvious one; `wal_3dxp.log` names the number of rows
  lost. Use fullscreen if you want the whole frame.
- `widescreen_hud` (default `true`): the dump lays its 2D layer out in 640×480
  units and the engine maps that onto whatever it renders into, so a render
  size that isn't 4:3 used to stretch the HUD, the menus and the attract movie
  sideways — a third too wide at 16:9. The loader widens that layer's
  projection to match instead, which keeps it undistorted and centred and
  shows world where the stretch was. It does nothing at 4:3. Elements the game
  anchors near the left or right edge sit a little inward of it; turn this off
  if you'd rather have the old behaviour.
- `ignore_alarm` (default `true`): the game sets a 50 Hz timer for a signal it
  never handles, which left alone ends the game at a random moment with no
  error and no crash dump — if it has ever "just closed" on you, this was why.
  The loader tells the system to ignore that signal. There is no reason to turn
  this off except to see the old behaviour.
- `fullscreen = true` is borderless at your monitor's current resolution; the
  game's `width` × `height` image is scaled to fit, letterboxed if the aspect
  ratios differ. This is the safe choice, and the one to use for a 4:3 render
  size on a 16:9 panel.
- `fullscreen_exclusive = true` (with `fullscreen = true`) changes the display
  mode to `width` × `height` instead. When your monitor offers that mode you
  get it exactly: no letterbox, no upscale, sharpest possible. When it doesn't
  — and 4:3 modes like 1440×1080 are rarely offered on a 16:9 panel — the
  closest mode is used instead, and the log names what you got and lists what
  the monitor has. A mode change also disturbs the rest of your desktop, which
  is why it's off by default.
- `shader_mode` (default `auto`): the dump's shaders ship compiled for NVIDIA
  only. Before the game reads them, the loader asks your driver whether it has
  the extensions they need; if it doesn't, it keeps the originals as `*.orig`
  and compiles portable replacements from the `.cg` sources. On an NVIDIA GPU
  the originals are left in place, because they're the better ones — and if
  some of them are missing, only those are rebuilt, for that card rather than
  portable. Set
  `portable` to always convert, or `original` to never touch the files — only
  worth doing if your driver reports support it doesn't honour. Whatever it
  decides, it says so in `wal_3dxp.log`.
- Networking stays off unless `local_ip` is set.
- Loadable `plugins/*.so` aren't supported on Windows. The BGM packs under
  `plugins/bgms` are read either way, since `custom_bgm` is built in rather
  than a plugin.

## Laptops with two GPUs

On a hybrid laptop Windows usually starts the game on the integrated GPU, so
an Intel UHD can end up rendering while a discrete NVIDIA or AMD card sits
idle. `wal_3dxp.dll` exports the two symbols the vendors look for
(`NvOptimusEnablement`, `AmdPowerXpressRequestHighPerformance`) to ask for the
discrete one, but both vendors document those as coming from the *executable*
and we are a DLL that LINE loads, so the driver may ignore them.

Check which card you actually got — `wal_3dxp.log` names it near the top:

```
[wal_3dxp] GL renderer: Intel(R) UHD Graphics
```

If that isn't the card you wanted, set it per application, which always
works. Either Settings → System → Display → Graphics → Browse → pick
`line.exe` → Options → High performance, or run this once, with the path
edited to match your install:

```powershell
$exe = "D:\Wangan Midnight Maximum Tune 3DX+ (Export) (2010)\line.exe"
$key = "HKCU:\Software\Microsoft\DirectX\UserGpuPreferences"
New-Item -Path $key -Force | Out-Null
New-ItemProperty -Path $key -Name $exe -Value "GpuPreference=2;" -PropertyType String -Force | Out-Null
```

That writes exactly what the Settings page writes, for your user only, and
the Settings page will show it and can undo it. `GpuPreference=1;` is the
integrated GPU instead, and removing the entry restores the default. Set it
on `line.exe`, not on `start.bat` — the batch file is not what renders.

Worth knowing on an NVIDIA card: it has the `GL_NV_*` extensions the dump's
shipped shaders need, so `shader_mode = auto` will leave them in place rather
than converting them, which is the better of the two.

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

`cg.dll` and `cgGL.dll` are NVIDIA's Cg Toolkit 3.1, included unmodified
under the terms in `licenses/cg-toolkit/LICENSE.txt`:

> No Modification. The SOFTWARE may be redistributed providing that
> distributed Cg compiler and runtime binaries are unmodified, except for
> decompression and compression.

Cg is proprietary and its source is not available, so this package as a whole
is not all-free-software — worth knowing if you repackage it. Debian ships
the same toolkit in its `non-free` area. `licenses/cg-toolkit/SOURCE.txt`
records which installer and which files they came from.
