//! MTP 1.1 datacodes (ADR-0037; [MTP] appendices A, B, C, D, E, F, G),
//! recognised by value only, never by range [MTP 3.3].

/// Container types [SIC 7.1.1].
pub mod container {
    /// A command block.
    pub const COMMAND: u16 = 1;
    /// A data block.
    pub const DATA: u16 = 2;
    /// A response block.
    pub const RESPONSE: u16 = 3;
    /// An event block (interrupt pipe only).
    pub const EVENT: u16 = 4;
}

/// Operation codes [MTP D.1, E.1].
#[allow(missing_docs)]
pub mod op {
    pub const GET_DEVICE_INFO: u16 = 0x1001;
    pub const OPEN_SESSION: u16 = 0x1002;
    pub const CLOSE_SESSION: u16 = 0x1003;
    pub const GET_STORAGE_IDS: u16 = 0x1004;
    pub const GET_STORAGE_INFO: u16 = 0x1005;
    pub const GET_NUM_OBJECTS: u16 = 0x1006;
    pub const GET_OBJECT_HANDLES: u16 = 0x1007;
    pub const GET_OBJECT_INFO: u16 = 0x1008;
    pub const GET_OBJECT: u16 = 0x1009;
    pub const GET_THUMB: u16 = 0x100A;
    pub const DELETE_OBJECT: u16 = 0x100B;
    pub const SEND_OBJECT_INFO: u16 = 0x100C;
    pub const SEND_OBJECT: u16 = 0x100D;
    pub const FORMAT_STORE: u16 = 0x100F;
    pub const RESET_DEVICE: u16 = 0x1010;
    pub const SET_OBJECT_PROTECTION: u16 = 0x1012;
    pub const POWER_DOWN: u16 = 0x1013;
    pub const GET_DEVICE_PROP_DESC: u16 = 0x1014;
    pub const GET_DEVICE_PROP_VALUE: u16 = 0x1015;
    pub const SET_DEVICE_PROP_VALUE: u16 = 0x1016;
    pub const RESET_DEVICE_PROP_VALUE: u16 = 0x1017;
    pub const MOVE_OBJECT: u16 = 0x1019;
    pub const COPY_OBJECT: u16 = 0x101A;
    pub const GET_PARTIAL_OBJECT: u16 = 0x101B;
    pub const GET_OBJECT_PROPS_SUPPORTED: u16 = 0x9801;
    pub const GET_OBJECT_PROP_DESC: u16 = 0x9802;
    pub const GET_OBJECT_PROP_VALUE: u16 = 0x9803;
    pub const SET_OBJECT_PROP_VALUE: u16 = 0x9804;
    pub const GET_OBJECT_PROP_LIST: u16 = 0x9805;
    pub const SET_OBJECT_PROP_LIST: u16 = 0x9806;
    pub const SEND_OBJECT_PROP_LIST: u16 = 0x9808;
    pub const GET_OBJECT_REFERENCES: u16 = 0x9810;
    pub const SET_OBJECT_REFERENCES: u16 = 0x9811;
}

/// Response codes [MTP F.1].
#[allow(missing_docs)]
pub mod rc {
    pub const OK: u16 = 0x2001;
    pub const GENERAL_ERROR: u16 = 0x2002;
    pub const SESSION_NOT_OPEN: u16 = 0x2003;
    pub const INVALID_TRANSACTION_ID: u16 = 0x2004;
    pub const OPERATION_NOT_SUPPORTED: u16 = 0x2005;
    pub const PARAMETER_NOT_SUPPORTED: u16 = 0x2006;
    pub const INCOMPLETE_TRANSFER: u16 = 0x2007;
    pub const INVALID_STORAGE_ID: u16 = 0x2008;
    pub const INVALID_OBJECT_HANDLE: u16 = 0x2009;
    pub const DEVICE_PROP_NOT_SUPPORTED: u16 = 0x200A;
    pub const INVALID_OBJECT_FORMAT_CODE: u16 = 0x200B;
    pub const STORE_FULL: u16 = 0x200C;
    pub const OBJECT_WRITE_PROTECTED: u16 = 0x200D;
    pub const STORE_READ_ONLY: u16 = 0x200E;
    pub const ACCESS_DENIED: u16 = 0x200F;
    pub const NO_THUMBNAIL_PRESENT: u16 = 0x2010;
    pub const PARTIAL_DELETION: u16 = 0x2012;
    pub const STORE_NOT_AVAILABLE: u16 = 0x2013;
    pub const SPECIFICATION_BY_FORMAT_UNSUPPORTED: u16 = 0x2014;
    pub const NO_VALID_OBJECT_INFO: u16 = 0x2015;
    pub const DEVICE_BUSY: u16 = 0x2019;
    pub const INVALID_PARENT_OBJECT: u16 = 0x201A;
    pub const INVALID_DEVICE_PROP_FORMAT: u16 = 0x201B;
    pub const INVALID_DEVICE_PROP_VALUE: u16 = 0x201C;
    pub const INVALID_PARAMETER: u16 = 0x201D;
    pub const SESSION_ALREADY_OPEN: u16 = 0x201E;
    pub const TRANSACTION_CANCELLED: u16 = 0x201F;
    pub const INVALID_OBJECT_PROP_CODE: u16 = 0xA801;
    pub const INVALID_OBJECT_PROP_FORMAT: u16 = 0xA802;
    pub const INVALID_OBJECT_PROP_VALUE: u16 = 0xA803;
    pub const GROUP_NOT_SUPPORTED: u16 = 0xA805;
    pub const INVALID_DATASET: u16 = 0xA806;
    pub const SPECIFICATION_BY_GROUP_UNSUPPORTED: u16 = 0xA807;
    pub const OBJECT_TOO_LARGE: u16 = 0xA809;
    pub const OBJECT_PROP_NOT_SUPPORTED: u16 = 0xA80A;
}

