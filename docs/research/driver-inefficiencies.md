# Windows Mobile driver inefficiencies (evidence log)

A running list of places where Dell's and Intel's 2700G drivers
(`ddi_2700g.dll`, `libGLES_CL.dll`, A08 ROM) look wasteful or conservative,
collected while reverse-engineering them (ADR-0024, DESIGN.md G1c "beat
stock"). **Evidence only: nothing here is acted on yet.** Each entry says
what was seen, where, how sure we are, and what it could be worth. When an
entry is used, record the before/after in `docs/perf.md` and link it here.

Confidence: **measured** (timed on the X51v), **read** (static, the code
says so), **likely** (an inference from code, not confirmed).

| # | What | Where | Confidence | What it could be worth |
|---|---|---|---|---|
| 1 | **GPU waits busy-spin the CPU.** The wait helper polls a memory flag, calling a delay loop between polls (`0x02270694`: an inner count of 10 kept in stack memory, a load and a store per step) and only yields (`Sleep(0)`, `0x022706d4`) after 100 rounds; `GetTickCount` bounds it at 10 s. The CPU burns while the TA and the 3D stage work, though the core raises an interrupt at both (status bits 4 and 2, Dell's handler). | `libGLES_CL.dll` `0x0226fd48`, `0x022611d8`, `0x0227007c` | read | The CPU free during the whole render: a full-screen render takes ~12.5 ms (row 4), a large share of a 60 Hz frame. Wait on the interrupt instead. |
| 2 | **Both register tables are rewritten every scene.** The scene routine (called from `glDrawArrays`, `glDrawElements`, `glClear`, `glEnable`, `glScissor`, `eglMakeCurrent`...) rebuilds the TA table (9 pairs) and the 3D table (20 pairs) and writes all of them, through a function call per register. Most values never change between frames. | `0x0225e5c4`, `0x02261580`, writer `0x0226f4a0` -> `0x02270708` | read | ~29 register writes and calls per scene, most redundant. Write only what changed, inline. |
| 3 | **A full-screen render costs the same whatever is drawn.** ~12.48 ms for a 480 x 640 32-bit target (60 x 40 tiles), from one background triangle up to a full scene; 9.6 ms with every tile empty. | `rp-mbx3d` runs 3-18 | measured | Render only the tiles that changed (the region array lists tiles; leave the others out), or a 16-bit target (untested). |
| 4 | **The 3D clock is switched on per piece of work.** Windows Mobile idles `MBXCLK` at 2D-only (`0x5`); Dell's submit path writes the 3D enable before 3D work. | `ddi_2700g.dll` submit routine (rp-mbx3d notes, ADR-0024 M4) | read (how often it toggles: likely per submit, unmeasured) | Keep 3D on while a game runs; gate it only when idle (ADR power work). |
| 5 | **TA context save and restore on process switches.** When another context's TA work is pending, the driver ends the TA batch early (`0x85C` bit 7 clear), saves its state (`0x864`, `0x820`, `0x818`) and restores it later. | `0x022701c4`, `0x0226fe94`, `0x0227000c` | read | Nothing for one full-screen game; RetroPane can run one 3D client at a time and skip it. |
| 6 | **The clear is a full-screen triangle through the TA.** `glClear` sends a 3-vertex strip covering the target twice over, then the TA bins it into every tile. | `0x0226dff0` | read | Small (TA done in ~1.9 ms, measured, run 15); the 3D stage could start each tile from a background object instead. |
| 7 | **Software state block per draw.** State words go into the TA stream whenever a dirty bit is set (`ctx+0xEC`), with the clip word recomputed each time. | `0x02267868`, `0x0226deb4` | read | Small per draw; a purpose-built stream sends state once per batch. |
| 8 | **User-mode code writes the 3D core's registers directly.** The game maps Dell's register window (shared device structure, `VirtualCopy`) and writes it itself. | ADR-0024 M1 | read | Not a speed problem (it is fast) but a **security** one: any GLES program can program the GPU, whose engines can write anywhere in 2700G memory. RetroPane keeps the GPU behind the graphics service (docs/security.md). |

## Not driver waste (hardware facts, for contrast)

- The 2D engine never signals completion (measured, 2026-10-14): polling a
  fence is forced, not a driver choice.
- The 2D engine can't read system RAM (measured): surfaces must live in
  2700G memory.
- DMA into 2700G memory is no faster than the CPU (30 vs 33 MB/s,
  measured); its gain is a free CPU (84% free during a copy).
