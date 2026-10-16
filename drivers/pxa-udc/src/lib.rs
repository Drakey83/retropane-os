//! Driver for the PXA27x USB device controller (UDC), presenting the X51v to
//! a PC as a CDC-ACM serial port (ADR-0012, docs/hw/udc.md).
//!
//! Polled: call [`Udc::poll`] often (every few milliseconds while data
//! moves). The hardware answers SET_ADDRESS/SET_CONFIGURATION itself; this
//! driver answers descriptor and CDC requests on endpoint 0 and moves bytes
//! between two small rings and the bulk FIFOs. The board's D+ pull-up and
//! cable detect are GPIOs, switched by the caller.
//!
//! Throughput (audit item 74, docs/perf.md): the bulk endpoints are double
//! buffered (`UDCCRx.DE`), so the host fills (or empties) one packet buffer
//! while the CPU serves the other instead of being NAKed; every poll drains
//! all packets waiting. Bulk IN keeps one packet in flight by default:
//! streaming into both IN buffers ([`Udc::set_in_streaming`]) kept the data
//! intact but made Windows time out and reset the device during drive
//! reads on the X51v (docs/hw/udc.md), so it is off until understood.
//! Measured ([`Udc::probe_in_buffers`]): with double buffering `BNF` reads
//! 1 after one full IN packet, 0 after two, and 0 after a single short one.

#![no_std]
#![forbid(unsafe_code)]

pub mod usb;

use rp_hal_api::{Port, RegisterBus, RegisterBus8, Width};
use usb::{Cdc, Reply, Setup};

/// Register offsets and bits (docs/hw/udc.md).
pub mod reg {
    /// Control.
    pub const UDCCR: usize = 0x000;
    /// Interrupt enables, endpoint 0 and A–O.
    pub const UDCICR0: usize = 0x004;
    /// Interrupt enables, P–X and bus events.
    pub const UDCICR1: usize = 0x008;
    /// Interrupt status, endpoint 0 and A–O.
    pub const UDCISR0: usize = 0x00C;
    /// Interrupt status, P–X and bus events.
    pub const UDCISR1: usize = 0x010;
    /// USB port 2 output control (transceiver, internal pull-ups).
    pub const UP2OCR: usize = 0x020;
    /// USB port 3 output control.
    pub const UP3OCR: usize = 0x024;
    /// UP2OCR: D+ pull-up enable.
    pub const DPPUE: u32 = 1 << 4;
    /// UP2OCR: transceiver output enable.
    pub const HXOE: u32 = 1 << 17;

    /// Control/status of endpoint `x` (0 = endpoint 0, 1.. = A..).
    #[must_use]
    pub const fn csr(x: usize) -> usize {
        0x100 + 4 * x
    }
    /// Byte count of endpoint `x`.
    #[must_use]
    pub const fn bcr(x: usize) -> usize {
        0x200 + 4 * x
    }
    /// Data FIFO of endpoint `x`.
    #[must_use]
    pub const fn dr(x: usize) -> usize {
        0x300 + 4 * x
    }
    /// Configuration of endpoint `x` (1.. = A..).
    #[must_use]
    pub const fn cr(x: usize) -> usize {
        0x400 + 4 * x
    }

    /// UDCCR: enable.
    pub const UDE: u32 = 1 << 0;
    /// UDCCR: endpoint memory configuration error.
    pub const EMCE: u32 = 1 << 3;
    /// UDCCR: switch endpoint memory to the active configuration.
    pub const SMAC: u32 = 1 << 4;
    /// UDCCR: the only bits a write may carry.
    pub const UDCCR_WRITABLE: u32 = (1 << 31) | SMAC | (1 << 2) | UDE;
    /// UDCCR: active configuration number.
    pub const ACN_SHIFT: u32 = 11;

    /// UDCISR0/UDCICR0: endpoint 0 packet complete.
    pub const EP0_PACKET: u32 = 1 << 0;
    /// UDCISR1/UDCICR1: bus reset.
    pub const RESET: u32 = 1 << 27;
    /// Suspend.
    pub const SUSPEND: u32 = 1 << 28;
    /// Resume.
    pub const RESUME: u32 = 1 << 29;
    /// Configuration change.
    pub const CONFIG_CHANGE: u32 = 1 << 31;

    /// UDCCSR0 bits.
    pub const OPC: u32 = 1 << 0;
    /// IN packet ready.
    pub const IPR: u32 = 1 << 1;
    /// Flush transmit FIFO.
    pub const FTF: u32 = 1 << 2;
    /// Sent stall.
    pub const SST: u32 = 1 << 4;
    /// Force stall.
    pub const FST: u32 = 1 << 5;
    /// Receive FIFO not empty.
    pub const RNE: u32 = 1 << 6;
    /// SETUP active.
    pub const SA: u32 = 1 << 7;
    /// ACK response enable.
    pub const AREN: u32 = 1 << 8;
    /// ACK control mode.
    pub const ACM: u32 = 1 << 9;

