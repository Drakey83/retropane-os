//! Hardware-independent USB device logic for a composite device (ADR-0014,
//! ADR-0037): a CDC-ACM serial port plus a USB Mass Storage interface (the
//! drive layout) or an MTP interface (the portable-device layout, with
//! Microsoft's OS descriptors 1.0 and 2.0 naming it "MTP"). The descriptors
//! (`const` data, nothing built at run time), SETUP parsing, and answers to
//! the standard, CDC and mass-storage class requests that reach software.
//! The PXA27x answers SET_ADDRESS, SET_CONFIGURATION and SET_INTERFACE in
//! hardware (docs/hw/udc.md), so they never arrive here.

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
/// Mass storage bulk OUT endpoint number.
pub const EP_MSC_OUT: u8 = 4;
/// Mass storage bulk IN endpoint number.
pub const EP_MSC_IN: u8 = 5;
/// The mass storage interface number.
pub const MSC_INTERFACE: u16 = 2;
/// The MTP interface number (the drive's place in the MTP layout).
pub const MTP_INTERFACE: u16 = 2;
/// MTP's event (interrupt IN) endpoint number; its bulk pair is the
/// drive's (`EP_MSC_OUT`, `EP_MSC_IN`).
pub const EP_MTP_EVENT: u8 = 6;
/// MTP's event packet size: an event container is at most 24 bytes
/// [SIC 7.3.1].
pub const EVENT_SIZE: usize = 32;
/// The vendor request code of Microsoft's OS descriptors (ours, any
/// value the device doesn't use otherwise).
pub const MS_VENDOR_CODE: u8 = 0x52;

/// What the device presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The serial port alone, as before ADR-0014 (`bcdDevice` 26.00): the
    /// safety net when a host configures nothing else.
    Serial,
    /// Serial port + USB drive (ADR-0014, 26.01).
    Drive,
    /// Serial port + MTP (ADR-0037, 26.02).
    Mtp,
}

const fn lo(v: u16) -> u8 {
    v.to_le_bytes()[0]
}
const fn hi(v: u16) -> u8 {
    v.to_le_bytes()[1]
}

/// The device descriptor: class 0xEF/0x02/0x01 ("uses Interface Association
/// Descriptors"), so Windows 10/11, macOS and Linux bind their built-in
/// serial driver to the CDC pair and their storage driver to the mass
/// storage interface, with no INF.
pub const DEVICE: [u8; 18] = [
    18,
    1, // DEVICE
    0x00,
    0x02, // USB 2.0 (full speed)
    0xEF,
    0x02,
    0x01, // miscellaneous: IAD
    EP0_SIZE as u8,
    lo(VENDOR_ID),
    hi(VENDOR_ID),
    lo(PRODUCT_ID),
    hi(PRODUCT_ID),
    0x01,
    0x26, // bcdDevice 26.01: the composite layout (26.00 was serial only;
    // a new revision makes Windows enumerate it afresh)
    1,
    2,
    3, // manufacturer, product, serial strings
    1, // one configuration
];

/// Total length of [`CONFIG`].
const CONFIG_LEN: u16 = 98;

/// Configuration 1: the serial port (a communication interface with one
/// interrupt IN endpoint and a data interface with a bulk pair, grouped by
/// an Interface Association Descriptor) and the mass storage interface
/// (SCSI transparent command set, Bulk-Only Transport, its own bulk pair).
pub const CONFIG: [u8; CONFIG_LEN as usize] = [
    // Configuration
    9,
    2,
    lo(CONFIG_LEN),
    hi(CONFIG_LEN),
    3,    // interfaces
    1,    // bConfigurationValue
    0,    // no string
    0xC0, // self-powered (battery)
    50,   // 100 mA
    // Interface association: interfaces 0-1 are one CDC-ACM function
    8,
    0x0B,
    0,
    2,
    0x02,
    0x02,
    0x01,
    0,
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
    // Interface 2: mass storage, SCSI transparent, Bulk-Only Transport
    9,
    4,
    MSC_INTERFACE as u8,
    0,
    2,
    0x08,
    0x06,
    0x50,
    0,
    // Bulk OUT 0x04
    7,
    5,
    EP_MSC_OUT,
    0x02,
    BULK_SIZE as u8,
    0,
    0,
    // Bulk IN 0x85
    7,
    5,
    0x80 | EP_MSC_IN,
    0x02,
    BULK_SIZE as u8,
    0,
    0,
];

