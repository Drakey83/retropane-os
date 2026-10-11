//! Hardware-independent USB device logic for a CDC-ACM serial port: the
//! descriptors (`const` data, nothing built at run time), SETUP parsing, and
//! answers to the standard and CDC requests that reach software. The PXA27x
//! answers SET_ADDRESS, SET_CONFIGURATION and SET_INTERFACE in hardware
//! (docs/hw/udc.md), so they never arrive here.

/// pid.codes' open-source vendor ID (ADR-0012).
pub const VENDOR_ID: u16 = 0x1209;
/// pid.codes' testing product ID, for development only (ADR-0012: replace
/// with an assigned ID before any release).
pub const PRODUCT_ID: u16 = 0x0001;
/// Endpoint 0's packet size on the PXA27x.
pub const EP0_SIZE: usize = 16;
/// Bulk packet size (full speed maximum).
pub const BULK_SIZE: usize = 64;
/// Notification (interrupt IN) packet size.
pub const NOTIFY_SIZE: usize = 16;

/// USB endpoint numbers, as in the descriptors and the UDC's endpoint table:
/// a distinct number for each endpoint, as the PXA27x driver in Linux uses
/// (bulk OUT 1, bulk IN 2, interrupt IN separate).
pub const EP_OUT: u8 = 1;
/// Bulk IN endpoint number.
pub const EP_IN: u8 = 2;
/// Notification (interrupt IN) endpoint number.
pub const EP_NOTIFY: u8 = 3;

const fn lo(v: u16) -> u8 {
    v.to_le_bytes()[0]
}
const fn hi(v: u16) -> u8 {
    v.to_le_bytes()[1]
}

/// The device descriptor: class 0x02 (CDC) at device level, which Windows
/// 10/11, macOS and Linux bind to their built-in serial drivers.
pub const DEVICE: [u8; 18] = [
    18,
    1, // DEVICE
    0x00,
    0x02, // USB 2.0 (full speed)
    0x02,
    0x00,
    0x00, // class CDC
    EP0_SIZE as u8,
    lo(VENDOR_ID),
    hi(VENDOR_ID),
    lo(PRODUCT_ID),
    hi(PRODUCT_ID),
    0x00,
    0x26, // bcdDevice 26.00: RetroPane 26
    1,
    2,
    3, // manufacturer, product, serial strings
    1, // one configuration
];

/// Total length of [`CONFIG`].
const CONFIG_LEN: u16 = 67;

/// Configuration 1: a communication interface (CDC-ACM, one interrupt IN
/// endpoint) and a data interface (bulk OUT + bulk IN).
pub const CONFIG: [u8; CONFIG_LEN as usize] = [
    // Configuration
    9,
    2,
    lo(CONFIG_LEN),
    hi(CONFIG_LEN),
    2,    // interfaces
    1,    // bConfigurationValue
    0,    // no string
    0xC0, // self-powered (battery)
    50,   // 100 mA
    // Interface 0: communication, ACM
    9,
    4,
    0,
    0,
    1,
    0x02,
    0x02,
    0x01,
    0,
    // CDC header, version 1.10
    5,
    0x24,
    0x00,
    0x10,
    0x01,
    // Call management: none, data interface 1
    5,
    0x24,
    0x01,
    0x00,
    1,
    // ACM: line coding and serial state supported
    4,
    0x24,
    0x02,
    0x02,
    // Union: master 0, slave 1
    5,
    0x24,
    0x06,
    0,
    1,
    // Notification endpoint 0x83, interrupt
    7,
    5,
    0x80 | EP_NOTIFY,
    0x03,
    NOTIFY_SIZE as u8,
    0,
    16,
    // Interface 1: data
    9,
    4,
    1,
    0,
    2,
    0x0A,
    0x00,
    0x00,
    0,
    // Bulk OUT 0x01
    7,
    5,
    EP_OUT,
    0x02,
    BULK_SIZE as u8,
    0,
    0,
    // Bulk IN 0x82
    7,
    5,
    0x80 | EP_IN,
    0x02,
    BULK_SIZE as u8,
    0,
    0,
];

/// A string descriptor from ASCII text (UTF-16LE, no terminator).
const fn string<const N: usize>(text: &str) -> [u8; N] {
    let b = text.as_bytes();
    let mut out = [0u8; N];
    out[0] = N as u8;
    out[1] = 3;
    let mut i = 0;
    while i < b.len() {
        out[2 + 2 * i] = b[i];
        i += 1;
    }
    out
}

macro_rules! string_descriptor {
    ($name:ident, $text:expr) => {
        const TEXT: &str = $text;
        /// A string descriptor.
        pub const $name: [u8; 2 + 2 * TEXT.len()] = string(TEXT);
    };
}

/// String 0: supported languages (US English).
pub const LANGUAGES: [u8; 4] = [4, 3, 0x09, 0x04];
mod s1 {
    use super::string;
    string_descriptor!(MANUFACTURER, "RetroPane");
}
mod s2 {
    use super::string;
    string_descriptor!(PRODUCT, "RetroPane 26 console");
}
mod s3 {
    use super::string;
    string_descriptor!(SERIAL, "RP26-0001");
}
pub use s1::MANUFACTURER;
pub use s2::PRODUCT;
pub use s3::SERIAL;

/// A SETUP packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setup {
    /// bmRequestType.
    pub request_type: u8,
    /// bRequest.
    pub request: u8,
    /// wValue.
    pub value: u16,
    /// wIndex.
    pub index: u16,
    /// wLength.
    pub length: u16,
}