    /// UDCCSRx (A–X) bits: packet complete.
    pub const PC: u32 = 1 << 1;
    /// Tx/Rx NAK: the host's token found no data (write 1 to clear).
    pub const TRN: u32 = 1 << 2;
    /// Bit 6: for an IN endpoint, buffer not full (BNF); for an OUT
    /// endpoint, buffer not empty. It reads 0 while the endpoint isn't
    /// active (measured: an inactive endpoint looked "full").
    pub const BNF: u32 = 1 << 6;
    /// DMA enable (preserved by every write that should keep it).
    pub const DME: u32 = 1 << 3;
    /// Short packet.
    pub const SP: u32 = 1 << 7;
    /// Flush the FIFO.
    pub const FEF: u32 = 1 << 8;
}

/// The UDC's physical base address (PXA27x memory map).
pub const PHYS: u32 = 0x4060_0000;

/// A bulk endpoint, for DMA ([`Udc::dma_port`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bulk {
    /// The serial port's data to the host.
    SerialIn,
    /// The serial port's data from the host.
    SerialOut,
    /// Mass storage to the host.
    StorageIn,
    /// Mass storage from the host.
    StorageOut,
}

impl Bulk {
    const fn slot(self) -> usize {
        match self {
            Self::SerialIn => SLOT_IN,
            Self::SerialOut => SLOT_OUT,
            Self::StorageIn => SLOT_MSC_IN,
            Self::StorageOut => SLOT_MSC_OUT,
        }
    }
}

/// The UDC's endpoint slots used (A = 1 ... E = 5).
const SLOT_NOTIFY: usize = 1;
const SLOT_IN: usize = 2;
const SLOT_OUT: usize = 3;
const SLOT_MSC_OUT: usize = 4;
const SLOT_MSC_IN: usize = 5;

/// `UDCCRx.DE`: double buffering (two packet buffers in endpoint memory).
const DE: u32 = 1 << 1;

/// One `UDCCRx` value: configuration 1, alternate setting 0; bulk endpoints
/// double buffered.
const fn endpoint(interface: u32, number: u8, kind: u32, is_in: bool, size: usize) -> u32 {
    (if kind == 2 { DE } else { 0 })
        | (1 << 25)
        | (interface << 22)
        | ((number as u32) << 15)
        | (kind << 13)
        | ((is_in as u32) << 12)
        | ((size as u32) << 2)
        | 1
}

/// The endpoint table, matching [`usb::CONFIG`]'s endpoints. Every
/// endpoint is in interface 0 here, although the descriptors put the bulk
/// pair in interface 1: the PXA27x only activates endpoints of the active
/// interface (0 unless the host sends SET_INTERFACE, which a CDC host
/// doesn't), so with the bulk pair in interface 1 the port opened but no
/// data moved either way (measured on the real X51v). The Linux PXA27x
/// driver does the same for its serial gadget ("multiple interfaces are
/// really broken on the pxa").
///
/// The mass storage pair (descriptor interface 2) is in interface 0 for
/// the same reason (ADR-0014).
const TABLE: [(usize, u32); 5] = [
    (
        SLOT_NOTIFY,
        endpoint(0, usb::EP_NOTIFY, 3, true, usb::NOTIFY_SIZE),
    ),
    (SLOT_IN, endpoint(0, usb::EP_IN, 2, true, usb::BULK_SIZE)),
    (SLOT_OUT, endpoint(0, usb::EP_OUT, 2, false, usb::BULK_SIZE)),
    (
        SLOT_MSC_OUT,
        endpoint(0, usb::EP_MSC_OUT, 2, false, usb::BULK_SIZE),
    ),
    (
        SLOT_MSC_IN,
        endpoint(0, usb::EP_MSC_IN, 2, true, usb::BULK_SIZE),
    ),
];

/// Errors.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The hardware rejected the endpoint table (`UDCCR.EMCE`).
    EndpointTable,
}

