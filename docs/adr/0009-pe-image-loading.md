# ADR-0009: Loading PE images: an image view in the page cache, relocated on demand

- **Status:** Accepted (2026-10-11, by the project owner)
- **Date:** 2026-10-11

## Context

WM5 and WM6.x apps and DLLs are PE32 ARM images (machine `0x01C2`, ARMV4I).
ADR-0005 requires code to be demand-paged and shared libraries to exist once;
ADR-0003 and ADR-0008 give a single address space where CE5 puts each DLL at
the same address in every process.

Two facts about CE binaries make the obvious approaches poor:

- **File and memory alignment differ.** Sections are 512-byte aligned in the
  file but 4 KiB aligned in memory, so file pages can't be mapped directly.
  A page of a section usually straddles two file pages.
- **Relocation is per image, not per process.** Because a DLL sits at the same
  address in every process, it needs relocating at most once system-wide, and
  then only if its preferred base is taken.

The textbook options are to read and relocate the whole image at load (CE's
way: every page resident and private from the start), or to require
position-independent code (Linux's way, unavailable for existing binaries).

## Decision

1. **An image view in the page cache.** Loading an image creates a page-cache
   object that is the image *as laid out in memory* (headers, then each
   section at its RVA, zero elsewhere). Its pages are filled on first touch
   by assembling exactly the file ranges that page needs
   (`rp_kernel_pe::Image::fill_page`). Untouched pages are never read.
2. **Relocated on demand, page by page.** PE groups base relocations into
   blocks of one 4 KiB page each, so filling a view page applies only that
   page's block (`Relocations::apply`), plus the carry for a value straddling
   from the previous page. Each page relocates independently, in any order.
   An image at its preferred base does no relocation work at all.
3. **One relocated copy for everyone.** The view is relocated to the image's
   system-wide load address (the same in every process, CE5's model), so even
   relocated pages stay shared: one copy of each touched page of `coredll`,
   whatever the number of processes.
4. **Sections map as regions of the view.** Read-only and code pages map the
   view's frames directly (`Backing::File`, read-only or executable); writable
   sections are private copy-on-write per process (`shared: false`, ADR-0008),
   except sections marked `IMAGE_SCN_MEM_SHARED`, which map shared in the
   shared area (ADR-0008 rule 6). A page holding both code and data (images
   with small section alignment) needs write and execute together: refused,
   unless the process has the legacy W^X opt-in.
5. **Imports are resolved into the IAT pages, which are private.** The loader
   walks the import table (`for_each_import`; by name or by ordinal, since many
   CE apps import `coredll` by ordinal) and writes each target address into
   the process's copy of the IAT page. Only IAT pages become private.
6. **Untrusted input, bounded work.** Every structure is bounds-checked
   (malformed images give an error, never a panic), and table walks have hard
   limits.

## The four lenses

| Lens | Effect |
|---|---|
| 🔁 Iterate | Host tests with synthetic images; a round-trip relocation test over every page of real WM6 DLLs (local ROM extraction); load time and resident pages per app become device benchmarks |
| ✨ Modernize | NT-style image sections and lazy relocation instead of CE's eager copy |
| 🗄️ Legacy support | Unmodified WM5/6.x binaries, CE5's DLL placement, ordinal imports |
| ⚡ Better / faster | Launch reads only the pages used; relocation cost proportional to pages touched; one copy of each shared page system-wide |

## Alternatives considered

- **Eager load and relocate (CE):** simple, but every page of every image is
  read, relocated and resident at launch, defeating demand paging.
- **Per-process relocation:** unnecessary given CE5's same-address DLL rule,
  and it would make relocated pages private.
- **Mapping file pages directly:** impossible with 512-byte file alignment.

## Consequences

The page cache needs "filler" objects (a callback that produces a page)
rather than only plain file pages; the image view is the first. IAT pages
are private per process; everything else in an image is shared or
copy-on-write.

## Status of the parts

- Done (2026-10-11): `kernel/pe` parses images, fills view pages, relocates
  per page (including boundary-straddling values), lists imports and
  resolves exports (by ordinal; by name via the hint, then binary search;
  forwarders), tested against synthetic images and 12 real WM6 DLLs.
- Next: the page-cache filler object, the loader that turns an image into
  address-space regions, and IAT resolution.
