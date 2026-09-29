@echo off
rem WanganArcadeLoader for Windows. Runs the game's Linux "main" under LINE
rem (https://github.com/axylol/line) with wal_3dxp.dll as its plugin.
rem Run from the game folder, next to "main".
rem
rem The game folder name can contain spaces and ")", so %~dp0 and %CD% are
rem never expanded inside ( ) blocks, and every path is quoted.

setlocal EnableExtensions
cd /d "%~dp0"

if not exist "main" echo [start] ERROR: "main" not found. Put this file in the game folder.& exit /b 1
if not exist "wal_3dxp.dll" echo [start] ERROR: wal_3dxp.dll not found.& exit /b 1
for %%F in (line.exe msys-2.0.dll msys-gcc_s-1.dll msys-stdc++-6.dll cg.dll cgGL.dll soft_oal.dll config.toml keyconfig.toml gamecontrollerdb.txt tmp\usb-devices) do if not exist "%%F" echo [start] WARNING: %%F not found

rem One-time setup, as dist/start.sh does on Linux.
for %%D in (etc joinstar maxicoin ranking target) do if not exist "tmp\data\%%D\" mkdir "tmp\data\%%D"
if exist "data\sound\bgm\maxi3\sys_04.wav" if not exist "tmp\sys_04.wav" copy /y "data\sound\bgm\maxi3\sys_04.wav" "tmp\sys_04.wav" >nul

rem Dumps copied from Linux often turn symlinks into empty files, which LINE
rem can't load. Copy the real library over each one.
if exist "fix-libso.ps1" powershell -NoProfile -ExecutionPolicy Bypass -File "fix-libso.ps1"

if exist "data\shader\.recompiled" goto shaders_done
where cgc >nul 2>&1 || goto shaders_skipped
echo [start] Recompiling shaders to arbfp1/arbvp1, originals kept as *.orig
for %%S in ("data\shader\*.cg") do call :compile_shader "%%~nS"
type nul > "data\shader\.recompiled"
goto shaders_done
:shaders_skipped
echo [start] WARNING: cgc not found, so the shaders were NOT recompiled.
echo [start] The game ships them built for NVIDIA. On any other GPU every shader
echo [start] is rejected and the world renders BLACK while the HUD looks fine.
echo [start] Install the NVIDIA Cg Toolkit 3.1 so cgc is on PATH, then run again.
:shaders_done

set "LC_ALL=C"
set "LINE_EXECUTABLE=main"
set "LINE_PLUGIN=wal_3dxp.dll"
if not defined LINE_LIBRARY_PATH if exist "libso\" set "LINE_LIBRARY_PATH=libso"

rem calculate_elf prints "base_address=<hex> base_size=<hex>" for LINE_EXECUTABLE.
set "LINE_BASE_ADDRESS="
set "LINE_BASE_SIZE="
for /f "tokens=2,4 delims== " %%A in ('line.exe calculate_elf') do set "LINE_BASE_ADDRESS=%%A" & set "LINE_BASE_SIZE=%%B"
if not defined LINE_BASE_ADDRESS echo [start] ERROR: "line.exe calculate_elf" failed.& exit /b 1

if exist "line.exe.stackdump" del "line.exe.stackdump"
echo [start] Running the game. Console output goes to line_console.log, loader messages to wal_3dxp.log.
line.exe > "line_console.log" 2>&1
echo [start] line.exe exited with code %ERRORLEVEL%
if exist "line.exe.stackdump" echo [start] line.exe crashed. Send wal_3dxp.log, line_console.log and line.exe.stackdump.
exit /b 0

:compile_shader
set "WAL_SHADER=data\shader\%~1"
if exist "%WAL_SHADER%.fp" if not exist "%WAL_SHADER%.fp.orig" copy /y "%WAL_SHADER%.fp" "%WAL_SHADER%.fp.orig" >nul
if exist "%WAL_SHADER%.vp" if not exist "%WAL_SHADER%.vp.orig" copy /y "%WAL_SHADER%.vp" "%WAL_SHADER%.vp.orig" >nul
cgc -profile arbfp1 "%WAL_SHADER%.cg" -entry p_main -o "%WAL_SHADER%.fp" >nul
cgc -profile arbvp1 "%WAL_SHADER%.cg" -entry v_main -o "%WAL_SHADER%.vp" >nul
exit /b 0