/// Counters for the screen (bring-up: what the host has done so far).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bus resets seen.
    pub resets: u32,
    /// SETUP packets handled.
    pub setups: u32,
    /// Requests answered with STALL.
    pub stalls: u32,
    /// Configuration changes (SET_CONFIGURATION/SET_INTERFACE).
    pub config_changes: u32,
    /// Bytes received from the host.
    pub rx_bytes: u32,
    /// Bytes sent to the host.
    pub tx_bytes: u32,
    /// The last request: bmRequestType << 8 | bRequest.
    pub last_request: u16,
    /// Endpoint 0 IN packets written (descriptor answers): with SETUPs
    /// arriving but this stuck, answers aren't leaving.
    pub ep0_in: u32,
    /// `UDCCSR0` when the last SETUP arrived (bring-up diagnostics).
    pub setup_csr: u32,
    /// The last four requests (bmRequestType << 8 | bRequest), newest
    /// first: what the host did, for the screen.
    pub recent: [u16; 4],
    /// Bulk OUT packets whose byte count (`UDCBCRx`) disagreed with the
    /// short-packet bit: a full packet (SP clear) whose count read short.
    /// Diagnostics for the double-buffered OUT path (item 74).
    pub count_mismatch: u32,
    /// The last such packet: `UDCCSRx` << 16 | `UDCBCRx`.
    pub last_mismatch: u32,
    /// Packet-completes whose status or count changed between two reads
    /// (they are read until two agree).
    pub unsettled: u32,
}

/// A byte ring. `N` must be a power of two.
struct Ring<const N: usize> {
    buf: [u8; N],
    head: usize,
    tail: usize,
}

impl<const N: usize> Ring<N> {
    const fn new() -> Self {
        Self {
            buf: [0; N],
            head: 0,
            tail: 0,
        }
    }
    fn len(&self) -> usize {
        self.head.wrapping_sub(self.tail)
    }
    fn space(&self) -> usize {
        N - self.len()
    }
    fn push(&mut self, b: u8) {
        self.buf[self.head % N] = b;
        self.head = self.head.wrapping_add(1);
    }
    fn pop(&mut self) -> Option<u8> {
        (self.len() > 0).then(|| {
            let b = self.buf[self.tail % N];
            self.tail = self.tail.wrapping_add(1);
            b
        })
    }
    fn clear(&mut self) {
        self.tail = self.head;
    }
}

/// Endpoint 0's state between polls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ep0 {
    Idle,
    /// Sending `ep0_buf[pos..len]`.
    InData,
    /// All data sent; waiting for the host's zero-length status OUT.
    StatusOut,
    /// Receiving `len` bytes into `ep0_buf`.
    OutData,
    /// STALL forced; waiting for it to be sent.
    Stall,
}

/// The UDC as a CDC-ACM serial port.
pub struct Udc<B> {
    bus: B,
    cdc: Cdc,
    ep0: Ep0,
    ep0_buf: [u8; 128],
    ep0_len: usize,
    ep0_pos: usize,
    configuration: u8,
    rx: Ring<256>,
    tx: Ring<512>,
    /// Bulk IN streams into both buffers (off: one packet in flight).
    in_streaming: bool,
    /// Per slot: an IN packet was written and its packet-complete not yet
    /// seen (one-in-flight mode).
    in_wait: [bool; 6],
    /// Per slot: DMA serves the endpoint (`DME` set): the CPU paths leave
    /// its FIFO alone, and every status write carries `DME`.
    dma: [bool; 6],
    /// A bus reset or a mass storage reset happened: the storage function
    /// must abandon its command ([`Udc::take_msc_reset`]).
    msc_reset: bool,
    stats: Stats,
}

impl<B: RegisterBus + RegisterBus8> Udc<B> {
    /// A driver for the UDC registers in `bus` (base `0x40600000`).
    pub const fn new(bus: B) -> Self {
        Self {
            bus,
            cdc: Cdc::new(),
            ep0: Ep0::Idle,
            ep0_buf: [0; 128],
            ep0_len: 0,
            ep0_pos: 0,
            configuration: 0,
            rx: Ring::new(),
            tx: Ring::new(),
            in_streaming: false,
            in_wait: [false; 6],
            dma: [false; 6],
            msc_reset: false,
            stats: Stats {
                resets: 0,
                setups: 0,
                stalls: 0,
                config_changes: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                last_request: 0,
                ep0_in: 0,
                setup_csr: 0,
                recent: [0; 4],
                count_mismatch: 0,
                last_mismatch: 0,
                unsettled: 0,
            },
        }
    }

    /// Read-modify-write of UDCCR carrying only its writable bits.
    fn udccr(&mut self, set: u32, clear: u32) {
        let v = self.bus.read32(reg::UDCCR) & reg::UDCCR_WRITABLE;
        self.bus.write32(reg::UDCCR, (v | set) & !clear);
    }

