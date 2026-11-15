//! The first C program on RetroPane: a C self-check (`c/hello.c`) built by
//! the pinned Clang against picolibc, started from Rust. `cargo xtask
//! emu-chello` builds both, boots this in the emulator and looks for
//! [`MARKER`], which only the C program prints, after every check passed.
//!
//! Iterate, don't imitate (rule 0). A C program on Windows Mobile or Linux
//! starts from the C library's own start-up file over a kernel; an embedded
//! one from the vendor's `crt0`. Ours starts in Rust: the image's entry,
//! crash path and return to the loader are the ones every RetroPane image
//! has (`rp-image`), the C program's calls land in safe Rust
//! ([`rp_hal_cabi::Sys`]), and its heap is a run of RAM this file names, so
//! a C fault is reported like any other and nothing of the image is C's to
//! set up.
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

    use rp_drv_pxa_uart::PxaUart;
    use rp_hal_cabi::{Break, Errno, Sys};
    use rp_hal_pxa27x::Mmio;
    use rp_hal_pxa27x::rt;

    rp_image::entry!(main);

    /// The C heap: the free RAM between the image area and the loader
    /// (boot/link.x: 0xA8100000-0xA8400000, 3 MiB, mapped 1:1).
    const HEAP: (usize, usize) = (0xA810_0000, 0xA840_0000);

    /// What answers the C library here: the console, and the heap. No card,
    /// so no files (the port layer's files come with their own image).
    struct Hello {
        uart: PxaUart<Mmio>,
        heap: Break,
    }

    impl Sys for Hello {
        fn write(&mut self, fd: i32, bytes: &[u8]) -> Result<usize, Errno> {
            if fd != 1 && fd != 2 {
                return Err(Errno::BadHandle);
            }
            for &b in bytes {
                if b == b'\n' {
                    self.uart.write_byte(b'\r');
                }
                self.uart.write_byte(b);
            }
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
    }

    static SYS: rt::Slot<Hello> = rt::Slot::new();

    /// A C `exit()`: a failure of the image unless it is `exit(0)`.
    fn exit(code: i32) -> ! {
        if code == 0 {
            rp_image::finish()
        }
        rp_image::fail("the C program called exit with an error")
    }

    fn main() -> ! {
        let Some(bus) = rt::take_ffuart() else {
            rp_image::fail("no serial port")
        };
        let mut uart = PxaUart::new(bus);
        uart.init(115_200);
        rt::init_mmu();
        let _ = writeln!(uart, "\nRetroPane: starting a C program");
        let Some(sys) = SYS.put_with(|| Hello {
            uart,
            heap: Break::new(HEAP.0, HEAP.1),
        }) else {
            rp_image::fail("the C system was set up twice")
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