/// The MTP layout's device descriptor: USB 2.01 (so Windows reads the BOS
/// descriptor holding Microsoft's OS 2.0 capability), revision 26.02 (a
/// new revision: Windows records per revision whether a device has
/// Microsoft OS descriptors, and has recorded 26.00 and 26.01 without).
pub const DEVICE_MTP: [u8; 18] = {
    let mut d = DEVICE;
    d[2] = 0x01;
    d[3] = 0x02;
    d[12] = 0x02;
    d
};

/// Total length of [`CONFIG_MTP`].
const MTP_CONFIG_LEN: usize = CONFIG_LEN as usize + 7;
/// Where the drive's interface starts in [`CONFIG`] (the serial port's
/// descriptors before it are shared).
const SHARED: usize = CONFIG_LEN as usize - 23;

/// The MTP layout's configuration: the serial port as in [`CONFIG`], then
/// the MTP interface (vendor class, named "MTP" by string 5 and by
/// Microsoft's compatible ID: ADR-0037 Q1 A) with its bulk pair and event
/// endpoint.
pub const CONFIG_MTP: [u8; MTP_CONFIG_LEN] = {
    let mut c = [0u8; MTP_CONFIG_LEN];
    let mut i = 0;
    while i < SHARED {
        c[i] = CONFIG[i];
        i += 1;
    }
    c[2] = MTP_CONFIG_LEN as u8;
    c[3] = 0;
    let tail: [u8; 30] = [
        // Interface 2: vendor specific (MTP), three endpoints, string 5
        9,
        4,
        MTP_INTERFACE as u8,
        0,
        3,
        0xFF,
        0xFF,
        0x00,
        5,
        // Bulk OUT 0x04
        7,
        5,
        EP_MSC_OUT,
        0x02,
        BULK_SIZE as u8,
        0,
        0,
        // Bulk IN 0x85
        7,
        5,
        0x80 | EP_MSC_IN,
        0x02,
        BULK_SIZE as u8,
        0,
        0,
        // Interrupt IN 0x86: events, every 6 ms (Android's interval)
        7,
        5,
        0x80 | EP_MTP_EVENT,
        0x03,
        EVENT_SIZE as u8,
        0,
        6,
    ];
    let mut k = 0;
    while k < tail.len() {
        c[SHARED + k] = tail[k];
        k += 1;
    }
    c
};

/// Microsoft OS 1.0 string descriptor (index 0xEE): "MSFT100", our vendor
/// code. Windows asks for it once per device revision.
pub const MS_OS_STRING: [u8; 18] = [
    18,
    3,
    b'M',
    0,
    b'S',
    0,
    b'F',
    0,
    b'T',
    0,
    b'1',
    0,
    b'0',
    0,
    b'0',
    0,
    MS_VENDOR_CODE,
    0,
];

/// Microsoft OS 1.0 extended compatible ID (vendor request, `wIndex` 4):
/// interface 2 is "MTP", so Windows binds its MTP driver to it (its
/// wpdmtp.inf matches `USB\MS_COMP_MTP`, checked on Windows 11).
pub const MS_COMPAT_ID: [u8; 40] = [
    40,
    0,
    0,
    0, // dwLength
    0x00,
    0x01, // bcdVersion 1.00
    0x04,
    0x00, // wIndex: extended compatible ID
    1,    // one function section
    0,
    0,
    0,
    0,
    0,
    0,
    0,                   // reserved
    MTP_INTERFACE as u8, // first interface
    1,                   // reserved (1)
    b'M',
    b'T',
    b'P',
    0,
    0,
    0,
    0,
    0, // compatible ID
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0, // sub-compatible ID
    0,
    0,
    0,
    0,
    0,
    0, // reserved
];

/// Total length of [`MS_OS20_SET`].
const MS20_LEN: usize = 10 + 8 + 8 + 20;