    /// Resets the UDC (whatever Windows Mobile or HaRET left), writes the
    /// endpoint table and enables it. The host sees the device once the
    /// caller turns the D+ pull-up on.
    ///
    /// # Errors
    /// [`Error::EndpointTable`] if the hardware rejects the table.
    pub fn enable(&mut self) -> Result<(), Error> {
        self.bus.write32(reg::UDCICR0, 0);
        self.bus.write32(reg::UDCICR1, 0);
        self.udccr(0, reg::UDE);
        for (slot, cfg) in TABLE {
            self.bus.write32(reg::cr(slot), cfg);
        }
        // The X51v's UDC talks through port 2's transceiver: its output must
        // be on (WM leaves UP2OCR = HXOE; with it off the PC sees the device
        // attach but never gets an answer). Measured, docs/hw/udc.md. Only
        // written when off, and before the UDC is enabled: the run that
        // enumerated never wrote UP2OCR.
        let up2 = self.bus.read32(reg::UP2OCR);
        if up2 & reg::HXOE == 0 {
            self.bus.write32(reg::UP2OCR, up2 | reg::HXOE);
        }
        self.udccr(reg::UDE, 0);
        self.bus.write32(reg::csr(0), reg::ACM);
        if self.bus.read32(reg::UDCCR) & reg::EMCE != 0 {
            self.udccr(0, reg::UDE);
            return Err(Error::EndpointTable);
        }
        // Status bits for polling (the interrupt line stays masked in the
        // interrupt controller until the driver moves to interrupts).
        self.bus.write32(
            reg::UDCICR1,
            reg::RESET | reg::SUSPEND | reg::RESUME | reg::CONFIG_CHANGE,
        );
        self.bus.write32(reg::UDCICR0, reg::EP0_PACKET);
        self.ep0 = Ep0::Idle;
        Ok(())
    }

    /// Reads a UDC register directly (bring-up: board facts such as the
    /// port control registers Windows Mobile left).
    pub fn read_reg(&mut self, offset: usize) -> u32 {
        self.bus.read32(offset)
    }

    /// Writes a UDC register directly (bring-up experiments with the port
    /// control registers; not for the registers this driver manages).
    pub fn write_reg(&mut self, offset: usize, value: u32) {
        self.bus.write32(offset, value);
    }

    /// Turns the controller off.
    pub fn disable(&mut self) {
        self.bus.write32(reg::UDCICR1, 0);
        self.udccr(0, reg::UDE);
        self.configuration = 0;
    }

    /// Whether the host has configured the device (it's a COM port now).
    #[must_use]
    pub fn configured(&self) -> bool {
        self.configuration != 0
    }

    /// Whether a program on the host has the port open (DTR).
    #[must_use]
    pub fn host_open(&self) -> bool {
        self.configured() && self.cdc.dtr()
    }

    /// Counters for display.
    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Bytes queued for the host and not yet taken by it. While no program
    /// on the PC reads the port, nothing drains: a sender that keeps
    /// writing fills the ring and later writes are cut short.
    #[must_use]
    pub fn tx_pending(&self) -> usize {
        self.tx.len()
    }

    /// Queues bytes for the host; returns how many fit. Nothing is queued
    /// until the device is configured (bytes written before would be stale).
    pub fn write(&mut self, data: &[u8]) -> usize {
        if !self.configured() {
            return 0;
        }
        let n = data.len().min(self.tx.space());
        for &b in &data[..n] {
            self.tx.push(b);
        }
        n
    }

    /// Takes received bytes; returns how many. Once the ring is empty,
    /// waiting packets go straight from the FIFO into `out` (no ring copy)
    /// while a whole packet fits.
    pub fn read(&mut self, out: &mut [u8]) -> usize {
        let mut n = 0;
        while n < out.len() {
            let Some(b) = self.rx.pop() else { break };
            out[n] = b;
            n += 1;
        }
        while self.configured()
            && !self.dma[SLOT_OUT]
            && self.rx.len() == 0
            && out.len() - n >= usb::BULK_SIZE
        {
            match self.read_packet(SLOT_OUT, &mut out[n..]) {
                Some(got) => n += got,
                None => break,
            }
        }
        n
    }

    /// Services the controller: bus events, endpoint 0, then data.
    pub fn poll(&mut self) {
        let events = self.bus.read32(reg::UDCISR1);
        if events & reg::RESET != 0 {
            self.bus.write32(reg::UDCISR1, reg::RESET);
            self.bus_reset();
        }
        if events & reg::CONFIG_CHANGE != 0 {
            self.bus.write32(reg::UDCISR1, reg::CONFIG_CHANGE);
            self.config_change();
        }
        let other = events & (reg::SUSPEND | reg::RESUME);
        if other != 0 {
            self.bus.write32(reg::UDCISR1, other);
        }
        self.poll_ep0();
        if self.configured() {
            self.poll_out();
            self.poll_in();
        }
    }

