//! The driver against a simulated UDC driven by a scripted host.
//!
//! `SimUdc` models the register behaviour in docs/hw/udc.md (write-1-to-
//! clear status bits, FIFOs, hardware-handled SET_CONFIGURATION), and `Host`
//! plays what Windows does when the device is plugged in: a short device
//! descriptor read, a reset, the full descriptors, the strings,
//! SET_CONFIGURATION, then the CDC requests a terminal program sends on open.

extern crate std;
use std::collections::VecDeque;
use std::vec::Vec;

use super::*;
use rp_hal_api::{RegisterBus, RegisterBus8};

#[derive(Default)]
struct SimUdc {
    udccr: u32,
    isr1: u32,
    icr1: u32,
    csr0: u32,
    ep0_rx: VecDeque<u8>,
    ep0_tx: Vec<u8>,
    /// A packet ready for the host's next IN token (full FIFO, or IPR).
    ep0_pending: Option<Vec<u8>>,
    cr: [u32; 24],
    /// The bulk endpoints, by slot.
    bulk: [Bulk; 7],
    /// OUT byte counts read 0 (as if the count lagged the packet-complete
    /// of a packet that just reached the front).
    bcr_reads_zero: bool,
    /// Every bulk status write: (slot, value).
    csr_writes: Vec<(usize, u32)>,
    acked_config: u32,
    smac: u32,
    /// Every raw write to UDCCR, to check the masking rule.
    udccr_writes: Vec<u32>,
    reject_table: bool,
    up2ocr: u32,
    up2ocr_writes: u32,
    isr0: u32,
    icr0: u32,
}

/// A bulk endpoint: one packet buffer, or two with `UDCCRx.DE`.
#[derive(Default)]
struct Bulk {
    /// IN: bytes written into the buffer being filled.
    fifo: Vec<u8>,
    /// IN: committed packets waiting for the host's IN tokens; OUT:
    /// packets received, the front one being read.
    queue: VecDeque<VecDeque<u8>>,
    /// Packet complete (IN: a packet was taken by the host).
    pc: bool,
    /// IN: what the host received.
    sent: VecDeque<Vec<u8>>,
}

impl SimUdc {
    /// Packet buffers of `slot`.
    fn depth(&self, slot: usize) -> usize {
        if self.cr[slot] & (1 << 1) != 0 { 2 } else { 1 }
    }

    fn bulk_write(&mut self, slot: usize, bytes: &[u8]) {
        let depth = self.depth(slot);
        let b = &mut self.bulk[slot];
        assert!(
            b.queue.len() < depth,
            "wrote into a bulk IN FIFO while every buffer waits for the host"
        );
        assert!(
            b.queue.iter().all(|p| p.len() == usb::BULK_SIZE),
            "wrote into a bulk IN FIFO behind a short packet"
        );
        b.fifo.extend_from_slice(bytes);
        assert!(b.fifo.len() <= usb::BULK_SIZE, "IN FIFO overrun");
        if b.fifo.len() == usb::BULK_SIZE {
            self.bulk_commit(slot);
        }
    }

    fn bulk_commit(&mut self, slot: usize) {
        let depth = self.depth(slot);
        let b = &mut self.bulk[slot];
        assert!(
            b.queue.len() < depth,
            "bulk IN packet queued with no free buffer"
        );
        let p = core::mem::take(&mut b.fifo);
        b.queue.push_back(p.into_iter().collect());
    }

    /// The host's IN token on `slot`: takes the oldest waiting packet.
    fn host_in_on(&mut self, slot: usize) {
        let b = &mut self.bulk[slot];
        if let Some(p) = b.queue.pop_front() {
            b.sent.push_back(p.into_iter().collect());
            b.pc = true;
            self.isr0 |= reg::packet_irq(slot);
        }
    }

    /// The host's OUT packet on `slot`: accepted while a buffer is free
    /// (false: NAKed, the host retries).
    fn host_out_on(&mut self, slot: usize, packet: &[u8]) -> bool {
        if self.bulk[slot].queue.len() >= self.depth(slot) {
            return false;
        }
        self.bulk[slot]
            .queue
            .push_back(packet.iter().copied().collect());
        self.isr0 |= reg::packet_irq(slot);
        true
    }

    /// The serial port's OUT packet.
    fn host_out(&mut self, packet: &[u8]) -> bool {
        self.host_out_on(SLOT_OUT, packet)
    }

    /// Whether an OUT packet still waits in the serial port's buffers.
    fn out_waiting(&self) -> usize {
        self.bulk[SLOT_OUT].queue.len()
    }

    /// A full FIFO (or IPR) makes a packet ready; it waits there until the
    /// host's next IN token. Pessimistic about IPR: it is not set by the
    /// hardware for a full FIFO, so a driver can only tell the packet left
    /// from the packet-complete event.
    fn ep0_packet_ready(&mut self) {
        if self.ep0_tx.len() >= usb::EP0_SIZE || self.csr0 & reg::IPR != 0 {
            assert!(
                self.ep0_pending.is_none(),
                "endpoint 0 packet queued before the previous one left"
            );
            self.ep0_pending = Some(core::mem::take(&mut self.ep0_tx));
        }
    }

    fn ep0_write(&mut self, bytes: &[u8]) {
        assert!(
            self.ep0_pending.is_none(),
            "wrote into endpoint 0's FIFO while a packet waits for the host"
        );
        self.ep0_tx.extend_from_slice(bytes);
        self.ep0_packet_ready();
    }

    /// The host's bulk IN token on the serial port.
    fn host_in_bulk(&mut self) {
        self.host_in_on(SLOT_IN);
    }

    /// The host's IN token: takes the waiting packet, if any.
    fn host_in(&mut self) -> Option<Vec<u8>> {
        let p = self.ep0_pending.take()?;
        self.csr0 &= !reg::IPR;
        self.isr0 |= reg::EP0_PACKET;
        Some(p)
    }
}

