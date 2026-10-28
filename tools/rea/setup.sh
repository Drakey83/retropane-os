#!/usr/bin/env bash
# REA with Ghidra inside WSL (R82). Windows REA refuses Dell's Windows CE
# drivers (PE machine 0x01C2, legacy ARM/Thumb; it admits only ARMNT and
# ARM64), Linux REA opens them. Nothing needs root: everything goes in ~/re.
# Run from Windows: wsl -e bash /mnt/c/.../tools/rea/setup.sh
set -euo pipefail
mkdir -p ~/re && cd ~/re
jdk=https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.9%2B10/OpenJDK21U-jdk_x64_linux_hotspot_21.0.9_10.tar.gz
node=https://nodejs.org/dist/v24.22.1/node-v24.22.1-linux-x64.tar.xz
ghidra=https://github.com/NationalSecurityAgency/ghidra/releases/download/Ghidra_12.1.4_build/ghidra_12.1.4_PUBLIC_20260921.zip
check() { [ "$(sha256sum "$1" | cut -d' ' -f1)" = "$2" ] || { echo "$1: checksum mismatch" >&2; exit 1; }; }
curl -sSL -o jdk.tgz "$jdk" && check jdk.tgz 810d3773df7e0d6c4394e4e244b264c8b30e0b05a0acf542d065fd78a6b65c2f
curl -sSL -o node.txz "$node" && check node.txz 6c172a9ab6718f7b801fb5a7bd486af2ee5e32758086a04b84fcce2aa77095e6
curl -sSL -o ghidra.zip "$ghidra" && check ghidra.zip ddac49f903da9d5bac833e5cc79395098b9c33cfd3279be5f31bd00387d2d4db
tar xzf jdk.tgz && tar xJf node.txz
python3 -c "import zipfile; zipfile.ZipFile('ghidra.zip').extractall('.')"
chmod +x ghidra_12.1.4_PUBLIC/support/* ghidra_12.1.4_PUBLIC/ghidraRun \
    ghidra_12.1.4_PUBLIC/Ghidra/Features/Decompiler/os/linux_x86_64/*
export PATH=~/re/node-v24.22.1-linux-x64/bin:$PATH
mkdir -p rea && cd rea && npm install --no-fund --no-audit rea-agents@6.8.0
