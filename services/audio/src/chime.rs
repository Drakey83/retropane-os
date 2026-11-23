//! The reminder's chime (calendar-rule0.md, tasks-rule0.md: reminders
//! through the banner and a sound), made as it plays.
//!
//! Found: Windows Mobile plays a wave file from the ROM through its mixer
//! (`Alarm1.wav`, tens of KB); phones play a recorded chime the same way.
//! The first chime here (R224) was two plain sine notes of 880 and 1175 Hz
//! switched on and off, each edge a click.
//!
//! Ours: two notes a fourth apart, each a sine with a 4 ms rise and a
//! bell's decay (an exponential, one multiply a sample), the second struck
//! while the first still rings, each faded over its last 6 ms: no edge to
//! click, no file, no table beyond the sine the I2S driver already has.
//! The notes are where the X51v's speaker is loudest: its response,
//! measured (docs/hw/audio.md, the sweep of 2026-10-18), is flat from 1.5
//! to 2 kHz and falls about 20 dB an octave below 1 kHz, so 1568 and 2093
//! Hz come out about 9 and 4 dB louder than 880 and 1175 did at the same
//! level (or as loud for less power).

use rp_drv_pxa_i2s::Tone;

/// One note: its pitch, when it is struck and how long it rings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Note {
    /// Pitch, Hz.
    pub hz: u32,
    /// Struck this many ms after the chime starts.
    pub at_ms: u32,
    /// Rings this many ms (its fade included).
    pub ms: u32,
}

/// The reminder's notes: G6 then C7, a rising fourth.
pub const REMINDER: [Note; 2] = [
    Note {
        hz: 1568,
        at_ms: 0,
        ms: 300,
    },
    Note {
        hz: 2093,
        at_ms: 160,
        ms: 520,
    },
];

/// The rise, ms.
const ATTACK_MS: u32 = 4;
/// The fade at a note's end, ms.
const RELEASE_MS: u32 = 6;
/// The decay's time constant, ms (a small bell's).
const DECAY_MS: u32 = 220;
/// One in Q31.
const ONE: u64 = 1 << 31;

/// The chime's stereo frames ([`Tone`]'s format: the same sample on both
/// channels), [`Chime::frames`] of them.
pub struct Chime<const N: usize> {
    tones: [Tone; N],
    /// Each note's start and end frame.
    span: [(u32, u32); N],
    /// Each note's decay so far (Q31).
    env: [u64; N],
    /// The decay a frame (Q31).
    k: u64,
    attack: u32,
    release: u32,
    t: u32,
    frames: u32,
}

impl<const N: usize> Chime<N> {
    /// `notes` at `rate` frames a second, each at `amplitude` (0..=32767;
    /// overlapping notes add, so the sum of two stays under full scale at
    /// up to 16383).
    #[must_use]
    pub fn new(notes: &[Note; N], rate: u32, amplitude: u16) -> Self {
        let f = |ms: u32| (u64::from(ms) * u64::from(rate) / 1000) as u32;
        let span = notes.map(|n| (f(n.at_ms), f(n.at_ms + n.ms)));
        let frames = span.iter().map(|s| s.1).max().unwrap_or(0);
        // exp(-1 / (tau * rate)) to first order: its error, 1 / (2 (tau
        // rate)^2) a frame, is under a millionth of the decay.
        let tau = u64::from(f(DECAY_MS).max(1));
        Self {
            tones: notes.map(|n| Tone::new(n.hz, rate, amplitude)),
            span,
            env: [ONE; N],
            k: ONE - ONE / tau,
            attack: f(ATTACK_MS).max(1),
            release: f(RELEASE_MS).max(1),
            t: 0,
            frames,
        }
    }

    /// How many frames it lasts.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }
}