impl RegisterBus for SimUdc {
    fn read32(&mut self, offset: usize) -> u32 {
        match offset {
            reg::UDCCR => self.udccr,
            reg::UDCISR1 => self.isr1,
            reg::UP2OCR => self.up2ocr,
            reg::UDCISR0 => self.isr0,
            o if o == reg::csr(0) => {
                let rne = if self.ep0_rx.is_empty() { 0 } else { reg::RNE };
                self.csr0 | rne
            }
            o if o == reg::bcr(0) => self.ep0_rx.len() as u32,
            o if o == reg::dr(0) => {
                let mut w = [0u8; 4];
                for b in &mut w {
                    *b = self.ep0_rx.pop_front().unwrap_or(0);
                }
                u32::from_le_bytes(w)
            }
            o if (reg::csr(1)..=reg::csr(6)).contains(&o) => {
                let slot = (o - 0x100) / 4;
                let b = &self.bulk[slot];
                let pc = if b.pc { reg::PC } else { 0 };
                if self.cr[slot] & (1 << 12) != 0 {
                    // IN: "buffer not full" while a packet buffer is free,
                    // and (measured on the X51v) not while a short packet
                    // waits.
                    let short = b.queue.iter().any(|p| p.len() < usb::BULK_SIZE);
                    let bnf = if b.queue.len() < self.depth(slot) && !short {
                        reg::BNF
                    } else {
                        0
                    };
                    bnf | pc
                } else {
                    match b.queue.front() {
                        None => 0,
                        Some(p) if p.len() < usb::BULK_SIZE => reg::PC | reg::BNF | reg::SP,
                        Some(_) => reg::PC | reg::BNF,
                    }
                }
            }
            o if (reg::bcr(1)..=reg::bcr(6)).contains(&o) => {
                let slot = (o - 0x200) / 4;
                if self.bcr_reads_zero {
                    return 0;
                }
                self.bulk[slot].queue.front().map_or(0, |p| p.len() as u32)
            }
            o if (reg::dr(1)..=reg::dr(6)).contains(&o) => {
                let slot = (o - 0x300) / 4;
                let mut w = [0u8; 4];
                if let Some(p) = self.bulk[slot].queue.front_mut() {
                    for b in &mut w {
                        *b = p.pop_front().unwrap_or(0);
                    }
                }
                u32::from_le_bytes(w)
            }
            _ => 0,
        }
    }

    fn write32(&mut self, offset: usize, v: u32) {
        match offset {
            reg::UDCCR => {
                self.udccr_writes.push(v);
                if v & reg::SMAC != 0 {
                    self.smac += 1;
                }
                let enabling = v & reg::UDE != 0 && self.udccr & reg::UDE == 0;
                self.udccr = (self.udccr & !reg::UDE) | (v & reg::UDE);
                if enabling && self.reject_table {
                    self.udccr |= reg::EMCE;
                }
            }
            reg::UDCICR1 => self.icr1 = v,
            reg::UDCICR0 => self.icr0 = v,
            reg::UP2OCR => {
                assert_eq!(
                    self.udccr & reg::UDE,
                    0,
                    "UP2OCR written while the UDC runs"
                );
                self.up2ocr = v;
                self.up2ocr_writes += 1;
            }
            reg::UDCISR1 => self.isr1 &= !v,
            reg::UDCISR0 => self.isr0 &= !v,
            o if o == reg::csr(0) => {
                // Write-1-to-clear status, actions, and IPR/FST set by software.
                self.csr0 &= !(v & (reg::OPC | reg::SA | reg::SST));
                if v & reg::FTF != 0 {
                    self.ep0_tx.clear();
                    self.ep0_pending = None;
                }
                if v & reg::FST != 0 {
                    // The host's next token gets a STALL.
                    self.csr0 |= reg::SST;
                }
                if v & reg::AREN != 0 {
                    self.acked_config += 1;
                }
                if v & reg::IPR != 0 {
                    self.csr0 |= reg::IPR;
                    self.ep0_packet_ready();
                }
            }
            o if o == reg::dr(0) => self.ep0_write(&v.to_le_bytes()),
            o if (reg::cr(1)..=reg::cr(23)).contains(&o) => {
                assert_eq!(
                    self.udccr & reg::UDE,
                    0,
                    "endpoint table written while enabled"
                );
                self.cr[(o - 0x400) / 4] = v;
            }
            o if (reg::dr(1)..=reg::dr(6)).contains(&o) => {
                self.bulk_write((o - 0x300) / 4, &v.to_le_bytes());
            }
            o if (reg::csr(1)..=reg::csr(6)).contains(&o) => {
                let slot = (o - 0x100) / 4;
                self.csr_writes.push((slot, v));
                let is_in = self.cr[slot] & (1 << 12) != 0;
                if v & reg::PC != 0 {
                    if is_in {
                        self.bulk[slot].pc = false;
                    } else {
                        // Releases the packet just read; the next (if
                        // any) is ready at once.
                        self.bulk[slot].queue.pop_front();
                    }
                }
                if v & reg::FEF != 0 {
                    self.bulk[slot].fifo.clear();
                    self.bulk[slot].queue.clear();
                }
                if v & reg::SP != 0 && is_in {
                    self.bulk_commit(slot);
                }
            }
            _ => {}
        }
    }
}

impl RegisterBus8 for SimUdc {
    fn read8(&mut self, _offset: usize) -> u8 {
        0
    }
    fn write8(&mut self, offset: usize, v: u8) {
        if offset == reg::dr(0) {
            self.ep0_write(&[v]);
        } else if (reg::dr(1)..=reg::dr(6)).contains(&offset) {
            self.bulk_write((offset - 0x300) / 4, &[v]);
        }
    }
}

/// The scripted host, driving the device through `udc.poll()`.
struct Host {
    udc: Udc<SimUdc>,
}

impl Host {
    fn new() -> Self {
        let mut udc = Udc::new(SimUdc::default());
        udc.enable().unwrap();
        Self { udc }
    }

    fn sim(&mut self) -> &mut SimUdc {
        &mut self.udc.bus
    }

    fn poll(&mut self, n: usize) {
        for _ in 0..n {
            self.udc.poll();
        }
    }

    fn reset(&mut self) {
        self.sim().isr1 |= reg::RESET;
        self.poll(1);
    }

    fn setup(&mut self, s: [u8; 8]) {
        let sim = self.sim();
        sim.ep0_rx.extend(s);
        sim.csr0 |= reg::SA | reg::OPC;
        self.poll(1);
    }

    /// A control read: SETUP, collect IN packets until a short one, then
    /// the status OUT. `None` if the device stalled.
    fn control_in(&mut self, s: [u8; 8]) -> Option<Vec<u8>> {
        self.setup(s);
        let mut data = Vec::new();
        for _ in 0..64 {
            if self.sim().csr0 & reg::SST != 0 {
                self.poll(1);
                assert_eq!(self.sim().csr0 & reg::SST, 0, "SST not cleared");
                return None;
            }
            if let Some(p) = self.sim().host_in() {
                let short = p.len() < usb::EP0_SIZE;
                data.extend(p);
                if short {
                    // Status stage: zero-length OUT.
                    self.sim().csr0 |= reg::OPC;
                    self.poll(2);
                    assert_eq!(self.sim().csr0 & reg::OPC, 0, "status OUT not acknowledged");
                    return Some(data);
                }
            }
            // The device polls far more often than the host sends IN tokens.
            self.poll(3);
        }
        panic!("control read never finished");
    }

