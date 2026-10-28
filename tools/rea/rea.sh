#!/usr/bin/env bash
# REA's CLI with the tools setup.sh put in ~/re. Examples (from Windows):
#   wsl -e bash /mnt/c/.../tools/rea/rea.sh decompile ~/re/targets/ddi_2700g.dll 0x021270B8 --provider ghidra
#   wsl -e bash /mnt/c/.../tools/rea/rea.sh decompile-all ~/re/targets/ddi_2700g.dll --provider ghidra --output <dir under local/>
# Targets are copied into ~/re/targets (a local Linux volume). Output stays
# under local/ (never committed): Dell's code gives hardware facts only.
export PATH=~/re/node-v24.22.1-linux-x64/bin:~/re/jdk-21.0.9+10/bin:$PATH
export JAVA_HOME=~/re/jdk-21.0.9+10 GHIDRA_INSTALL_DIR=~/re/ghidra_12.1.4_PUBLIC
exec ~/re/rea/node_modules/.bin/rea "$@"
