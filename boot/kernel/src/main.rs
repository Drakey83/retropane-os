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
//! 6. Threads and IPC (ADR-0010, ADR-0016): the scheduler starts an idle
//!    thread and two client threads in process slot 5, which call a server
//!    process in slot 6 through a port at the same time, on a short quantum
//!    so they are preempted inside calls. Each call runs the server as a
//!    nested activation of the calling thread (PID and domain switched, a
//!    server stack per thread and depth, the caller's other registers
//!    cleared) and returns its result; the server calls itself 4 deep and
//!    the fifth level is refused; a server that faults unwinds to its
//!    caller with an error; a handle the client doesn't have is refused;
//!    each process sees only its own memory at the same address.
//! 7. A process from a PE file (ADR-0009): the programs are packaged at
//!    start as a PE image (`.text`, `.rdata`, `.data`, `.reloc`) in a RAM
//!    "file", and both processes map it at a base it doesn't prefer, each
//!    page with its section's protection. A page is loaded (filled and
//!    relocated) on its first fault, once, and shared; the client reaches a
//!    word in `.rdata` through two relocated addresses.
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
    use rp_hal_pxa27x::{Mmio, cpu, devmap, thread};
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
    use rp_kernel_pe::build::{self, Header, SectionSpec};
    use rp_kernel_pe::{Dir, Image, Relocations, dir, scn};
    use rp_kernel_sched::{Eid, LOWEST, Scheduler, Tid};

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
    /// The reclaim test's pages.
    const RECLAIM_VA: u32 = 0x0020_0000;
    const RECLAIM_PAGES: u32 = 16;

    /// The test image (a PE file built at start) as a page-cache object,
    /// loaded at `LOAD_BASE`, not the base it prefers, so every absolute
    /// address in it must be relocated.
    const PE_OBJECT: u32 = 2;
    const LOAD_BASE: u32 = 0x0010_0000;
    /// View pages the cache can hold for it.
    const PE_PAGES: usize = 8;

    /// The page cache: a one-page "file", and the test image's view pages.
    #[derive(Clone, Copy)]
    struct RamFile {
        phys: u32,
        refs: u32,
        /// The image's file (a pool frame; 0: none yet).
        pe_file: u32,
        /// Its view pages as loaded (filled and relocated); 0: not yet.
        pe: [u32; PE_PAGES],
        pe_refs: u32,
        /// Pages loaded so far (each once, however many processes map it).
        pe_loads: u32,
    }

    impl PageCache for RamFile {
        fn get(&mut self, object: u32, page: u32) -> Result<u32, IoError> {
            if object == FILE_OBJECT && page == 0 && self.phys != 0 {
                self.refs += 1;
                Ok(self.phys)
            } else if object == PE_OBJECT
                && let Some(&phys) = self.pe.get(page as usize).filter(|&&p| p != 0)
            {
                self.pe_refs += 1;
                Ok(phys)
            } else {
                Err(IoError)
            }
        }
        fn release(&mut self, phys: u32) {
            if phys == self.phys {
                debug_assert!(self.refs > 0, "bad page-cache release");
                self.refs -= 1;
            } else {
                debug_assert!(self.pe_refs > 0, "bad page-cache release");
                self.pe_refs -= 1;
            }
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
    /// Where the programs' stacks are in both processes.
    const STACK_VA: u32 = 0x0004_0000;
    /// The test image's layout (`build_image`).
    const TEXT_RVA: u32 = 0x1000;
    const RDATA_RVA: u32 = 0x2000;
    const DATA_RVA: u32 = 0x3000;
    const RELOC_RVA: u32 = 0x4000;
    /// User stack pages per thread: its own, then one per call depth (a
    /// thread in a server runs on that server's page for the depth).
    const STACK_PAGES: u32 = 1 + rp_kernel_ipc::MAX_DEPTH as u32;
    /// The stacks region in each process (demand paged: only touched pages
    /// cost a frame).
    const STACK_REGION: u32 = 64 * 4096;
    /// Client threads, each running the whole client program in slot 5 at
    /// once, sharing the server.
    const CLIENTS: usize = 2;
    /// Kernel stacks taken off the top of the pool (4 KiB each): the idle
    /// thread's and the clients'; the boot thread keeps its own.
    const KSTACKS: usize = 1 + CLIENTS;
    /// A short quantum (~150 us) so the clients are preempted many times,
    /// inside calls too.
    const QUANTUM: u32 = 500;
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
        first_call: u32,
        calls: u32,
    }

    static IPC: Global<IpcState> = Global::new(IpcState {
        ipc: Ipc::new(),
        first_call: 0,
        calls: 0,
    });

    /// The scheduler (all-zero when empty: `.bss`).
    static SCHED: Global<Scheduler> = Global::new(Scheduler::new());

    /// Who runs, and where.
    struct Run {
        current: Tid,
        /// Per thread: the slot it runs in now (its process's, or a
        /// server's while in a call; 0 for kernel threads).
        slots: [u8; rp_kernel_sched::MAX_THREADS],
        /// Set by each client as it finishes.
        done: Eid,
        /// The client program's entry and the handle it calls through.
        entry: u32,
        handle: u8,
        /// Each client's report (the `SYS_DONE` registers r0-r6).
        results: [[u32; 7]; CLIENTS],
        finished: usize,
        /// Thread switches away from a thread that was inside a call.
        in_call_switches: u32,
    }

    static RUN: Global<Run> = Global::new(Run {
        current: Tid(0),
        slots: [0; rp_kernel_sched::MAX_THREADS],
        done: Eid(0),
        entry: 0,
        handle: 0,
        results: [[0; 7]; CLIENTS],
        finished: 0,
        in_call_switches: 0,
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
        file: RamFile {
            phys: 0,
            refs: 0,
            pe_file: 0,
            pe: [0; PE_PAGES],
            pe_refs: 0,
            pe_loads: 0,
        },
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
                            let mut file = s.file;
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
                            s.file = file;
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
        // The top frames are kernel stacks for threads, never handed out.
        let (pool, kstacks) = pool.split_at_mut(pool.len() - KSTACKS * 1024);
        let mut kstacks = kstacks.chunks_exact_mut(1024);
        let built = FRAMES.with(|frames| -> Result<u32, &'static str> {
            frames
                .add_range(0, POOL_FRAMES - KSTACKS)
                .map_err(|e| e.name())?;
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

        // Reclaim (ADR-0005): a clock with a second chance. Fresh pages, each
        // marked; one sweep ages them all; half are touched again (minor
        // faults); the next sweep ages those again and compresses the rest;
        // an evicted page comes back intact.
        let region = layout::slot_base(PROC_SLOT) + RECLAIM_VA;
        let mapped = STATE.with(|s| {
            s.space
                .map(Region {
                    start: region,
                    len: RECLAIM_PAGES * 4096,
                    prot: Prot::RW,
                    backing: Backing::Zero,
                })
                .is_ok()
        });
        for p in 0..RECLAIM_PAGES {
            cpu::poke((RECLAIM_VA + p * 4096 + 12) as usize, 0xA6E0_0000 + p);
        }
        let sweep = || {
            with_vm(|vm, s| {
                let mut hand = region;
                vm.reclaim(&s.space, &mut hand, RECLAIM_PAGES, u32::MAX)
            })
        };
        let first = sweep();
        let faults0 = state().0;
        for p in 0..RECLAIM_PAGES / 2 {
            let _ = rt::fault_probe((RECLAIM_VA + p * 4096) as usize);
        }
        let minor = (state().0 - faults0, state().1);
        let free0 = FRAMES.with(|f| f.free_frames() as u32);
        let start = rt::ticks();
        let second = sweep();
        let sweep_ticks = rt::ticks().wrapping_sub(start);
        let freed = FRAMES.with(|f| f.free_frames() as u32) - free0;
        let back = rt::try_read((RECLAIM_VA + (RECLAIM_PAGES - 1) * 4096 + 12) as usize);
        let half = RECLAIM_PAGES / 2;
        let pass = mapped
            && first.is_ok_and(|r| r.scanned == RECLAIM_PAGES && r.aged == RECLAIM_PAGES)
            && minor == (half, Some(Outcome::Referenced))
            && second.is_ok_and(|r| r.aged == half && r.compressed == half && r.dropped == 0)
            && freed + 1 >= half
            && back == Some(0xA6E0_0000 + RECLAIM_PAGES - 1)
            && state().1 == Some(Outcome::Restored);
        ok &= report(&mut uart, "reclaim", pass, freed);
        let _ = writeln!(uart, "BENCH vm_reclaim_sweep {sweep_ticks} {RECLAIM_PAGES}");

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

        // 6-7. IPC, from a PE file: the client (slot 5) and the server (slot
        // 6) both map the test image (user code: no page of the kernel image
        // is ever user-visible), each with its own stack and data page.
        let programs = testprog::ipc_programs();
        let file_frame = FRAMES.with(|f| FrameAlloc::alloc(f, 0));
        let mapped = STATE.with(|s| -> Result<u32, &'static str> {
            let f = file_frame.ok_or("no file frame")?;
            let file_phys = BASE + f as u32 * 4096;
            let pool = s.pool.as_deref_mut().unwrap_or_else(|| unreachable!());
            let file =
                rp_hal_mem::words_as_bytes_mut(&mut pool[word(file_phys)..word(file_phys) + 1024]);
            build_image(file, &programs).map_err(|e| e.name())?;
            let image = Image::parse(file).map_err(|e| e.name())?;
            // W^X: an image with a page both writable and executable is
            // refused, not mapped.
            if image.writable_code() || image.pages() as usize > PE_PAGES {
                return Err("image refused");
            }
            s.file.pe_file = file_phys;
            let a5 = s.domains.activate(PROC_SLOT);
            let a6 = s.domains.activate(SERVER_SLOT);
            s.space.set_domain(a5.domain);
            s.server.set_domain(a6.domain);
            for (space, slot) in [(&mut s.space, PROC_SLOT), (&mut s.server, SERVER_SLOT)] {
                let base = layout::slot_base(slot);
                // Each view page as its section says; a page no section
                // covers stays unmapped (a guard).
                for page in 0..image.pages() {
                    let Some(q) = image.page_protection(page) else {
                        continue;
                    };
                    let prot = if q.exec {
                        Prot::RX
                    } else if q.write {
                        Prot::RW
                    } else {
                        Prot::R
                    };
                    space
                        .map(Region {
                            start: base + LOAD_BASE + page * 4096,
                            len: 4096,
                            prot,
                            backing: Backing::File {
                                object: PE_OBJECT,
                                offset: page * 4096,
                                shared: q.shared,
                            },
                        })
                        .map_err(|e| e.name())?;
                }
                for region in [
                    Region {
                        start: base + testprog::DATA,
                        len: 4096,
                        prot: Prot::RW,
                        backing: Backing::Zero,
                    },
                    // Stacks: per thread and call depth (`stack_top`).
                    Region {
                        start: base + STACK_VA,
                        len: STACK_REGION,
                        prot: Prot::RW,
                        backing: Backing::Zero,
                    },
                ] {
                    space.map(region).map_err(|e| e.name())?;
                }
            }
            Ok(image.pages())
        });
        let pe_pages = *mapped.as_ref().unwrap_or(&0);
        let mapped = mapped.map(|_| ());
        let client_handle = IPC.with(|i| -> Option<Handle> {
            let server = LOAD_BASE + TEXT_RVA + programs.server;
            let (port, _) = i.ipc.open(SERVER_SLOT as u8, server).ok()?;
            i.ipc.install(PROC_SLOT as u8, port, Rights::CALL).ok()
        });
        // Threads: this code becomes the boot thread; an idle thread; the
        // clients, each entering the client program in slot 5.
        let threads = SCHED.with(|s| -> Option<Eid> {
            s.set_quantum(QUANTUM);
            let boot = s.create(0, 150).ok()?;
            let _ = s.schedule(rt::ticks());
            let done = s.create_event(false, false).ok()?;
            let idle = s.create(0, LOWEST).ok()?;
            thread::prepare(idle.index(), kstacks.next()?, idle_thread, 0);
            for _ in 0..CLIENTS {
                let t = s.create(PROC_SLOT as u8, 200).ok()?;
                thread::prepare(t.index(), kstacks.next()?, client_thread, 0);
            }
            RUN.with(|r| r.current = boot);
            Some(done)
        });
        let (Ok(()), Some(h), Some(done)) = (mapped, client_handle, threads) else {
            let _ = uart.write_str("IPC setup FAIL\n");
            halt()
        };
        RUN.with(|r| {
            r.done = done;
            r.entry = LOAD_BASE + TEXT_RVA + programs.client;
            r.handle = h.0;
        });
        user::set_syscall_handler(on_syscall);
        user::set_user_fault_handler(on_user_fault);
        rt::set_preempt_hook(reschedule);
        let _ = writeln!(uart, "IPC {CLIENTS} clients starting (handle {})", h.0);
        while RUN.with(|r| r.finished) < CLIENTS {
            wait(done);
        }
        ok &= report_clients(&mut uart, pe_pages);
        if ok {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
        }
        halt()
    }

    /// A client: enters the client program in its process, for good.
    extern "C" fn client_thread(_: usize) -> ! {
        let (me, entry, h) = RUN.with(|r| {
            r.slots[r.current.index()] = PROC_SLOT as u8;
            (r.current, r.entry, r.handle)
        });
        switch_to(PROC_SLOT);
        user::enter_user_at(entry, stack_top(me, 0), u32::from(h))
    }

    /// Where thread `t`'s user stack starts at call depth `depth` (0: its
    /// own process), in whichever slot it runs.
    fn stack_top(t: Tid, depth: usize) -> u32 {
        STACK_VA + 4096 * (u32::from(t.0) * STACK_PAGES + depth as u32 + 1)
    }

    extern "C" fn idle_thread(_: usize) -> ! {
        loop {
            let due = match SCHED.with(|s| s.next_deadline()) {
                Some(at) => !rt::set_alarm(at),
                None => false,
            };
            if !due {
                rt::idle_until_interrupt(|| SCHED.with(|s| s.has_ready()));
            }
            reschedule();
        }
    }

    /// A scheduling point: runs the most urgent ready thread, in the slot
    /// it was running in (a server's, if it is inside a call).
    fn reschedule() {
        let (next, deadline) = SCHED.with(|s| (s.schedule(rt::ticks()), s.next_deadline()));
        if let Some(at) = deadline
            && !rt::set_alarm(at)
        {
            let _ = rt::set_alarm(rt::ticks().wrapping_add(64));
        }
        let Some(next) = next else {
            return;
        };
        let (cur, slot) = RUN.with(|r| {
            (
                core::mem::replace(&mut r.current, next),
                r.slots[next.index()],
            )
        });
        if cur != next {
            if IPC.with(|i| i.ipc.depth(cur.0)) > 0 {
                RUN.with(|r| r.in_call_switches += 1);
            }
            if slot != 0 {
                switch_to(u32::from(slot));
            }
            thread::switch(cur.index(), next.index());
        }
    }

    /// Blocks the running thread until `e` is set.
    fn wait(e: Eid) {
        if SCHED.with(|s| s.wait(e, None)) == Ok(None) {
            reschedule();
        }
    }

    /// Whether the running thread's quantum (or a sleeper's time) is up.
    fn deadline_passed() -> bool {
        SCHED
            .with(|s| s.next_deadline())
            .is_some_and(|d| rt::ticks().wrapping_sub(d) as i32 >= 0)
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
    /// Each is a scheduling point: a thread whose quantum ran out while
    /// in the kernel gives way here.
    fn on_syscall(f: &mut Frame) {
        if deadline_passed() {
            reschedule();
        }
        let me = RUN.with(|r| r.current);
        match f.number() {
            SYS_CALL => {
                let slot = STATE.with(|s| s.pid) as u8;
                let r = IPC.with(|i| {
                    if i.calls == 0 {
                        i.first_call = rt::ticks();
                    }
                    i.calls += 1;
                    i.ipc.call(me.0, slot, Handle(f.arg(0) as u8))
                });
                match r {
                    Ok(e) => {
                        // The server runs as a nested activation of this
                        // thread: the caller's registers and slot wait in
                        // this frame on the kernel stack. The server gets
                        // its arguments and nothing else, on its stack for
                        // this depth.
                        enter_slot(me, e.slot);
                        let result = user::call_user(
                            me.index(),
                            e.entry,
                            stack_top(me, e.depth),
                            [0, f.arg(1), f.arg(2), f.arg(3)],
                        );
                        enter_slot(me, slot);
                        f.set_result(result);
                    }
                    Err(e) => f.set_result(ERR | e as u32),
                }
            }
            SYS_RETURN => {
                finish_call(me, f.arg(0));
                f.set_result(ERR | IpcError::NotInCall as u32);
            }
            SYS_DONE => done(me, f),
            _ => f.set_result(ERR | 0xFF),
        }
    }

    /// Thread `t` now runs in `slot` (switched now, and on every switch
    /// back to it).
    fn enter_slot(t: Tid, slot: u8) {
        RUN.with(|r| r.slots[t.index()] = slot);
        switch_to(u32::from(slot));
    }

    /// Ends the call thread `t` is in: its `SYS_CALL` returns `result` (and
    /// switches back to the caller's slot). Returns only if the thread is
    /// in no call.
    fn finish_call(t: Tid, result: u32) {
        if IPC.with(|i| i.ipc.ret(t.0)).is_ok() {
            user::resume_call(t.index(), result);
        }
    }

    /// Faults in user mode: demand paging first; a server faulting inside a
    /// call unwinds to its caller with an error; anything else stops.
    fn on_user_fault(_: &mut Frame, kind: rt::Fault, addr: u32) {
        let prefetch = matches!(kind, rt::Fault::PrefetchAbort);
        let (fsr, _) = cpu::fault_registers();
        let resolved = access_for(arm::classify(fsr), prefetch).is_some_and(|access| {
            with_vm(|vm, s| {
                load_pe_page(vm, addr);
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
        finish_call(RUN.with(|r| r.current), ERR_FAULT);
        on_fault(kind, addr)
    }

    /// Packages the test programs as a PE file in `file`: `.text` (the
    /// code), `.rdata` (`MAGIC`), `.data` (a pointer to `.rdata`) and
    /// `.reloc` (fix-ups for that pointer and the code's one absolute
    /// literal), preferring `testprog::PREFERRED_BASE`.
    fn build_image(file: &mut [u8], p: &testprog::Programs) -> Result<(), rp_kernel_pe::Error> {
        let mut code = [0u8; 512];
        let code = code
            .get_mut(..p.code.len() * 4)
            .ok_or(rp_kernel_pe::Error::Truncated)?;
        for (b, w) in code.as_chunks_mut::<4>().0.iter_mut().zip(p.code) {
            *b = w.to_le_bytes();
        }
        let rdata = testprog::MAGIC.to_le_bytes();
        let data = (testprog::PREFERRED_BASE + RDATA_RVA).to_le_bytes();
        let mut rel = [0u8; 32];
        let a = build::reloc_block(&mut rel, TEXT_RVA, &[p.reloc as u16])?;
        let b = build::reloc_block(&mut rel[a..], DATA_RVA, &[0])?;
        let rel = &rel[..a + b];
        let header = Header {
            machine: rp_kernel_pe::MACHINE_ARM,
            image_base: testprog::PREFERRED_BASE,
            entry: TEXT_RVA + p.client,
            dll: false,
            relocs: Some(Dir {
                rva: RELOC_RVA,
                size: rel.len() as u32,
            }),
        };
        let sections = [
            SectionSpec {
                name: b".text",
                rva: TEXT_RVA,
                vsize: code.len() as u32,
                data: code,
                characteristics: scn::CODE | scn::EXECUTE | scn::READ,
            },
            SectionSpec {
                name: b".rdata",
                rva: RDATA_RVA,
                vsize: 4,
                data: &rdata,
                characteristics: scn::READ,
            },
            SectionSpec {
                name: b".data",
                rva: DATA_RVA,
                vsize: 4,
                data: &data,
                characteristics: scn::READ | scn::WRITE,
            },
            SectionSpec {
                name: b".reloc",
                rva: RELOC_RVA,
                vsize: rel.len() as u32,
                data: rel,
                characteristics: scn::READ,
            },
        ];
        build::write(file, &header, &sections).map(|_| ())
    }

    /// The page cache's fill for the test image: if `addr` lies in it and
    /// its view page isn't loaded, loads it into a new frame (filled from
    /// the file and relocated, then written back from the cache so user
    /// mode reads and fetches it).
    fn load_pe_page(vm: &mut KernelVm<'_>, addr: u32) {
        let Some(page) = addr
            .checked_sub(LOAD_BASE)
            .map(|o| o / 4096)
            .filter(|&p| (p as usize) < PE_PAGES)
        else {
            return;
        };
        if vm.cache.pe[page as usize] != 0 || vm.cache.pe_file == 0 {
            return;
        }
        let Some(f) = FrameAlloc::alloc(vm.frames, 0) else {
            return;
        };
        let phys = BASE + f as u32 * 4096;
        if load_view_page(vm.mem.pool, vm.cache.pe_file, phys, page).is_some() {
            cpu::clean_invalidate_range(phys, 4096);
            vm.cache.pe[page as usize] = phys;
            vm.cache.pe_loads += 1;
        } else {
            FrameAlloc::free(vm.frames, f);
        }
    }

    /// Loads view page `page` of the image in the file frame `file_phys`
    /// into the frame `out_phys`. `None` if the image or page is bad.
    fn load_view_page(pool: &mut [u32], file_phys: u32, out_phys: u32, page: u32) -> Option<()> {
        let (fi, oi) = (word(file_phys), word(out_phys));
        let (file, out) = if fi < oi {
            let (lo, hi) = pool.split_at_mut(oi);
            (&mut lo[fi..fi + 1024], &mut hi[..1024])
        } else {
            let (lo, hi) = pool.split_at_mut(fi);
            (&mut hi[..1024], &mut lo[oi..oi + 1024])
        };
        let file: &[u8] = rp_hal_mem::words_as_bytes_mut(file);
        let out: &mut [u8; 4096] = rp_hal_mem::words_as_bytes_mut(out).try_into().ok()?;
        let read = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
            let src = file.get(off as usize..off as usize + buf.len()).ok_or(())?;
            buf.copy_from_slice(src);
            Ok(())
        };
        let image = Image::parse(file).ok()?;
        let d = image.dirs[dir::BASERELOC];
        let mut rel = [0u8; 64];
        let rel = rel.get_mut(..d.size as usize)?;
        image.read_view(d.rva, rel, read).ok()?;
        let relocs = Relocations::new(rel).ok()?;
        let delta = LOAD_BASE.wrapping_sub(image.image_base);
        image.load_page(page, out, &relocs, delta, read).ok()
    }

    /// A client's report: kept for the boot thread, then the thread ends.
    fn done(me: Tid, f: &mut Frame) -> ! {
        let done = RUN.with(|r| {
            let n = r.finished.min(CLIENTS - 1);
            r.results[n].copy_from_slice(&f.r[..7]);
            r.finished += 1;
            r.done
        });
        let _ = SCHED.with(|s| {
            let _ = s.set_event(done);
            s.exit(me)
        });
        reschedule();
        unreachable!("an exited thread never runs again")
    }

    /// Checks every client's report, the server's own word, and that
    /// threads were switched while inside calls.
    fn report_clients(uart: &mut PxaUart<Mmio>, pe_pages: u32) -> bool {
        let ticks = rt::ticks().wrapping_sub(IPC.with(|i| i.first_call));
        switch_to(SERVER_SLOT);
        let server_word = rt::try_read((layout::slot_base(SERVER_SLOT) + testprog::DATA) as usize);
        let (results, switches) = RUN.with(|r| (r.results, r.in_call_switches));
        let calls = IPC.with(|i| i.calls);
        let all = |f: &dyn Fn(&[u32; 7]) -> bool| results.iter().all(f);
        let first = results[0];
        let mut ok = report_ipc(uart, "call", all(&|r| r[0] == 42), first[0]);
        ok &= report_ipc(uart, "server fault", all(&|r| r[1] == ERR_FAULT), first[1]);
        let bad = ERR | IpcError::BadHandle as u32;
        ok &= report_ipc(uart, "foreign handle", all(&|r| r[2] == bad), first[2]);
        ok &= report_ipc(uart, "nested", all(&|r| r[4] == 45), first[4]);
        let deep = ERR | IpcError::TooDeep as u32;
        ok &= report_ipc(uart, "too deep", all(&|r| r[5] == deep), first[5]);
        let isolated = all(&|r| r[3] == 0x1111_1111) && server_word == Some(0x2222_2222);
        ok &= report_ipc(uart, "isolation", isolated, first[3]);
        ok &= report_ipc(uart, "preempted in calls", switches > 0, switches);
        // The image: relocated (the client reached MAGIC through two fixed-up
        // addresses), and loaded on demand, each page once though both
        // processes map it: code, `.rdata` and `.data`, never the headers
        // or `.reloc`.
        let relocated = all(&|r| r[6] == testprog::MAGIC);
        ok &= report_ipc(uart, "PE image relocated", relocated, first[6]);
        let loads = STATE.with(|s| s.file.pe_loads);
        let demand = loads == 3 && pe_pages == 5;
        ok &= report_ipc(uart, "PE pages loaded on demand", demand, loads);
        let _ = writeln!(uart, "BENCH ipc_call_roundtrip {ticks} {calls}");
        ok
    }

    fn report_ipc(uart: &mut PxaUart<Mmio>, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str("IPC ");
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(" OK\n");
        } else {
            let _ = writeln!(uart, " FAIL (got {got:#010x})");
        }
        pass
    }

    fn report(uart: &mut PxaUart<Mmio>, what: &str, pass: bool, got: u32) -> bool {
        let _ = uart.write_str("VM ");
        let _ = uart.write_str(what);
        if pass {
            let _ = uart.write_str(" OK\n");
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