    /// A control write with no data: expects the zero-length status IN.
    /// The PXA27x sends the status stage of a host-to-device request by
    /// itself: the driver must not queue a packet of its own (docs/hw/udc.md).
    /// The device sends the zero-length status IN of a host-to-device
    /// request (measured on the real X51v, docs/hw/udc.md).
    fn control_ack(&mut self, s: [u8; 8]) {
        self.setup(s);
        self.poll(2);
        let p = self.sim().host_in().expect("no status IN");
        assert!(p.is_empty(), "status IN carried data");
    }

    /// A control write with a data stage.
    fn control_out(&mut self, s: [u8; 8], data: &[u8]) {
        self.setup(s);
        for chunk in data.chunks(usb::EP0_SIZE) {
            self.sim().ep0_rx.extend(chunk);
            self.sim().csr0 |= reg::OPC;
            self.poll(1);
        }
        self.poll(2);
        let p = self.sim().host_in().expect("no status IN");
        assert!(p.is_empty(), "status IN carried data");
    }

    /// What the hardware does on SET_CONFIGURATION(1).
    fn set_configuration(&mut self) {
        let sim = self.sim();
        sim.udccr |= 1 << reg::ACN_SHIFT;
        sim.isr1 |= reg::CONFIG_CHANGE;
        self.poll(1);
    }

    fn enumerate(&mut self) {
        self.reset();
        // Windows: first 64 bytes of the device descriptor, reset, address.
        let d = self.control_in([0x80, 6, 0, 1, 0, 0, 64, 0]).unwrap();
        assert_eq!(d, usb::DEVICE);
        self.reset();
        let d = self.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap();
        assert_eq!(d, usb::DEVICE);
        let c = self.control_in([0x80, 6, 0, 2, 0, 0, 9, 0]).unwrap();
        assert_eq!(c, usb::CONFIG[..9]);
        let total = usize::from(u16::from_le_bytes([c[2], c[3]]));
        let c = self.control_in([0x80, 6, 0, 2, 0, 0, 255, 0]).unwrap();
        assert_eq!(c.len(), total);
        assert_eq!(c, usb::CONFIG);
        for i in 0..4u8 {
            let s = self.control_in([0x80, 6, i, 3, 9, 4, 255, 0]).unwrap();
            assert_eq!(usize::from(s[0]), s.len());
            assert_eq!(s[1], 3);
        }
        // Full speed only: the device qualifier request stalls.
        assert!(self.control_in([0x80, 6, 0, 6, 0, 0, 10, 0]).is_none());
        self.set_configuration();
    }
}

#[test]
fn enable_writes_the_table_then_enables() {
    let h = Host::new();
    let sim = &h.udc.bus;
    assert_eq!(sim.udccr & reg::UDE, reg::UDE);
    // All in configuration 1, interface 0 (the PXA27x activates only the
    // active interface's endpoints). Slot A: EP3 interrupt IN, 16 bytes.
    assert_eq!(
        sim.cr[1],
        (1 << 25) | (3 << 15) | (3 << 13) | (1 << 12) | (16 << 2) | 1
    );
    // Slot B: EP2 bulk IN, 64. Slot C: EP1 bulk OUT, 64. Bulk endpoints
    // double buffered (DE, bit 1).
    assert_eq!(
        sim.cr[2],
        (1 << 25) | (2 << 15) | (2 << 13) | (1 << 12) | (64 << 2) | 2 | 1
    );
    assert_eq!(
        sim.cr[3],
        (1 << 25) | (1 << 15) | (2 << 13) | (64 << 2) | 2 | 1
    );
    assert_eq!(sim.cr[1] & 2, 0, "the interrupt endpoint stays single");
    for (slot, cr) in sim.cr.iter().enumerate().take(4).skip(1) {
        assert_eq!((cr >> 22) & 7, 0, "slot {slot} not in interface 0");
    }
    assert_ne!(sim.icr1 & reg::CONFIG_CHANGE, 0);
}

#[test]
fn the_transceiver_output_is_turned_on_only_when_off() {
    // As Windows Mobile leaves it: left alone.
    let mut udc = Udc::new(SimUdc {
        up2ocr: reg::HXOE,
        ..SimUdc::default()
    });
    udc.enable().unwrap();
    assert_eq!(udc.bus.up2ocr_writes, 0);
    // Off (e.g. a cold start without WM): turned on, other bits kept.
    let mut udc = Udc::new(SimUdc {
        up2ocr: 1 << 24,
        ..SimUdc::default()
    });
    udc.enable().unwrap();
    assert_eq!(udc.bus.up2ocr, (1 << 24) | reg::HXOE);
}

#[test]
fn udccr_writes_carry_only_writable_bits() {
    let mut h = Host::new();
    h.sim().udccr |= reg::EMCE | (3 << 8); // read-only/status bits set
    h.enumerate();
    for w in &h.udc.bus.udccr_writes {
        assert_eq!(w & !reg::UDCCR_WRITABLE, 0, "UDCCR write {w:#x}");
    }
}

#[test]
fn a_rejected_table_is_reported_and_the_udc_left_off() {
    let mut udc = Udc::new(SimUdc {
        reject_table: true,
        ..SimUdc::default()
    });
    assert_eq!(udc.enable(), Err(Error::EndpointTable));
    assert_eq!(udc.bus.udccr & reg::UDE, 0);
}

#[test]
fn enumerates_like_windows() {
    let mut h = Host::new();
    h.enumerate();
    assert!(h.udc.configured());
    assert_eq!(h.sim().acked_config, 1);
    assert_eq!(h.sim().smac, 1);
    let s = h.udc.stats();
    assert_eq!(s.resets, 2);
    assert_eq!(s.config_changes, 1);
    assert_eq!(s.stalls, 1);
}

#[test]
fn reads_split_into_16_byte_packets_with_a_short_last_one() {
    let mut h = Host::new();
    h.reset();
    h.setup([0x80, 6, 0, 2, 0, 0, 255, 0]);
    let mut sizes = Vec::new();
    for _ in 0..20 {
        while let Some(p) = h.sim().host_in() {
            sizes.push(p.len());
        }
        h.poll(3);
    }
    // 98 bytes: six full packets, then 2.
    assert_eq!(sizes, [16, 16, 16, 16, 16, 16, 2]);
}

#[test]
fn an_exact_multiple_of_16_ends_with_a_zero_length_packet() {
    let mut h = Host::new();
    h.reset();
    // Ask for exactly 32 bytes of the configuration descriptor.
    let d = h.control_in([0x80, 6, 0, 2, 0, 0, 32, 0]).unwrap();
    assert_eq!(d, usb::CONFIG[..32]);
}