/// Microsoft OS 2.0 descriptor set (vendor request, `wIndex` 7): Windows
/// 8.1 and later; the configuration's function at interface 2 is "MTP".
/// Only Windows asks for it: whether it asks on every connection is the
/// measurement ADR-0037 rule 4 waits on.
pub const MS_OS20_SET: [u8; MS20_LEN] = [
    // Set header: Windows 8.1 (0x06030000), total length
    10,
    0,
    0x00,
    0x00,
    0x00,
    0x00,
    0x03,
    0x06,
    MS20_LEN as u8,
    0,
    // Configuration subset header: configuration index 0, its length
    8,
    0,
    0x01,
    0x00,
    0,
    0,
    (MS20_LEN - 10) as u8,
    0,
    // Function subset header: interface 2, its length
    8,
    0,
    0x02,
    0x00,
    MTP_INTERFACE as u8,
    0,
    28,
    0,
    // Compatible ID: "MTP"
    20,
    0,
    0x03,
    0x00,
    b'M',
    b'T',
    b'P',
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];

/// The BOS descriptor: one platform capability, Microsoft's OS 2.0
/// ({D8DD60DF-4589-4CC7-9CD2-659D9E648A9F}), pointing at [`MS_OS20_SET`].
pub const BOS: [u8; 33] = [
    5,
    0x0F,
    33,
    0,
    1, // BOS header: total 33, one capability
    28,
    0x10,
    0x05,
    0, // platform capability
    0xDF,
    0x60,
    0xDD,
    0xD8,
    0x89,
    0x45,
    0xC7,
    0x4C,
    0x9C,
    0xD2,
    0x65,
    0x9D,
    0x9E,
    0x64,
    0x8A,
    0x9F,
    0x00,
    0x00,
    0x03,
    0x06, // Windows 8.1 and later
    MS20_LEN as u8,
    0,              // descriptor set length
    MS_VENDOR_CODE, // vendor code
    0,              // no alternate enumeration
];

/// The serial-only layout, byte for byte what RetroPane presented before
/// ADR-0014 (class CDC, `bcdDevice` 26.00): the fallback when a host won't
/// configure the composite device, matching what Windows already knows.
pub const DEVICE_SERIAL: [u8; 18] = {
    let mut d = DEVICE;
    d[4] = 0x02;
    d[5] = 0x00;
    d[6] = 0x00;
    d[12] = 0x00;
    d
};

/// Length of [`CONFIG_SERIAL`]: without the IAD and the storage interface.
const SERIAL_CONFIG_LEN: usize = CONFIG_LEN as usize - 8 - 23;

/// The serial-only configuration, derived from [`CONFIG`] at compile time
/// (so the two can't drift): the header with two interfaces, then the CDC
/// interfaces without the IAD in front of them.
pub const CONFIG_SERIAL: [u8; SERIAL_CONFIG_LEN] = {
    let mut c = [0u8; SERIAL_CONFIG_LEN];
    let mut i = 0;
    while i < 9 {
        c[i] = CONFIG[i];
        i += 1;
    }
    c[2] = SERIAL_CONFIG_LEN as u8;
    c[3] = 0;
    c[4] = 2;
    while i < SERIAL_CONFIG_LEN {
        c[i] = CONFIG[i + 8];
        i += 1;
    }
    c
};

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
    string_descriptor!(PRODUCT, "RetroPane 26");
}
mod s3 {
    use super::string;
    // 16 upper-case hex digits, as the Bulk-Only Transport specification
    // requires ("RP260001" in hex): Windows keys the drive letter on it.
    string_descriptor!(SERIAL, "5250323630303031");
}
mod s4 {
    use super::string;
    // The serial-only layout's serial number, as before ADR-0014, so the
    // fallback matches the device instance Windows already has.
    string_descriptor!(SERIAL_LEGACY, "RP26-0001");
}
pub use s1::MANUFACTURER;
pub use s2::PRODUCT;
pub use s3::SERIAL;
pub use s4::SERIAL_LEGACY;
mod s5 {
    use super::string;
    // The MTP interface's name: libmtp (Linux) finds MTP devices by it.
    string_descriptor!(MTP_NAME, "MTP");
}
pub use s5::MTP_NAME;

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

/// MTP's class requests [SIC 5.2] (`wIndex`: the interface, errata 2007).
mod mtp {
    pub const CANCEL: u8 = 0x64;
    pub const DEVICE_RESET: u8 = 0x66;
    pub const GET_DEVICE_STATUS: u8 = 0x67;
}

