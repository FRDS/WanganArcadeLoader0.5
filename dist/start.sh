#!/bin/bash
mkdir -p ./tmp/data/etc/
mkdir -p ./tmp/data/joinstar/
mkdir -p ./tmp/data/maxicoin/
mkdir -p ./tmp/data/ranking/
mkdir -p ./tmp/data/target/
cp ./data/sound/bgm/maxi3/sys_04.wav ./tmp/sys_04.wav

# Remove old C++ libs
if [ -f ./libso/libstdc++.so.6 ]; then 
	mv ./libso/libstdc++.so.6 ./libso/libstdc++.so.6.bak
fi
if [ -f ./libso/libstdc++.so.6.0.7 ]; then 
	mv ./libso/libstdc++.so.6.0.7 ./libso/libstdc++.so.6.0.7.bak
fi
if [ -f ./libso/libz.so ]; then 
	mv ./libso/libz.so ./libso/libz.so.bak
fi
if [ -f ./libso/libz.so.1 ]; then 
	mv ./libso/libz.so.1 ./libso/libz.so.1.bak
fi

# Shaders are handled by the loader now (src/shader.rs), which checks whether
# this driver can run the ones the dump shipped and only replaces them if it
# can't. It uses the compiler inside libCg.so, which ships beside this script,
# so cgc does not need to be installed.
#
# The block that used to be here needed cgc, and was destructive: it wrote
# .recompiled before doing any work and deleted every .fp/.vp with no backup,
# so a single run without cgc left the game with no shaders at all and then
# permanently skipped the retry.

# Fix a dump with broken soft links
for f in ./libso/*; do
	file_type=$(file -b "$f")
	if [[ "$file_type" =~ ASCII ]]; then
		link=$(cat "$f")
		rm "$f"
		ln -s "$link" "$f"
		echo "$f"
	elif [[ "$file_type" =~ data ]]; then
		header=$(cat "$f" | cut -c -8)
		if [[ "$header" =~ IntxLNK ]]; then
			link=$(cat "$f" | tr -d '\0' | cut -c 9-)
			rm "$f"
			ln -s "$link" "$f"
		fi
	fi
done

export LD_LIBRARY_PATH="${PWD};${PWD}/libso"
export MANGOHUD_CONFIG="fps_limit=60;no_display=1"
LC_ALL="C" LD_PRELOAD="libwal_3dxp.so" mangohud --dlsym ./main
