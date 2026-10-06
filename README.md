## Required Packages
### Arch Linux
```
paru -Sy nvidia-cg-toolkit mangohud lib32-glibc lib32-gcc-libs lib32-libx11 lib32-libxcb lib32-libpulse lib32-alsa-lib lib32-libxau lib32-libxdmcp lib32-sndio lib32-libbsd lib32-libmd lib32-mangohud
```
### Debian/Ubuntu/Kali
```
sudo dpkg --add-architecture i386
sudo apt update
sudo apt install nvidia-cg-toolkit mangohud libc6:i386 libx11-dev:i386 libsndio-dev:i386 pulseaudio:i386
```

## Windows
The same loader builds as a plugin for [LINE](https://github.com/axylol/line) on Windows. See [dist-windows/README.md](dist-windows/README.md).

## Credits
- [vixen256/WanganArcadeLoader0.5](https://github.com/vixen256/WanganArcadeLoader0.5) — the loader this is a fork of.
- [axylol/line](https://github.com/axylol/line) — the Windows ELF loader this runs inside as a plugin.
- **tattoohanz** — the widescreen research, the Gemballa unlocker, and the C++ LINE plugin whose comments record how the card printer, the sprite layer and the force-feedback board actually behave. His work is GPL-3, as is this project, which is what makes porting it legitimate; where this loader reimplements one of his measurements, the module says so.
- [vixen256/audio_3dxp](https://github.com/vixen256/audio_3dxp) — GPLv3, the basis for the custom BGM support planned as P5.

The Cg runtime redistributed in the Windows bundle is NVIDIA's, under a licence that permits redistribution of **unmodified** binaries only. The game's own assets are not redistributed by this project in any form.