#[test]
fn cdc_requests_on_open() {
    let mut h = Host::new();
    h.enumerate();
    // What usbser.sys sends when a program opens the port.
    let coding = [0x80, 0x25, 0, 0, 0, 0, 8]; // 9600 8N1
    h.control_out([0x21, 0x20, 0, 0, 0, 0, 7, 0], &coding);
    assert_eq!(h.udc.cdc.baud(), 9600);
    let back = h.control_in([0xA1, 0x21, 0, 0, 0, 0, 7, 0]).unwrap();
    assert_eq!(back, coding);
    h.control_ack([0x21, 0x22, 3, 0, 0, 0, 0, 0]); // DTR + RTS
    assert!(h.udc.host_open());
    h.control_ack([0x21, 0x22, 0, 0, 0, 0, 0, 0]);
    assert!(!h.udc.host_open());
}

#[test]
fn unknown_requests_stall_and_recover() {
    let mut h = Host::new();
    h.enumerate();
    assert!(h.control_in([0xC0, 0x55, 0, 0, 0, 0, 4, 0]).is_none());
    // The next request works normally.
    let d = h.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap();
    assert_eq!(d, usb::DEVICE);
}

#[test]
fn standard_status_requests() {
    let mut h = Host::new();
    h.enumerate();
    assert_eq!(h.control_in([0x80, 0, 0, 0, 0, 0, 2, 0]).unwrap(), [1, 0]); // self-powered
    assert_eq!(
        h.control_in([0x82, 0, 0, 0, 0x82, 0, 2, 0]).unwrap(),
        [0, 0]
    );
    assert_eq!(h.control_in([0x80, 8, 0, 0, 0, 0, 1, 0]).unwrap(), [1]);
    h.control_ack([0x02, 1, 0, 0, 0x82, 0, 0, 0]); // CLEAR_FEATURE(halt)
}

#[test]
fn nothing_is_queued_before_configuration() {
    let mut h = Host::new();
    assert_eq!(h.udc.write(b"early"), 0);
    h.enumerate();
    assert_eq!(h.udc.write(b"late"), 4);
}

#[test]
fn bulk_in_sends_full_packets_then_a_short_one() {
    let mut h = Host::new();
    h.enumerate();
    let data: Vec<u8> = (0..150u8).collect();
    assert_eq!(h.udc.write(&data), 150);
    // The device polls several times between the host's IN tokens.
    for _ in 0..10 {
        h.poll(3);
        h.sim().host_in_bulk();
    }
    let sent: Vec<Vec<u8>> = h.sim().bulk[SLOT_IN].sent.drain(..).collect();
    assert_eq!(sent.iter().map(Vec::len).collect::<Vec<_>>(), [64, 64, 22]);
    assert_eq!(sent.concat(), data);
    assert_eq!(h.udc.stats().tx_bytes, 150);
}

#[test]
fn bulk_out_is_received_in_order() {
    let mut h = Host::new();
    h.enumerate();
    let mut got = Vec::new();
    for packet in [&b"hello, "[..], &[b'x'; 64][..], b"axim"] {
        assert!(h.sim().host_out(packet));
        h.poll(1);
        assert_eq!(h.sim().out_waiting(), 0, "packet not released");
        let mut buf = [0u8; 128];
        let n = h.udc.read(&mut buf);
        got.extend_from_slice(&buf[..n]);
    }
    let mut want = b"hello, ".to_vec();
    want.extend([b'x'; 64]);
    want.extend(b"axim");
    assert_eq!(got, want);
}

#[test]
fn a_full_receive_ring_holds_the_host_off() {
    let mut h = Host::new();
    h.enumerate();
    // Fill the 256-byte ring with four 64-byte packets, then offer more:
    // the fifth and sixth stay in the two packet buffers, the seventh is
    // NAKed (the host retries) until there's room.
    for _ in 0..4 {
        assert!(h.sim().host_out(&[7u8; 64]));
        h.poll(1);
    }
    assert!(h.sim().host_out(&[7u8; 64]));
    assert!(h.sim().host_out(&[7u8; 64]));
    assert!(!h.sim().host_out(&[7u8; 64]), "both buffers full: NAK");
    h.poll(1);
    assert_eq!(h.sim().out_waiting(), 2);
    let mut buf = [0u8; 63];
    assert_eq!(h.udc.read(&mut buf), 63);
    h.poll(1);
    assert_eq!(
        h.sim().out_waiting(),
        2,
        "63 bytes of room hold a packet off"
    );
    let mut buf = [0u8; 1];
    assert_eq!(h.udc.read(&mut buf), 1);
    h.poll(1);
    assert_eq!(h.sim().out_waiting(), 1);
}

#[test]
fn a_bus_reset_drops_the_configuration_and_queued_data() {
    let mut h = Host::new();
    h.enumerate();
    h.udc.write(b"pending");
    h.reset();
    assert!(!h.udc.configured());
    h.enumerate();
    for _ in 0..4 {
        h.poll(3);
        h.sim().host_in_bulk();
    }
    assert!(
        h.sim().bulk[SLOT_IN].sent.is_empty(),
        "stale bytes sent after reset"
    );
}

#[test]
fn descriptors_are_consistent() {
    let c = &usb::CONFIG;
    assert_eq!(usize::from(u16::from_le_bytes([c[2], c[3]])), c.len());
    // Walk the descriptors: lengths chain exactly to the end.
    let mut i = 0;
    let mut endpoints = Vec::new();
    while i < c.len() {
        let len = usize::from(c[i]);
        assert!(len >= 2);
        if c[i + 1] == 5 {
            endpoints.push((c[i + 2], c[i + 3], u16::from_le_bytes([c[i + 4], c[i + 5]])));
        }
        i += len;
    }
    assert_eq!(i, c.len());
    // The endpoint table matches the descriptors.
    assert_eq!(
        endpoints,
        [
            (0x83, 3, 16),
            (0x01, 2, 64),
            (0x82, 2, 64),
            (0x04, 2, 64),
            (0x85, 2, 64)
        ]
    );
    assert_eq!(c[4], 3, "three interfaces");
    assert_eq!(&c[9..13], &[8, 0x0B, 0, 2], "IAD over interfaces 0-1");
    assert_eq!(&usb::DEVICE[4..7], &[0xEF, 0x02, 0x01]);
    // The serial number: 16 upper-case hex digits (Bulk-Only Transport).
    let serial: Vec<u8> = usb::SERIAL[2..].iter().step_by(2).copied().collect();
    assert_eq!(serial.len(), 16);
    assert!(
        serial
            .iter()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_lowercase())
    );
    for (slot, cfg) in TABLE {
        let number = ((cfg >> 15) & 0xF) as u8;
        let is_in = cfg & (1 << 12) != 0;
        let size = ((cfg >> 2) & 0x3FF) as u16;
        let addr = number | if is_in { 0x80 } else { 0 };
        assert!(
            endpoints.iter().any(|&(a, _, s)| a == addr && s == size),
            "slot {slot}"
        );
    }
    assert_eq!(usb::PRODUCT[0] as usize, usb::PRODUCT.len());
    assert_eq!(&usb::PRODUCT[2..6], &[b'R', 0, b'e', 0]);
}

