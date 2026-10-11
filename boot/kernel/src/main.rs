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
//! Each step prints `VM <step> OK` on serial (or `FAIL` with details), then
//! [`MARKER`] if all passed; `cargo xtask emu` checks them.

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
    use rp_hal_pxa27x::{Mmio, cpu};
    use rp_hal_x51v::devices::{RAM_POOL_BASE, RAM_POOL_SIZE, take_ram_pool};
    use rp_kernel_mm::arm::{self, Ap, Attrs, Cache};
    use rp_kernel_mm::frames::Frames;
    use rp_kernel_mm::layout;
    use rp_kernel_mm::pagetable::{FrameAlloc, Mmu, PageTables, PhysMem};
    use rp_kernel_mm::space::{
        AddressSpace, Backing, IoError, Outcome, PageCache, Prot, Region, Vm, access_for,
    };

    rp_hal_pxa27x::entry!(main);

    const POOL_FRAMES: usize = RAM_POOL_SIZE / 4096;
    const BASE: u32 = RAM_POOL_BASE as u32;
    const KERNEL_DOMAIN: u32 = 0;
    const PROC_DOMAIN: u32 = 1;
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

    /// Everything else the VM owns.
    struct State {
        space: AddressSpace,
        file: RamFile,
        pool: Option<&'static mut [u32]>,
        l1: u32,
        pid: u32,
        faults: u32,
        last: Option<Outcome>,
        error: Option<&'static str>,
    }

    static STATE: Global<State> = Global::new(State {
        space: AddressSpace::process(PROC_SLOT, PROC_DOMAIN),
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
                        };
                        f(&mut vm, s)
                    };
                    s.file.refs = file.refs;
                    s.pool = Some(pool);
                    r
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
        let _ = writeln!(uart, "\nFAULT {} at {addr:#010x} ({why})", kind.name());
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
        let Some(bus) = rt::take_ffuart() else { halt() };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::set_fault_handler(on_fault);
        rt::init_mmu();
        let _ = writeln!(uart, "\nRetroPane 26 kernel: VM bring-up");
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
            // The bring-up map's RAM, peripherals, SRAM and vectors. Not the
            // device windows below 0x42000000 (CS0-CS5): they'd overlap the
            // process slots, which is why device windows move to kernel space
            // next.
            for r in X51V_BRINGUP {
                let attrs = Attrs {
                    ap: Ap::Kernel,
                    cache: cache_for(r.memory),
                };
                // Clip each region to 0x40000000 and up: the peripheral block
                // (UART, interrupt controller, timer) stays, the CS0-CS5
                // windows below don't.
                let end = u64::from(r.virt) + u64::from(r.len);
                if end <= 0x4000_0000 {
                    continue;
                }
                let mut off = 0x4000_0000u32.saturating_sub(r.virt);
                while off < r.len {
                    TABLES
                        .with(|t| {
                            t.map_section(
                                &mut mem,
                                r.virt + off,
                                r.phys + off,
                                KERNEL_DOMAIN,
                                attrs,
                            )
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
                let _ = writeln!(uart, "VM tables FAIL {e}");
                halt()
            }
        };
        let l1 = STATE.with(|s| s.l1);
        cpu::set_dacr(arm::dacr((1 << KERNEL_DOMAIN) | (1 << PROC_DOMAIN)));
        cpu::switch_table(l1);
        let _ = writeln!(
            uart,
            "VM tables OK (L1 at {l1:#010x}, {free_after_setup} frames free)"
        );
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
            let _ = writeln!(uart, "VM process FAIL {e}");
            halt()
        }
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

        // Unmap everything: every frame and page-cache reference returns.
        let slot = layout::slot_base(PROC_SLOT);
        let (unmapped, refs) = with_vm(|vm, s| {
            let r = vm.unmap(&mut s.space, slot, layout::SLOT_SIZE);
            (r.is_ok(), vm.cache.refs)
        });
        // The file's own frame stays allocated (it's the "page cache").
        let free_now = FRAMES.with(|f| f.free_frames() as u32);
        let pass = unmapped && free_now == free_after_setup - 1 && refs == 0;
        ok &= report(&mut uart, "unmap", pass, free_now);

        if ok {
            let _ = writeln!(uart, "{}", super::MARKER);
        }
        halt()
    }

    fn report(uart: &mut PxaUart<Mmio>, what: &str, pass: bool, got: u32) -> bool {
        if pass {
            let _ = writeln!(uart, "VM {what} OK");
        } else {
            let _ = writeln!(uart, "VM {what} FAIL (got {got:#010x})");
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
            let _ = writeln!(uart, "\nPANIC at {}:{}", l.file(), l.line());
        }
        loop {
            core::hint::spin_loop();
        }
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