    fn bus_reset(&mut self) {
        self.stats.resets += 1;
        self.configuration = 0;
        // Flush endpoint 0, but keep a SETUP that has already arrived: its
        // OPC is part of what `poll_ep0` waits for, and clearing it would
        // lose the host's first request (a host that ignores the 10 ms reset
        // recovery, or a slow poll; found with the emulator's host).
        let setup_waiting = self.bus.read32(reg::csr(0)) & reg::SA != 0;
        self.bus.write32(
            reg::csr(0),
            if setup_waiting {
                reg::FTF
            } else {
                reg::FTF | reg::OPC
            },
        );
        self.ep0 = Ep0::Idle;
        self.rx.clear();
        self.tx.clear();
        self.in_wait = [false; 6];
        self.dma = [false; 6];
        self.msc_reset = true;
    }

    /// SET_CONFIGURATION/SET_INTERFACE, already done by the hardware:
    /// record it, acknowledge the status stage and switch endpoint memory.
    fn config_change(&mut self) {
        self.stats.config_changes += 1;
        let udccr = self.bus.read32(reg::UDCCR);
        self.configuration = ((udccr >> reg::ACN_SHIFT) & 3) as u8;
        self.bus.write32(reg::csr(0), reg::AREN | reg::ACM);
        self.udccr(reg::SMAC, 0);
        self.ep0 = Ep0::Idle;
    }

    fn poll_ep0(&mut self) {
        let csr = self.bus.read32(reg::csr(0));
        let setup = reg::SA | reg::OPC | reg::RNE;
        if csr & setup == setup {
            self.setup();
            return;
        }
        match self.ep0 {
            Ep0::Idle => {
                // A stray status OUT (e.g. after an aborted transfer).
                if csr & reg::OPC != 0 {
                    self.bus.write32(reg::csr(0), reg::OPC);
                }
            }
            Ep0::InData => {
                // The next packet only once the previous one has left: the
                // packet-complete event, as an interrupt-driven driver would
                // wait for (IPR alone may read clear while a full FIFO is
                // still waiting for the host's IN token).
                let sent = self.bus.read32(reg::UDCISR0) & reg::EP0_PACKET != 0;
                if sent && csr & reg::IPR == 0 {
                    self.send_ep0_packet();
                }
            }
            Ep0::StatusOut => {
                if csr & reg::OPC != 0 {
                    self.bus.write32(reg::csr(0), reg::OPC);
                    self.ep0 = Ep0::Idle;
                }
            }
            Ep0::OutData => {
                if csr & reg::OPC != 0 {
                    self.receive_ep0_packet();
                }
            }
            Ep0::Stall => {
                if csr & reg::SST != 0 {
                    self.bus.write32(reg::csr(0), reg::SST);
                    self.ep0 = Ep0::Idle;
                }
            }
        }
    }

    fn setup(&mut self) {
        self.stats.setup_csr = self.bus.read32(reg::csr(0));
        let mut raw = [0u8; 8];
        for half in raw.as_chunks_mut::<4>().0 {
            half.copy_from_slice(&self.bus.read32(reg::dr(0)).to_le_bytes());
        }
        self.bus.write32(reg::csr(0), reg::SA | reg::OPC);
        let s = Setup::parse(raw);
        self.stats.setups += 1;
        self.stats.last_request = u16::from(s.request_type) << 8 | u16::from(s.request);
        self.stats.recent.copy_within(0..3, 1);
        self.stats.recent[0] = self.stats.last_request;
        let configuration = self.configuration;
        match self.cdc.handle(&s, configuration) {
            Reply::In(data) => {
                let n = data.len().min(self.ep0_buf.len());
                self.ep0_buf[..n].copy_from_slice(&data[..n]);
                self.ep0_len = n;
                self.ep0_pos = 0;
                self.ep0 = Ep0::InData;
                self.send_ep0_packet();
            }
            // Host-to-device request without data: software sends the
            // zero-length status IN (IPR). Measured on the real X51v: with
            // it, Windows opened the port (DTR arrived); without it, opening
            // failed with "device not functioning" (docs/hw/udc.md).
            Reply::Ack => {
                self.bus.write32(reg::csr(0), reg::IPR);
                self.ep0 = Ep0::Idle;
            }
            Reply::Out(n) => {
                self.ep0_len = n.min(self.ep0_buf.len());
                self.ep0_pos = 0;
                self.ep0 = Ep0::OutData;
            }
            Reply::Stall => {
                self.stats.stalls += 1;
                self.bus.write32(reg::csr(0), reg::FST | reg::FTF);
                self.ep0 = Ep0::Stall;
            }
        }
        if self.cdc.take_msc_reset() {
            self.msc_reset = true;
        }
    }

