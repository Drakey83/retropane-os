//! The audio service (ADR-0032), step S1: the one owner of the WM8750
//! codec, the PXA's I2S link and the speaker amplifier.
//!
//! What S1 does (ADR-0032 R5, R6, conditions 5 and 6):
//!
//! - **Power-up** in one order: the codec's supply, the pins, the I2S link
//!   (the codec's master clock) before the first I2C write, the codec set
//!   up **muted** at the session rate (44.1 kHz, vote A1), then the jack read
//!   (it is valid only now), the speaker amplifier only for the speaker,
//!   then the route and the level. Anything failing on the way puts
//!   everything back down: nothing is left half on.
//! - **Power-down** in the reverse order, the I2C pins held low as Windows
//!   Mobile does for an unpowered chip.
//! - **The restart guard:** on the X51v a switch-on of I2S within ~645 ms
//!   of a switch-off is dropped (docs/hw/audio.md). A power-up that comes
//!   sooner waits the rest of [`RESTART_GUARD_MS`] and says how long.
//! - **The headphone jack by its edge latch:** while audio is up, a jack
//!   edge is debounced ([`JACK_DEBOUNCE_MS`]) and re-routes; **while audio is
//!   down an edge only marks the jack dirty** (no power-up just to look);
//!   the next power-up reads it.
//!
//! When audio powers down is the caller's policy. ADR-0032's baseline is
//! Windows Mobile's: up while the device is awake, down on suspend; an idle
//! power-down between sounds is built only if S1's measurement of an idle
//! powered codec says it's worth it (the 645 ms guard makes it a latency
//! cost).
//!
//! The mixer, roles and volumes per output come in S2 and S3; S1 plays one
//! stream at a time ([`Audio::play`]).

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use rp_drv_pxa_i2s::{PxaI2s, Rate, StreamError, Streamed, stream};
use rp_drv_wm8750::{Output, SampleRate, Wm8750};
use rp_hal_api::{DmaRing, I2cBus, I2cError, RegisterBus};

/// The session rate on the I2S side (ADR-0032 A1: 44.1 kHz fixed).
pub const LINK_RATE: Rate = Rate::Khz44_1;
/// The same rate on the codec side.
pub const CODEC_RATE: SampleRate = SampleRate::Khz44_1;

/// How long after an I2S switch-off a switch-on is trusted: the measured
/// ~645 ms (docs/hw/audio.md, 2026-10-18) and a margin.
pub const RESTART_GUARD_MS: u32 = 700;
/// After the codec's supply comes on, before anything else. Copied from
/// Dell's driver (1 ms; imitation audit): `rp-tone` run 6 found the codec at
/// 1 ms and 300 ms alike. Measured again in S1.
// inherited: imitation-audit Audio, Dell wavedev.dll (1 ms after codec power)
pub const POWER_SETTLE_US: u32 = 1_000;
/// After I2S starts (the codec's master clock), before the first I2C
/// write. Copied from Dell's driver (1 ms; imitation audit).
// inherited: imitation-audit Audio, Dell wavedev.dll (1 ms after I2S on)
pub const CLOCK_SETTLE_US: u32 = 1_000;
/// After the speaker amplifier comes on. Copied from Dell's driver (70 ms;
/// imitation audit): its real settle is judged by ear in S1.
// inherited: imitation-audit Audio, Dell wavedev.dll (70 ms amplifier settle)
pub const AMP_SETTLE_US: u32 = 70_000;
/// After the codec's set-up, before the jack is read: the jack line
/// (GPIO83) is valid only with the codec powered and set up. Copied from
/// Dell's driver (it reads the jack 75 ms after its reset and power writes;
/// imitation audit); S1's device run samples the line at 0, 25, 75 and
/// 150 ms to replace it with a measurement. Paid once per power-up.
// inherited: imitation-audit Audio, Dell wavedev.dll (jack read 75 ms after set-up)
pub const JACK_SETTLE_US: u32 = 75_000;
/// A jack edge must hold this long before the route follows it.
pub const JACK_DEBOUNCE_MS: u32 = 50;

/// The board's audio lines, as one owner sees them.
pub trait Board {
    /// The codec's supply (X51v: CPLD `0x1C` bit 1, through the shared shadow).
    fn codec_power(&mut self, on: bool);
    /// The speaker amplifier (X51v: CPLD `0x14` bit 1).
    fn speaker_amp(&mut self, on: bool);
    /// The pins for a powered codec: I2C (SCL, SDA) to their functions, the
    /// codec's I2C gate closed, the I2S pins (bit clock, data, frame, system
    /// clock) to theirs.
    fn pins_up(&mut self);
    /// The pins for an unpowered codec: the I2C lines plain outputs held low
    /// (as Windows Mobile does), so nothing drives an unpowered chip.
    fn pins_down(&mut self);
    /// The headphone jack's level: plugged in (X51v: GPIO83 high). Valid
    /// only while the codec is powered and set up.
    fn jack_in(&mut self) -> bool;
    /// Whether the jack's edge latch caught an edge since the last call
    /// (X51v: GPIO83's `GEDR` bit, both edges armed); clears it.
    fn take_jack_edge(&mut self) -> bool;
    /// A millisecond clock (wrapping).
    fn now_ms(&mut self) -> u32;
    /// Waits `us` microseconds (keeping any watchdog fed).
    fn delay_us(&mut self, us: u32);
}

