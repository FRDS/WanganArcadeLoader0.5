#!/bin/bash
# Runs wal_3dxp.dll inside a mock LINE host under Wine. Usage:
#   tests/mockline/run.sh path/to/wal_3dxp.dll
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
dll="$(realpath "$1")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cc=i686-w64-mingw32-gcc

# Fake soft_oal.dll: every OpenAL function the loader redirects. alGenSources
# returns a marker the host checks.
funcs=$(sed -n '/^const FUNCS/,/^];/p' "$repo/src/al.rs" | grep -o '"al[A-Za-z0-9]*"' | tr -d '"')
{
	for f in $funcs; do
		if [ "$f" = alGenSources ]; then
			echo "__declspec(dllexport) int $f(void) { return 0xA1; }"
		else
			echo "__declspec(dllexport) int $f(void) { return 0; }"
		fi
	done
} > "$work/fake_oal.c"
$cc -shared -O2 -o "$work/soft_oal.dll" "$work/fake_oal.c"

$cc -O2 -msse2 -mincoming-stack-boundary=4 -mpreferred-stack-boundary=4 -c "$here/game.c" -o "$work/game.o"
$cc -O2 -Wall -Wno-cast-function-type "$here/host.c" "$work/game.o" -o "$work/mockline.exe"

cp "$dll" "$work/wal_3dxp.dll"
cp "$repo/dist/keyconfig.toml" "$work/"
cat > "$work/config.toml" <<'TOML'
width = 1280
height = 720
dongle = "285013501138"
TOML

cd "$work"
export WINEDEBUG=-all
status=0
wine mockline.exe wal_3dxp.dll 2>&1 | tr -d '\r' | tee output.txt || true
grep -q "HOST RESULT: ALL CHECKS PASSED" output.txt || status=1
# The revision has to be read before any hook is installed, since a hook that
# gates on it reads it while installing.
grep -q "game revision W3P100-2-NA-DAT0-B02" wal_3dxp.log ||
	{ echo "FAIL: gRomInfo was not read into a known revision"; status=1; }
if [ -f wal_3dxp.log ]; then
	echo "--- wal_3dxp.log"
	cat wal_3dxp.log
fi

# Again with no keyconfig.toml, which used to be fatal: read_to_string().unwrap()
# before a window existed to say why. The shipped bindings are compiled in for
# exactly this case, so every check above must still pass.
echo "--- again with no keyconfig.toml"
rm -f keyconfig.toml card.bin wal_3dxp.log
wine mockline.exe wal_3dxp.dll 2>&1 | tr -d '\r' > defaults.txt || true
grep -q "HOST RESULT: ALL CHECKS PASSED" defaults.txt ||
	{ echo "FAIL: the loader does not boot without keyconfig.toml"; status=1; }
grep -q "using the shipped bindings" wal_3dxp.log ||
	{ echo "FAIL: nothing said about falling back to the shipped bindings"; status=1; }
echo "no keyconfig.toml: $(grep -c 'HOST PASS' defaults.txt) checks still pass"

exit $status