impl<const N: usize> Iterator for Chime<N> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.t >= self.frames {
            return None;
        }
        let t = self.t;
        let mut sum: i32 = 0;
        for i in 0..N {
            let (s, e) = self.span[i];
            if t < s || t >= e {
                continue;
            }
            let raw = i64::from(self.tones[i].next().unwrap_or(0) as u16 as i16);
            // Rise, decay and fade, all in Q31.
            let mut g = self.env[i];
            if t - s < self.attack {
                g = g * u64::from(t - s) / u64::from(self.attack);
            }
            if e - t < self.release {
                g = g * u64::from(e - t) / u64::from(self.release);
            }
            sum += ((raw * g as i64) >> 31) as i32;
            self.env[i] = (self.env[i] * self.k) >> 31;
        }
        self.t += 1;
        let v = sum.clamp(-32767, 32767) as i16 as u16;
        Some(u32::from(v) << 16 | u32::from(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(rate: u32) -> std::vec::Vec<i32> {
        Chime::new(&REMINDER, rate, 14_000)
            .map(|f| i32::from(f as u16 as i16))
            .collect()
    }

    #[test]
    fn it_lasts_its_notes_and_both_channels_match() {
        let c = Chime::new(&REMINDER, 44_100, 14_000);
        assert_eq!(c.frames(), 44_100 * 680 / 1000);
        let n = c.frames() as usize;
        let all: std::vec::Vec<u32> = c.collect();
        assert_eq!(all.len(), n);
        assert!(all.iter().all(|f| f >> 16 == f & 0xFFFF));
    }

    #[test]
    fn no_edge_clicks_it_starts_and_ends_at_silence() {
        for rate in [44_100, 48_000, 22_050] {
            let v = samples(rate);
            assert_eq!(v[0], 0, "{rate}: from silence");
            assert!(v.last().unwrap().abs() < 300, "{rate}: to silence");
            // The steepest step a sample is the loudest sine's own (2 pi f
            // A / rate) plus a little: no jump where a note starts or ends.
            let steep = (2.0 * core::f64::consts::PI * 2093.0 * 2.0 * 14_000.0 / f64::from(rate))
                as i32
                + 64;
            let worst = v.windows(2).map(|w| (w[1] - w[0]).abs()).max().unwrap();
            assert!(worst <= steep, "{rate}: a step of {worst} (sine's {steep})");
            // Where the first note stops (300 ms) nothing jumps either.
            let end = (u64::from(rate) * 300 / 1000) as usize;
            let around = v[end - 3..end + 3].windows(2).map(|w| (w[1] - w[0]).abs());
            assert!(around.max().unwrap() <= steep);
        }
    }

    #[test]
    fn it_rings_down_like_a_bell_and_never_clips() {
        let v = samples(44_100);
        let peak = |a: usize, b: usize| v[a..b].iter().map(|x| x.abs()).max().unwrap();
        let ms = |m: usize| m * 44_100 / 1000;
        // Loudest early, softer as it rings: the first note at 20 ms beats
        // itself at 140 ms (before the second is struck).
        assert!(peak(ms(15), ms(25)) > peak(ms(135), ms(145)) * 3 / 2);
        assert!(v.iter().all(|x| x.abs() <= 32767));
        assert!(peak(0, v.len()) > 10_000, "loud enough");
    }

    #[test]
    fn the_notes_are_their_pitches() {
        // Zero crossings in the first note alone (20-150 ms) and the
        // second alone (310-600 ms): twice the pitch a second.
        let v = samples(48_000);
        let rate = 48_000.0;
        let pitch = |a: usize, b: usize| {
            let z = v[a..b]
                .windows(2)
                .filter(|w| (w[0] < 0) != (w[1] < 0))
                .count();
            z as f64 / 2.0 / ((b - a) as f64 / rate)
        };
        let hz1 = pitch(960, 7_200);
        let hz2 = pitch(14_880, 28_800);
        assert!((hz1 - 1568.0).abs() < 10.0, "{hz1}");
        assert!((hz2 - 2093.0).abs() < 10.0, "{hz2}");
    }
}