/// The I2S link at the session rate.
pub trait Link {
    /// Switches it on; `false` if the controller didn't take it.
    fn start(&mut self) -> bool;
    /// Switches it off.
    fn stop(&mut self);
}

impl<B: RegisterBus> Link for PxaI2s<B> {
    fn start(&mut self) -> bool {
        PxaI2s::start(self, LINK_RATE)
    }

    fn stop(&mut self) {
        PxaI2s::stop(self);
    }
}

/// Why audio didn't come up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    /// The I2S controller didn't take the switch-on even after the guard.
    LinkRefused,
    /// The codec didn't answer or refused a write.
    Codec(I2cError),
}

/// Why a sound didn't play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayError {
    /// Audio couldn't come up.
    Up(AudioError),
    /// The DMA stream failed.
    Stream(StreamError),
}

/// What a power-up did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Powered {
    /// Milliseconds waited for the restart guard (0 if none was due).
    pub waited_ms: u32,
    /// Where sound goes.
    pub output: Output,
    /// A jack edge came while audio was down (and was read now, not then).
    pub jack_was_dirty: bool,
    /// The codec's I2C address.
    pub address: u8,
}

/// The codec: its bus while down, the driver while up.
enum Codec<I> {
    Down(I),
    Up(Wm8750<I>),
    /// Only between the two, inside a call.
    Moving,
}

/// The audio service's S1 core.
pub struct Audio<B, I, L> {
    board: B,
    link: L,
    codec: Codec<I>,
    level: u32,
    /// When I2S was last switched off (`now_ms`), for the restart guard.
    stopped_at: Option<u32>,
    /// A jack edge came while down.
    jack_dirty: bool,
    /// A jack edge seen while up, waiting out the debounce: since when.
    jack_pending: Option<u32>,
    amp: bool,
}

impl<B: Board, I: I2cBus, L: Link> Audio<B, I, L> {
    /// Takes the board, the I2S link and the codec's I2C bus; audio is down
    /// (nothing switched). `level` is the output level, 0-100.
    pub fn new(board: B, link: L, bus: I, level: u32) -> Self {
        Self {
            board,
            link,
            codec: Codec::Down(bus),
            level: level.min(100),
            stopped_at: None,
            jack_dirty: false,
            jack_pending: None,
            amp: false,
        }
    }

    /// Whether audio is up.
    #[must_use]
    pub fn is_up(&self) -> bool {
        matches!(self.codec, Codec::Up(_))
    }

    /// Where sound goes while up.
    #[must_use]
    pub fn output(&self) -> Option<Output> {
        match &self.codec {
            Codec::Up(c) => Some(c.output()),
            _ => None,
        }
    }

    /// Whether a jack edge came while down and hasn't been read yet.
    #[must_use]
    pub fn jack_dirty(&self) -> bool {
        self.jack_dirty
    }

    /// The board (for its log or its own lines).
    pub fn board_mut(&mut self) -> &mut B {
        &mut self.board
    }

    /// The output level, 0-100.
    #[must_use]
    pub fn level(&self) -> u32 {
        self.level
    }

    /// Sets the output level (0-100), at once if up.
    pub fn set_level(&mut self, level: u32) -> Result<(), I2cError> {
        self.level = level.min(100);
        match &mut self.codec {
            Codec::Up(c) => c.set_level(self.level),
            _ => Ok(()),
        }
    }

    /// Brings audio up (nothing to do if it is). See the crate's notes for
    /// the order; any failure puts everything back down.
    pub fn up(&mut self) -> Result<Powered, AudioError> {
        if let Codec::Up(c) = &self.codec {
            return Ok(Powered {
                waited_ms: 0,
                output: c.output(),
                jack_was_dirty: false,
                address: c.address(),
            });
        }
        let waited_ms = self.wait_out_guard();
        self.board.codec_power(true);
        self.board.delay_us(POWER_SETTLE_US);
        self.board.pins_up();
        if !self.link.start() {
            self.back_down();
            return Err(AudioError::LinkRefused);
        }
        self.board.delay_us(CLOCK_SETTLE_US);
        let Codec::Down(bus) = core::mem::replace(&mut self.codec, Codec::Moving) else {
            unreachable!("audio was down")
        };
        // Muted until the route is known: nothing reaches the wrong output.
        let mut codec = match Wm8750::init_at(bus, 0, CODEC_RATE) {
            Ok(c) => c,
            Err((e, bus)) => {
                self.codec = Codec::Down(bus);
                self.back_down();
                return Err(AudioError::Codec(e));
            }
        };
        // The jack is valid once the codec is set up and settled; the
        // latch's edges until here are this read.
        self.board.delay_us(JACK_SETTLE_US);
        let plugged = self.board.jack_in();
        let _ = self.board.take_jack_edge();
        let jack_was_dirty = core::mem::take(&mut self.jack_dirty);
        self.jack_pending = None;
        let output = output(plugged);
        if output == Output::Speaker {
            self.amp_on();
        }
        let set = codec
            .route(output)
            .and_then(|()| codec.set_level(self.level));
        let address = codec.address();
        self.codec = Codec::Up(codec);
        if let Err(e) = set {
            let _ = self.down();
            return Err(AudioError::Codec(e));
        }
        Ok(Powered {
            waited_ms,
            output,
            jack_was_dirty,
            address,
        })
    }

