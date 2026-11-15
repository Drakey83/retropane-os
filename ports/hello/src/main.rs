//! The first C program on RetroPane: a C self-check (`c/hello.c`) built by
//! the pinned Clang against picolibc, started from Rust. `cargo xtask
//! emu-chello` builds both, boots this in the emulator with a card and
//! looks for [`MARKER`], which only the C program prints, after every check
//! passed: the C library itself, and the card's files through `fopen`.
//!
//! Iterate, don't imitate (rule 0). A C program on Windows Mobile or Linux
//! starts from the C library's own start-up file over a kernel; an embedded
//! one from the vendor's `crt0`. Ours starts in Rust: the image's entry,
//! crash path and return to the loader are the ones every RetroPane image
//! has (`rp-image`), the C program's calls land in safe Rust (the port
//! layer's [`rp_port::sys::System`] over the card, or a console alone
//! without one), and its heap is a run of RAM this file names, so a C fault
//! is reported like any other and nothing of the image is C's to set up.
//!
//! On the host, and in builds without the C toolchain, there is no program
//! here (the `c` feature).

#![cfg_attr(target_os = "none", no_std, no_main)]
#![forbid(unsafe_code)]

/// Printed by the C program once every check passed.
pub const MARKER: &str = "RETROPANE-C-HELLO-OK";

#[cfg(target_os = "none")]
mod image {
    use core::fmt::Write;

    use rp_drv_cpld::Cpld;
    use rp_drv_pxa_mmc::PxaMmc;
    use rp_drv_pxa_uart::PxaUart;
    use rp_drv_sdcard::{SdCard, mbr_partitions};
    use rp_fat::AnyVolume;
    use rp_hal_api::{BlockDevice, BlockError};
    use rp_hal_cabi::{Break, Errno, Sys};
    use rp_hal_pxa27x::dma::Dma;
    use rp_hal_pxa27x::{Mmio, rt};
    use rp_hal_x51v::devices;
    use rp_port::clock::Micros;
    use rp_port::files::Files;
    use rp_port::sys::{Console, System};

    rp_image::entry!(main);

    /// The C heap: the free RAM between the image area and the loader
    /// (boot/link.x: 0xA8100000-0xA8400000, 3 MiB, mapped 1:1).
    const HEAP: (usize, usize) = (0xA810_0000, 0xA840_0000);
    /// Of the scratch RAM: the file system's sector work.
    const FAT_WORK: usize = 64 * 1024;
    /// The program's own folder on the card (the card is shared: its test
    /// file goes nowhere else).
    const FOLDER: &[u8] = b"rp-chello";

    type Sd = SdCard<PxaMmc<Mmio, Dma>>;

    /// The card, checking in with the run limit at every transfer.
    struct Card(Sd);

    impl BlockDevice for Card {
        fn read_blocks(&mut self, lba: u32, buf: &mut [u8]) -> Result<(), BlockError> {
            rt::check_in();
            self.0.read_blocks(lba, buf)
        }
        fn write_blocks(&mut self, lba: u32, buf: &[u8]) -> Result<(), BlockError> {
            rt::check_in();
            self.0.write_blocks(lba, buf)
        }
    }

    /// The serial line as a C program's console.
    struct Serial(PxaUart<Mmio>);

    impl Console for Serial {
        fn put(&mut self, bytes: &[u8]) {
            for &b in bytes {
                if b == b'\n' {
                    self.0.write_byte(b'\r');
                }
                self.0.write_byte(b);
            }
        }
    }

    /// What answers the C library without a card: the console and the heap.
    struct Bare {
        console: Serial,
        heap: Break,
        clock: Micros,
    }

