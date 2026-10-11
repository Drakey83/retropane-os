#!/bin/bash
# Builds HaRET (the Windows Mobile program that boots RetroPane from the SD
# card) inside WSL, from pinned sources, and copies haret.exe to $1.
#
# HaRET: https://github.com/KevinOConnor/haret (GPL-2.0), built with the CeGCC
# arm-mingw32ce toolchain (https://github.com/MaxKellermann/cegcc-build). The
# toolchain build takes a while once (~10 minutes); later runs reuse it.
#
# Local fixes for a 2026 host (GCC 15, Python 3, newer binutils) are applied
# below; HaRET's behaviour is unchanged.
set -euo pipefail

OUT="$1"
CEGCC_REV=392b058298c7c6d7d4fefeb9afad7e82a55217d7
HARET_REV=5f82ea95423126097cbfa95bc33841f33c171741
CEGCC_SRC="$HOME/cegcc-build"
CEGCC="$HOME/cegcc"
HARET="$HOME/haret"
export PATH=/usr/local/bin:/usr/bin:/bin

if [ ! -x "$CEGCC/bin/arm-mingw32ce-g++" ]; then
    echo "    building the CeGCC toolchain (first run only) ..."
    if [ ! -d "$CEGCC_SRC" ]; then
        git clone -q https://github.com/MaxKellermann/cegcc-build "$CEGCC_SRC"
    fi
    git -C "$CEGCC_SRC" checkout -q "$CEGCC_REV"
    git -C "$CEGCC_SRC" submodule update -q --init
    # GCC 15 defaults to C23 and rejects old K&R code in binutils/readline:
    # compile the host tools as C17, permissively. Cross tools are unaffected.
    WRAP="$HOME/cegcc-wrap"
    mkdir -p "$WRAP"
    printf '#!/bin/sh\nexec /usr/bin/gcc -std=gnu17 -fpermissive -fcommon -w "$@"\n' > "$WRAP/gcc"
    printf '#!/bin/sh\nexec /usr/bin/g++ -std=gnu++14 -fpermissive -w "$@"\n' > "$WRAP/g++"
    chmod +x "$WRAP/gcc" "$WRAP/g++"
    ln -sf gcc "$WRAP/cc"
    rm -rf "$HOME/cegcc-output" && mkdir "$HOME/cegcc-output"
    (cd "$HOME/cegcc-output" &&
        PATH="$WRAP:$PATH" "$CEGCC_SRC/build.sh" --prefix="$CEGCC" -j"$(nproc)" \
            > "$HOME/cegcc-build.log" 2>&1) ||
        { echo "CeGCC build failed; see ~/cegcc-build.log" >&2; exit 1; }
fi

if [ ! -d "$HARET" ]; then
    git clone -q https://github.com/KevinOConnor/haret.git "$HARET"
fi
cd "$HARET"
git checkout -q -f "$HARET_REV"
git clean -qfdx
# Python 3 (the machine-list generator was written for Python 2).
sed -i '1s|.*|#!/usr/bin/env python3|; s/line += reader.next()/line += next(reader)/' tools/buildmachs.py
# Newer CeGCC headers no longer define min()/max().
cat > include/rp-compat.h <<'H'
#ifdef __cplusplus
template <class T> static inline T min(T a, T b) { return a < b ? a : b; }
template <class T> static inline T max(T a, T b) { return a > b ? a : b; }
#endif
H
# swp's destination must not share a register with its address (early clobber).
sed -i 's/: "=r" (readval)/: "=\&r" (readval)/' src/l1trace.cpp
# Newer objdump prints references to .text without "+offset".
sed -i 's/^IGNORE=.*/IGNORE=" ARM_26D | ARM_32[ ]*[.]text([+-][^ ]*)?[ ]*$"/' tools/checkrelocs
mkdir -p out
# -fno-jump-tables: the relocatable preloader and IRQ code must not contain
# absolute jump tables (checkrelocs enforces this).
# Static C++ runtime: a stock Windows Mobile device has no libstdc++-6.dll,
# and an import it can't find makes the loader refuse the program ("not
# signed with a trusted certificate, or one of its components cannot be
# found").
rm -f out/haret-debug.exe out/haret.exe
make -s BASE="$CEGCC" CXXFLAGS="-Wall -O -g -MD -march=armv4 -Iinclude \
    -fno-exceptions -fno-rtti -fno-jump-tables -include rp-compat.h -fpermissive -w" \
    LDFLAGS="-Wl,--major-subsystem-version=2,--minor-subsystem-version=10 -static-libstdc++ -static-libgcc" \
    out/haret-debug > "$HOME/haret-build.log" 2>&1 || true
"$CEGCC/bin/arm-mingw32ce-strip" out/haret-debug.exe -o out/haret.exe ||
    { echo "HaRET build failed; see ~/haret-build.log" >&2; exit 1; }
# Refuse to ship anything importing more than a stock device's own DLLs.
if "$CEGCC/bin/arm-mingw32ce-objdump" -p out/haret.exe | grep -i "DLL Name" | grep -viE "coredll|winsock"; then
    echo "haret.exe imports a DLL a stock Windows Mobile device doesn't have" >&2
    exit 1
fi
cp out/haret.exe "$OUT/haret.exe"
echo "    haret.exe built ($(stat -c %s out/haret.exe) bytes, HaRET $HARET_REV)"
