@echo off
rem WanganArcadeLoader on LINE: Rust plugin go/no-go test.
rem Run from the game folder, next to "main".
rem   start.bat            normal test (wal_line.dll)
rem   start.bat misalign   stack alignment diagnostic (wal_line_misalign.dll)
rem
rem The game folder name can contain spaces and ")", so %~dp0 and %CD% are
rem never expanded inside ( ) blocks, and every path is quoted.

setlocal EnableExtensions
cd /d "%~dp0"

set "WAL_PLUGIN=wal_line.dll"
set "WAL_LOG_SUFFIX="
if /i "%~1"=="misalign" set "WAL_PLUGIN=wal_line_misalign.dll"
if /i "%~1"=="misalign" set "WAL_LOG_SUFFIX=_misalign"

if not exist "main" echo [start] ERROR: "main" not found. Put this file in the game folder.& exit /b 1
if not exist "%WAL_PLUGIN%" echo [start] ERROR: %WAL_PLUGIN% not found.& exit /b 1
for %%F in (line.exe msys-2.0.dll msys-gcc_s-1.dll msys-stdc++-6.dll cg.dll cgGL.dll soft_oal.dll config.toml tmp\usb-devices) do if not exist "%%F" echo [start] WARNING: %%F not found

rem One-time setup, as dist/start.sh does on Linux.
for %%D in (etc joinstar maxicoin ranking target) do if not exist "tmp\data\%%D\" mkdir "tmp\data\%%D"
if exist "data\sound\bgm\maxi3\sys_04.wav" if not exist "tmp\sys_04.wav" copy /y "data\sound\bgm\maxi3\sys_04.wav" "tmp\sys_04.wav" >nul

if exist "data\shader\.recompiled" goto shaders_done
where cgc >nul 2>&1 || goto shaders_skipped
echo [start] Recompiling shaders to arbfp1/arbvp1, originals kept as *.orig
for %%S in ("data\shader\*.cg") do call :compile_shader "%%~nS"
type nul > "data\shader\.recompiled"
goto shaders_done
:shaders_skipped
echo [start] cgc not found, shaders not recompiled. This only matters on non-NVIDIA GPUs once the game renders.
:shaders_done

set "LC_ALL=C"
set "LINE_EXECUTABLE=main"
set "LINE_PLUGIN=%WAL_PLUGIN%"
if not defined LINE_LIBRARY_PATH if exist "libso\" set "LINE_LIBRARY_PATH=libso"

rem calculate_elf prints "base_address=<hex> base_size=<hex>" for LINE_EXECUTABLE.
set "LINE_BASE_ADDRESS="
set "LINE_BASE_SIZE="
for /f "tokens=2,4 delims== " %%A in ('line.exe calculate_elf') do set "LINE_BASE_ADDRESS=%%A" & set "LINE_BASE_SIZE=%%B"
if not defined LINE_BASE_ADDRESS echo [start] ERROR: "line.exe calculate_elf" failed.& exit /b 1
echo [start] plugin=%LINE_PLUGIN% base_address=%LINE_BASE_ADDRESS% base_size=%LINE_BASE_SIZE% library_path=%LINE_LIBRARY_PATH%

if exist "wal_line.log" del "wal_line.log"
if exist "line.exe.stackdump" del "line.exe.stackdump"
echo [start] Running line.exe, console output goes to line_console%WAL_LOG_SUFFIX%.log
line.exe > "line_console%WAL_LOG_SUFFIX%.log" 2>&1
echo [start] line.exe exited with code %ERRORLEVEL%
if not "%WAL_LOG_SUFFIX%"=="" if exist "wal_line.log" move /y "wal_line.log" "wal_line%WAL_LOG_SUFFIX%.log" >nul
if not "%WAL_LOG_SUFFIX%"=="" if exist "line.exe.stackdump" move /y "line.exe.stackdump" "line%WAL_LOG_SUFFIX%.exe.stackdump" >nul
if exist "line%WAL_LOG_SUFFIX%.exe.stackdump" echo [start] line.exe crashed. Also send line%WAL_LOG_SUFFIX%.exe.stackdump.

findstr /c:"REACHED admCreateWindowi" "wal_line%WAL_LOG_SUFFIX%.log" >nul 2>&1 && goto result_go
echo [start] RESULT: did not reach window creation. Send wal_line%WAL_LOG_SUFFIX%.log and line_console%WAL_LOG_SUFFIX%.log.
exit /b 1
:result_go
echo [start] RESULT: GO, reached admCreateWindowi. Send wal_line%WAL_LOG_SUFFIX%.log and line_console%WAL_LOG_SUFFIX%.log.
exit /b 0

:compile_shader
set "WAL_SHADER=data\shader\%~1"
if exist "%WAL_SHADER%.fp" if not exist "%WAL_SHADER%.fp.orig" copy /y "%WAL_SHADER%.fp" "%WAL_SHADER%.fp.orig" >nul
if exist "%WAL_SHADER%.vp" if not exist "%WAL_SHADER%.vp.orig" copy /y "%WAL_SHADER%.vp" "%WAL_SHADER%.vp.orig" >nul
cgc -profile arbfp1 "%WAL_SHADER%.cg" -entry p_main -o "%WAL_SHADER%.fp" >nul
cgc -profile arbvp1 "%WAL_SHADER%.cg" -entry v_main -o "%WAL_SHADER%.vp" >nul
exit /b 0