    impl Sys for Bare {
        fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, Errno> {
            if fd != 1 && fd != 2 {
                return Err(Errno::BadHandle);
            }
            self.console.put(bytes);
            Ok(bytes.len())
        }
        fn read(&mut self, _fd: i32, _into: &mut [u8]) -> Result<usize, Errno> {
            Err(Errno::BadHandle)
        }
        fn open(&mut self, _path: &[u8], _flags: i32) -> Result<i32, Errno> {
            Err(Errno::NoEntry)
        }
        fn close(&mut self, _fd: i32) -> Result<(), Errno> {
            Err(Errno::BadHandle)
        }
        fn seek(&mut self, _fd: i32, _by: i64, _whence: i32) -> Result<i64, Errno> {
            Err(Errno::BadHandle)
        }
        fn remove(&mut self, _path: &[u8]) -> Result<(), Errno> {
            Err(Errno::NoEntry)
        }
        fn grow(&mut self, by: isize) -> Option<usize> {
            self.heap.grow(by)
        }
        fn now_us(&mut self) -> u64 {
            self.clock.at(rt::ticks())
        }
    }

    static BARE: rt::Slot<Bare> = rt::Slot::new();
    static WITH_CARD: rt::Slot<System<'static, Card, 8, Serial>> = rt::Slot::new();

    /// A C `exit()`: a failure of the image unless it is `exit(0)`.
    fn exit(code: i32) -> ! {
        if code == 0 {
            rp_image::finish()
        }
        rp_image::fail("the C program called exit with an error")
    }

    /// The SD card, as the loader starts it (power through the CPLD, then
    /// the card); `None` if there is none.
    fn open_card() -> Option<Sd> {
        let (bus, table) = devices::take_cpld()?;
        let (mut cpld, _) = Cpld::adopt(bus, table);
        let (mmc, dma) = (rt::take_mmc()?, rt::take_dma(rp_hal_pxa27x::dma_map::SD)?);
        cpld.set_sd_power(true);
        rt::delay_us(10_000);
        SdCard::try_init(PxaMmc::with_dma(mmc, dma)).ok()
    }

    /// The card's first partition (or the whole card), mounted.
    fn mount(mut sd: Sd) -> Option<AnyVolume<Card>> {
        let mut block = [0u8; rp_drv_sdcard::BLOCK];
        sd.read(0, &mut block).ok()?;
        let start = mbr_partitions(&block)
            .and_then(|p| p[0])
            .map_or(0, |p| p.start);
        AnyVolume::mount(Card(sd), start).ok()
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else {
            rp_image::fail("no serial port")
        };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::init_mmu();
        let _ = writeln!(uart, "\nRetroPane: starting a C program");
        let heap = Break::new(HEAP.0, HEAP.1);
        let volume = open_card().and_then(mount);
        let sys: &'static mut dyn Sys = match (volume, devices::take_scratch()) {
            (Some(vol), Some(ram)) => {
                let _ = writeln!(uart, "card: mounted");
                let (work, room) = ram.split_at_mut(FAT_WORK);
                // Made straight into its slot: the file table is 5 KiB, and
                // a copy or two of it on the way would be most of a stack.
                let Some(s) = WITH_CARD.put_with(|| {
                    System::new(Files::new(vol, work, room), Serial(uart), heap, rt::ticks)
                }) else {
                    rp_image::fail("the C system was set up twice")
                };
                if s.files().mkdir(FOLDER).is_err() {
                    rp_image::fail("the program's folder could not be made on the card");
                }
                s
            }
            _ => {
                let _ = writeln!(uart, "card: none");
                match BARE.put_with(|| Bare {
                    console: Serial(uart),
                    heap,
                    clock: Micros::starting_at(rt::ticks()),
                }) {
                    Some(s) => s,
                    None => rp_image::fail("the C system was set up twice"),
                }
            }
        };
        rp_hal_cabi::start(sys, exit);
        let code = rp_hal_cabi::run();
        let mut uart = PxaUart::new(rt::emergency_uart());
        let _ = writeln!(uart, "the C program returned {code}");
        if code != 0 {
            rp_image::fail("the C program's checks failed");
        }
        rp_image::finish()
    }
}

#[cfg(not(target_os = "none"))]
fn main() {}