impl Setup {
    /// Parses the 8 bytes of a SETUP packet.
    #[must_use]
    pub fn parse(b: [u8; 8]) -> Self {
        Self {
            request_type: b[0],
            request: b[1],
            value: u16::from_le_bytes([b[2], b[3]]),
            index: u16::from_le_bytes([b[4], b[5]]),
            length: u16::from_le_bytes([b[6], b[7]]),
        }
    }

    /// Whether the data stage (if any) goes device-to-host.
    #[must_use]
    pub fn is_in(&self) -> bool {
        self.request_type & 0x80 != 0
    }
}

/// What endpoint 0 does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Reply<'a> {
    /// Send this data (already cut to wLength), then expect the status OUT.
    In(&'a [u8]),
    /// No data stage: acknowledge with a zero-length status IN.
    Ack,
    /// Receive this many bytes, hand them to [`Cdc::out_data`], then ACK.
    Out(usize),
    /// Not supported: STALL.
    Stall,
}

/// CDC request codes.
mod cdc {
    pub const SET_LINE_CODING: u8 = 0x20;
    pub const GET_LINE_CODING: u8 = 0x21;
    pub const SET_CONTROL_LINE_STATE: u8 = 0x22;
    pub const SEND_BREAK: u8 = 0x23;
}

/// The serial port's software state: what the host has set.
#[derive(Debug)]
pub struct Cdc {
    /// dwDTERate, bCharFormat, bParityType, bDataBits (the host's choice;
    /// meaningless over USB, but it reads back what it wrote).
    line_coding: [u8; 7],
    /// DTR (bit 0) and RTS (bit 1) from SET_CONTROL_LINE_STATE.
    control: u16,
    /// Reply buffer for computed answers (status, line coding).
    buf: [u8; 7],
    /// The pending OUT data request.
    pending_out: u8,
}

impl Default for Cdc {
    fn default() -> Self {
        Self::new()
    }
}

impl Cdc {
    /// 115200 8N1 until the host says otherwise.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            line_coding: [0x00, 0xC2, 0x01, 0x00, 0, 0, 8],
            control: 0,
            buf: [0; 7],
            pending_out: 0,
        }
    }

    /// Whether the host has the port open (DTR set: terminal programs and
    /// Windows' serial API set it on open).
    #[must_use]
    pub fn dtr(&self) -> bool {
        self.control & 1 != 0
    }

    /// The line coding the host set (rate in bits per second).
    #[must_use]
    pub fn baud(&self) -> u32 {
        u32::from_le_bytes([
            self.line_coding[0],
            self.line_coding[1],
            self.line_coding[2],
            self.line_coding[3],
        ])
    }

    /// Answers a SETUP packet. `configuration` is the active configuration
    /// (0 before SET_CONFIGURATION), for GET_CONFIGURATION.
    pub fn handle(&mut self, s: &Setup, configuration: u8) -> Reply<'_> {
        let want = usize::from(s.length);
        let cut = |d: &'static [u8]| Reply::In(&d[..d.len().min(want)]);
        match (s.request_type, s.request) {
            // GET_DESCRIPTOR
            (0x80, 6) => match ((s.value >> 8) as u8, s.value as u8) {
                (1, _) => cut(&DEVICE),
                (2, 0) => cut(&CONFIG),
                (3, 0) => cut(&LANGUAGES),
                (3, 1) => cut(&MANUFACTURER),
                (3, 2) => cut(&PRODUCT),
                (3, 3) => cut(&SERIAL),
                // Device qualifier and anything else: full speed only, STALL
                // (what the USB 2.0 spec asks of a full-speed-only device).
                _ => Reply::Stall,
            },
            // GET_STATUS: device (self-powered), interface, endpoint.
            (0x80..=0x82, 0) => {
                self.buf[0] = u8::from(s.request_type == 0x80);
                self.buf[1] = 0;
                Reply::In(&self.buf[..2.min(want)])
            }
            // GET_CONFIGURATION
            (0x80, 8) => {
                self.buf[0] = configuration;
                Reply::In(&self.buf[..1.min(want)])
            }
            // GET_INTERFACE: no alternate settings.
            (0x81, 10) => {
                self.buf[0] = 0;
                Reply::In(&self.buf[..1.min(want)])
            }
            // CLEAR_FEATURE / SET_FEATURE (remote wake-up, endpoint halt):
            // accepted; nothing to do for a console.
            (0x00..=0x02, 1 | 3) => Reply::Ack,
            (0x21, cdc::SET_LINE_CODING) if want == 7 => {
                self.pending_out = cdc::SET_LINE_CODING;
                Reply::Out(7)
            }
            (0xA1, cdc::GET_LINE_CODING) => {
                self.buf = self.line_coding;
                Reply::In(&self.buf[..7.min(want)])
            }
            (0x21, cdc::SET_CONTROL_LINE_STATE) => {
                self.control = s.value;
                Reply::Ack
            }
            (0x21, cdc::SEND_BREAK) => Reply::Ack,
            _ => Reply::Stall,
        }
    }

    /// The data stage of an OUT request announced by [`Reply::Out`].
    pub fn out_data(&mut self, data: &[u8]) {
        if self.pending_out == cdc::SET_LINE_CODING && data.len() == 7 {
            self.line_coding.copy_from_slice(data);
        }
        self.pending_out = 0;
    }
}