/// Mass storage class request codes (Bulk-Only Transport 3.1, 3.2).
mod msc {
    pub const GET_MAX_LUN: u8 = 0xFE;
    pub const RESET: u8 = 0xFF;
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
    /// Reply buffer for Get Device Status.
    buf_status: [u8; 12],
    /// The pending OUT data request.
    pending_out: u8,
    /// Get Max LUN's answer.
    max_lun: u8,
    /// The host sent Bulk-Only Mass Storage Reset.
    msc_reset: bool,
    /// What the device presents.
    layout: Layout,
    /// What the host asked for of Microsoft's OS descriptors since the
    /// last enumeration: string 0xEE, the 1.0 compatible ID, the BOS, the
    /// 2.0 set (the Windows signal, ADR-0037 rule 4).
    ms_seen: u8,
    /// MTP: a Cancel request's transaction ID, for the function.
    mtp_cancel: Option<u32>,
    /// MTP: a Device Reset request, for the function.
    mtp_reset: bool,
    /// MTP: Get Device Status's answer (the function sets it).
    mtp_status: [u8; 12],
    mtp_status_len: usize,
    /// An endpoint the host cleared a halt on (CLEAR_FEATURE), for the
    /// controller.
    clear_halt: Option<u8>,
}

/// Bits of [`Cdc::ms_seen`].
pub mod ms {
    /// String 0xEE was read.
    pub const STRING: u8 = 1;
    /// The OS 1.0 compatible ID was read.
    pub const COMPAT_ID: u8 = 2;
    /// The BOS descriptor was read.
    pub const BOS: u8 = 4;
    /// The OS 2.0 descriptor set was read (only Windows asks).
    pub const SET20: u8 = 8;
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
            buf_status: [0; 12],
            pending_out: 0,
            max_lun: 0,
            msc_reset: false,
            layout: Layout::Drive,
            ms_seen: 0,
            mtp_cancel: None,
            mtp_reset: false,
            mtp_status: [4, 0, 0x01, 0x20, 0, 0, 0, 0, 0, 0, 0, 0],
            mtp_status_len: 4,
            clear_halt: None,
        }
    }

    /// Chooses the drive layout (serial port + drive) or the serial-only
    /// fallback; takes effect at the next enumeration.
    pub fn set_composite(&mut self, on: bool) {
        self.layout = if on { Layout::Drive } else { Layout::Serial };
    }

    /// Whether a composite layout (drive or MTP) is presented.
    #[must_use]
    pub fn composite(&self) -> bool {
        self.layout != Layout::Serial
    }

    /// Chooses what the device presents at the next enumeration.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
    }

    /// What the device presents.
    #[must_use]
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// What the host read of Microsoft's OS descriptors ([`ms`] bits),
    /// cleared by a bus reset's caller ([`Cdc::forget_ms`]).
    #[must_use]
    pub fn ms_seen(&self) -> u8 {
        self.ms_seen
    }

    /// Clears [`Cdc::ms_seen`] (a new enumeration).
    pub fn forget_ms(&mut self) {
        self.ms_seen = 0;
    }

    /// A Cancel request's transaction ID, once.
    pub fn take_mtp_cancel(&mut self) -> Option<u32> {
        self.mtp_cancel.take()
    }

    /// Whether a Device Reset request came, once.
    pub fn take_mtp_reset(&mut self) -> bool {
        core::mem::take(&mut self.mtp_reset)
    }

    /// Sets Get Device Status's answer (at most 12 bytes).
    pub fn set_mtp_status(&mut self, status: &[u8]) {
        let n = status.len().min(self.mtp_status.len());
        self.mtp_status[..n].copy_from_slice(&status[..n]);
        self.mtp_status_len = n;
    }

    /// An endpoint whose halt the host cleared, once.
    pub fn take_clear_halt(&mut self) -> Option<u8> {
        self.clear_halt.take()
    }

    /// Sets Get Max LUN's answer (logical units - 1).
    pub fn set_max_lun(&mut self, max_lun: u8) {
        self.max_lun = max_lun;
    }

    /// Whether the host asked for a mass storage reset since the last call.
    pub fn take_msc_reset(&mut self) -> bool {
        core::mem::take(&mut self.msc_reset)
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
        let mtp = self.layout == Layout::Mtp;
        let drive = self.layout == Layout::Drive;
        match (s.request_type, s.request) {
            // GET_DESCRIPTOR
            (0x80, 6) => match ((s.value >> 8) as u8, s.value as u8) {
                (1, _) => cut(match self.layout {
                    Layout::Drive => &DEVICE,
                    Layout::Mtp => &DEVICE_MTP,
                    Layout::Serial => &DEVICE_SERIAL,
                }),
                (2, 0) => cut(match self.layout {
                    Layout::Drive => &CONFIG,
                    Layout::Mtp => &CONFIG_MTP,
                    Layout::Serial => &CONFIG_SERIAL,
                }),
                (3, 0) => cut(&LANGUAGES),
                (3, 1) => cut(&MANUFACTURER),
                (3, 2) => cut(&PRODUCT),
                (3, 3) if self.layout != Layout::Serial => cut(&SERIAL),
                (3, 3) => cut(&SERIAL_LEGACY),
                (3, 5) if mtp => cut(&MTP_NAME),
                (3, 0xEE) if mtp => {
                    self.ms_seen |= ms::STRING;
                    cut(&MS_OS_STRING)
                }
                (0x0F, 0) if mtp => {
                    self.ms_seen |= ms::BOS;
                    cut(&BOS)
                }
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
            (0x00..=0x02, 1 | 3) => {
                // CLEAR_FEATURE(ENDPOINT_HALT) on one of MTP's bulk
                // endpoints after a stall [SIC 7.2.1]: the controller
                // clears it.
                if mtp
                    && s.request_type == 0x02
                    && s.request == 1
                    && s.value == 0
                    && matches!(s.index, 0x04 | 0x85)
                {
                    self.clear_halt = Some(s.index as u8);
                }
                Reply::Ack
            }
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
            // Microsoft's OS descriptors (device or interface recipient).
            (0xC0 | 0xC1, MS_VENDOR_CODE) if mtp && s.index == 4 => {
                self.ms_seen |= ms::COMPAT_ID;
                cut(&MS_COMPAT_ID)
            }
            (0xC0, MS_VENDOR_CODE) if mtp && s.index == 7 => {
                self.ms_seen |= ms::SET20;
                cut(&MS_OS20_SET)
            }
            // MTP's class requests on its interface [SIC 5.2].
            (0x21, mtp::CANCEL) if mtp && s.index == MTP_INTERFACE && want == 6 => {
                self.pending_out = mtp::CANCEL;
                Reply::Out(6)
            }
            (0x21, mtp::DEVICE_RESET) if mtp && s.index == MTP_INTERFACE && want == 0 => {
                self.mtp_reset = true;
                Reply::Ack
            }
            (0xA1, mtp::GET_DEVICE_STATUS) if mtp && s.index == MTP_INTERFACE => {
                let n = self.mtp_status_len.min(want);
                self.buf_status = self.mtp_status;
                Reply::In(&self.buf_status[..n])
            }
            (0xA1, msc::GET_MAX_LUN) if drive && s.index == MSC_INTERFACE && s.value == 0 => {
                self.buf[0] = self.max_lun;
                Reply::In(&self.buf[..1.min(want)])
            }
            (0x21, msc::RESET)
                if drive && s.index == MSC_INTERFACE && s.value == 0 && want == 0 =>
            {
                self.msc_reset = true;
                Reply::Ack
            }
            _ => Reply::Stall,
        }
    }

    /// The data stage of an OUT request announced by [`Reply::Out`].
    pub fn out_data(&mut self, data: &[u8]) {
        if self.pending_out == cdc::SET_LINE_CODING && data.len() == 7 {
            self.line_coding.copy_from_slice(data);
        }
        // Cancel: the cancellation code, then the transaction [SIC 5.2.1.2].
        if self.pending_out == mtp::CANCEL
            && data.len() == 6
            && u16::from_le_bytes([data[0], data[1]]) == 0x4001
        {
            self.mtp_cancel = Some(u32::from_le_bytes([data[2], data[3], data[4], data[5]]));
        }
        self.pending_out = 0;
    }
}
