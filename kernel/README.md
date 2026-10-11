# kernel/

Scheduler, CE5-style slot memory model with FCSE (ADR-0003), IPC, syscalls, PE loader, on-screen crash screen. Layer: Core. Arrives in Phase 2.

- `mm/`: memory management: frame allocator, page tables, address spaces,
  demand paging and copy-on-write (ADR-0005, ADR-0006, ADR-0008).
- `pe/`: PE/COFF images for the loader: the image view and per-page
  relocation (ADR-0009).
