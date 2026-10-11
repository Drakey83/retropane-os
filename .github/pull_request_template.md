## What and why

<!-- What does this change, and why? Link the issue/phase. -->

## The four lenses (DESIGN.md §1.0)

A change should advance at least one lens and must not set back any.

- [ ] 🔁 **Iterate:** there's a test or benchmark capturing this, so future work can beat it
- [ ] ✨ **Modernize:** brings 2026 standards, security or UX (or N/A: explain)
- [ ] 🗄️ **Legacy support:** WM5/WM6.x apps and user expectations still work (or N/A: explain)
- [ ] ⚡ **Better/faster:** no performance regression; improvements measured (numbers below)

## Checklist

- [ ] `cargo xtask ci` passes locally
- [ ] New behaviour has unit tests (drivers: test-first against `rp-hal-mock`)
- [ ] Hardware facts are specified in `docs/hw/` with confidence levels
- [ ] Follows `docs/CLEANROOM.md`: no proprietary code, binaries, or disassembly committed
- [ ] Tested on: ☐ host ☐ emulator ☐ real X51v

## Measurements (if relevant)

| Benchmark | Before | After |
|---|---|---|