#[test]
fn a_setup_that_arrives_before_the_reset_is_handled_survives_it() {
    let mut h = Host::new();
    h.reset();
    // The host resets and, too soon, sends GET_DESCRIPTOR before the
    // driver has seen the reset.
    h.sim().isr1 |= reg::RESET;
    let d = h.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap();
    assert_eq!(d, usb::DEVICE);
}

#[test]
fn the_serial_only_fallback_is_the_old_layout() {
    // Derived from the composite configuration at compile time; check it is
    // exactly the pre-ADR-0014 layout: two interfaces, no IAD, three
    // endpoints, class CDC, revision 26.00, the old serial number.
    let c = &usb::CONFIG_SERIAL;
    assert_eq!(c.len(), 67);
    assert_eq!(usize::from(u16::from_le_bytes([c[2], c[3]])), 67);
    assert_eq!(c[4], 2);
    let mut i = 0;
    let mut kinds = Vec::new();
    while i < c.len() {
        kinds.push(c[i + 1]);
        i += usize::from(c[i]);
    }
    assert_eq!(i, c.len());
    assert!(!kinds.contains(&0x0B), "no IAD");
    assert_eq!(kinds.iter().filter(|&&k| k == 5).count(), 3);
    assert_eq!(&usb::DEVICE_SERIAL[4..7], &[0x02, 0, 0]);
    assert_eq!(&usb::DEVICE_SERIAL[12..14], &[0x00, 0x26]);
    let mut h = Host::new();
    h.udc.set_composite(false);
    h.reset();
    assert_eq!(
        h.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap(),
        usb::DEVICE_SERIAL
    );
    assert_eq!(
        h.control_in([0x80, 6, 0, 2, 0, 0, 255, 0]).unwrap(),
        usb::CONFIG_SERIAL
    );
    assert_eq!(
        h.control_in([0x80, 6, 3, 3, 9, 4, 255, 0]).unwrap(),
        usb::SERIAL_LEGACY
    );
    // No storage requests in the serial-only layout.
    assert!(h.control_in([0xA1, 0xFE, 0, 0, 2, 0, 1, 0]).is_none());
}

#[test]
fn mass_storage_class_requests() {
    let mut h = Host::new();
    h.reset();
    h.udc.set_msc_luns(2);
    // Get Max LUN on the storage interface: 1 (two units).
    let d = h.control_in([0xA1, 0xFE, 0, 0, 2, 0, 1, 0]).unwrap();
    assert_eq!(d, [1]);
    // ... but not on the serial port's interfaces: STALL.
    assert!(h.control_in([0xA1, 0xFE, 0, 0, 0, 0, 1, 0]).is_none());
    // Bulk-Only Mass Storage Reset: acknowledged and reported once.
    // The bus reset at attach is reported first (the function starts clean).
    assert!(h.udc.take_msc_reset());
    h.setup([0x21, 0xFF, 0, 0, 2, 0, 0, 0]);
    h.poll(3);
    assert!(h.udc.take_msc_reset());
    assert!(!h.udc.take_msc_reset());
}

/// Hostile input: random SETUP packets (and random OUT data) never panic,
/// and no reply is longer than the host asked for.
#[test]
fn random_setup_packets_never_panic_or_overrun() {
    let mut x: u64 = 0x0C0F_FEE5;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut cdc = usb::Cdc::new();
    for i in 0..50_000u32 {
        let mut b = [0u8; 8];
        for v in &mut b {
            *v = rnd() as u8;
        }
        // Bias towards the request types the code handles.
        if i % 3 == 0 {
            b[0] = [0x80, 0x81, 0x82, 0x00, 0x21, 0xA1][(rnd() % 6) as usize];
            b[1] = [0, 1, 3, 6, 8, 10, 0x20, 0x21, 0x22, 0x23, 0xFE, 0xFF][(rnd() % 12) as usize];
        }
        let s = usb::Setup::parse(b);
        cdc.set_composite(rnd() % 2 == 0);
        match cdc.handle(&s, 1) {
            usb::Reply::In(d) => assert!(d.len() <= usize::from(s.length), "{s:?}"),
            usb::Reply::Out(n) => {
                assert!(n <= usize::from(s.length));
                let data: Vec<u8> = (0..n).map(|_| rnd() as u8).collect();
                cdc.out_data(&data);
            }
            usb::Reply::Ack | usb::Reply::Stall => {}
        }
    }
    // And through the whole driver, with the simulated controller.
    let mut h = Host::new();
    h.reset();
    for _ in 0..2000 {
        let mut b = [0u8; 8];
        for v in &mut b {
            *v = rnd() as u8;
        }
        b[6] = (rnd() % 140) as u8; // wLength up to past the buffer
        b[7] = 0;
        if b[0] & 0x80 != 0 {
            let _ = h.control_in(b);
        } else {
            h.setup(b);
            h.poll(3);
        }
    }
}

/// The CPU keeps one bulk IN packet in flight (two buffers or not): the
/// next only after the previous one's packet-complete.
#[test]
fn bulk_in_keeps_one_packet_in_flight() {
    let mut h = Host::new();
    h.enumerate();
    let data: Vec<u8> = (0..=199u8).collect();
    assert_eq!(h.udc.write(&data), 200);
    for _ in 0..3 {
        h.poll(1);
        assert_eq!(h.sim().bulk[SLOT_IN].queue.len(), 1);
        h.sim().host_in_bulk();
    }
    for _ in 0..4 {
        h.poll(1);
        h.sim().host_in_bulk();
    }
    let sent: Vec<Vec<u8>> = h.sim().bulk[SLOT_IN].sent.drain(..).collect();
    assert_eq!(sent.concat(), data);
}

/// With the ring empty, `read` takes packets straight from both buffers.
#[test]
fn reads_take_waiting_packets_straight_from_the_fifo() {
    let mut h = Host::new();
    h.enumerate();
    let a: Vec<u8> = (0..64u8).collect();
    let b: Vec<u8> = (64..128u8).collect();
    assert!(h.sim().host_out(&a));
    assert!(h.sim().host_out(&b));
    let mut buf = [0u8; 256];
    assert_eq!(h.udc.read(&mut buf), 128);
    assert_eq!(&buf[..64], &a[..]);
    assert_eq!(&buf[64..128], &b[..]);
    assert_eq!(h.sim().out_waiting(), 0);
    assert_eq!(h.udc.stats().rx_bytes, 128);
}

