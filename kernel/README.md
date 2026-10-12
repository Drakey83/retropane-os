# kernel/

Scheduler, CE5-style slot memory model with FCSE (ADR-0003), IPC, syscalls, PE loader, on-screen crash screen. Layer: Core. Arrives in Phase 2.

- `mm/`: memory management: frame allocator, page tables, address spaces,
  demand paging and copy-on-write (ADR-0005, ADR-0006, ADR-0008).
- `fat/`: FAT32 on a block device: find and read root-directory files,
  replace one by copy-then-swap, create one (directory entry last), long
  names and Windows' short names, ADR-0013. Never a torn file on the card.
  Host tests include a power-loss sweep (the device fails at every write
  index in turn). The seed of the OS's file system support.
- `pe/`: PE/COFF images for the loader: the image view and per-page
  relocation (ADR-0009).