    /// Takes audio down (nothing to do if it is): the amplifier off, the
    /// codec's insides down (muted first), I2S off (the guard starts), the
    /// pins held low, the supply off. Every step runs; the first codec
    /// write that failed is reported.
    pub fn down(&mut self) -> Result<(), I2cError> {
        if !self.is_up() {
            return Ok(());
        }
        let Codec::Up(codec) = core::mem::replace(&mut self.codec, Codec::Moving) else {
            unreachable!("audio was up")
        };
        self.amp_off();
        let (r, bus) = codec.power_down();
        self.codec = Codec::Down(bus);
        self.back_down();
        // The jack line falls with the codec's supply (it reads high only
        // while powered): that edge is ours, not a jack change. (Caught by
        // the emulator's model, 2026-10-18.)
        let _ = self.board.take_jack_edge();
        r
    }

    /// Looks at the jack's edge latch: while up, an edge that holds for
    /// [`JACK_DEBOUNCE_MS`] re-routes (the new output returned); while
    /// down, an edge only marks the jack dirty. Call it from the GPIO
    /// interrupt's event, or on a timer.
    pub fn poll_jack(&mut self) -> Option<Output> {
        let edge = self.board.take_jack_edge();
        if !self.is_up() {
            self.jack_dirty |= edge;
            return None;
        }
        let now = self.board.now_ms();
        if edge {
            // A new edge restarts the debounce.
            self.jack_pending = Some(now);
        }
        let since = self.jack_pending?;
        if now.wrapping_sub(since) < JACK_DEBOUNCE_MS {
            return None;
        }
        self.jack_pending = None;
        let want = output(self.board.jack_in());
        let Codec::Up(c) = &self.codec else {
            return None;
        };
        if c.output() == want {
            return None;
        }
        self.reroute(want).ok().map(|()| want)
    }

    /// Plays `frames` frames from `source` through `dma` by looping `ring`
    /// (as [`rp_drv_pxa_i2s::stream`]), bringing audio up first. Audio
    /// stays up after (the session rule).
    #[allow(clippy::too_many_arguments)]
    pub fn play(
        &mut self,
        dma: &mut dyn DmaRing,
        ring: &mut [u8],
        source: &mut dyn Iterator<Item = u32>,
        frames: u32,
        now: &mut dyn FnMut() -> u32,
        stall: u32,
        idle: &mut dyn FnMut(),
    ) -> Result<Streamed, PlayError> {
        self.up().map_err(PlayError::Up)?;
        stream(dma, ring, source, frames, now, stall, idle).map_err(PlayError::Stream)
    }

    /// To headphones: the speaker pair muted first, then the amplifier off.
    /// To the speaker: the amplifier on and settled, then the route.
    fn reroute(&mut self, to: Output) -> Result<(), I2cError> {
        if to == Output::Speaker {
            self.amp_on();
        }
        let r = match &mut self.codec {
            Codec::Up(c) => c.route(to),
            _ => Ok(()),
        };
        if to == Output::Headphones {
            self.amp_off();
        }
        r
    }

    fn amp_on(&mut self) {
        if !self.amp {
            self.board.speaker_amp(true);
            self.amp = true;
            self.board.delay_us(AMP_SETTLE_US);
        }
    }

    fn amp_off(&mut self) {
        if self.amp {
            self.board.speaker_amp(false);
            self.amp = false;
        }
    }

    /// Everything but the codec's own registers and the amplifier, down: I2S
    /// (its switch-off time kept for the guard), the pins, the supply. The
    /// amplifier is on only while the codec is up, and [`Self::down`] takes
    /// it off first.
    fn back_down(&mut self) {
        debug_assert!(!self.amp, "the amplifier outlived the codec");
        self.link.stop();
        self.stopped_at = Some(self.board.now_ms());
        self.board.pins_down();
        self.board.codec_power(false);
    }

    /// Waits until the restart guard has passed; how long it waited (ms).
    fn wait_out_guard(&mut self) -> u32 {
        let Some(at) = self.stopped_at else { return 0 };
        let since = self.board.now_ms().wrapping_sub(at);
        if since >= RESTART_GUARD_MS {
            return 0;
        }
        let left = RESTART_GUARD_MS - since;
        self.board.delay_us(left * 1_000);
        left
    }
}

fn output(plugged: bool) -> Output {
    if plugged {
        Output::Headphones
    } else {
        Output::Speaker
    }
}

#[cfg(test)]
mod tests;