/// Soak: random host timing on all four bulk endpoints (IN tokens, OUT
/// packets, polls, reads in odd sizes) never loses, reorders or duplicates
/// a byte in either direction, on either function.
#[test]
fn random_interleaving_keeps_both_streams_intact() {
    let mut x: u32 = 0xB0B5_1EED;
    let mut rnd = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut h = Host::new();
    h.enumerate();
    let total = 40_000usize;
    let down: Vec<u8> = (0..total).map(|i| (i * 7 + i / 251) as u8).collect();
    let up: Vec<u8> = (0..total).map(|i| (i * 13 + i / 97) as u8).collect();
    let (mut sent_down, mut got_down) = (0, Vec::new());
    let (mut sent_up, mut msc_down, mut msc_up) = (0, 0usize, Vec::new());
    let mut msc_got = Vec::new();
    let mut pkt = [0u8; 64];
    for _ in 0..200_000 {
        match rnd() % 8 {
            0 | 1 if sent_down < total => {
                let n = ((rnd() % 64) as usize + 1).min(total - sent_down);
                if h.sim().host_out(&down[sent_down..sent_down + n]) {
                    sent_down += n;
                }
            }
            2 => h.sim().host_in_bulk(),
            3 => h.udc.poll(),
            4 => {
                let mut buf = [0u8; 200];
                let want = (rnd() % 200) as usize + 1;
                let n = h.udc.read(&mut buf[..want]);
                got_down.extend_from_slice(&buf[..n]);
            }
            5 if sent_up < total => {
                let n = ((rnd() % 300) as usize).min(total - sent_up);
                sent_up += h.udc.write(&up[sent_up..sent_up + n]);
            }
            6 => {
                // Mass storage: full packets out, then echoed back in.
                if msc_down < total
                    && h.sim()
                        .host_out_on(SLOT_MSC_OUT, &down[msc_down..msc_down + 64])
                {
                    msc_down += 64;
                }
                if let Some(n) = h.udc.msc_read(&mut pkt) {
                    msc_got.extend_from_slice(&pkt[..n]);
                }
            }
            _ => {
                h.sim().host_in_on(SLOT_MSC_IN);
                if msc_up.len() < msc_got.len() && h.udc.msc_can_write() {
                    let at = msc_up.len();
                    let n = (msc_got.len() - at).min(64);
                    let chunk = msc_got[at..at + n].to_vec();
                    h.udc.msc_write(&chunk);
                    msc_up.extend_from_slice(&chunk);
                }
            }
        }
    }
    for _ in 0..2000 {
        h.udc.poll();
        h.sim().host_in_bulk();
        let mut buf = [0u8; 256];
        let n = h.udc.read(&mut buf);
        got_down.extend_from_slice(&buf[..n]);
    }
    assert!(
        sent_down > total / 2 && sent_up > total / 2,
        "soak too short"
    );
    assert_eq!(got_down, down[..sent_down]);
    let host_up: Vec<u8> = h.sim().bulk[SLOT_IN].sent.drain(..).flatten().collect();
    assert_eq!(host_up, up[..sent_up]);
    assert_eq!(msc_got, down[..msc_down]);
    let msc_host: Vec<u8> = h.sim().bulk[SLOT_MSC_IN].sent.drain(..).flatten().collect();
    assert_eq!(msc_host, msc_up[..msc_host.len()]);
    assert!(msc_host.len() > total / 4);
}

/// A byte count that reads short for a full packet (the double-buffered
/// OUT race suspected on the device) loses nothing: a full packet's size
/// comes from the short-packet bit, and the disagreement is counted.
#[test]
fn a_lagging_byte_count_loses_no_data() {
    let mut h = Host::new();
    h.enumerate();
    h.sim().bcr_reads_zero = true;
    let data: Vec<u8> = (0..128u8).collect();
    assert!(h.sim().host_out(&data[..64]));
    assert!(h.sim().host_out(&data[64..]));
    let mut buf = [0u8; 256];
    assert_eq!(h.udc.read(&mut buf), 128);
    assert_eq!(&buf[..128], &data[..]);
    assert_eq!(h.udc.stats().count_mismatch, 2);
    // A real short packet still reads its count.
    h.sim().bcr_reads_zero = false;
    assert!(h.sim().host_out(b"tail"));
    h.poll(1);
    assert_eq!(h.udc.read(&mut buf), 4);
    assert_eq!(&buf[..4], b"tail");
}

/// An endpoint handed to DMA is left alone by the CPU paths (its packets
/// stay for the DMA controller), every status write to it keeps `DME`, and
/// handing it back resumes the CPU paths. Ports: slot A is request 25.
#[test]
fn an_endpoint_under_dma_is_left_to_the_dma_controller() {
    let mut h = Host::new();
    h.enumerate();
    let p = Udc::<SimUdc>::dma_port(super::Bulk::SerialOut);
    assert_eq!(p.request, 24 + SLOT_OUT as u8);
    assert_eq!(p.addr, 0x4060_0300 + 4 * SLOT_OUT as u32);
    h.udc.set_dma(super::Bulk::SerialOut, true);
    assert!(h.sim().host_out(&[1u8; 64]));
    h.poll(3);
    let mut buf = [0u8; 256];
    assert_eq!(h.udc.read(&mut buf), 0);
    assert_eq!(h.sim().out_waiting(), 1, "the CPU took a DMA packet");
    h.udc.set_dma(super::Bulk::SerialOut, false);
    h.poll(1);
    assert_eq!(h.udc.read(&mut buf), 64);
    // IN: nothing written by the CPU while DMA has it; SP for a partial
    // last packet carries DME.
    h.udc.set_dma(super::Bulk::StorageIn, true);
    assert!(!h.udc.msc_can_write());
    h.sim().csr_writes.clear();
    h.udc.dma_in_end(super::Bulk::StorageIn, 100);
    assert_eq!(h.sim().csr_writes, [(SLOT_MSC_IN, reg::SP | reg::DME)]);
    h.sim().csr_writes.clear();
    h.udc.dma_in_end(super::Bulk::StorageIn, 128);
    assert!(h.sim().csr_writes.is_empty(), "whole packets need no SP");
    h.udc.set_dma(super::Bulk::StorageIn, false);
    assert!(h.sim().csr_writes.iter().all(|&(_, v)| v & reg::DME == 0));
}

impl SimUdc {
    /// Whether the USB client line is raised (an enabled event pending).
    fn line(&self) -> bool {
        self.isr0 & self.icr0 != 0 || self.isr1 & self.icr1 != 0
    }
}

