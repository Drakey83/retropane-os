# REA with Ghidra (R82)

[REA](https://github.com/morluto/rea) drives Ghidra's decompiler for the
agent: a standing tool (the owner, 2026-10-28: use it whenever it helps),
first for reading Dell's Windows CE drivers (hardware facts only, the
clean-room rule; output under `local/`, never committed).

- **Linux (WSL) only for Dell's drivers.** REA 6.8.0 on Windows refuses PE
  machine `0x01C2` (Windows CE's ARM/Thumb); in WSL it opens them as ARM.
- `setup.sh` installs JDK 21, Node 24.22.1, Ghidra 12.1.4 and REA 6.8.0 in
  `~/re`, each checked against its published SHA-256; no root.
- `rea.sh` runs REA's CLI with them. One query costs about 30 s (Ghidra
  analyses the file each time; a snapshot barely helps), so for a whole
  driver run `decompile-all` once (`ddi_2700g.dll`: 869 functions in 54 s)
  and search the files.
- Not registered as an MCP server: its 229 tools would sit in every
  session's context; the CLI costs nothing until used.

First use, 2026-10-28: `ddi_2700g`'s doubling code (`0x021270B8`, found by hand
with `llvm-objdump`, 2700g.md) decompiled in 28 s, the same facts as the
hand reading and the instructions, plus the function around it
(`0x02126c90`, the mode set), its callers and where the two flags come from.