    /// Chooses the composite layout (serial port + drive, the default) or
    /// the serial-only fallback. Call while disabled; the host sees it at the
    /// next enumeration.
    pub fn set_composite(&mut self, on: bool) {
        self.cdc.set_composite(on);
    }

    /// Whether the composite layout is presented.
    #[must_use]
    pub fn composite(&self) -> bool {
        self.cdc.composite()
    }

    /// Sets the mass storage function's logical unit count (Get Max LUN).
    pub fn set_msc_luns(&mut self, luns: u8) {
        self.cdc.set_max_lun(luns.saturating_sub(1));
    }

    /// Whether a bus reset or a mass storage reset happened since the last
    /// call: the storage function abandons its command.
    pub fn take_msc_reset(&mut self) -> bool {
        core::mem::take(&mut self.msc_reset)
    }

    /// Takes the next mass storage OUT packet, if one arrived.
    pub fn msc_read(&mut self, buf: &mut [u8; usb::BULK_SIZE]) -> Option<usize> {
        if !self.configured() || self.dma[SLOT_MSC_OUT] {
            return None;
        }
        self.read_packet(SLOT_MSC_OUT, buf)
    }

    /// Reads the waiting OUT packet of `slot` into `buf` (at most 64 bytes)
    /// and releases its buffer; `None` if none waits. With two buffers the
    /// status and count can still be changing as the next packet completes
    /// (measured: full packets read with a short count, and lost when
    /// released unread), so both are read until two reads agree; a full
    /// packet's size then comes from the short-packet bit (clear: 64).
    fn read_packet(&mut self, slot: usize, buf: &mut [u8]) -> Option<usize> {
        let mut csr = self.bus.read32(reg::csr(slot));
        if csr & reg::PC == 0 {
            return None;
        }
        let mut bcr = (self.bus.read32(reg::bcr(slot)) & 0x3FF) as usize;
        for _ in 0..8 {
            let c = self.bus.read32(reg::csr(slot));
            let b = (self.bus.read32(reg::bcr(slot)) & 0x3FF) as usize;
            if (c ^ csr) & (reg::PC | reg::SP) == 0 && b == bcr {
                break;
            }
            self.stats.unsettled += 1;
            (csr, bcr) = (c, b);
        }
        let count = if csr & reg::SP == 0 {
            if bcr != usb::BULK_SIZE {
                self.stats.count_mismatch += 1;
                self.stats.last_mismatch = (csr & 0x3FF) << 16 | bcr as u32;
            }
            usb::BULK_SIZE
        } else {
            bcr
        }
        .min(buf.len())
        .min(usb::BULK_SIZE);
        let (words, rest) = buf[..count].as_chunks_mut::<4>();
        for w in words {
            *w = self.bus.read32(reg::dr(slot)).to_le_bytes();
        }
        if !rest.is_empty() {
            let w = self.bus.read32(reg::dr(slot)).to_le_bytes();
            let n = rest.len();
            rest.copy_from_slice(&w[..n]);
        }
        self.stats.rx_bytes += count as u32;
        self.csr_write(slot, reg::PC);
        Some(count)
    }

    /// Whether a bulk IN packet may be written to `slot`: a buffer is free
    /// (`BNF`; measured to read 0 while a short packet waits, so nothing is
    /// queued behind one) and, one packet in flight, the previous one's
    /// packet-complete has been seen (`BNF`/IPR alone don't prove a packet
    /// left, docs/hw/udc.md).
    fn in_room(&mut self, slot: usize) -> bool {
        let csr = self.bus.read32(reg::csr(slot));
        if csr & reg::PC != 0 {
            self.csr_write(slot, reg::PC);
            self.in_wait[slot] = false;
        }
        csr & reg::BNF != 0 && (self.in_streaming || !self.in_wait[slot])
    }

    /// A bulk endpoint's status write: `bits`, plus `DME` while DMA serves
    /// it (a write without it would turn DMA off).
    fn csr_write(&mut self, slot: usize, bits: u32) {
        let dme = if self.dma[slot] { reg::DME } else { 0 };
        self.bus.write32(reg::csr(slot), bits | dme);
    }

    /// The DMA port of a bulk endpoint: its FIFO (32-bit accesses) and
    /// request line. Request lines: endpoint 0 is 24, slot A 25, and so on
    /// (Toradex's PXA27x DMA library table; measured on the X51v by `udc
    /// dma-out` / `dma-in`, docs/hw/udc.md).
    #[must_use]
    pub const fn dma_port(ep: Bulk) -> Port {
        let slot = ep.slot();
        Port {
            request: 24 + slot as u8,
            addr: PHYS + reg::dr(slot) as u32,
            width: Width::Word,
        }
    }

