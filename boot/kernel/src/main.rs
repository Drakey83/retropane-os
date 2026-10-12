//! The RetroPane kernel image: the kernel's virtual memory running on the
//! (emulated or real) CPU.
//!
//! So far it brings up the VM layer (`rp-kernel-mm`, ADR-0008) for real:
//!
//! 1. Hands the top 48 MiB of SDRAM to the frame allocator.
//! 2. Builds the system's page tables with `PageTables` and switches the MMU
//!    to them (RAM, the peripheral block and the vectors, all kernel-only).
//! 3. Installs an abort handler that resolves page faults with
//!    `Vm::handle_fault`, then returns to retry the faulting instruction.
//! 4. Creates a process address space in slot 5, switches the FCSE PID to
//!    it, and touches its memory: real faults drive demand-zero paging, file
//!    pages from a (tiny, in-RAM) page cache, and copy-on-write.
//! 5. Checks FCSE relocation, measures the zero-fill fault cost, unmaps
//!    everything and checks every frame came back.
//!
//! 6. IPC (ADR-0016): a client process in slot 5 calls a server process in
//!    slot 6 through a port. Each call migrates the client's thread into the
//!    server's slot (PID and domain switched, the server's own stack, the
//!    client's other registers cleared) and back with the result; a server
//!    that faults unwinds to its caller with an error; a handle the client
//!    doesn't have is refused; each process sees only its own memory at the
//!    same address.
//!
//! Each step prints `VM <step> OK` / `IPC <step> OK` on serial (or `FAIL`
//! with details), then [`MARKER`] if all passed; `cargo xtask emu` checks
//! them.

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed when every VM check passed.
pub const MARKER: &str = "RETROPANE-KERNEL-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_pxa27x::mmu::{Memory, X51V_BRINGUP};
    use rp_hal_pxa27x::rt::{self, Global};
    use rp_hal_pxa27x::testprog::{self, SYS_CALL, SYS_DONE, SYS_RETURN};
    use rp_hal_pxa27x::user::{self, Frame};
    use rp_hal_pxa27x::{Mmio, cpu, devmap};
    use rp_hal_x51v::devices::{RAM_POOL_BASE, RAM_POOL_SIZE, take_ram_pool};
    use rp_kernel_ipc::{Error as IpcError, Handle, Ipc, Rights};
    use rp_kernel_mm::arm::{self, Ap, Attrs, Cache};
    use rp_kernel_mm::domains::{self, Domains};
    use rp_kernel_mm::frames::Frames;
    use rp_kernel_mm::layout;
    use rp_kernel_mm::pagetable::{FrameAlloc, Mmu, PageTables, PhysMem};
    use rp_kernel_mm::space::{
        AddressSpace, Backing, IoError, Outcome, PageCache, Prot, Region, Vm, access_for,
    };
    use rp_kernel_mm::zpool::{ZBuffers, ZPool};

    rp_hal_pxa27x::entry!(main);

    const POOL_FRAMES: usize = RAM_POOL_SIZE / 4096;
    const BASE: u32 = RAM_POOL_BASE as u32;
    const KERNEL_DOMAIN: u32 = 0;
    const PROC_SLOT: u32 = 5;
    const FILE_OBJECT: u32 = 1;
    const FILE_PATTERN: u32 = 0xC0FF_EE00;
    /// Process addresses (as the process sees them: slot 0, relocated by FCSE).
    const ZERO_VA: u32 = 0x0001_0000;
    const ZERO_PAGES: u32 = 64;
    const FILE_VA: u32 = 0x0008_0000;

    /// A one-page "file" standing in for the page cache.
    struct RamFile {
        phys: u32,
        refs: u32,
    }

    impl PageCache for RamFile {
        fn get(&mut self, object: u32, page: u32) -> Result<u32, IoError> {
            if object == FILE_OBJECT && page == 0 && self.phys != 0 {
                self.refs += 1;
                Ok(self.phys)
            } else {
                Err(IoError)
            }
        }
        fn release(&mut self, phys: u32) {
            debug_assert!(phys == self.phys && self.refs > 0, "bad page-cache release");
            self.refs -= 1;
        }
    }

    /// The pool, seen as physical memory. Under this image's map, physical
    /// RAM is identity-mapped, so the pool's addresses are its physical ones.
    struct Mem<'a> {
        pool: &'a mut [u32],
        l1: u32,
    }

    fn word(phys: u32) -> usize {
        ((phys - BASE) / 4) as usize
    }

    impl PhysMem for Mem<'_> {
        fn l1(&mut self) -> &mut [u32; 4096] {
            let i = word(self.l1);
            (&mut self.pool[i..i + 4096])
                .try_into()
                .unwrap_or_else(|_| unreachable!())
        }
        fn l1_phys(&self) -> u32 {
            self.l1
        }
        fn sync(&mut self, phys: u32, len: u32) {
            cpu::clean_range(phys, len);
        }
        fn l2(&mut self, phys: u32) -> &mut [u32; 256] {
            let i = word(phys);
            (&mut self.pool[i..i + 256])
                .try_into()
                .unwrap_or_else(|_| unreachable!())
        }
        fn frame_phys(&self, f: usize) -> u32 {
            BASE + f as u32 * 4096
        }
        fn phys_frame(&self, phys: u32) -> usize {
            ((phys - BASE) / 4096) as usize
        }
        fn zero(&mut self, phys: u32) {
            let i = word(phys);
            self.pool[i..i + 1024].fill(0);
            // Written through the identity map; the process reads it at
            // another address (VIVT): push it to RAM and drop our lines.
            cpu::clean_invalidate_range(phys, 4096);
        }
        fn copy(&mut self, dst: u32, src: u32) {
            let (d, s) = (word(dst), word(src));
            self.pool.copy_within(s..s + 1024, d);
            cpu::clean_invalidate_range(dst, 4096);
        }
        fn with_page<R>(&mut self, phys: u32, f: impl FnOnce(&mut [u8; 4096]) -> R) -> R {
            let i = word(phys);
            let page: &mut [u8; 4096] = rp_hal_mem::words_as_bytes_mut(&mut self.pool[i..i + 1024])
                .try_into()
                .unwrap_or_else(|_| unreachable!());
            let r = f(page);
            // Touched through the identity map: write back and drop those
            // lines, so the page's other mappings see RAM (VIVT).
            cpu::clean_invalidate_range(phys, 4096);
            r
        }
        fn kernel_page(&mut self, phys: u32) -> &mut [u8; 4096] {
            let i = word(phys);
            rp_hal_mem::words_as_bytes_mut(&mut self.pool[i..i + 1024])
                .try_into()
                .unwrap_or_else(|_| unreachable!())
        }
    }

    struct XscaleMmu;

    impl Mmu for XscaleMmu {
        fn clean_page(&mut self, mva: u32) {
            cpu::clean_invalidate_range(mva, 4096);
        }
        fn invalidate_tlb(&mut self, mva: u32) {
            cpu::invalidate_tlb_entry(mva);
        }
        fn clean_all(&mut self) {
            cpu::clean_invalidate_all();
        }
        fn invalidate_tlb_all(&mut self) {
            cpu::invalidate_tlb_all();
        }
    }

    /// The frame allocator and page tables: all-zero when empty, so in their
    /// own statics they cost nothing in the image (`.bss`). Sharing a static
    /// with non-zero state would move all 70 KiB into `.data`.
    static FRAMES: Global<Frames<POOL_FRAMES>> = Global::new(Frames::new());
    static TABLES: Global<PageTables> = Global::new(PageTables::new());
    /// Compressed copies of idle pages (all-zero when empty: `.bss`).
    static ZPOOL: Global<ZPool> = Global::new(ZPool::new());
    /// Compression buffers (12 KiB, `.bss`): never on a kernel stack (the
    /// first version put a page buffer on the 4 KiB abort stack and
    /// overflowed it into the frame allocator).
    static ZBUF: Global<ZBuffers> = Global::new(ZBuffers::new());

    /// The IPC test's server process and what it needs.
    const SERVER_SLOT: u32 = 6;
    /// Where the programs' page appears in both processes, and their stacks.
    const CODE_VA: u32 = 0x0000_8000;
    const STACK_VA: u32 = 0x0004_0000;
    /// The one thread the test runs (the boot thread, gone to user mode).
    const TID: u8 = 1;
    /// What a refused or failed call returns: an error code with the top
    /// bits set (no sum of two small numbers looks like it).
    const ERR: u32 = 0xFFFF_FF00;
    const ERR_FAULT: u32 = ERR | 0xFE;
    // The IPC tables hold one handle table per process slot of the layout.
    const _: () = assert!(
        rp_kernel_ipc::FIRST_SLOT as u32 == layout::FIRST_PROCESS_SLOT
            && rp_kernel_ipc::LAST_SLOT as u32 == layout::LAST_PROCESS_SLOT
    );

    struct IpcState {
        ipc: Ipc,
        vm_ok: bool,
        first_call: u32,
        calls: u32,
    }

    static IPC: Global<IpcState> = Global::new(IpcState {
        ipc: Ipc::new(),
        vm_ok: false,
        first_call: 0,
        calls: 0,
    });

    /// Everything else the VM owns.
    struct State {
        space: AddressSpace,
        server: AddressSpace,
        domains: Domains,
        file: RamFile,
        pool: Option<&'static mut [u32]>,
        l1: u32,
        pid: u32,
        faults: u32,
        last: Option<Outcome>,
        error: Option<&'static str>,
    }

    static STATE: Global<State> = Global::new(State {
        domains: Domains::new(),
        // Its real domain comes from `Domains::activate` before first use.
        space: AddressSpace::process(PROC_SLOT, domains::PARKED),
        server: AddressSpace::process(SERVER_SLOT, domains::PARKED),
        file: RamFile { phys: 0, refs: 0 },
        pool: None,
        l1: 0,
        pid: 0,
        faults: 0,
        last: None,
        error: None,
    });

    type KernelVm<'a> = Vm<'a, Mem<'a>, Frames<POOL_FRAMES>, XscaleMmu, RamFile>;

    /// Runs `f` with the VM assembled from the three globals (taken in a
    /// fixed order: frames, tables, state).
    fn with_vm<R>(f: impl FnOnce(&mut KernelVm<'_>, &mut State) -> R) -> R {
        FRAMES.with(|frames| {
            TABLES.with(|tables| {
                ZPOOL.with(|zpool| {
                    ZBUF.with(|zbuf| {
                        STATE.with(|s| {
                            let mut pool = s.pool.take().unwrap_or_else(|| unreachable!());
                            let mut file = RamFile {
                                phys: s.file.phys,
                                refs: s.file.refs,
                            };
                            let r = {
                                let mut mem = Mem {
                                    pool: &mut pool,
                                    l1: s.l1,
                                };
                                let mut vm = Vm {
                                    tables,
                                    mem: &mut mem,
                                    frames,
                                    mmu: &mut XscaleMmu,
                                    cache: &mut file,
                                    zpool,
                                    zbuf,
                                };
                                f(&mut vm, s)
                            };
                            s.file.refs = file.refs;
                            s.pool = Some(pool);
                            r
                        })
                    })
                })
            })
        })
    }

    /// Page faults: resolve with the VM, or report the fault.
    fn on_abort(a: rt::Abort) -> bool {
        let Some(access) = access_for(arm::classify(a.fsr), !a.data) else {
            return false;
        };
        with_vm(|vm, s| {
            // The XScale's FAR holds the modified (FCSE-relocated) address, but
            // QEMU reports the address before relocation, and prefetch aborts
            // give the PC as the process saw it. Relocating is right for all
            // three: addresses already at or above 32 MiB pass through.
            let mva = layout::mva(a.addr, s.pid);
            match vm.handle_fault(&s.space, mva, access) {
                Ok(o) => {
                    s.faults += 1;
                    s.last = Some(o);
                    true
                }
                Err(e) => {
                    s.error = Some(e.name());
                    false
                }
            }
        })
    }

    fn on_fault(kind: rt::Fault, addr: u32) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        let why = STATE.with(|s| s.error).unwrap_or("not a page fault");
        // Words via write_str, numbers via u32 formatting: no `&str` Display
        // (and its padding code) in the image (ADR-0005 rule 8).
        let _ = uart.write_str("\nFAULT ");
        let _ = uart.write_str(kind.name());
        let _ = writeln!(uart, " at {addr:#010x}");
        let _ = uart.write_str(why);
        let _ = uart.write_str("\n");
        loop {
            core::hint::spin_loop();
        }
    }

    fn cache_for(m: Memory) -> Cache {
        match m {
            Memory::Normal => Cache::WriteBack,
            Memory::WriteCombine => Cache::WriteCombine,
            Memory::Device => Cache::Device,
        }
    }

    fn main() -> ! {
        // Until the kernel's device map is live, print through a temporary
        // handle at the identity address; the real one is taken after the
        // switch, so it carries the new virtual address.
        let mut early = PxaUart::new(rt::emergency_uart());
        early.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = writeln!(early, "\nRetroPane 26 kernel: VM bring-up");
        let mut ok = true;

        // 1-2. Frames, then the kernel's own page tables.
        let Some(pool) = take_ram_pool() else { halt() };
        let pool: &'static mut [u32] = pool;
        let built = FRAMES.with(|frames| -> Result<u32, &'static str> {
            frames.add_range(0, POOL_FRAMES).map_err(|e| e.name())?;
            let l1_frame = FrameAlloc::alloc(frames, 2).ok_or("no L1")?;
            let l1 = BASE + l1_frame as u32 * 4096;
            let i = word(l1);
            pool[i..i + 4096].fill(0);
            cpu::clean_range(l1, 16 * 1024);
            let mut mem = Mem {
                pool: &mut *pool,
                l1,
            };
            // RAM (identity, where this code runs) and the vectors from the
            // bring-up map, and every device window at its kernel address
            // (devmap): nothing at the devices' physical addresses, which lie
            // in the process slots and the shared area.
            let ram = X51V_BRINGUP
                .iter()
                .filter(|r| r.memory == Memory::Normal && r.phys >= layout::RAM_PHYS)
                .map(|r| (r.virt, r.phys, r.len, r.memory));
            let devices = devmap::KERNEL_WINDOWS
                .iter()
                .map(|w| (w.virt, w.phys, w.len, w.memory));
            for (virt, phys, len, memory) in ram.chain(devices) {
                let attrs = Attrs {
                    ap: Ap::Kernel,
                    cache: cache_for(memory),
                };
                let mut off = 0;
                while off < len {
                    TABLES
                        .with(|t| {
                            t.map_section(&mut mem, virt + off, phys + off, KERNEL_DOMAIN, attrs)
                        })
                        .map_err(|e| e.name())?;
                    off += 1 << 20;
                }
            }
            STATE.with(|s| s.l1 = l1);
            Ok(frames.free_frames() as u32)
        });
        STATE.with(|s| s.pool = Some(pool));
        let free_after_setup = match built {
            Ok(n) => n,
            Err(e) => {
                let _ = early.write_str("VM tables FAIL ");
                let _ = early.write_str(e);
                halt()
            }
        };
        let l1 = STATE.with(|s| s.l1);
        cpu::set_dacr(arm::dacr(1 << KERNEL_DOMAIN));
        cpu::switch_table(l1);
        devmap::use_kernel_map();
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        let _ = writeln!(
            uart,
            "VM tables OK (L1 at {l1:#010x}, {free_after_setup} frames free)"
        );
        // The device map: the UART answers at its kernel address, and its
        // physical address (in process slot 32) maps nothing.
        let devmap_ok = with_vm(|vm, _| {
            vm.tables.translate(vm.mem, 0x9010_0000).map(|(p, _)| p) == Some(0x4010_0000)
                && vm.tables.translate(vm.mem, 0x4010_0000).is_none()
                && vm.tables.translate(vm.mem, 0x1000_0000).is_none()
        });
        ok &= report(&mut uart, "device map", devmap_ok, 0);
        rt::set_abort_handler(on_abort);

        // 3-4. A process in slot 5: zero-fill memory and a private file page.
        let file_frame = FRAMES.with(|f| FrameAlloc::alloc(f, 0));
        let mapped = STATE.with(|s| -> Result<(), &'static str> {
            let f = file_frame.ok_or("no file frame")?;
            let phys = BASE + f as u32 * 4096;
            let pool = s.pool.as_deref_mut().unwrap_or_else(|| unreachable!());
            for (k, w) in pool[word(phys)..word(phys) + 1024].iter_mut().enumerate() {
                *w = FILE_PATTERN + k as u32;
            }
            cpu::clean_invalidate_range(phys, 4096);
            s.file.phys = phys;
            let slot = layout::slot_base(PROC_SLOT);
            s.space
                .map(Region {
                    start: slot + ZERO_VA,
                    len: ZERO_PAGES * 4096,
                    prot: Prot::RW,
                    backing: Backing::Zero,
                })
                .map_err(|e| e.name())?;
            s.space
                .map(Region {
                    start: slot + FILE_VA,
                    len: 4096,
                    prot: Prot::RW,
                    backing: Backing::File {
                        object: FILE_OBJECT,
                        offset: 0,
                        shared: false,
                    },
                })
                .map_err(|e| e.name())?;
            s.pid = PROC_SLOT;
            Ok(())
        });
        if let Err(e) = mapped {
            let _ = uart.write_str("VM process FAIL ");
            let _ = uart.write_str(e);
            halt()
        }
        // Give slot 5 its ARM domain and run it: PID and DACR, nothing else.
        let act5 = STATE.with(|s| {
            let a = s.domains.activate(PROC_SLOT);
            s.space.set_domain(a.domain);
            a
        });
        cpu::set_dacr(act5.dacr);
        cpu::set_pid(PROC_SLOT);
        let state = || STATE.with(|s| (s.faults, s.last));

        // Demand-zero: the first read faults in a zeroed page.
        let v = rt::fault_probe(ZERO_VA as usize);
        cpu::poke((ZERO_VA + 4) as usize, 0x1234_ABCD);
        let back = rt::fault_probe((ZERO_VA + 4) as usize);
        let pass = v == 0 && back == 0x1234_ABCD && state() == (1, Some(Outcome::Mapped));
        ok &= report(&mut uart, "zero page", pass, v);

        // A private file page: read maps the page cache's copy...
        let v = rt::fault_probe((FILE_VA + 8) as usize);
        let pass = v == FILE_PATTERN + 2 && state() == (2, Some(Outcome::Mapped));
        ok &= report(&mut uart, "file page", pass, v);

        // ...and writing copies it: the process sees its write, the page cache
        // keeps the original. This code runs privileged, and ARMv5 lets
        // privileged stores through "user read-only" pages, so a plain store
        // would silently change the page cache's copy (the first run of this
        // test did exactly that). Kernel writes to user memory therefore go
        // through prepare_user_write first (ADR-0008 rule 8), which copies.
        let prepared = with_vm(|vm, s| {
            vm.prepare_user_write(&s.space, layout::mva(FILE_VA + 8, PROC_SLOT), 4)
        });
        cpu::poke((FILE_VA + 8) as usize, 0xDEAD_BEEF);
        let mine = rt::fault_probe((FILE_VA + 8) as usize);
        let file_phys = STATE.with(|s| s.file.phys);
        let original = rt::fault_probe((file_phys + 8) as usize);
        let pass = prepared.is_ok() && mine == 0xDEAD_BEEF && original == FILE_PATTERN + 2;
        ok &= report(&mut uart, "copy-on-write", pass, original);

        // FCSE: switching PID moves "address 0x10004" to another slot, while
        // the kernel can still reach slot 5 by its absolute (modified) address.
        cpu::set_pid(6);
        let via_slot = rt::fault_probe((layout::slot_base(PROC_SLOT) + ZERO_VA + 4) as usize);
        let elsewhere = with_vm(|vm, _| vm.tables.translate(vm.mem, layout::mva(ZERO_VA, 6)));
        cpu::set_pid(PROC_SLOT);
        // Two faults so far: the zero page and the file page (the copy was
        // made by prepare_user_write, not by a fault).
        let pass = via_slot == 0x1234_ABCD && elsewhere.is_none() && state().0 == 2;
        ok &= report(&mut uart, "FCSE", pass, via_slot);

        // Domains: with slot 6 running, slot 5's pages are in a domain the
        // DACR doesn't enable, so even the kernel can't read them (CE5 let any
        // process read every slot). try_read turns the domain fault into None
        // instead of a crash; switching back to slot 5 restores access.
        let slot5_word = (layout::slot_base(PROC_SLOT) + ZERO_VA + 4) as usize;
        let act6 = STATE.with(|s| s.domains.activate(6));
        cpu::set_dacr(act6.dacr);
        cpu::set_pid(6);
        let from_6 = rt::try_read(slot5_word);
        cpu::set_dacr(act5.dacr);
        cpu::set_pid(PROC_SLOT);
        let from_5 = rt::try_read(slot5_word);
        let pass = act6.domain != act5.domain
            && from_6.is_none()
            && from_5 == Some(0x1234_ABCD)
            && state().0 == 2;
        ok &= report(&mut uart, "domains", pass, from_5.unwrap_or(0));

        // Zero-fill fault cost: touch fresh pages, one fault each.
        let pages = ZERO_PAGES - 1;
        let start = rt::ticks();
        for p in 1..ZERO_PAGES {
            let _ = rt::fault_probe((ZERO_VA + p * 4096) as usize);
        }
        let ticks = rt::ticks().wrapping_sub(start);
        let pass = state().0 == 2 + pages;
        ok &= report(&mut uart, "fault loop", pass, state().0);
        let _ = writeln!(uart, "BENCH vm_fault_zero {ticks} {pages}");

        // Compressed RAM (ADR-0005 rule 6): compress 32 idle zero-fill pages;
        // their frames go free and their entries hold zpool handles. Touching
        // one faults it back in, intact.
        let free_before = FRAMES.with(|f| f.free_frames() as u32);
        let start = rt::ticks();
        let compressed = with_vm(|vm, s| {
            // A plain loop: the iterator chain cost 2 KB of image.
            let mut n = 0;
            for p in 1..=32u32 {
                if vm.compress(&s.space, layout::mva(ZERO_VA + p * 4096, PROC_SLOT)) == Ok(true) {
                    n += 1;
                }
            }
            n
        });
        let compress_ticks = rt::ticks().wrapping_sub(start);
        let freed = FRAMES.with(|f| f.free_frames() as u32) - free_before;
        let start = rt::ticks();
        let back = rt::try_read((ZERO_VA + 5 * 4096 + 8) as usize);
        let restore_ticks = rt::ticks().wrapping_sub(start);
        let stored = ZPOOL.with(|z| z.stats().1);
        let pass = compressed == 32
            && freed == 31 // 32 frames back, one zpool frame holds them all
            && back == Some(0)
            && state() == (2 + pages + 1, Some(Outcome::Restored))
            && stored == 31;
        ok &= report(&mut uart, "compressed RAM", pass, freed);
        let _ = writeln!(uart, "BENCH vm_compress_page {compress_ticks} 32");
        let _ = writeln!(uart, "BENCH vm_restore_page {restore_ticks} 1");

        // Unmap everything: every frame and page-cache reference returns.
        let slot = layout::slot_base(PROC_SLOT);
        let (unmapped, refs) = with_vm(|vm, s| {
            let r = vm.unmap(&mut s.space, slot, layout::SLOT_SIZE);
            (r.is_ok(), vm.cache.refs)
        });
        // The file's own frame stays allocated (it's the "page cache").
        let free_now = FRAMES.with(|f| f.free_frames() as u32);
        let zpool_empty = ZPOOL.with(|z| z.stats() == (0, 0, 0));
        let pass = unmapped && free_now == free_after_setup - 1 && refs == 0 && zpool_empty;
        ok &= report(&mut uart, "unmap", pass, free_now);

        // 6. IPC: the client (slot 5) and the server (slot 6), each with the
        // programs' page (user code, copied into a frame of its own: no page
        // of the kernel image is ever user-visible), a stack and a data page.
        let (code, client_off, server_off) = testprog::ipc_programs();
        let code_frame = FRAMES.with(|f| FrameAlloc::alloc(f, 0));
        let mapped = STATE.with(|s| -> Result<(), &'static str> {
            let f = code_frame.ok_or("no code frame")?;
            let code_phys = BASE + f as u32 * 4096;
            let pool = s.pool.as_deref_mut().unwrap_or_else(|| unreachable!());
            let page = &mut pool[word(code_phys)..word(code_phys) + 1024];
            page.fill(0);
            page.get_mut(..code.len())
                .ok_or("code too big")?
                .copy_from_slice(code);
            // Out of the D-cache (and any stale I-cache lines) before user
            // mode fetches it through another address.
            cpu::clean_invalidate_range(code_phys, 4096);
            let a5 = s.domains.activate(PROC_SLOT);
            let a6 = s.domains.activate(SERVER_SLOT);
            s.space.set_domain(a5.domain);
            s.server.set_domain(a6.domain);
            for (space, slot) in [(&mut s.space, PROC_SLOT), (&mut s.server, SERVER_SLOT)] {
                let base = layout::slot_base(slot);
                for region in [
                    Region {
                        start: base + CODE_VA,
                        len: 4096,
                        prot: Prot::RX,
                        backing: Backing::Phys {
                            base: code_phys,
                            cache: Cache::WriteBack,
                        },
                    },
                    Region {
                        start: base + testprog::DATA,
                        len: 4096,
                        prot: Prot::RW,
                        backing: Backing::Zero,
                    },
                    // A stack page per call depth (the server may be called
                    // by itself); demand paged, so unused depths cost
                    // nothing.
                    Region {
                        start: base + STACK_VA,
                        len: 4096 * rp_kernel_ipc::MAX_DEPTH as u32,
                        prot: Prot::RW,
                        backing: Backing::Zero,
                    },
                ] {
                    space.map(region).map_err(|e| e.name())?;
                }
            }
            Ok(())
        });
        let client_handle = IPC.with(|i| -> Option<Handle> {
            i.vm_ok = ok;
            let (port, _) = i.ipc.open(SERVER_SLOT as u8, CODE_VA + server_off).ok()?;
            i.ipc.install(PROC_SLOT as u8, port, Rights::CALL).ok()
        });
        let (Ok(()), Some(h)) = (mapped, client_handle) else {
            let _ = uart.write_str("IPC setup FAIL\n");
            halt()
        };
        user::set_syscall_handler(on_syscall);
        user::set_user_fault_handler(on_user_fault);
        switch_to(PROC_SLOT);
        let _ = writeln!(uart, "IPC client starting (handle {})", h.0);
        user::enter_user_at(CODE_VA + client_off, STACK_VA + 4096, u32::from(h.0))
    }

    /// Runs slot `slot`: its domain in the DACR, its PID.
    fn switch_to(slot: u32) {
        let a = STATE.with(|s| {
            s.pid = slot;
            s.domains.activate(slot)
        });
        cpu::set_dacr(a.dacr);
        cpu::set_pid(slot);
    }

    /// The test's system calls: a migrating call, its return, the report.
    fn on_syscall(f: &mut Frame) {
        match f.number() {
            SYS_CALL => {
                let slot = STATE.with(|s| s.pid) as u8;
                let r = IPC.with(|i| {
                    if i.calls == 0 {
                        i.first_call = rt::ticks();
                    }
                    i.calls += 1;
                    i.ipc.call(TID, slot, Handle(f.arg(0) as u8))
                });
                match r {
                    Ok(e) => {
                        // The server runs as a nested activation of this
                        // thread: the caller's registers wait in this frame
                        // on the kernel stack. The server gets its arguments
                        // and nothing else, on its stack for this depth.
                        switch_to(u32::from(e.slot));
                        let result = user::call_user(
                            usize::from(TID),
                            e.entry,
                            STACK_VA + 4096 * e.depth as u32,
                            [0, f.arg(1), f.arg(2), f.arg(3)],
                        );
                        f.set_result(result);
                    }
                    Err(e) => f.set_result(ERR | e as u32),
                }
            }
            SYS_RETURN => {
                finish_call(f.arg(0));
                f.set_result(ERR | IpcError::NotInCall as u32);
            }
            SYS_DONE => done(f),
            _ => f.set_result(ERR | 0xFF),
        }
    }

    /// Ends the call this thread is in: back to the caller's slot, and its
    /// `SYS_CALL` returns `result`. Returns only if the thread is in no
    /// call.
    fn finish_call(result: u32) {
        if let Some(r) = IPC.with(|i| i.ipc.ret(TID).ok()) {
            switch_to(u32::from(r.slot));
            user::resume_call(usize::from(TID), result);
        }
    }

    /// Faults in user mode: demand paging first; a server faulting inside a
    /// call unwinds to its caller with an error; anything else stops.
    fn on_user_fault(_: &mut Frame, kind: rt::Fault, addr: u32) {
        let prefetch = matches!(kind, rt::Fault::PrefetchAbort);
        let (fsr, _) = cpu::fault_registers();
        let resolved = access_for(arm::classify(fsr), prefetch).is_some_and(|access| {
            with_vm(|vm, s| {
                let mva = layout::mva(addr, s.pid);
                let space = if s.pid == SERVER_SLOT {
                    &s.server
                } else {
                    &s.space
                };
                vm.handle_fault(space, mva, access).is_ok()
            })
        });
        if resolved {
            return;
        }
        // A server faulting inside a call: the caller gets an error.
        finish_call(ERR_FAULT);
        on_fault(kind, addr)
    }

    /// The client's report: check it, and the server's own word.
    fn done(f: &mut Frame) -> ! {
        let ticks = rt::ticks().wrapping_sub(IPC.with(|i| i.first_call));
        let (sum, fault_err, handle_err, word) = (f.arg(0), f.arg(1), f.arg(2), f.arg(3));
        let mut uart = PxaUart::new(rt::emergency_uart());
        switch_to(SERVER_SLOT);
        let server_word = rt::try_read((layout::slot_base(SERVER_SLOT) + testprog::DATA) as usize);
        switch_to(PROC_SLOT);
        let calls = IPC.with(|i| i.calls);
        let mut ok = IPC.with(|i| i.vm_ok);
        ok &= report_ipc(&mut uart, "call", sum == 42, sum);
        ok &= report_ipc(&mut uart, "server fault", fault_err == ERR_FAULT, fault_err);
        ok &= report_ipc(
            &mut uart,
            "foreign handle",
            handle_err == ERR | IpcError::BadHandle as u32,
            handle_err,
        );
        let (nested, too_deep) = (f.r[4], f.r[5]);
        ok &= report_ipc(&mut uart, "nested", nested == 45, nested);
        ok &= report_ipc(
            &mut uart,
            "too deep",
            too_deep == ERR | IpcError::TooDeep as u32,
            too_deep,
        );
        ok &= report_ipc(
            &mut uart,
            "isolation",
            word == 0x1111_1111 && server_word == Some(0x2222_2222),
            word,
        );
        let _ = writeln!(uart, "BENCH ipc_call_roundtrip {ticks} {calls}");
        if ok {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
        }
        halt()
    }

    fn report_ipc(uart: &mut PxaUart<Mmio>, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str("IPC ");
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(
                " OK
",
            );
        } else {
            let _ = writeln!(uart, " FAIL (got {got:#010x})");
        }
        pass
    }

    fn report(uart: &mut PxaUart<Mmio>, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str("VM ");
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(
                " OK
",
            );
        } else {
            let _ = writeln!(uart, " FAIL (got {got:#010x})");
        }
        pass
    }

    fn halt() -> ! {
        loop {
            rt::idle();
        }
    }

    #[panic_handler]
    fn panic(info: &core::panic::PanicInfo) -> ! {
        let mut uart = PxaUart::new(rt::emergency_uart());
        if let Some(l) = info.location() {
            let _ = uart.write_str("\nPANIC at ");
            let _ = uart.write_str(l.file());
            let _ = writeln!(uart, ":{}", l.line());
        }
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
