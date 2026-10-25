# Which decision records govern which code

Read by `tools/remind/rules_gate.py` (the owner's hard rule "ADR before
code", 2026-10-25): a commit touching a path below must name, in its
message, at least one of that path's decision records (`ADR-0034`), as
the record that the design was checked against that ADR's numbered rules.
The longest matching prefix wins; a path matching none needs no ADR. Add a
row with every new ADR that decides an area of code.

| Path prefix | ADRs |
|---|---|
| drivers/mbx-display/ | 0022, 0023, 0027, 0030, 0034 |
| drivers/blit2d/ | 0023, 0030 |
| drivers/mbx-render/ | 0023, 0030 |
| drivers/gfx/ | 0021, 0022 |
| drivers/draw/ | 0021, 0022 |
| drivers/vram-map/ | 0027, 0034 |
| drivers/mbx3d/ | 0024, 0026, 0035 |
| drivers/mbx3d-render/ | 0024, 0026, 0035 |
| drivers/mved/ | 0025 |
| drivers/pxa-udc/ | 0012, 0013, 0014 |
| drivers/usb-msc/ | 0014 |
| drivers/wm8750/ | 0032 |
| drivers/pxa-i2s/ | 0032 |
| drivers/cf/ | 0005, 0013 |
| drivers/sdcard/ | 0005, 0013 |
| drivers/pxa-keypad/ | 0033 |
| drivers/tsc2046/ | 0033 |
| kernel/fat/ | 0005, 0013 |
| kernel/mm/ | 0003, 0006, 0008, 0011, 0019 |
| kernel/sched/ | 0010 |
| kernel/ipc/ | 0016 |
| kernel/pe/ | 0009 |
| kernel/power/ | 0031 |
| kernel/crypto/ | 0029 |
| hal/pxa27x/src/clocks.rs | 0031 |
| boot/loader/ | 0012, 0013, 0017 |
| boot/usb/ | 0012, 0014 |
| boot/kernel/ | 0003, 0008, 0010 |
| boot/gpubench/ | 0024, 0026, 0027, 0035 |
| boot/glesfeat/ | 0024, 0026, 0035 |
| boot/mbx3d/ | 0024, 0026 |
| boot/mbx3d-api/ | 0024, 0026 |
| boot/blend/ | 0022, 0034 |
| boot/mbx/ | 0023 |
| boot/mbx-blend/ | 0023, 0030 |
| boot/mved/ | 0025 |
| boot/clocks/ | 0031, 0035 |
| boot/noise/ | 0029 |
| boot/tone/ | 0032 |
