//! The organiser's shared parts: one store for appointments, contacts,
//! tasks and notes ([`store`]) and calendar arithmetic ([`date`]). No
//! drawing and no I/O: the shell draws, the device reads and writes the
//! card.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod birthdays;
pub mod calc;
pub mod contact;
pub mod date;
pub mod expr;
pub mod net;
pub mod phone;
pub mod quick;
pub mod repeat;
pub mod store;
