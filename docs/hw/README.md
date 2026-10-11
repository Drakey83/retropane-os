# Hardware specifications

This folder is the **specification** side of the clean room (see
[`../CLEANROOM.md`](../CLEANROOM.md)). Drivers and emulator models are written
from these documents only.

## Format

One file per hardware block (`cpld.md`, `mbx/registers.md`, `acx100.md`...).
Every fact is a table row:

| Field / register | Offset / bits | Meaning | Confidence | Evidence |
|---|---|---|---|---|
| example | `0x00` [3:0] | ... | confirmed / likely / guess | experiment or trace ID, ROM version |

Rules:

- Facts only: no code, no pseudo-code transcribed from disassembly, no listings.
- Each `confirmed` fact needs a matching spec test in the driver crate.
- Record what we **don't** know too (an "Open questions" section).