    /// Hands a bulk endpoint to DMA (`on`) or back to the CPU. Arm the DMA
    /// channel first for OUT (the request comes as soon as a packet waits),
    /// after for IN. Stale packet-complete and NAK flags are cleared.
    pub fn set_dma(&mut self, ep: Bulk, on: bool) {
        let slot = ep.slot();
        self.dma[slot] = on;
        let stale = self.bus.read32(reg::csr(slot)) & (reg::PC | reg::TRN);
        let keep = if ep == Bulk::SerialOut || ep == Bulk::StorageOut {
            0
        } else {
            stale
        };
        self.csr_write(slot, keep);
    }

    /// A bulk endpoint's `UDCCSRx` (diagnostics).
    pub fn dma_csr(&mut self, ep: Bulk) -> u32 {
        self.bus.read32(reg::csr(ep.slot()))
    }

    /// Whether the serial port's bulk IN has nothing queued for the host
    /// that the driver knows of (its last packet's complete seen).
    pub fn serial_in_idle(&mut self) -> bool {
        self.in_room(SLOT_IN) && !self.in_wait[SLOT_IN]
    }

    /// After an IN DMA of `len` bytes ended: a partial last packet is sent
    /// (SP); full packets went by themselves. The endpoint stays with DMA
    /// until [`Udc::set_dma`] hands it back.
    pub fn dma_in_end(&mut self, ep: Bulk, len: usize) {
        let slot = ep.slot();
        self.in_wait[slot] = true;
        if !len.is_multiple_of(usb::BULK_SIZE) {
            self.csr_write(slot, reg::SP);
        }
    }

    /// Bulk IN into both buffers (`true`) or one packet in flight (the
    /// default, measured good). Diagnostics: streaming made Windows reset
    /// the device during drive reads.
    pub fn set_in_streaming(&mut self, on: bool) {
        self.in_streaming = on;
    }

    /// Writes one bulk IN packet (at most 64 bytes; shorter ends the
    /// transfer). Returns whether it was short.
    fn write_packet(&mut self, slot: usize, data: &[u8]) -> bool {
        // A NAKed IN token (TRN) is cleared before the packet goes in, as
        // the hardware's known-good driver does (write-1-to-clear).
        if self.bus.read32(reg::csr(slot)) & reg::TRN != 0 {
            self.csr_write(slot, reg::TRN);
        }
        let data = &data[..data.len().min(usb::BULK_SIZE)];
        let (words, rest) = data.as_chunks::<4>();
        for w in words {
            self.bus.write32(reg::dr(slot), u32::from_le_bytes(*w));
        }
        for &b in rest {
            self.bus.write8(reg::dr(slot), b);
        }
        self.stats.tx_bytes += data.len() as u32;
        self.in_wait[slot] = true;
        let short = data.len() < usb::BULK_SIZE;
        if short {
            self.csr_write(slot, reg::SP);
        }
        short
    }

    /// Whether a mass storage IN packet can be queued ([`Udc::in_room`]).
    pub fn msc_can_write(&mut self) -> bool {
        self.configured() && !self.dma[SLOT_MSC_IN] && self.in_room(SLOT_MSC_IN)
    }

    /// Queues one mass storage IN packet (at most 64 bytes; shorter, even
    /// empty, ends the transfer). Call only after [`Udc::msc_can_write`].
    pub fn msc_write(&mut self, data: &[u8]) {
        self.write_packet(SLOT_MSC_IN, data);
    }

    /// The serial port's bulk OUT endpoint: `UDCCSRx` and `UDCBCRx`, and
    /// bytes waiting in the receive ring (diagnostics).
    pub fn out_state(&mut self) -> (u32, u32, usize) {
        (
            self.bus.read32(reg::csr(SLOT_OUT)),
            self.bus.read32(reg::bcr(SLOT_OUT)),
            self.rx.len(),
        )
    }

