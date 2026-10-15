//! LegacySync: the Windows companion that does ActiveSync's jobs for
//! RetroPane (ADR-0028).
//!
//! This first part stands in for ActiveSync's Application Manager
//! (`CeAppMgr.exe`): old Pocket PC `setup.exe` installers hand it an INI,
//! it checks the INI's CABs, chooses the one for the X51v and stages it
//! with a readable install plan. Sending it to the device waits for the
//! LegacySync session on the USB link (docs/legacysync/device-link.md).
//!
//! Interfaces it presents are documented in
//! docs/legacysync/activesync-interfaces.md (facts from the owner's discs
//! and PC; no Microsoft code).

#![forbid(unsafe_code)]

pub mod ini;
pub mod registration;
pub mod select;
pub mod stage;
