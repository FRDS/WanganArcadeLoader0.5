# wal_line: Rust-on-Windows go/no-go test

This crate is a test, not a playable loader. It checks whether WanganArcadeLoader can run on Windows as a **Rust plugin for [LINE](https://github.com/axylol/line)**, the Windows ELF loader used by the archived C++ port [line-wal0.5](https://github.com/axylol/line-wal0.5).

It boots the game through the dongle, network, I/O board, card, OpenAL and display hooks until the game calls `admCreateWindowi`. It then logs `REACHED admCreateWindowi — GO` and exits. It doesn't create a window, render anything or read input.

The Linux loader in `../src` is unchanged. Some code here is copied from it for this test: the dongle emulation, the ADM stubs, the OpenAL list and `card.rs`. It gets merged back into one copy during the real port.

## What it checks

| Log line in `wal_line.log` | What it proves |
|---|---|
| `OnInitialize v1` | LINE loads the Rust DLL and calls its exports |
| `boot: N hooks, M missing, K failed` | Symbol lookup and hooking through LINE's function table work |
| `hasp_login called`, `system(...)` | The game calls our Rust hooks correctly |
| `cl_main original returned` | Calling back from Rust into game code works |
| `REACHED admCreateWindowi — GO` | **GO**: Rust is viable on Windows |

Two builds are provided:

- `wal_line.dll` is the real test. Every call from Rust back into game code goes through `shim/align.c`, which aligns the stack to 16 bytes as the game's Linux compiler expects.
- `wal_line_misalign.dll` is a diagnostic. It makes the same call with the stack deliberately misaligned, to show whether the game actually depends on the alignment.

## Setup (Windows, x86/x64)

Put these files in the game folder, next to `main`:

- `line.exe`, `msys-2.0.dll`, `msys-gcc_s-1.dll`, `msys-stdc++-6.dll` (built from axylol/line)
- `cg.dll`, `cgGL.dll`: 32-bit DLLs from the NVIDIA Cg Toolkit
- `soft_oal.dll`: 32-bit DLL from an [OpenAL Soft](https://openal-soft.org) Windows release (`bin/Win32`)
- From the build artifact: `wal_line.dll`, `wal_line_misalign.dll`, `start.bat`, `config.toml`, `tmp/` and `data/config.lua`. The last one replaces the game's own `data/config.lua`, as on Linux, so back up the original first.

`start.bat` sets `LINE_LIBRARY_PATH=libso`, so LINE loads the game's own Linux libraries from `libso/`. If you ran the Linux `start.sh` on this folder before, it renamed `libso/libstdc++.so.6*` and `libso/libz.so*` to `*.bak`. Rename them back if LINE's console log shows missing `std::` or zlib symbols.

## Running

From a command prompt in the game folder:

```bat
start.bat
start.bat misalign
```

The script does the same one-time setup as the Linux `start.sh`:

- creates the `tmp\data\...` folders
- copies `sys_04.wav`
- recompiles the shaders, but only if `cgc` is on `PATH`; originals are kept as `*.orig`

It then asks `line.exe calculate_elf` for the load address and runs LINE with the plugin. At the end it prints `RESULT: GO` or `RESULT: did not reach window creation`.

Send back:

- `wal_line.log` and `line_console.log` (normal run)
- `wal_line_misalign.log` and `line_console_misalign.log` (misalign run)

## Reading the result

- **The normal run reaches GO:** Rust works as a LINE plugin. The misalign run then shows whether the alignment shim is strictly needed. We keep it either way.
- **It stops earlier:** the last lines of `wal_line.log` show how far it got. Symbols missing from the Export build show up as `hook: symbol not found`. A Rust panic is logged as `PANIC: ...` with its location.

## Building

On Linux, with `gcc-mingw-w64-i686`. The mingw compiler must use DWARF exceptions, as Ubuntu 24.04's does.

```sh
rustup target add i686-pc-windows-gnu
cd line-plugin
cargo build --release                                                         # wal_line.dll
cargo build --release --features misalign-test --target-dir target-misalign   # diagnostic build
```

CI (`.github/workflows/windows-test.yml`) builds both DLLs and checks their exports, imports and the shim's disassembly. It uploads them with `start.bat` as the `wal_line-windows-test` artifact.