    /// Bring-up probe (item 74): how the mass storage IN endpoint's status
    /// reads with 0, 1 and 2 packets queued, full and short, double
    /// buffered. Each entry is `UDCCSRx` (bits 9:0) | `UDCBCRx` << 16, after:
    /// a flush; one full packet; two; a flush; one short packet; the same
    /// read again (nothing is written behind a short packet: on the X51v
    /// `BNF` already reads 0); a flush; one full then one short; a flush. Run only while the
    /// storage function is idle (no command: the host sends no IN tokens
    /// then, so the packets stay queued); the endpoint is flushed after.
    pub fn probe_in_buffers(&mut self) -> [u32; 9] {
        let slot = SLOT_MSC_IN;
        let mut out = [0u32; 9];
        let mut at = 0;
        let mut note = |u: &mut Self| {
            out[at] = (u.bus.read32(reg::csr(slot)) & 0x3FF)
                | (u.bus.read32(reg::bcr(slot)) & 0x3FF) << 16;
            at += 1;
        };
        let full = [0x5Au8; usb::BULK_SIZE];
        let short = [0xA5u8; 12];
        self.bus.write32(reg::csr(slot), reg::FEF);
        note(self);
        self.write_packet(slot, &full);
        note(self);
        self.write_packet(slot, &full);
        note(self);
        self.bus.write32(reg::csr(slot), reg::FEF);
        note(self);
        self.write_packet(slot, &short);
        note(self);
        note(self);
        self.bus.write32(reg::csr(slot), reg::FEF);
        note(self);
        self.write_packet(slot, &full);
        self.write_packet(slot, &short);
        note(self);
        self.bus
            .write32(reg::csr(slot), reg::FEF | reg::PC | reg::TRN);
        note(self);
        self.in_wait[slot] = false;
        out
    }

    /// Writes the next endpoint 0 packet. A short packet (including an
    /// empty one after an exact multiple of 16) needs IPR and ends the data
    /// stage; a full one is sent by the hardware when the FIFO fills.
    fn send_ep0_packet(&mut self) {
        self.stats.ep0_in += 1;
        // Clear the packet-complete event; it's set again once this packet
        // has gone (polled by `poll_ep0`).
        self.bus.write32(reg::UDCISR0, reg::EP0_PACKET);
        let end = (self.ep0_pos + usb::EP0_SIZE).min(self.ep0_len);
        let chunk = self.ep0_pos..end;
        let n = chunk.len();
        let mut i = chunk.start;
        while i + 4 <= end {
            let w = u32::from_le_bytes([
                self.ep0_buf[i],
                self.ep0_buf[i + 1],
                self.ep0_buf[i + 2],
                self.ep0_buf[i + 3],
            ]);
            self.bus.write32(reg::dr(0), w);
            i += 4;
        }
        while i < end {
            self.bus.write8(reg::dr(0), self.ep0_buf[i]);
            i += 1;
        }
        self.ep0_pos = end;
        if n < usb::EP0_SIZE {
            self.bus.write32(reg::csr(0), reg::IPR);
            self.ep0 = Ep0::StatusOut;
        }
    }

    fn receive_ep0_packet(&mut self) {
        let count = (self.bus.read32(reg::bcr(0)) & 0x3FF) as usize;
        let mut got = 0;
        while got < count {
            let w = self.bus.read32(reg::dr(0)).to_le_bytes();
            for &b in &w[..(count - got).min(4)] {
                if self.ep0_pos < self.ep0_len {
                    self.ep0_buf[self.ep0_pos] = b;
                    self.ep0_pos += 1;
                }
            }
            got += 4;
        }
        self.bus.write32(reg::csr(0), reg::OPC);
        if self.ep0_pos >= self.ep0_len || count < usb::EP0_SIZE {
            let (len, buf) = (self.ep0_pos, self.ep0_buf);
            self.cdc.out_data(&buf[..len]);
            // Status stage: a zero-length IN, as for `Reply::Ack`.
            self.bus.write32(reg::csr(0), reg::IPR);
            self.ep0 = Ep0::Idle;
        }
    }

    /// Bulk OUT: every waiting packet (both buffers) while it fits in the
    /// receive ring (otherwise the hardware NAKs the host until there's
    /// room).
    fn poll_out(&mut self) {
        if self.dma[SLOT_OUT] {
            return;
        }
        let mut pkt = [0u8; usb::BULK_SIZE];
        for _ in 0..2 {
            let csr = self.bus.read32(reg::csr(SLOT_OUT));
            if csr & reg::PC == 0 || self.rx.space() < usb::BULK_SIZE {
                return;
            }
            let Some(n) = self.read_packet(SLOT_OUT, &mut pkt) else {
                return;
            };
            for &b in &pkt[..n] {
                self.rx.push(b);
            }
        }
    }

    /// Bulk IN: packets while [`Udc::in_room`] allows.
    fn poll_in(&mut self) {
        for _ in 0..2 {
            if self.tx.len() == 0 || self.dma[SLOT_IN] || !self.in_room(SLOT_IN) {
                return;
            }
            let n = self.tx.len().min(usb::BULK_SIZE);
            let mut pkt = [0u8; usb::BULK_SIZE];
            for b in &mut pkt[..n] {
                *b = self.tx.pop().unwrap_or(0);
            }
            self.write_packet(SLOT_IN, &pkt[..n]);
        }
    }
}

#[cfg(test)]
mod tests;
