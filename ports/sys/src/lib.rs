//! The port layer: what a game written in C calls for files, sound, input
//! and the screen ([half-life-readiness.md](../../../docs/design/half-life-readiness.md),
//! lines 4 and 5): the files ([`files`]), the clock ([`clock`]), sound
//! ([`sound`]), input ([`input`]) and what
//! answers a C program's library calls over them ([`sys`]).
//!
//! Iterate, don't imitate (rule 0). Windows Mobile gives a C program
//! `CreateFile` over a kernel file cache; Linux and every console port give
//! it POSIX files, most often through SDL; both pay for a general cache in
//! RAM the X51v does not have to spare. Ours is a table of handles straight
//! over the card's own FAT or exFAT code (`rp_fat`'s node layer, proven on
//! the X51v's cards): a read goes to the card in the runs the card is
//! fastest at, the C library's own buffer is the only one, and a game's big
//! files are read once into place. A file written is kept whole in the
//! game's room and put on the card when it is closed, named only once every
//! byte is there (the node layer's rule: a power cut leaves the old file or
//! the new one, never half a save).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod clock;
pub mod files;
pub mod input;
pub mod sound;
pub mod sys;
