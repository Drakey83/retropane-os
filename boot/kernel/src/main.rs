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
//! 8. Supervision (ADR-0016 step 3): a host in slot 7 (the server's code)
//!    supervised by a service manager in slot 8. A client gets the host's
//!    handle from the manager (`SYS_GRANT`; servers see their caller's
//!    slot), makes the host fault, and the kernel restarts it once no thread
//!    is inside: fresh memory, a new port generation (the old handle is
//!    `Stale`), the manager's handle renewed; the client asks again and the
//!    new instance answers.
//! 9. Processes at run time: the manager starts a worker (`SYS_SPAWN`; the
//!    manager only), which checks its memory starts zeroed and a handle it
//!    was never given is refused, then exits (`SYS_EXIT`); everything it had
//!    comes back (frames, handles, ports), and a second worker reuses its
//!    slot and record without seeing the first one's memory.
//! 10. A real driver in a host (item 46, ADR-0016 decision 5): the
//!     display+input host, a compiled Rust process (`services/display-host`,
//!     its own PE file embedded at build time), runs the backlight driver on
//!     PWM0's registers, mapped into it uncached by the board's grant, and
//!     waits for the power button's interrupt, granted to it alone. A client
//!     in another process sets a level through its port and makes it fault
//!     in a call; a test thread makes its own thread fault. Each time the
//!     host restarts with fresh memory (its blocked thread re-enters at its
//!     entry) and carries on from the level the hardware shows. Then the
//!     power button (pressed by the emulator) turns the light off and on.
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
    use rp_hal_pxa27x::rt::IrqLine;
    use rp_hal_pxa27x::rt::{self, Global};
    use rp_hal_pxa27x::soc::{self, Edge};
    use rp_hal_pxa27x::testprog::{
        self, SYS_CALL, SYS_DONE, SYS_EXIT, SYS_GRANT, SYS_RETURN, SYS_SPAWN,
    };
    use rp_hal_pxa27x::user::{self, Frame};
    use rp_hal_pxa27x::{Mmio, cpu, devmap, diag, thread};
    use rp_hal_user::{SYS_IRQ_WAIT, SYS_REPORT};
    use rp_hal_x51v::devices::take_trace_area;
    use rp_hal_x51v::devices::{RAM_POOL_BASE, RAM_POOL_SIZE, take_ram_pool};
    use rp_kernel_ipc::{Error as IpcError, Handle, HandlePages, Ipc, PortId, Rights};
    use rp_kernel_mm::arm::{self, Ap, Attrs, Cache};
    use rp_kernel_mm::domains::{self, Domains};
    use rp_kernel_mm::frames::Frames;
    use rp_kernel_mm::layout;
    use rp_kernel_mm::pagetable::{FrameAlloc, Mmu, PageTables, PhysMem};
    use rp_kernel_mm::regions::RegionPages;
    use rp_kernel_mm::space::{
        AddressSpace, Backing, IoError, Outcome, PageCache, Prot, Region, Vm, access_for,
    };
    use rp_kernel_mm::zpool::{ZBuffers, ZPool};
    use rp_kernel_pe::build::{self, Header, SectionSpec};
    use rp_kernel_pe::{Dir, Image, Protection, Relocations, dir, scn};
    use rp_kernel_sched::{Eid, LOWEST, Scheduler, Tid};
    use rp_kernel_trace::{Kind, Persistent};

    rp_image::entry!(main);

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
    /// The pinned-memory test's pages.
    const PIN_VA: u32 = 0x0030_0000;
    const PIN_PAGES: u32 = 4;
    /// Many regions (ADR-0019): one page each, a page apart, from here.
    const MANY_VA: u32 = 0x0040_0000;
    const MANY_REGIONS: u32 = 300;

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
        /// The display host's view pages (its image is in the kernel's own
        /// read-only data: nothing to copy).
        host: [u32; PE_PAGES],
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
            } else if object == HOST_OBJECT
                && let Some(&phys) = self.host.get(page as usize).filter(|&&p| p != 0)
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
        fn holds(&self, phys: u32) -> bool {
            phys != 0 && (phys == self.phys || self.pe.contains(&phys) || self.host.contains(&phys))
        }
    }

    /// The pool, seen as physical memory. Under this image's map, physical
    /// RAM is identity-mapped, so the pool's addresses are its physical ones.
    struct Mem<'a> {
        pool: &'a mut [u32],
        l1: u32,
        /// The first-level table in on-chip SRAM, once moved there.
        l1_sram: Option<&'a mut [u32; 4096]>,
    }

    fn word(phys: u32) -> usize {
        ((phys - BASE) / 4) as usize
    }

    impl PhysMem for Mem<'_> {
        fn l1(&mut self) -> &mut [u32; 4096] {
            if let Some(t) = self.l1_sram.as_deref_mut() {
                return t;
            }
            let i = word(self.l1);
            (&mut self.pool[i..i + 4096])
                .try_into()
                .unwrap_or_else(|_| unreachable!())
        }
        fn l1_phys(&self) -> u32 {
            self.l1
        }
        fn sync(&mut self, phys: u32, len: u32) {
            // RAM is identity mapped; the SRAM has its own window.
            let sram = diag::SRAM_BASE..diag::SRAM_BASE + diag::SRAM_SIZE as u32;
            let at = if sram.contains(&phys) {
                devmap::va(phys as usize) as u32
            } else {
                phys
            };
            cpu::clean_range(at, len);
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
    const KSTACKS: usize = 1 + CLIENTS + SPAWN_STACKS + DISPLAY_THREADS;
    /// The display host's thread, its client, the fault-test thread.
    const DISPLAY_THREADS: usize = 3;
    /// A short quantum (~150 us) so the clients are preempted many times,
    /// inside calls too.
    const QUANTUM: u32 = 500;
    /// What a refused or failed call returns: an error code with the top
    /// bits set (no sum of two small numbers looks like it).
    const ERR: u32 = 0xFFFF_FF00;
    const ERR_FAULT: u32 = ERR | 0xFE;
    /// A call to a server being restarted (fails at once).
    const ERR_RESTARTING: u32 = ERR | 0xFD;
    /// A call that was inside a server when it faulted: whatever the broken
    /// instance would have answered is dropped.
    const ERR_RESTARTED: u32 = ERR | 0xFC;
    /// The supervised host (the server's code) and its supervisor: the OS's
    /// own processes, so in native slots (above the kernel, `layout`),
    /// leaving CE's 31 to Windows Mobile programs.
    const HOST_SLOT: u32 = layout::FIRST_NATIVE_SLOT + 1;
    const MANAGER_SLOT: u32 = layout::FIRST_NATIVE_SLOT;
    /// Process records: reused as processes start and end (each ~1.6 KB),
    /// so memory follows the processes running, not the 31 slots.
    const MAX_PROCS: usize = 7;
    /// Kernel stacks kept for processes started at run time.
    const SPAWN_STACKS: usize = 2;
    /// A system call only the service manager may make.
    const ERR_DENIED: u32 = ERR | 0xFB;
    /// The display+input host (item 46), a native process, and the test's
    /// client of it, a CE-slot process: both run the host's image, a
    /// compiled program packaged by `cargo xtask` and embedded here.
    const DISPLAY_SLOT: u32 = layout::FIRST_NATIVE_SLOT + 2;
    const DISPLAY_CLIENT_SLOT: u32 = 10;
    static DISPLAY_IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rp-display-host.pe"));
    const HOST_OBJECT: u32 = 3;
    /// The board's grants to the display host (ADR-0016 decision 5): PWM0's
    /// registers, one uncached page at `rp_hal_user::Device::Pwm0`'s
    /// address, and the power button's interrupt (GPIO0) as its line 0.
    const PWM0_PHYS: u32 = 0x40B0_0000;
    const POWER_BUTTON_GPIO: u8 = 0;
    /// The host's thread roles (`services/display-host`).
    const ROLE_HOST: u32 = 0;
    const ROLE_TEST_CLIENT: u32 = 1;
    const ROLE_TEST_FAULT: u32 = 2;
    /// The level the display client sets.
    const DISPLAY_LEVEL: u32 = 30;
    // The IPC tables hold one handle table per process slot of the layout,
    // CE's and the native ones; every per-thread table covers every thread
    // the scheduler can number.
    const _: () = assert!(
        rp_kernel_ipc::FIRST_SLOT as u32 == layout::FIRST_PROCESS_SLOT
            && rp_kernel_ipc::LAST_SLOT as u32 == layout::LAST_PROCESS_SLOT
            && rp_kernel_ipc::FIRST_NATIVE_SLOT as u32 == layout::FIRST_NATIVE_SLOT
            && rp_kernel_ipc::LAST_NATIVE_SLOT as u32 == layout::LAST_NATIVE_SLOT
            && rp_kernel_ipc::MAX_SLOTS == layout::PROCESS_SLOTS
            && rp_kernel_ipc::MAX_THREADS == rp_kernel_sched::MAX_THREADS
            && thread::MAX_THREADS == rp_kernel_sched::MAX_THREADS
    );

    /// A process slot's index in per-slot tables (`layout::slot_index`);
    /// anything else maps to the spare last entry, never out of bounds.
    fn slot_ix(slot: u8) -> usize {
        layout::slot_index(u32::from(slot)).unwrap_or(layout::PROCESS_SLOTS)
    }

    /// Handle pages for the IPC tables: kernel pages from the same frame
    /// allocator as region lists (ADR-0019's pattern).
    struct IpcPages<'a, P>(&'a mut P);

    impl<P: RegionPages> HandlePages for IpcPages<'_, P> {
        fn alloc_page(&mut self) -> Option<u32> {
            self.0.alloc_page()
        }
        fn free_page(&mut self, page: u32) {
            self.0.free_page(page);
        }
        fn page(&mut self, page: u32) -> &mut [u8; 4096] {
            self.0.page(page)
        }
    }

    /// Runs `f` on the IPC tables with their pages at hand.
    fn with_ipc<R>(f: impl FnOnce(&mut IpcState, &mut dyn HandlePages) -> R) -> R {
        let mut f = Some(f);
        let mut r = None;
        ipc_and_pages(&mut |i, hp| r = f.take().map(|f| f(i, hp)));
        r.unwrap_or_else(|| unreachable!("ipc_and_pages runs its closure once"))
    }

    /// [`with_ipc`]'s one copy of the VM and IPC locking: each caller's
    /// closure is called through it, not given its own copy (one per call
    /// site was ~650 bytes each).
    #[inline(never)]
    fn ipc_and_pages(f: &mut dyn FnMut(&mut IpcState, &mut dyn HandlePages)) {
        with_vm(|vm, _| {
            let mut rp = vm.region_pages();
            IPC.with(|i| f(i, &mut IpcPages(&mut rp)));
        });
    }

    /// A handle number from a system call: anything past 16 bits is no
    /// handle (refused, never truncated onto a real one).
    fn handle_arg(v: u32) -> Handle {
        Handle(u16::try_from(v).unwrap_or(u16::MAX))
    }

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

    /// The trace ring (docs/design-brief.md): the newest kernel events,
    /// always on, in the trace region (item 54) so it outlives the image:
    /// carried on from the loader if it started this run.
    static TRACE: Global<Option<Persistent<'static>>> = Global::new(None);

    fn trace(kind: Kind, a: u32, b: u32) {
        TRACE.with(|t| {
            if let Some(t) = t {
                t.record(rt::ticks(), kind, a, b);
            }
        });
    }

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
        /// Threads inside each process slot as a server, now (by
        /// [`slot_ix`]: CE's and the native slots).
        inside: [u8; layout::PROCESS_SLOTS + 1],
        /// Bit `slot`: that server faulted and restarts when it empties.
        restarting: u64,
        /// Supervised ports: (port, server slot, supervisor slot).
        supervised: [Option<(PortId, u8, u8)>; 2],
        restarts: u32,
        /// The restarted host's memory came back fresh.
        restart_clean: bool,
        /// Roles handed to client threads as they start.
        next_role: u32,
        /// The image's code length (spawn entries must lie inside it).
        code_len: u32,
        /// Workers' exit codes, in order.
        exits: [u32; SPAWN_STACKS],
        exited: usize,
        /// Workers started, and the free frames before each.
        spawned: usize,
        frames_before: [u32; SPAWN_STACKS],
        /// The service manager's port (a started process's one handle).
        manager_port: Option<PortId>,
        /// After each exit: frames free, and the slot's handles and ports
        /// left (must be none).
        after_exit: [(u32, usize, usize); SPAWN_STACKS],
        /// The display host's entry and its own thread.
        display_entry: u32,
        display_thread: Option<Tid>,
        /// Threads that must re-enter their process at its entry (their
        /// process restarted under them), by thread index.
        reentry: u64,
        /// Times the display host's thread started (first start and each
        /// re-entry), and the display host's restarts.
        display_starts: u32,
        display_restarts: u32,
        /// The display client's report, and its and the fault-test
        /// thread's kernel stacks.
        display_report: Option<[u32; 4]>,
        fault_stack: Option<&'static mut [u32]>,
        client_stack: Option<&'static mut [u32]>,
        /// The power button's event.
        button: Eid,
        /// A display host restart didn't come back fresh (false: all
        /// zero, so this record stays in zeroed memory, not the image).
        display_dirty: bool,
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
        inside: [0; layout::PROCESS_SLOTS + 1],
        restarting: 0,
        supervised: [None, None],
        restarts: 0,
        restart_clean: false,
        next_role: 0,
        code_len: 0,
        exits: [0; SPAWN_STACKS],
        exited: 0,
        spawned: 0,
        frames_before: [0; SPAWN_STACKS],
        manager_port: None,
        after_exit: [(0, 0, 0); SPAWN_STACKS],
        display_entry: 0,
        display_thread: None,
        reentry: 0,
        display_starts: 0,
        display_restarts: 0,
        display_report: None,
        fault_stack: None,
        client_stack: None,
        button: Eid(0),
        display_dirty: false,
    });

    /// The power button's interrupt line (granted to the display host).
    static BUTTON: Global<Option<IrqLine>> = Global::new(None);

    /// Kernel stacks for processes started at run time.
    static SPARE_STACKS: Global<[Option<&'static mut [u32]>; SPAWN_STACKS]> =
        Global::new([None, None]);

    /// Everything else the VM owns.
    /// A process record: its slot (0: free) and address space.
    struct Proc {
        slot: u32,
        space: AddressSpace,
    }

    struct State {
        /// Record 0 is always the clients' process (slot 5).
        procs: [Proc; MAX_PROCS],
        /// The test image's view pages' protections (`None`: a guard).
        pe_prot: [Option<Protection>; PE_PAGES],
        /// The display host image's.
        host_prot: [Option<Protection>; PE_PAGES],
        domains: Domains,
        file: RamFile,
        pool: Option<&'static mut [u32]>,
        l1: u32,
        /// The first-level table once it lives in on-chip SRAM (ADR-0006
        /// rule 1); `l1` is then its physical address there.
        l1_sram: Option<&'static mut [u32; 4096]>,
        pid: u32,
        faults: u32,
        last: Option<Outcome>,
        error: Option<&'static str>,
    }

    const FREE_PROC: Proc = Proc {
        slot: 0,
        space: AddressSpace::process(PROC_SLOT, domains::PARKED),
    };

    static STATE: Global<State> = Global::new(State {
        domains: Domains::new(),
        // Real domains come from `Domains::activate` before first use.
        procs: [
            Proc {
                slot: PROC_SLOT,
                space: AddressSpace::process(PROC_SLOT, domains::PARKED),
            },
            Proc {
                slot: SERVER_SLOT,
                space: AddressSpace::process(SERVER_SLOT, domains::PARKED),
            },
            Proc {
                slot: HOST_SLOT,
                space: AddressSpace::process(HOST_SLOT, domains::PARKED),
            },
            Proc {
                slot: MANAGER_SLOT,
                space: AddressSpace::process(MANAGER_SLOT, domains::PARKED),
            },
            Proc {
                slot: DISPLAY_SLOT,
                space: AddressSpace::process(DISPLAY_SLOT, domains::PARKED),
            },
            Proc {
                slot: DISPLAY_CLIENT_SLOT,
                space: AddressSpace::process(DISPLAY_CLIENT_SLOT, domains::PARKED),
            },
            FREE_PROC,
        ],
        pe_prot: [None; PE_PAGES],
        host_prot: [None; PE_PAGES],
        file: RamFile {
            phys: 0,
            refs: 0,
            pe_file: 0,
            pe: [0; PE_PAGES],
            pe_refs: 0,
            pe_loads: 0,
            host: [0; PE_PAGES],
        },
        pool: None,
        l1: 0,
        l1_sram: None,
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
                            let mut l1_sram = s.l1_sram.take();
                            let r = {
                                let mut mem = Mem {
                                    pool: &mut pool,
                                    l1: s.l1,
                                    l1_sram: l1_sram.as_deref_mut(),
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
                            s.l1_sram = l1_sram;
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
            let r = space_of(s, s.pid)
                .map_or(Err(rp_kernel_mm::space::FaultError::NoRegion), |sp| {
                    vm.handle_fault(sp, mva, access)
                });
            match r {
                Ok(o) => {
                    s.faults += 1;
                    s.last = Some(o);
                    trace(Kind::Fault, o as u32, a.addr);
                    true
                }
                Err(e) => {
                    s.error = Some(e.name());
                    false
                }
            }
        })
    }

    /// The kernel's line on every crash (rp-image's crash path): why the
    /// last page fault it handled was refused, if one was. Words via
    /// `write_str`: no `&str` Display (and its padding code) in the image
    /// (ADR-0005 rule 8).
    fn crash_note(w: &mut dyn Write) -> core::fmt::Result {
        match STATE.with(|s| s.error) {
            Some(why) => {
                w.write_str("page fault refused: ")?;
                w.write_str(why)
            }
            None => w.write_str("no page fault refused"),
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
        rp_image::crash_note(crash_note);
        rt::init_mmu();
        let _ = writeln!(early, "\nRetroPane 26 kernel: VM bring-up");
        TRACE.with(|t| *t = take_trace_area().and_then(Persistent::resume));
        let mut ok = true;

        // 1-2. Frames, then the kernel's own page tables.
        let Some(pool) = take_ram_pool() else {
            rp_image::fail("no RAM pool")
        };
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
                l1_sram: None,
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
        let mut free_after_setup = match built {
            Ok(n) => n,
            Err(e) => {
                let _ = early.write_str("VM tables FAIL ");
                let _ = early.write_str(e);
                rp_image::fail("VM tables")
            }
        };
        let l1 = STATE.with(|s| s.l1);
        cpu::set_dacr(arm::dacr(1 << KERNEL_DOMAIN));
        cpu::switch_table(l1);
        devmap::use_kernel_map();
        let Some(bus) = rt::take_ffuart() else {
            rp_image::fail("no UART")
        };
        let mut uart = PxaUart::new(bus);
        let _ = writeln!(
            uart,
            "VM tables OK (L1 at {l1:#010x}, {free_after_setup} frames free)"
        );
        // ADR-0006 rule 1: the first-level table into on-chip SRAM, so TLB
        // refills read on-chip memory (5.2x cheaper misses measured on the
        // device, docs/perf.md). Built in SDRAM above (the SRAM is reached
        // through its window only under the kernel map), moved now.
        if let Some(words) = diag::take_sram_words() {
            let (table, _rest) = words.split_at_mut(4096);
            if let Ok(table) = <&mut [u32; 4096]>::try_from(table) {
                diag::move_l1_table_at(table, diag::SRAM_BASE);
                let old = STATE.with(|s| {
                    let old = s.l1;
                    s.l1 = diag::SRAM_BASE;
                    s.l1_sram = Some(table);
                    old
                });
                // The SDRAM copy (an order-2 block, 16 KiB) goes back.
                FRAMES.with(|f| FrameAlloc::free(f, ((old - BASE) / 4096) as usize));
                // Setup now ends here: the checks below count from this.
                free_after_setup = FRAMES.with(|f| f.free_frames() as u32);
                let _ = writeln!(
                    uart,
                    "L1 table moved to on-chip SRAM at {:#010x}",
                    diag::SRAM_BASE
                );
            }
        }
        // Boot time (docs/design-brief.md): entry to the kernel's own page
        // tables and device map live.
        let _ = writeln!(uart, "BENCH boot_kernel_tables {} 1", rt::since_start());
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
        let mapped = with_vm(|vm, s| -> Result<(), &'static str> {
            let f = file_frame.ok_or("no file frame")?;
            let phys = BASE + f as u32 * 4096;
            for (k, w) in vm.mem.pool[word(phys)..word(phys) + 1024]
                .iter_mut()
                .enumerate()
            {
                *w = FILE_PATTERN + k as u32;
            }
            cpu::clean_invalidate_range(phys, 4096);
            // The VM's copy of the file: written back when `with_vm` ends.
            vm.cache.phys = phys;
            let slot = layout::slot_base(PROC_SLOT);
            vm.map(
                &mut s.procs[0].space,
                Region {
                    start: slot + ZERO_VA,
                    len: ZERO_PAGES * 4096,
                    prot: Prot::RW,
                    backing: Backing::Zero,
                },
            )
            .map_err(|e| e.name())?;
            vm.map(
                &mut s.procs[0].space,
                Region {
                    start: slot + FILE_VA,
                    len: 4096,
                    prot: Prot::RW,
                    backing: Backing::File {
                        object: FILE_OBJECT,
                        offset: 0,
                        shared: false,
                    },
                },
            )
            .map_err(|e| e.name())?;
            s.pid = PROC_SLOT;
            Ok(())
        });
        if let Err(e) = mapped {
            let _ = uart.write_str("VM process FAIL ");
            let _ = uart.write_str(e);
            rp_image::fail("VM process")
        }
        // Give slot 5 its ARM domain and run it: PID and DACR, nothing else.
        let act5 = STATE.with(|s| {
            let a = s.domains.activate(PROC_SLOT);
            s.procs[0].space.set_domain(a.domain);
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
            vm.prepare_user_write(&s.procs[0].space, layout::mva(FILE_VA + 8, PROC_SLOT), 4)
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
                if vm.compress(
                    &s.procs[0].space,
                    layout::mva(ZERO_VA + p * 4096, PROC_SLOT),
                ) == Ok(true)
                {
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
        let mapped = with_vm(|vm, s| {
            vm.map(
                &mut s.procs[0].space,
                Region {
                    start: region,
                    len: RECLAIM_PAGES * 4096,
                    prot: Prot::RW,
                    backing: Backing::Zero,
                },
            )
            .is_ok()
        });
        for p in 0..RECLAIM_PAGES {
            cpu::poke((RECLAIM_VA + p * 4096 + 12) as usize, 0xA6E0_0000 + p);
        }
        let sweep = || {
            with_vm(|vm, s| {
                let mut hand = region;
                vm.reclaim(&s.procs[0].space, &mut hand, RECLAIM_PAGES, u32::MAX)
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

        // Pinned memory (docs/design-brief.md): pages loaded up front and
        // never aged, compressed or evicted, so touching them never faults:
        // the input path's guarantee. Full sweeps of the whole process
        // leave them where they are.
        let pin = layout::slot_base(PROC_SLOT) + PIN_VA;
        let populated = with_vm(|vm, s| {
            vm.map(
                &mut s.procs[0].space,
                Region {
                    start: pin,
                    len: PIN_PAGES * 4096,
                    prot: Prot::RW.pinned(),
                    backing: Backing::Zero,
                },
            )
            .ok()?;
            vm.populate(&s.procs[0].space, pin, PIN_PAGES * 4096).ok()
        });
        let faults0 = state().0;
        for p in 0..PIN_PAGES {
            cpu::poke((PIN_VA + p * 4096) as usize, 0x919E_0000 + p);
        }
        for _ in 0..3 {
            let _ = with_vm(|vm, s| {
                let mut hand = layout::slot_base(PROC_SLOT);
                vm.reclaim(&s.procs[0].space, &mut hand, 4096, u32::MAX)
            });
        }
        let mut intact = true;
        for p in 0..PIN_PAGES {
            intact &= rt::try_read((PIN_VA + p * 4096) as usize) == Some(0x919E_0000 + p);
        }
        let faults = state().0 - faults0;
        let pass = populated == Some(PIN_PAGES) && intact && faults == 0;
        ok &= report(&mut uart, "pinned", pass, faults);

        // Many regions (ADR-0019): far more than the old fixed table's 64,
        // in kernel pages that follow use. 300 one-page reservations with
        // gaps, the last one touched, then all unmapped: every frame back,
        // list pages included.
        let many_base = layout::slot_base(PROC_SLOT) + MANY_VA;
        let free_before = FRAMES.with(|f| f.free_frames() as u32);
        let many = with_vm(|vm, s| {
            let space = &mut s.procs[0].space;
            let before = space.region_count();
            for k in 0..MANY_REGIONS {
                let region = Region {
                    start: many_base + k * 0x2000,
                    len: 4096,
                    prot: Prot::RW,
                    backing: Backing::Zero,
                };
                vm.map(space, region).ok()?;
            }
            let count = space.region_count();
            let pages_ok = space.region_pages() == count.div_ceil(rp_kernel_mm::regions::PER_PAGE);
            Some(count - before == MANY_REGIONS as usize && pages_ok)
        });
        let last = MANY_VA + (MANY_REGIONS - 1) * 0x2000;
        cpu::poke(last as usize, 0x3A11_0000);
        let back = rt::try_read(last as usize);
        let unmapped = with_vm(|vm, s| {
            vm.unmap(&mut s.procs[0].space, many_base, MANY_REGIONS * 0x2000)
                .is_ok()
        });
        let free_after = FRAMES.with(|f| f.free_frames() as u32);
        let pass = many == Some(true)
            && back == Some(0x3A11_0000)
            && unmapped
            && free_after == free_before;
        ok &= report(
            &mut uart,
            "many regions",
            pass,
            free_before.wrapping_sub(free_after),
        );

        // Unmap everything: every frame and page-cache reference returns.
        let slot = layout::slot_base(PROC_SLOT);
        let (unmapped, refs) = with_vm(|vm, s| {
            let r = vm.unmap(&mut s.procs[0].space, slot, layout::SLOT_SIZE);
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
        let mapped = with_vm(|vm, s| -> Result<u32, &'static str> {
            let f = file_frame.ok_or("no file frame")?;
            let file_phys = BASE + f as u32 * 4096;
            let pages = {
                let file = rp_hal_mem::words_as_bytes_mut(
                    &mut vm.mem.pool[word(file_phys)..word(file_phys) + 1024],
                );
                build_image(file, &programs).map_err(|e| e.name())?;
                let image = Image::parse(file).map_err(|e| e.name())?;
                // W^X: an image with a page both writable and executable is
                // refused, not mapped.
                if image.writable_code() || image.pages() as usize > PE_PAGES {
                    return Err("image refused");
                }
                for page in 0..image.pages() {
                    s.pe_prot[page as usize] = image.page_protection(page);
                }
                image.pages()
            };
            // The VM's copy of the file: written back when `with_vm` ends.
            vm.cache.pe_file = file_phys;
            // The display host's image: built by `cargo xtask`, refused as
            // any image would be (W^X, size).
            let host = Image::parse(DISPLAY_IMAGE).map_err(|_| "no display host image")?;
            if host.writable_code() || host.pages() as usize > PE_PAGES {
                return Err("display host image refused");
            }
            for page in 0..host.pages() {
                s.host_prot[page as usize] = host.page_protection(page);
            }
            RUN.with(|r| r.display_entry = LOAD_BASE + host.entry);
            for slot in [
                PROC_SLOT,
                SERVER_SLOT,
                HOST_SLOT,
                MANAGER_SLOT,
                DISPLAY_SLOT,
                DISPLAY_CLIENT_SLOT,
            ] {
                let a = s.domains.activate(slot);
                let (object, prot, grant) = image_of(s, slot);
                let space = space_mut(s, slot).ok_or("no process")?;
                space.set_domain(a.domain);
                map_process(&mut vm.region_pages(), space, slot, object, &prot, grant)?;
            }
            Ok(pages)
        });
        let pe_pages = *mapped.as_ref().unwrap_or(&0);
        let mapped = mapped.map(|_| ());
        // Ports: the server's (unsupervised); the host's, supervised by the
        // manager, which holds it with every right (its handle 0); the
        // manager's own, which clients hold (their handle 1).
        let client_handle = with_ipc(|i, hp| -> Option<Handle> {
            let server = LOAD_BASE + TEXT_RVA + programs.server;
            let (port, _) = i.ipc.open(hp, SERVER_SLOT as u8, server).ok()?;
            let h = i
                .ipc
                .install(hp, PROC_SLOT as u8, port, Rights::CALL)
                .ok()?;
            let (host, _) = i.ipc.open(hp, HOST_SLOT as u8, server).ok()?;
            i.ipc
                .install(hp, MANAGER_SLOT as u8, host, Rights::ALL)
                .ok()?;
            let manager = LOAD_BASE + TEXT_RVA + programs.manager;
            let (mport, _) = i.ipc.open(hp, MANAGER_SLOT as u8, manager).ok()?;
            i.ipc
                .install(hp, PROC_SLOT as u8, mport, Rights::CALL)
                .ok()?;
            // The display host's port: the manager's handle 2 (every right;
            // its handle 1 is its own port), the display client's handle 0;
            // the manager's port is the client's handle 1.
            let entry = RUN.with(|r| r.display_entry);
            let (display, _) = i.ipc.open(hp, DISPLAY_SLOT as u8, entry).ok()?;
            i.ipc
                .install(hp, MANAGER_SLOT as u8, display, Rights::ALL)
                .ok()?;
            i.ipc
                .install(hp, DISPLAY_CLIENT_SLOT as u8, display, Rights::CALL)
                .ok()?;
            i.ipc
                .install(hp, DISPLAY_CLIENT_SLOT as u8, mport, Rights::CALL)
                .ok()?;
            RUN.with(|r| {
                r.supervised = [
                    Some((host, HOST_SLOT as u8, MANAGER_SLOT as u8)),
                    Some((display, DISPLAY_SLOT as u8, MANAGER_SLOT as u8)),
                ];
                r.manager_port = Some(mport);
            });
            Some(h)
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
            // The display host's own thread (a driver's: urgent). Its
            // client starts once the clients above are done (`start_display_client`).
            let button = s.create_event(false, false).ok()?;
            let host = s.create(DISPLAY_SLOT as u8, 120).ok()?;
            RUN.with(|r| {
                r.slots[host.index()] = DISPLAY_SLOT as u8;
                r.display_thread = Some(host);
                r.button = button;
                r.fault_stack = kstacks.next();
                r.client_stack = kstacks.next();
            });
            thread::prepare(
                host.index(),
                kstacks.next()?,
                user_thread,
                ROLE_HOST as usize,
            );
            RUN.with(|r| r.current = boot);
            Some(done)
        });
        let (Ok(()), Some(h), Some(done)) = (mapped, client_handle, threads) else {
            let _ = uart.write_str("IPC setup FAIL: ");
            let _ = uart.write_str(mapped.err().unwrap_or("handles or threads"));
            let _ = uart.write_str("\n");
            rp_image::fail("IPC setup")
        };
        RUN.with(|r| {
            r.done = done;
            r.entry = LOAD_BASE + TEXT_RVA + programs.client;
            // The test programs take it in r0's low byte (an inline handle).
            r.handle = u8::try_from(h.0).unwrap_or(u8::MAX);
            r.code_len = programs.code.len() as u32 * 4;
        });
        SPARE_STACKS.with(|st| {
            for slot in st.iter_mut() {
                *slot = kstacks.next();
            }
        });
        // The display host's grants: PWM0's clock on (a user process can't
        // reach the clock unit) and the power button's interrupt, signalled
        // to its event.
        let _ = rt::take_pwm0();
        rt::set_signal_hook(on_signal);
        BUTTON.with(|b| *b = rt::take_gpio_irq(POWER_BUTTON_GPIO, Edge::Both));
        user::set_syscall_handler(on_syscall);
        user::set_user_fault_handler(on_user_fault);
        rt::set_preempt_hook(reschedule);
        let _ = writeln!(uart, "IPC {CLIENTS} clients starting (handle {})", h.0);
        while RUN.with(|r| r.finished) < CLIENTS {
            wait(done);
        }
        // The display test after the others, so their frame accounting
        // (spawn and exit) sees nothing else allocate.
        start_display_client();
        while RUN.with(|r| r.display_restarts) < 2 {
            wait(done);
        }
        ok &= report_clients(&mut uart, pe_pages);
        ok &= report_display(&mut uart);
        if ok {
            let _ = uart.write_str(super::MARKER);
            let _ = uart.write_str("\n");
            power_button(&mut uart);
        }
        halt()
    }

    /// Starts the display client (a thread of the host's image in its own
    /// process).
    fn start_display_client() {
        let Some(stack) = RUN.with(|r| r.client_stack.take()) else {
            return;
        };
        let _ = SCHED.with(|s| {
            let t = s.create(DISPLAY_CLIENT_SLOT as u8, 200)?;
            RUN.with(|r| r.slots[t.index()] = DISPLAY_CLIENT_SLOT as u8);
            thread::prepare(t.index(), stack, user_thread, ROLE_TEST_CLIENT as usize);
            Ok::<(), rp_kernel_sched::Error>(())
        });
    }

    /// A thread of the display host's image (its role in `r1`), in the
    /// slot it was created for.
    extern "C" fn user_thread(role: usize) -> ! {
        let (me, slot, entry) =
            RUN.with(|r| (r.current, r.slots[r.current.index()], r.display_entry));
        if role == ROLE_HOST as usize {
            RUN.with(|r| r.display_starts += 1);
        }
        switch_to(u32::from(slot));
        user::enter_user_with(entry, stack_top(me, 0), 0, role as u32)
    }

    /// Interrupts the kernel signals: the power button wakes its waiter.
    fn on_signal(line: u32) {
        if line == soc::gpio_line(POWER_BUTTON_GPIO) {
            let e = RUN.with(|r| r.button);
            let _ = SCHED.with(|s| s.set_event(e));
        }
    }

    /// PWM0's duty register, as the hardware has it (the host's work).
    fn pwm_duty() -> u32 {
        rt::try_read(devmap::va(PWM0_PHYS as usize + 4)).unwrap_or(0)
    }

    /// The power button's level now (GPIO0 in `GPLR0`).
    fn button_level() -> u32 {
        rt::try_read(devmap::va(0x40E0_0000)).map_or(0, |v| v & 1)
    }

    /// The display host's interrupt wait (`SYS_IRQ_WAIT`, its line 0 only):
    /// re-arms the line, blocks until it fires, answers the pin's level. A
    /// restart while it waits re-enters the thread at its entry instead.
    fn irq_wait(me: Tid, f: &mut Frame) {
        let from = STATE.with(|s| s.pid);
        if from != DISPLAY_SLOT || f.arg(0) != 0 || IPC.with(|i| i.ipc.depth(me.0)) > 0 {
            f.set_result(ERR_DENIED);
            return;
        }
        let e = RUN.with(|r| r.button);
        BUTTON.with(|b| b.as_mut().map(IrqLine::done));
        loop {
            if take_reentry(me) {
                reenter(f, me, ROLE_HOST);
                return;
            }
            if BUTTON.with(|b| b.as_mut().is_some_and(IrqLine::take)) {
                f.set_result(button_level());
                return;
            }
            wait(e);
        }
    }

    /// The display client's report (`SYS_REPORT`): kept, then its thread
    /// ends, and a thread that makes the host's own code fault starts.
    fn display_report(me: Tid, f: &mut Frame) -> ! {
        let from = STATE.with(|s| s.pid);
        let stack = RUN.with(|r| {
            if from == DISPLAY_CLIENT_SLOT && r.display_report.is_none() {
                r.display_report = Some([f.r[0], f.r[1], f.r[2], f.r[3]]);
                r.fault_stack.take()
            } else {
                None
            }
        });
        exit_thread(me, |s| {
            if let Some(stack) = stack
                && let Ok(t) = s.create(DISPLAY_SLOT as u8, 150)
            {
                RUN.with(|r| r.slots[t.index()] = DISPLAY_SLOT as u8);
                thread::prepare(t.index(), stack, user_thread, ROLE_TEST_FAULT as usize);
            }
        })
    }

    /// A client: enters the client program in its process, for good.
    extern "C" fn client_thread(_: usize) -> ! {
        let (me, entry, arg) = RUN.with(|r| {
            r.slots[r.current.index()] = PROC_SLOT as u8;
            let role = r.next_role;
            r.next_role += 1;
            (r.current, r.entry, u32::from(r.handle) | role << 8)
        });
        switch_to(PROC_SLOT);
        user::enter_user_at(entry, stack_top(me, 0), arg)
    }

    /// The address space of the process in `slot`; `None` if no process
    /// lives there (never another process's space by mistake).
    fn space_of(s: &State, slot: u32) -> Option<&AddressSpace> {
        s.procs
            .iter()
            .find(|p| p.slot == slot && slot != 0)
            .map(|p| &p.space)
    }

    fn space_mut(s: &mut State, slot: u32) -> Option<&mut AddressSpace> {
        s.procs
            .iter_mut()
            .find(|p| p.slot == slot && slot != 0)
            .map(|p| &mut p.space)
    }

    /// Maps a test process: the image (each view page as its section says;
    /// a page no section covers stays unmapped, a guard), its data page and
    /// its stacks.
    fn map_process(
        rp: &mut impl RegionPages,
        space: &mut AddressSpace,
        slot: u32,
        object: u32,
        pe: &[Option<Protection>; PE_PAGES],
        grant: bool,
    ) -> Result<(), &'static str> {
        let base = layout::slot_base(slot);
        for (page, q) in (0u32..).zip(pe) {
            let Some(q) = q else {
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
                .map(
                    rp,
                    Region {
                        start: base + LOAD_BASE + page * 4096,
                        len: 4096,
                        prot,
                        backing: Backing::File {
                            object,
                            offset: page * 4096,
                            shared: q.shared,
                        },
                    },
                )
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
            space.map(rp, region).map_err(|e| e.name())?;
        }
        if grant {
            // The board's grant: PWM0's registers, uncached, read + write,
            // never executable or cacheable (`protect` keeps the policy).
            space
                .map(
                    rp,
                    Region {
                        start: base + rp_hal_user::Device::Pwm0.va() as u32,
                        len: 4096,
                        prot: Prot::RW,
                        backing: Backing::Phys {
                            base: PWM0_PHYS,
                            cache: Cache::Device,
                        },
                    },
                )
                .map_err(|e| e.name())?;
        }
        Ok(())
    }

    /// Which image a slot maps, its pages' rights, and whether it gets the
    /// display host's grant.
    fn image_of(s: &State, slot: u32) -> (u32, [Option<Protection>; PE_PAGES], bool) {
        if slot == DISPLAY_SLOT || slot == DISPLAY_CLIENT_SLOT {
            (HOST_OBJECT, s.host_prot, slot == DISPLAY_SLOT)
        } else {
            (PE_OBJECT, s.pe_prot, false)
        }
    }

    /// Starts a process from the image at code offset `off`: a free slot
    /// and record, the image mapped (code shared, everything else fresh), a
    /// handle to the service manager only, and a first thread. Its slot, or
    /// an error code.
    fn spawn(off: u32) -> Result<u32, u32> {
        let code_len = RUN.with(|r| r.code_len);
        if !off.is_multiple_of(4) || off >= code_len {
            return Err(ERR | IpcError::Range as u32);
        }
        let Some(stack) = SPARE_STACKS.with(|st| st.iter_mut().find_map(Option::take)) else {
            return Err(ERR | IpcError::Full as u32);
        };
        // Free frames now: all must be back when this process has exited.
        RUN.with(|r| {
            if let Some(b) = r.frames_before.get_mut(r.spawned) {
                *b = FRAMES.with(|f| f.free_frames() as u32);
            }
            r.spawned += 1;
        });
        let slot = with_vm(|vm, s| -> Option<u32> {
            // Started by the service manager: the OS's own, so a native slot.
            let slot = (layout::FIRST_NATIVE_SLOT..=layout::LAST_NATIVE_SLOT)
                .find(|&n| s.procs.iter().all(|p| p.slot != n))?;
            let i = s.procs.iter().position(|p| p.slot == 0)?;
            let a = s.domains.activate(slot);
            let prot = s.pe_prot;
            let p = &mut s.procs[i];
            p.slot = slot;
            let mut rp = vm.region_pages();
            p.space.reset_process(&mut rp, slot, a.domain);
            map_process(&mut rp, &mut p.space, slot, PE_OBJECT, &prot, false).ok()?;
            Some(slot)
        });
        let Some(slot) = slot else {
            return Err(ERR | IpcError::Full as u32);
        };
        // Its one handle: the manager's port (CALL).
        let handle = with_ipc(|i, hp| {
            let manager = RUN.with(|r| r.manager_port)?;
            i.ipc.install(hp, slot as u8, manager, Rights::CALL).ok()
        });
        let t = SCHED.with(|s| s.create(slot as u8, 100).ok());
        let (Some(h), Some(t)) = (handle, t) else {
            return Err(ERR | IpcError::Full as u32);
        };
        RUN.with(|r| {
            r.slots[t.index()] = slot as u8;
        });
        // A new process's table is empty, so its first handle is 0: the
        // thread starts with it in r0.
        if h.0 != 0 {
            return Err(ERR | IpcError::Full as u32);
        }
        let entry = LOAD_BASE + TEXT_RVA + off;
        trace(Kind::Spawn, slot, entry);
        thread::prepare(t.index(), stack, process_thread, entry as usize);
        Ok(slot)
    }

    /// The first thread of a process started at run time, at `entry`.
    extern "C" fn process_thread(entry: usize) -> ! {
        let (me, slot) = RUN.with(|r| (r.current, r.slots[r.current.index()]));
        switch_to(u32::from(slot));
        user::enter_user_at(entry as u32, stack_top(me, 0), 0)
    }

    /// A process ends (its one thread called `SYS_EXIT`): its ports close,
    /// its handles go, its whole address space is unmapped (every frame and
    /// page-cache reference back), its domain and record are freed, and
    /// the thread ends.
    fn exit_process(me: Tid, code: u32) -> ! {
        let slot = RUN.with(|r| r.slots[me.index()]);
        trace(Kind::Exit, u32::from(slot), code);
        let _ = with_ipc(|i, hp| i.ipc.exit(hp, slot));
        with_vm(|vm, s| {
            let base = layout::slot_base(u32::from(slot));
            if let Some(p) = s.procs.iter_mut().find(|p| p.slot == u32::from(slot)) {
                let _ = vm.unmap(&mut p.space, base, layout::SLOT_SIZE);
                // Its region-list pages too (ADR-0019): nothing outlives it.
                p.space
                    .reset_process(&mut vm.region_pages(), u32::from(slot), domains::PARKED);
                p.slot = 0;
            }
            s.domains.release(u32::from(slot));
        });
        let left = with_ipc(|i, hp| (i.ipc.handle_count(hp, slot), i.ipc.ports_of(slot)));
        let free = FRAMES.with(|f| f.free_frames() as u32);
        RUN.with(|r| {
            r.slots[me.index()] = 0;
            if let Some(e) = r.exits.get_mut(r.exited) {
                *e = code;
                r.after_exit[r.exited] = (free, left.0, left.1);
            }
            r.exited += 1;
        });
        exit_thread(me, |_| {})
    }

    /// Restarts the supervised server in `slot` (no thread is inside): its
    /// whole address space is dropped and mapped afresh (code and constants
    /// come back from the shared page cache, everything writable zeroed),
    /// and its port's generation moves on, so every handle but the server's
    /// and its supervisor's is stale.
    fn restart_server(slot: u8) {
        trace(Kind::Restart, u32::from(slot), 0);
        let Some((port, _, supervisor)) = supervised(slot) else {
            return;
        };
        let remapped = with_vm(|vm, s| {
            let (object, prot, grant) = image_of(s, u32::from(slot));
            let base = layout::slot_base(u32::from(slot));
            space_mut(s, u32::from(slot)).is_some_and(|space| {
                vm.unmap(space, base, layout::SLOT_SIZE).is_ok()
                    && map_process(
                        &mut vm.region_pages(),
                        space,
                        u32::from(slot),
                        object,
                        &prot,
                        grant,
                    )
                    .is_ok()
            })
        });
        let renewed = with_ipc(|i, hp| i.ipc.restart(hp, port, Some(supervisor))).is_ok();
        // Its data page reads zero again (a fresh demand-zero page).
        let back = STATE.with(|s| s.pid);
        switch_to(u32::from(slot));
        let word = rt::try_read((layout::slot_base(u32::from(slot)) + testprog::DATA) as usize);
        switch_to(back);
        let clean = remapped && renewed && word == Some(0);
        let host_thread = RUN.with(|r| {
            r.restarting &= !(1 << slot);
            if u32::from(slot) == DISPLAY_SLOT {
                r.display_restarts += 1;
                r.display_dirty |= !clean;
                // Its own thread starts again at the entry, wherever it is
                // blocked: woken now, it re-enters instead of resuming.
                if let Some(t) = r.display_thread {
                    r.reentry |= 1 << t.0;
                }
                Some(r.button)
            } else {
                r.restarts += 1;
                r.restart_clean = clean;
                None
            }
        });
        if let Some(e) = host_thread {
            let done = RUN.with(|r| r.done);
            let _ = SCHED.with(|s| {
                let _ = s.set_event(done);
                s.set_event(e)
            });
        }
    }

    /// The supervision of the server in `slot`, if it has one.
    fn supervised(slot: u8) -> Option<(PortId, u8, u8)> {
        RUN.with(|r| r.supervised.iter().flatten().copied().find(|e| e.1 == slot))
    }

    /// Whether thread `t` must re-enter its process (and clears it).
    fn take_reentry(t: Tid) -> bool {
        RUN.with(|r| {
            let due = r.reentry & 1 << t.0 != 0;
            r.reentry &= !(1 << t.0);
            due
        })
    }

    /// The thread returns to user mode at its process's entry, as when it
    /// started: a fresh stack, every register cleared but its role.
    fn reenter(f: &mut Frame, t: Tid, role: u32) {
        let entry = RUN.with(|r| {
            r.display_starts += 1;
            r.display_entry
        });
        f.r = [0; 13];
        f.r[1] = role;
        f.sp = stack_top(t, 0);
        f.lr = 0;
        f.pc = entry;
    }

    /// Whether the server in `slot` is waiting to be restarted.
    fn restarting(slot: u8) -> bool {
        RUN.with(|r| r.restarting & 1 << slot != 0)
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

    /// The one way a thread ends: the IPC side forgets the calls it was in
    /// (`Ipc::thread_exit`), `last` runs on the scheduler (a wake, a
    /// successor), the scheduler removes it, and the next thread runs. The
    /// scheduler reuses thread numbers, so the two exits always pair here.
    fn exit_thread(me: Tid, last: impl FnOnce(&mut Scheduler)) -> ! {
        let _ = IPC.with(|i| i.ipc.thread_exit(me.0));
        let _ = SCHED.with(|s| {
            last(s);
            s.exit(me)
        });
        reschedule();
        unreachable!("an exited thread never runs again")
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
            trace(Kind::Switch, u32::from(cur.0), u32::from(next.0));
            thread::switch(cur.index(), next.index());
        }
    }

    /// Blocks the running thread until `e` is set.
    fn wait(e: Eid) {
        if SCHED.with(|s| s.wait(e, None)) == Ok(None) {
            reschedule();
        }
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
    /// (A quantum that ran out while in the kernel is paid on the way back
    /// to user mode: the HAL's owed preemption.)
    fn on_syscall(f: &mut Frame) {
        let me = RUN.with(|r| r.current);
        match f.number() {
            SYS_CALL => {
                let slot = STATE.with(|s| s.pid) as u8;
                let r = with_ipc(|i, hp| {
                    if i.calls == 0 {
                        i.first_call = rt::ticks();
                    }
                    i.calls += 1;
                    i.ipc.call(hp, me.0, slot, handle_arg(f.arg(0)))
                });
                match r {
                    Ok(_) if r.is_ok_and(|e| restarting(e.slot)) => {
                        let _ = IPC.with(|i| i.ipc.ret(me.0));
                        f.set_result(ERR_RESTARTING);
                    }
                    Ok(e) => {
                        // The server runs as a nested activation of this
                        // thread: the caller's registers and slot wait in
                        // this frame on the kernel stack. The server gets
                        // its arguments and nothing else, on its stack for
                        // this depth.
                        // The server is told who called: the kernel's word,
                        // not the caller's.
                        enter_slot(me, e.slot);
                        RUN.with(|r| r.inside[slot_ix(e.slot)] += 1);
                        trace(Kind::Call, u32::from(e.slot), e.depth as u32);
                        let mut result = user::call_user(
                            me.index(),
                            e.entry,
                            stack_top(me, e.depth),
                            [u32::from(slot), f.arg(1), f.arg(2), f.arg(3)],
                        );
                        enter_slot(me, slot);
                        trace(Kind::Return, u32::from(slot), result);
                        let left = RUN.with(|r| {
                            r.inside[slot_ix(e.slot)] -= 1;
                            r.inside[slot_ix(e.slot)]
                        });
                        if restarting(e.slot) {
                            if result != ERR_FAULT {
                                result = ERR_RESTARTED;
                            }
                            if left == 0 {
                                restart_server(e.slot);
                            }
                        }
                        f.set_result(result);
                    }
                    Err(e) => f.set_result(ERR | e as u32),
                }
            }
            SYS_RETURN => {
                finish_call(me, f.arg(0));
                f.set_result(ERR | IpcError::NotInCall as u32);
            }
            SYS_GRANT => {
                // A copy of a handle this process holds (with GRANT), into
                // another's table, with the same rights or fewer.
                let from = STATE.with(|s| s.pid) as u8;
                // A slot number past 8 bits names no slot (never truncated
                // onto a real one).
                let to = u8::try_from(f.arg(1)).unwrap_or(0);
                let rights = Rights::from_bits(f.arg(2));
                let live = to != 0 && STATE.with(|s| space_of(s, u32::from(to)).is_some());
                let r = if live {
                    with_ipc(|i, hp| i.ipc.grant(hp, from, handle_arg(f.arg(0)), to, rights))
                } else {
                    Err(IpcError::Range)
                };
                f.set_result(r.map_or_else(|e| ERR | e as u32, |h| u32::from(h.0)));
            }
            SYS_SPAWN => {
                // Only the service manager starts processes.
                let from = STATE.with(|s| s.pid);
                let r = if from == MANAGER_SLOT {
                    spawn(f.arg(0))
                } else {
                    Err(ERR_DENIED)
                };
                f.set_result(r.unwrap_or_else(|e| e));
                // The new process's thread is more urgent: it runs now.
                reschedule();
            }
            SYS_EXIT => exit_process(me, f.arg(0)),
            SYS_DONE => done(me, f),
            SYS_IRQ_WAIT => irq_wait(me, f),
            SYS_REPORT => display_report(me, f),
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
    fn on_user_fault(f: &mut Frame, kind: rt::Fault, addr: u32) {
        let prefetch = matches!(kind, rt::Fault::PrefetchAbort);
        let (fsr, _) = cpu::fault_registers();
        let resolved = access_for(arm::classify(fsr), prefetch).is_some_and(|access| {
            with_vm(|vm, s| {
                load_page(vm, s.pid, addr);
                let mva = layout::mva(addr, s.pid);
                space_of(s, s.pid).is_some_and(|sp| vm.handle_fault(sp, mva, access).is_ok())
            })
        });
        if resolved {
            return;
        }
        // A supervised server faulting: it restarts once every thread has
        // left it. Inside a call, the caller gets an error.
        let me = RUN.with(|r| r.current);
        let slot = RUN.with(|r| r.slots[me.index()]);
        let depth = IPC.with(|i| i.ipc.depth(me.0));
        let supervised = supervised(slot).is_some();
        if supervised {
            RUN.with(|r| r.restarting |= 1 << slot);
        }
        if depth > 0 {
            finish_call(me, ERR_FAULT);
        } else if supervised {
            own_thread_fault(f, me, slot);
            return;
        }
        rp_image::fault(kind, addr)
    }

    /// A supervised host's own thread faulted (in no call): the host
    /// restarts now if no call is inside it (else the last caller out
    /// restarts it). Its main thread then re-enters at the entry; any other
    /// thread of it ends.
    fn own_thread_fault(f: &mut Frame, me: Tid, slot: u8) {
        trace(Kind::Mark, u32::from(slot), f.pc);
        if RUN.with(|r| r.inside[slot_ix(slot)]) == 0 {
            restart_server(slot);
        }
        if RUN.with(|r| r.display_thread) == Some(me) {
            let e = RUN.with(|r| r.button);
            while !take_reentry(me) {
                wait(e);
            }
            reenter(f, me, ROLE_HOST);
            return;
        }
        RUN.with(|r| r.slots[me.index()] = 0);
        let done = RUN.with(|r| r.done);
        exit_thread(me, |s| {
            let _ = s.set_event(done);
        })
    }

    /// Packages the test programs as a PE file in `file`: `.text` (the
    /// code), `.rdata` (`MAGIC`), `.data` (a pointer to `.rdata`) and
    /// `.reloc` (fix-ups for that pointer and the code's one absolute
    /// literal), preferring `testprog::PREFERRED_BASE`.
    fn build_image(file: &mut [u8], p: &testprog::Programs) -> Result<(), rp_kernel_pe::Error> {
        let mut code = [0u8; 1024];
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
    fn load_page(vm: &mut KernelVm<'_>, slot: u32, addr: u32) {
        let Some(page) = addr
            .checked_sub(LOAD_BASE)
            .map(|o| o / 4096)
            .filter(|&p| (p as usize) < PE_PAGES)
        else {
            return;
        };
        let host = slot == DISPLAY_SLOT || slot == DISPLAY_CLIENT_SLOT;
        let loaded = if host {
            vm.cache.host[page as usize]
        } else {
            vm.cache.pe[page as usize]
        };
        if loaded != 0 || (!host && vm.cache.pe_file == 0) {
            return;
        }
        let Some(f) = FrameAlloc::alloc(vm.frames, 0) else {
            return;
        };
        let phys = BASE + f as u32 * 4096;
        let ok = if host {
            let i = word(phys);
            let out = rp_hal_mem::words_as_bytes_mut(&mut vm.mem.pool[i..i + 1024]);
            out.try_into()
                .ok()
                .and_then(|out| load_from(DISPLAY_IMAGE, out, page))
        } else {
            load_view_page(vm.mem.pool, vm.cache.pe_file, phys, page)
        };
        if ok.is_some() {
            cpu::clean_invalidate_range(phys, 4096);
            if host {
                vm.cache.host[page as usize] = phys;
            } else {
                vm.cache.pe[page as usize] = phys;
                vm.cache.pe_loads += 1;
            }
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
        load_from(file, out, page)
    }

    /// Loads view page `page` of the PE file `file` into `out`, relocated
    /// to `LOAD_BASE`. Its relocations are read in place from the file
    /// (bounded to the section holding them): no copy, no size limit.
    fn load_from(file: &[u8], out: &mut [u8; 4096], page: u32) -> Option<()> {
        let read = |off: u32, buf: &mut [u8]| -> Result<(), ()> {
            let end = (off as usize).checked_add(buf.len()).ok_or(())?;
            buf.copy_from_slice(file.get(off as usize..end).ok_or(())?);
            Ok(())
        };
        let image = Image::parse(file).ok()?;
        let d = image.dirs[dir::BASERELOC];
        let rel = if d.size == 0 {
            &[][..]
        } else {
            let s = image
                .sections()
                .find(|s| d.rva >= s.rva && d.rva - s.rva < s.raw_size)?;
            let at = s.raw_offset.checked_add(d.rva - s.rva)? as usize;
            let len = (d.size as usize).min((s.raw_size - (d.rva - s.rva)) as usize);
            file.get(at..at.checked_add(len)?)?
        };
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
        exit_thread(me, |s| {
            let _ = s.set_event(done);
        })
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
        // Supervision: client 0's five results, kept in its own memory.
        switch_to(PROC_SLOT);
        let base = (layout::slot_base(PROC_SLOT) + testprog::RESTART_RESULTS) as usize;
        let mut d = [0u32; 5];
        for (k, v) in d.iter_mut().enumerate() {
            *v = rt::try_read(base + 4 * k).unwrap_or(0);
        }
        let (restarts, clean) = RUN.with(|r| (r.restarts, r.restart_clean));
        let opened = d[0] < 32 && d[1] == 7;
        ok &= report_ipc(uart, "service manager", opened, d[0]);
        let restarted = d[2] == ERR_FAULT
            && d[3] == ERR | IpcError::Stale as u32
            && d[4] == 7
            && restarts == 1
            && clean;
        ok &= report_ipc(uart, "supervised restart", restarted, d[3]);
        // Processes at run time: two workers, one after the other, in the
        // same slot; each saw fresh memory and was refused a handle it was
        // never given; nothing of theirs stayed.
        let spawned = [
            rt::try_read(base + 20).unwrap_or(0),
            rt::try_read(base + 24).unwrap_or(0),
        ];
        let (exits, exited, before, after) =
            RUN.with(|r| (r.exits, r.exited, r.frames_before, r.after_exit));
        let clean = after
            .iter()
            .zip(before)
            .all(|(&(free, h, p), b)| free == b && h == 0 && p == 0);
        let lifecycle = exited == 2
            && exits == [testprog::WORKER_OK; 2]
            && spawned[0] == spawned[1]
            && spawned[0] > MANAGER_SLOT
            && clean;
        ok &= report_ipc(uart, "spawn and exit", lifecycle, exits[1]);
        // RetroPane's own processes (the manager, the hosts, what the
        // manager starts) live in native slots above the kernel, so CE's 31
        // stay for Windows Mobile programs; the manager's code, which has
        // run, is really mapped there (0xC0000000 up).
        let native = layout::is_native_slot(spawned[0])
            && [MANAGER_SLOT, HOST_SLOT, DISPLAY_SLOT]
                .iter()
                .all(|&s| layout::is_native_slot(s))
            && with_vm(|vm, _| {
                let code = layout::slot_base(MANAGER_SLOT) + LOAD_BASE + TEXT_RVA;
                code >= layout::NATIVE_BASE && vm.tables.translate(vm.mem, code).is_some()
            });
        ok &= report_ipc(uart, "native slots", native, spawned[0]);
        ok &= check_handle_pages(uart);
        ok &= check_trace(uart);
        let _ = writeln!(uart, "BENCH ipc_call_roundtrip {ticks} {calls}");
        ok
    }

    /// A handle table grows into a page only when its inline handles are
    /// taken, and the page goes back when the process exits: one handle
    /// past the inline ones into an empty slot's table costs one frame.
    fn check_handle_pages(uart: &mut PxaUart<Mmio>) -> bool {
        const EMPTY_SLOT: u8 = 20;
        let free = || FRAMES.with(|f| f.free_frames() as u32);
        let before = free();
        let grew = with_ipc(|i, hp| -> Option<(usize, usize)> {
            let port = RUN.with(|r| r.manager_port)?;
            for _ in 0..=rp_kernel_ipc::INLINE_HANDLES {
                i.ipc.install(hp, EMPTY_SLOT, port, Rights::CALL).ok()?;
            }
            Some((
                i.ipc.handle_pages(EMPTY_SLOT),
                i.ipc.handle_count(hp, EMPTY_SLOT),
            ))
        });
        let during = free();
        let _ = with_ipc(|i, hp| i.ipc.exit(hp, EMPTY_SLOT));
        let after = free();
        let pass = grew == Some((1, rp_kernel_ipc::INLINE_HANDLES + 1))
            && before - during == 1
            && after == before;
        report_ipc(uart, "handle pages", pass, before.wrapping_sub(during))
    }

    /// The trace ring after the run: it wrapped many times and kept the
    /// newest 128 events intact and in order; and a record costs little.
    fn check_trace(uart: &mut PxaUart<Mmio>) -> bool {
        let (total, intact, ordered, cap) = TRACE.with(|t| {
            let Some(t) = t else {
                return (0, 0, false, 0);
            };
            let mut prev: Option<(u32, u32)> = None;
            let (mut n, mut ordered) = (0u32, true);
            for e in t.events() {
                if let Some((seq, time)) = prev {
                    ordered &= e.seq == seq + 1 && e.time.wrapping_sub(time) < 1 << 31;
                }
                prev = Some((e.seq, e.time));
                n += 1;
            }
            (t.total(), n, ordered, t.capacity() as u32)
        });
        // The cost of one record, over 1000.
        let start = rt::ticks();
        for i in 0..1000 {
            trace(Kind::Mark, i, i);
        }
        let ticks = rt::ticks().wrapping_sub(start);
        let pass = cap > 0 && total > 4 * cap && intact == cap && ordered;
        let _ = writeln!(uart, "BENCH trace_event {ticks} 1000");
        report_as_trace(uart, pass, total, cap)
    }

    fn report_as_trace(uart: &mut PxaUart<Mmio>, pass: bool, total: u32, cap: u32) -> bool {
        let _ = uart.write_str("TRACE ring");
        if pass {
            let _ = writeln!(uart, " OK ({total} events, newest {cap} kept)");
        } else {
            let _ = writeln!(uart, " FAIL (got {total})");
        }
        pass
    }

    /// The display host's checks (item 46): its client's report, both
    /// restarts, and the level in the hardware.
    fn report_display(uart: &mut PxaUart<Mmio>) -> bool {
        let (report, restarts, starts, clean) = RUN.with(|r| {
            (
                r.display_report.unwrap_or([0; 4]),
                r.display_restarts,
                r.display_starts,
                !r.display_dirty,
            )
        });
        let duty = pwm_duty();
        let [set, fault, handle, after] = report;
        let served = set == DISPLAY_LEVEL && duty != 0;
        let mut ok = report_ipc(uart, "driver host served", served, set);
        let restarted = fault == ERR_FAULT
            && handle < 32
            && after == DISPLAY_LEVEL
            && restarts == 2
            && starts == 3
            && clean;
        ok &= report_ipc(uart, "driver host restarted", restarted, after);
        ok
    }

    /// After the marker: the power button, pressed by the emulator, turns
    /// the light off through the host, and on again at the level it had.
    fn power_button(uart: &mut PxaUart<Mmio>) {
        let duty = pwm_duty;
        let on = duty();
        let _ = writeln!(uart, "DISPLAY waiting for the power button (duty {on})");
        let wait_for = |want: &dyn Fn(u32) -> bool| {
            // Up to ~60 s, checked every ~50 ms.
            for _ in 0..1200 {
                if want(duty()) {
                    return true;
                }
                let at = rt::ticks().wrapping_add(162_500);
                if SCHED.with(|s| s.sleep_until(at)).is_ok() {
                    reschedule();
                }
            }
            false
        };
        if wait_for(&|d| d == 0) {
            let _ = writeln!(uart, "DISPLAY button: light off");
        }
        if wait_for(&|d| d == on) {
            let _ = writeln!(uart, "DISPLAY button: light back on");
        }
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
}

#[cfg(not(target_os = "none"))]
fn main() {}