/// Interrupt-driven (ADR-0007): packets raise the line, one poll serves
/// them and lowers it, so the owner can sleep until the next one; polled,
/// bulk packets don't raise it. Endpoint 0's event survives a poll while
/// its state machine still waits on it.
#[test]
fn interrupts_raise_the_line_until_a_poll_serves_them() {
    let mut h = Host::new();
    h.enumerate();
    h.poll(2);
    assert!(!h.sim().line(), "quiet after enumeration");
    assert!(h.sim().host_out(b"polled"));
    assert!(!h.sim().line(), "bulk events are off while polled");
    h.poll(1);
    let mut buf = [0u8; 64];
    assert_eq!(h.udc.read(&mut buf), 6);
    h.udc.set_interrupts(true);
    assert!(h.sim().host_out(b"hello"));
    assert!(h.sim().line(), "an OUT packet raises the line");
    h.poll(1);
    assert!(!h.sim().line(), "the poll served it");
    assert_eq!(h.udc.read(&mut buf), 5);
    h.udc.write(b"reply");
    h.poll(1);
    h.sim().host_in_bulk();
    assert!(h.sim().line(), "a packet taken raises the line");
    h.poll(1);
    assert!(!h.sim().line());
    // A descriptor read longer than one 16-byte packet still completes.
    let d = h.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap();
    assert_eq!(d.len(), 18);
    h.udc.disable();
    assert!(!h.sim().line(), "disabled: no events enabled");
}

// --- The MTP layout (ADR-0037) ----------------------------------------------

fn mtp_host() -> Host {
    let mut udc = Udc::new(SimUdc::default());
    udc.set_layout(Layout::Mtp);
    udc.enable().unwrap();
    Host { udc }
}

/// Walks a configuration: every descriptor's (type, bytes), lengths
/// chaining exactly to the end.
fn walk(c: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let len = usize::from(c[i]);
        assert!(len >= 2);
        out.push((c[i + 1], c[i..i + len].to_vec()));
        i += len;
    }
    assert_eq!(i, c.len());
    out
}

#[test]
fn the_mtp_layout_names_its_interface_mtp_and_matches_its_table() {
    let c = &usb::CONFIG_MTP;
    assert_eq!(usize::from(u16::from_le_bytes([c[2], c[3]])), c.len());
    assert_eq!(c[4], 3, "three interfaces");
    // The serial port's descriptors are the drive layout's, byte for byte.
    assert_eq!(c[9..75], usb::CONFIG[9..75]);
    let d = walk(c);
    let iface2 = d.iter().find(|(t, b)| *t == 4 && b[2] == 2).unwrap();
    assert_eq!(
        &iface2.1[4..9],
        &[3, 0xFF, 0xFF, 0x00, 5],
        "3 endpoints, vendor class, string 5"
    );
    let endpoints: Vec<(u8, u8, u16)> = d
        .iter()
        .filter(|(t, _)| *t == 5)
        .map(|(_, b)| (b[2], b[3], u16::from_le_bytes([b[4], b[5]])))
        .collect();
    assert_eq!(
        endpoints,
        [
            (0x83, 3, 16),
            (0x01, 2, 64),
            (0x82, 2, 64),
            (0x04, 2, 64),
            (0x85, 2, 64),
            (0x86, 3, 32)
        ]
    );
    for (slot, cfg) in TABLE_MTP {
        let number = ((cfg >> 15) & 0xF) as u8;
        let is_in = cfg & (1 << 12) != 0;
        let size = ((cfg >> 2) & 0x3FF) as u16;
        let kind = ((cfg >> 13) & 3) as u8;
        let addr = number | if is_in { 0x80 } else { 0 };
        assert!(
            endpoints
                .iter()
                .any(|&(a, k, s)| a == addr && s == size && k == kind),
            "slot {slot}"
        );
    }
    // USB 2.01 (a BOS descriptor), revision 26.02; the drive layout stays.
    assert_eq!(&usb::DEVICE_MTP[2..4], &[0x01, 0x02]);
    assert_eq!(&usb::DEVICE_MTP[12..14], &[0x02, 0x26]);
    assert_eq!(&usb::DEVICE[2..4], &[0x00, 0x02]);
    assert_eq!(&usb::DEVICE[12..14], &[0x01, 0x26]);
    assert_eq!(usb::MTP_NAME, [8, 3, b'M', 0, b'T', 0, b'P', 0]);
}

#[test]
fn microsofts_descriptors_are_well_formed() {
    let s = &usb::MS_OS_STRING;
    assert_eq!(usize::from(s[0]), s.len());
    let name: Vec<u8> = "MSFT100"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(&s[2..16], &name[..]);
    assert_eq!(s[16], usb::MS_VENDOR_CODE);
    let c = &usb::MS_COMPAT_ID;
    assert_eq!(
        u32::from_le_bytes([c[0], c[1], c[2], c[3]]) as usize,
        c.len()
    );
    assert_eq!(&c[4..8], &[0x00, 0x01, 0x04, 0x00]);
    assert_eq!(c[16], 2, "the MTP interface");
    assert_eq!(&c[18..26], b"MTP\0\0\0\0\0");
    let b = &usb::BOS;
    assert_eq!(usize::from(u16::from_le_bytes([b[2], b[3]])), b.len());
    assert_eq!(usize::from(b[5]), b.len() - 5);
    assert_eq!(
        &b[9..25],
        &[
            0xDF, 0x60, 0xDD, 0xD8, 0x89, 0x45, 0xC7, 0x4C, 0x9C, 0xD2, 0x65, 0x9D, 0x9E, 0x64,
            0x8A, 0x9F
        ]
    );
    assert_eq!(
        usize::from(u16::from_le_bytes([b[29], b[30]])),
        usb::MS_OS20_SET.len()
    );
    assert_eq!(b[31], usb::MS_VENDOR_CODE);
    let set = &usb::MS_OS20_SET;
    assert_eq!(usize::from(u16::from_le_bytes([set[8], set[9]])), set.len());
    // Each descriptor's own length chains to the end; the subsets' totals
    // cover what follows them.
    let mut i = 0;
    let mut kinds = Vec::new();
    while i < set.len() {
        let len = usize::from(u16::from_le_bytes([set[i], set[i + 1]]));
        kinds.push(u16::from_le_bytes([set[i + 2], set[i + 3]]));
        i += len;
    }
    assert_eq!(i, set.len());
    assert_eq!(kinds, [0, 1, 2, 3]);
    assert_eq!(usize::from(set[16]), set.len() - 10, "configuration subset");
    assert_eq!(set[22], 2, "function at the MTP interface");
    assert_eq!(usize::from(set[24]), 28, "function subset");
    assert_eq!(&set[30..38], b"MTP\0\0\0\0\0");
}