/// Event codes [MTP G.1].
#[allow(missing_docs)]
pub mod ev {
    pub const OBJECT_ADDED: u16 = 0x4002;
    pub const OBJECT_REMOVED: u16 = 0x4003;
    pub const STORE_ADDED: u16 = 0x4004;
    pub const STORE_REMOVED: u16 = 0x4005;
    pub const DEVICE_PROP_CHANGED: u16 = 0x4006;
    pub const OBJECT_INFO_CHANGED: u16 = 0x4007;
    pub const STORAGE_INFO_CHANGED: u16 = 0x400C;
    pub const UNREPORTED_STATUS: u16 = 0x400E;
}

/// Object format codes [MTP A.1].
#[allow(missing_docs)]
pub mod fmt {
    pub const UNDEFINED: u16 = 0x3000;
    pub const ASSOCIATION: u16 = 0x3001;
    pub const TEXT: u16 = 0x3004;
    pub const HTML: u16 = 0x3005;
    pub const WAV: u16 = 0x3008;
    pub const MP3: u16 = 0x3009;
    pub const AVI: u16 = 0x300A;
    pub const MPEG: u16 = 0x300B;
    pub const ASF: u16 = 0x300C;
    pub const EXIF_JPEG: u16 = 0x3801;
    pub const BMP: u16 = 0x3804;
    pub const GIF: u16 = 0x3807;
    pub const JFIF: u16 = 0x3808;
    pub const PNG: u16 = 0x380B;
    pub const TIFF: u16 = 0x380D;
    pub const WMA: u16 = 0xB901;
    pub const OGG: u16 = 0xB902;
    pub const AAC: u16 = 0xB903;
    pub const FLAC: u16 = 0xB906;
    pub const WMV: u16 = 0xB981;
    pub const MP4: u16 = 0xB982;
    pub const M3U: u16 = 0xBA11;
    pub const XML: u16 = 0xBA82;
    pub const DOC: u16 = 0xBA83;
    pub const XLS: u16 = 0xBA85;
    pub const PPT: u16 = 0xBA86;
}

/// Object property codes [MTP B.1].
#[allow(missing_docs)]
pub mod prop {
    pub const STORAGE_ID: u16 = 0xDC01;
    pub const OBJECT_FORMAT: u16 = 0xDC02;
    pub const PROTECTION_STATUS: u16 = 0xDC03;
    pub const OBJECT_SIZE: u16 = 0xDC04;
    pub const ASSOCIATION_TYPE: u16 = 0xDC05;
    pub const ASSOCIATION_DESC: u16 = 0xDC06;
    pub const OBJECT_FILE_NAME: u16 = 0xDC07;
    pub const DATE_CREATED: u16 = 0xDC08;
    pub const DATE_MODIFIED: u16 = 0xDC09;
    pub const PARENT_OBJECT: u16 = 0xDC0B;
    pub const HIDDEN: u16 = 0xDC0D;
    pub const SYSTEM_OBJECT: u16 = 0xDC0E;
    pub const PERSISTENT_UID: u16 = 0xDC41;
    pub const NAME: u16 = 0xDC44;
    pub const DATE_ADDED: u16 = 0xDC4E;
}

/// Device property codes [MTP C.1].
#[allow(missing_docs)]
pub mod dprop {
    pub const SYNC_PARTNER: u16 = 0xD401;
    pub const FRIENDLY_NAME: u16 = 0xD402;
    pub const DEVICE_ICON: u16 = 0xD405;
    pub const SESSION_INITIATOR: u16 = 0xD406;
    pub const PERCEIVED_TYPE: u16 = 0xD407;
}

/// Simple type codes [MTP 3.2.1].
#[allow(missing_docs)]
pub mod ty {
    pub const UINT8: u16 = 0x0002;
    pub const UINT16: u16 = 0x0004;
    pub const UINT32: u16 = 0x0006;
    pub const UINT64: u16 = 0x0008;
    pub const UINT128: u16 = 0x000A;
    pub const AUINT8: u16 = 0x4002;
    pub const STR: u16 = 0xFFFF;
}

/// Class requests [SIC 5.2] (`wIndex` = the interface number: the 2007
/// errata).
#[allow(missing_docs)]
pub mod class {
    pub const CANCEL: u8 = 0x64;
    pub const GET_EXTENDED_EVENT_DATA: u8 = 0x65;
    pub const DEVICE_RESET: u8 = 0x66;
    pub const GET_DEVICE_STATUS: u8 = 0x67;
    /// The cancellation code in a Cancel request's data [SIC 5.2.1.2].
    pub const CANCEL_CODE: u16 = 0x4001;
}
