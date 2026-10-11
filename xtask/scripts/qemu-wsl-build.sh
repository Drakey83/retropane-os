#!/usr/bin/env bash
# Builds the RetroPane emulator (retropane-qemu) inside WSL/Linux.
# Called by `cargo xtask qemu` with the Windows checkout's path.
#
# The build copy lives at ~/retropane-qemu (Linux file system: much faster than
# /mnt/c). It's synced through git, not by copying the Windows working tree,
# because Git for Windows checks symlinks out as plain files and QEMU uses
# symlinks. Committed state comes from the repository itself; uncommitted
# edits in the Windows checkout are then overlaid file by file.
set -euo pipefail

src="${1:?usage: qemu-wsl-build.sh <retropane-qemu checkout>}"
dst="$HOME/retropane-qemu"
log=/tmp/retropane-qemu-build.log
git_src() { git -c safe.directory='*' -C "$src" "$@"; }

if [ ! -d "$dst/.git" ]; then
    rm -rf "$dst"
    git -c safe.directory='*' clone -q "$src" "$dst"
fi
git -C "$dst" -c safe.directory='*' fetch -q "$src" HEAD
git -C "$dst" checkout -q -f FETCH_HEAD

# Overlay uncommitted changes (modified and new files) from the Windows checkout.
{ git_src diff --name-only HEAD; git_src ls-files --others --exclude-standard; } |
while IFS= read -r f; do
    [ -f "$src/$f" ] || continue
    mkdir -p "$dst/$(dirname "$f")"
    cp "$src/$f" "$dst/$f"
done

mkdir -p "$dst/build"
cd "$dst/build"
if [ ! -f build.ninja ]; then
    # GCC 15 needs the qualifier warning relaxed for QEMU 9.1's bundled dtc.
    ../configure --target-list=arm-softmmu --disable-docs --disable-werror \
        --disable-tools --disable-guest-agent --enable-sdl --enable-slirp \
        --extra-cflags=-Wno-error=discarded-qualifiers > "$log" 2>&1 \
        || { tail -20 "$log"; exit 1; }
fi
if ! make -j"$(nproc)" qemu-system-arm > "$log" 2>&1; then
    grep -A8 FAILED "$log" || tail -20 "$log"
    exit 1
fi
echo "    built: $(./qemu-system-arm --version | head -1)"