#[test]
fn enumerates_as_mtp_like_windows_and_records_what_windows_read() {
    let mut h = mtp_host();
    h.reset();
    assert_eq!(
        h.control_in([0x80, 6, 0, 1, 0, 0, 18, 0]).unwrap(),
        usb::DEVICE_MTP
    );
    assert_eq!(
        h.control_in([0x80, 6, 0, 2, 0, 0, 255, 0]).unwrap(),
        usb::CONFIG_MTP
    );
    assert_eq!(
        h.control_in([0x80, 6, 5, 3, 9, 4, 255, 0]).unwrap(),
        usb::MTP_NAME
    );
    assert_eq!(h.udc.ms_seen(), 0);
    assert_eq!(
        h.control_in([0x80, 6, 0xEE, 3, 0, 0, 255, 0]).unwrap(),
        usb::MS_OS_STRING
    );
    assert_eq!(
        h.control_in([0xC0, usb::MS_VENDOR_CODE, 0, 0, 4, 0, 16, 0])
            .unwrap(),
        usb::MS_COMPAT_ID[..16]
    );
    assert_eq!(
        h.control_in([0xC1, usb::MS_VENDOR_CODE, 0, 0, 4, 0, 40, 0])
            .unwrap(),
        usb::MS_COMPAT_ID
    );
    assert_eq!(
        h.control_in([0x80, 6, 0, 0x0F, 0, 0, 5, 0]).unwrap(),
        usb::BOS[..5]
    );
    assert_eq!(
        h.control_in([0x80, 6, 0, 0x0F, 0, 0, 33, 0]).unwrap(),
        usb::BOS
    );
    assert_eq!(h.udc.ms_seen() & usb::ms::SET20, 0);
    assert_eq!(
        h.control_in([0xC0, usb::MS_VENDOR_CODE, 0, 0, 7, 0, 46, 0])
            .unwrap(),
        usb::MS_OS20_SET
    );
    assert_eq!(
        h.udc.ms_seen(),
        usb::ms::STRING | usb::ms::COMPAT_ID | usb::ms::BOS | usb::ms::SET20
    );
    // Another vendor index stalls; a new enable forgets what was read.
    assert!(
        h.control_in([0xC0, usb::MS_VENDOR_CODE, 0, 0, 5, 0, 16, 0])
            .is_none()
    );
    h.udc.disable();
    h.udc.enable().unwrap();
    assert_eq!(h.udc.ms_seen(), 0);
}

#[test]
fn the_drive_layout_offers_none_of_microsofts_descriptors() {
    let mut h = Host::new();
    h.reset();
    assert!(h.control_in([0x80, 6, 0xEE, 3, 0, 0, 255, 0]).is_none());
    assert!(
        h.control_in([0xC0, usb::MS_VENDOR_CODE, 0, 0, 4, 0, 16, 0])
            .is_none()
    );
    assert!(h.control_in([0x80, 6, 0, 0x0F, 0, 0, 5, 0]).is_none());
    assert!(h.control_in([0x80, 6, 5, 3, 9, 4, 255, 0]).is_none());
    // MTP's class requests neither.
    assert!(h.control_in([0xA1, 0x67, 0, 0, 2, 0, 12, 0]).is_none());
}

#[test]
fn mtp_class_requests_reach_the_function() {
    let mut h = mtp_host();
    h.reset();
    // Cancel: code 0x4001 and the transaction.
    h.control_out(
        [0x21, 0x64, 0, 0, 2, 0, 6, 0],
        &[0x01, 0x40, 0x34, 0x12, 0, 0],
    );
    assert_eq!(h.udc.take_mtp_cancel(), Some(0x1234));
    assert_eq!(h.udc.take_mtp_cancel(), None);
    // A wrong cancellation code is ignored.
    h.control_out([0x21, 0x64, 0, 0, 2, 0, 6, 0], &[0x02, 0x40, 1, 0, 0, 0]);
    assert_eq!(h.udc.take_mtp_cancel(), None);
    // Device Reset.
    h.control_ack([0x21, 0x66, 0, 0, 2, 0, 0, 0]);
    assert!(h.udc.take_mtp_reset());
    // Get Device Status: OK by default, then what the function says.
    assert_eq!(
        h.control_in([0xA1, 0x67, 0, 0, 2, 0, 12, 0]).unwrap(),
        [4, 0, 0x01, 0x20]
    );
    h.udc.set_mtp_status(0x2019);
    assert_eq!(
        h.control_in([0xA1, 0x67, 0, 0, 2, 0, 12, 0]).unwrap(),
        [4, 0, 0x19, 0x20]
    );
    // On another interface: STALL.
    assert!(h.control_in([0xA1, 0x67, 0, 0, 0, 0, 12, 0]).is_none());
}

#[test]
fn a_stalled_pipe_reports_its_endpoints_until_the_host_clears_them() {
    let mut h = mtp_host();
    h.reset();
    h.set_configuration();
    h.udc.mtp_stall();
    let writes: Vec<(usize, u32)> = core::mem::take(&mut h.sim().csr_writes);
    for slot in [SLOT_MSC_OUT, SLOT_MSC_IN] {
        assert!(
            writes.iter().any(|&(s, v)| s == slot && v & reg::FST != 0),
            "slot {slot} halted"
        );
    }
    h.udc.set_mtp_status(0x201F);
    assert_eq!(
        h.control_in([0xA1, 0x67, 0, 0, 2, 0, 12, 0]).unwrap(),
        [12, 0, 0x1F, 0x20, 0x04, 0, 0, 0, 0x85, 0, 0, 0]
    );
    h.control_ack([0x02, 1, 0, 0, 0x04, 0, 0, 0]);
    assert!(!h.udc.take_mtp_cleared(), "one of two cleared");
    h.control_ack([0x02, 1, 0, 0, 0x85, 0, 0, 0]);
    assert!(h.udc.take_mtp_cleared());
    let writes: Vec<(usize, u32)> = core::mem::take(&mut h.sim().csr_writes);
    assert!(
        writes
            .iter()
            .any(|&(s, v)| s == SLOT_MSC_IN && v & reg::SST != 0 && v & reg::FST == 0)
    );
    h.udc.set_mtp_status(0x2001);
    assert_eq!(
        h.control_in([0xA1, 0x67, 0, 0, 2, 0, 12, 0]).unwrap(),
        [4, 0, 0x01, 0x20]
    );
}

#[test]
fn events_go_out_on_their_own_endpoint() {
    let mut h = mtp_host();
    // The MTP table has the event endpoint; the drive table doesn't.
    assert_ne!(h.sim().cr[SLOT_MTP_EVENT], 0);
    assert_eq!(Host::new().sim().cr[SLOT_MTP_EVENT], 0);
    h.reset();
    assert!(!h.udc.mtp_event_can_write(), "not before configuration");
    h.set_configuration();
    assert!(h.udc.mtp_event_can_write());
    let event = [16, 0, 0, 0, 4, 0, 0x02, 0x40, 0, 0, 0, 0, 7, 0, 0, 0];
    h.udc.mtp_event_write(&event);
    h.sim().host_in_on(SLOT_MTP_EVENT);
    assert_eq!(
        h.sim().bulk[SLOT_MTP_EVENT].sent.pop_front().unwrap(),
        event
    );
}
