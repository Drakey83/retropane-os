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
    /// Packets the host has taken from endpoint 0.
    ep0_sent: VecDeque<Vec<u8>>,
    cr: [u32; 24],
    in_fifo: Vec<u8>,
    in_sent: VecDeque<Vec<u8>>,
    out_fifo: VecDeque<u8>,
    out_pc: bool,
    acked_config: u32,
    smac: u32,
    /// Every raw write to UDCCR, to check the masking rule.
    udccr_writes: Vec<u32>,
    reject_table: bool,
}

impl SimUdc {
    fn ep0_packet_ready(&mut self) {
        if self.ep0_tx.len() >= usb::EP0_SIZE || self.csr0 & reg::IPR != 0 {
            let p = core::mem::take(&mut self.ep0_tx);
            self.ep0_sent.push_back(p);
            self.csr0 &= !reg::IPR;
        }
    }
}

impl RegisterBus for SimUdc {
    fn read32(&mut self, offset: usize) -> u32 {
        match offset {
            reg::UDCCR => self.udccr,
            reg::UDCISR1 => self.isr1,
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
            o if o == reg::csr(SLOT_IN) => {
                if self.in_fifo.len() < usb::BULK_SIZE {
                    reg::BNF
                } else {
                    0
                }
            }
            o if o == reg::csr(SLOT_OUT) => {
                if self.out_pc {
                    reg::PC | reg::BNF
                } else {
                    0
                }
            }
            o if o == reg::bcr(SLOT_OUT) => self.out_fifo.len() as u32,
            o if o == reg::dr(SLOT_OUT) => {
                let mut w = [0u8; 4];
                for b in &mut w {
                    *b = self.out_fifo.pop_front().unwrap_or(0);
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
            reg::UDCISR1 => self.isr1 &= !v,
            o if o == reg::csr(0) => {
                // Write-1-to-clear status, actions, and IPR/FST set by software.
                self.csr0 &= !(v & (reg::OPC | reg::SA | reg::SST));
                if v & reg::FTF != 0 {
                    self.ep0_tx.clear();
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
            o if o == reg::dr(0) => {
                self.ep0_tx.extend_from_slice(&v.to_le_bytes());
                self.ep0_packet_ready();
            }
            o if (reg::cr(1)..=reg::cr(23)).contains(&o) => {
                assert_eq!(
                    self.udccr & reg::UDE,
                    0,
                    "endpoint table written while enabled"
                );
                self.cr[(o - 0x400) / 4] = v;
            }
            o if o == reg::dr(SLOT_IN) => {
                self.in_fifo.extend_from_slice(&v.to_le_bytes());
                assert!(self.in_fifo.len() <= usb::BULK_SIZE, "IN FIFO overrun");
                if self.in_fifo.len() == usb::BULK_SIZE {
                    self.in_sent.push_back(core::mem::take(&mut self.in_fifo));
                }
            }
            o if o == reg::csr(SLOT_IN) && v & reg::SP != 0 => {
                self.in_sent.push_back(core::mem::take(&mut self.in_fifo));
            }
            o if o == reg::csr(SLOT_OUT) && v & reg::PC != 0 => {
                self.out_pc = false;
                self.out_fifo.clear();
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
            self.ep0_tx.push(v);
            self.ep0_packet_ready();
        } else if offset == reg::dr(SLOT_IN) {
            self.in_fifo.push(v);
            assert!(self.in_fifo.len() <= usb::BULK_SIZE, "IN FIFO overrun");
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
            if let Some(p) = self.sim().ep0_sent.pop_front() {
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
            self.poll(1);
        }
        panic!("control read never finished");
    }

    /// A control write with no data: expects the zero-length status IN.
    fn control_ack(&mut self, s: [u8; 8]) {
        self.setup(s);
        let p = self.sim().ep0_sent.pop_front().expect("no status IN");
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
        let p = self.sim().ep0_sent.pop_front().expect("no status IN");
        assert!(p.is_empty());
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
    // Slot A: config 1, interface 0, EP1 interrupt IN, 16 bytes.
    assert_eq!(
        sim.cr[1],
        (1 << 25) | (1 << 15) | (3 << 13) | (1 << 12) | (16 << 2) | 1
    );
    // Slot B: interface 1, EP2 bulk IN, 64. Slot C: EP2 bulk OUT, 64.
    assert_eq!(
        sim.cr[2],
        (1 << 25) | (1 << 22) | (2 << 15) | (2 << 13) | (1 << 12) | (64 << 2) | 1
    );
    assert_eq!(
        sim.cr[3],
        (1 << 25) | (1 << 22) | (2 << 15) | (2 << 13) | (64 << 2) | 1
    );
    assert_ne!(sim.icr1 & reg::CONFIG_CHANGE, 0);
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
        while let Some(p) = h.sim().ep0_sent.pop_front() {
            sizes.push(p.len());
        }
        h.poll(1);
    }
    // 67 bytes: four full packets, then 3.
    assert_eq!(sizes, [16, 16, 16, 16, 3]);
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
    h.poll(10);
    let sent: Vec<Vec<u8>> = h.sim().in_sent.drain(..).collect();
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
        h.sim().out_fifo.extend(packet);
        h.sim().out_pc = true;
        h.poll(1);
        assert!(!h.sim().out_pc, "packet not released");
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
    // Fill the 256-byte ring with four 64-byte packets, then offer a fifth.
    for _ in 0..5 {
        h.sim().out_fifo.extend([7u8; 64]);
        h.sim().out_pc = true;
        h.poll(1);
    }
    // The fifth stays in the FIFO (the hardware NAKs) until there's room.
    assert!(h.sim().out_pc);
    let mut buf = [0u8; 64];
    assert_eq!(h.udc.read(&mut buf), 64);
    h.poll(1);
    assert!(!h.sim().out_pc);
}

#[test]
fn a_bus_reset_drops_the_configuration_and_queued_data() {
    let mut h = Host::new();
    h.enumerate();
    h.udc.write(b"pending");
    h.reset();
    assert!(!h.udc.configured());
    h.enumerate();
    h.poll(4);
    assert!(h.sim().in_sent.is_empty(), "stale bytes sent after reset");
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
    assert_eq!(endpoints, [(0x81, 3, 16), (0x02, 2, 64), (0x82, 2, 64)]);
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
