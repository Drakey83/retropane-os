//! The mix's last steps (ADR-0032 R4 steps 5-7), per output, a block at a
//! time, in this fixed order (the owner's condition 2):
//!
//! 5. **Speaker protection** on the speaker output only, always on, not a
//!    user setting: a second-order high-pass ([`SPEAKER_HP_HZ`]) that keeps
//!    the bass the small speaker can't play (docs/hw/audio.md: about
//!    -20 dB per octave below 1 kHz) out of the amplifier.
//! 6. **The limiter** (condition 7): the block's peak drives a gain, down
//!    within the block (**~0.7 ms attack**, [`BLOCK`] frames), held
//!    [`HOLD_BLOCKS`] blocks (20 ms: a 50 Hz wave's period, so the gain
//!    doesn't creep up between a low note's peaks and clip the next), then
//!    back up with a **100 ms release**; a **hard clip only as the
//!    backstop** for a peak the attack can't reach in time (counted).
//! 7. **TPDF dither** back to 16 bits, only where the mix doesn't land on a
//!    16-bit value already: a lone 44.1 kHz stream at full volume comes out
//!    bit-exact (A1), and digital silence stays silent.
//!
//! Samples are Q8 (16-bit shifted up 8) in 32 bits. All of the chain's
//! state is one small [`State`], so the mixer can keep a copy at every
//! rewrite point (R2) and re-mix from there exactly.

use crate::resample::sin;

/// Frames per block: the limiter's step (0.73 ms at 44.1 kHz).
pub const BLOCK: usize = 32;
/// The speaker's high-pass corner (Hz). Ours, not copied: below the 440 Hz
/// the owner already finds weak on this speaker, so it costs that note
/// about 1 dB while taking out the 100-200 Hz the speaker turns into
/// crackle. Tuned by ear with the owner (S2 device run, the A/B test).
pub const SPEAKER_HP_HZ: u32 = 300;
/// Full scale, Q8.
pub const CEIL: i32 = (i16::MAX as i32) << 8;
/// Where the limiter, once an overload set it off, holds the peaks: 0.5 dB
/// under full scale, so a peak that grows from block to block (two notes
/// beating) stays under the backstop while the gain follows it. Sound that
/// never overloads is never touched (bit-exact, A1).
pub const TARGET: i32 = CEIL - CEIL / 18;
/// [`State`] as bytes ([`State::save`]).
pub const STATE_BYTES: usize = 42;
/// Unity gain, Q16.
const UNITY: i32 = 1 << 16;
/// The release per block, Q16: `1 - exp(-BLOCK / (0.1 s * 44100))`.
const RELEASE: i32 = 474;
/// Blocks the gain holds after the last peak that needed it (20.3 ms).
pub const HOLD_BLOCKS: u16 = 28;

/// A biquad's coefficients, Q30, `a0` divided out: `[b0, b1, b2, a1, a2]`.
pub type Biquad = [i32; 5];

/// The RBJ cookbook high-pass at `hz`, Q = 1/sqrt(2), for 44.1 kHz.
#[must_use]
pub const fn high_pass(hz: u32) -> Biquad {
    let w0 = 2.0 * core::f64::consts::PI * hz as f64 / crate::resample::OUT as f64;
    let (s, c) = (sin(w0), sin(w0 + core::f64::consts::FRAC_PI_2));
    let alpha = s / (2.0 * core::f64::consts::FRAC_1_SQRT_2);
    let a0 = 1.0 + alpha;
    let q = (1u64 << 30) as f64;
    let b0 = (1.0 + c) / 2.0 / a0;
    [
        round(b0 * q),
        round(-(1.0 + c) / a0 * q),
        round(b0 * q),
        round(-2.0 * c / a0 * q),
        round((1.0 - alpha) / a0 * q),
    ]
}

const fn round(v: f64) -> i32 {
    if v >= 0.0 {
        (v + 0.5) as i64 as i32
    } else {
        (v - 0.5) as i64 as i32
    }
}

/// The speaker's filter.
pub const SPEAKER_HP: Biquad = high_pass(SPEAKER_HP_HZ);
// The high-pass's numerator is b0 (1, -2, 1) exactly in its stored Q30
// words, so [`high_pass_step`] may take it as one product.
const _: () = assert!(SPEAKER_HP[2] == SPEAKER_HP[0] && SPEAKER_HP[1] == -2 * SPEAKER_HP[0]);

/// The chain's state between blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    /// The limiter's gain, Q16 (unity: 65536).
    pub gain: i32,
    /// Blocks left before the gain may rise.
    hold: u16,
    /// The high-pass per channel: `x[n-1], x[n-2], y[n-1], y[n-2]`, Q8.
    hp: [[i32; 4]; 2],
    /// The dither's generator (xorshift32, never 0).
    rng: u32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            gain: UNITY,
            hold: 0,
            hp: [[0; 4]; 2],
            rng: 0x2545_F491,
        }
    }
}

/// What a block did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// Samples the backstop clipped.
    pub clipped: u32,
    /// The limiter was below unity somewhere in the block.
    pub limited: bool,
}

impl State {
    /// The state as bytes (kept by the mixer at its rewrite points, in the
    /// memory it was given).
    #[must_use]
    pub fn save(&self) -> [u8; STATE_BYTES] {
        let mut b = [0u8; STATE_BYTES];
        b[0..4].copy_from_slice(&self.gain.to_le_bytes());
        b[4..6].copy_from_slice(&self.hold.to_le_bytes());
        b[6..10].copy_from_slice(&self.rng.to_le_bytes());
        for (i, v) in self.hp.iter().flatten().enumerate() {
            b[10 + 4 * i..14 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// Back from [`State::save`]'s bytes.
    #[must_use]
    pub fn load(b: &[u8; STATE_BYTES]) -> Self {
        let w = |i: usize| i32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut hp = [[0; 4]; 2];
        for (i, v) in hp.iter_mut().flatten().enumerate() {
            *v = w(10 + 4 * i);
        }
        Self {
            gain: w(0),
            hold: u16::from_le_bytes([b[4], b[5]]),
            rng: w(6) as u32,
            hp,
        }
    }

    /// Runs one block: `mix` is [`BLOCK`] stereo frames, interleaved, Q8;
    /// `speaker` turns the speaker's filter on; `out` gets the 16-bit
    /// frames, left in the low half (the PXA's `SADR`), little-endian.
    ///
    /// R360 (2026-11-23), the same output bit for bit as
    /// [`State::run_reference`] (a test holds them equal), in fewer cycles
    /// on the PXA270 (its disassembly, run 339a's runner): the filter's
    /// sums as one `SMULL` and two `SMLAL`s a sample (the denominator's
    /// coefficients stored negated, and hidden from the compiler, which
    /// made each product with a constant a `UMULL` and an `MLA` sign fix
    /// and each subtraction a 64-bit one: 16 instructions a sample where
    /// there were about 32), the peak branch-free, then the dither and the
    /// packing in one branch-free pass with the generator in a register
    /// (it was stored every frame) and one word store a frame where the
    /// ring is word-aligned (it always is: the DMA ring starts on a cache
    /// line), four byte stores otherwise.
    pub fn run(&mut self, mix: &mut [i32; 2 * BLOCK], speaker: bool, out: &mut [u8]) -> Block {
        debug_assert_eq!(out.len(), 4 * BLOCK);
        let peak = if speaker {
            self.high_pass_both(mix)
        } else {
            peak_of(mix)
        };
        let mut b = self.limit_at(mix, peak);
        let mut rng = self.rng;
        if let Some(words) = rp_hal_simd::words_mut(out).filter(|w| w.len() == BLOCK) {
            for (w, f) in words.iter_mut().zip(mix.as_chunks::<2>().0) {
                let (l, r) = dither_fast(&mut rng, f[0], f[1]);
                *w = (u32::from(l as u16) | u32::from(r as u16) << 16).to_le();
            }
        } else {
            for (o, f) in out
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(mix.as_chunks::<2>().0)
            {
                let (l, r) = dither_fast(&mut rng, f[0], f[1]);
                let v = u32::from(l as u16) | u32::from(r as u16) << 16;
                o.copy_from_slice(&v.to_le_bytes());
            }
        }
        self.rng = rng;
        b.limited |= self.gain < UNITY;
        b
    }

    /// The speaker's filter on both channels, a channel at a time (its
    /// state in registers: both at once spilled to the stack), returning
    /// the filtered block's peak ([`peak_of`]).
    fn high_pass_both(&mut self, mix: &mut [i32; 2 * BLOCK]) -> i32 {
        // The coefficients as values the compiler can't see into: with them
        // as constants it multiplied each 64-bit product by UMULL and an
        // MLA sign fix (six multiplies a sample, the disassembly of run
        // 339a's runner); as plain i32s each is one SMULL or SMLAL.
        let c = core::hint::black_box([SPEAKER_HP[0], NEG_A1, NEG_A2]);
        for (ch, z) in self.hp.iter_mut().enumerate() {
            let mut st = *z;
            for f in mix.as_chunks_mut::<2>().0 {
                f[ch] = high_pass_mac(&c, &mut st, f[ch]);
            }
            *z = st;
        }
        peak_of(mix)
    }

    /// The old way (before R360's fused pass, kept for the A/B:
    /// `-rpsndchain0` in the port runner, and as the reference its test
    /// holds [`State::run`] to).
    pub fn run_reference(
        &mut self,
        mix: &mut [i32; 2 * BLOCK],
        speaker: bool,
        out: &mut [u8],
    ) -> Block {
        debug_assert_eq!(out.len(), 4 * BLOCK);
        if speaker {
            // A channel at a time, its state held in locals.
            for (c, z) in self.hp.iter_mut().enumerate() {
                let mut st = *z;
                for f in 0..BLOCK {
                    let s = &mut mix[2 * f + c];
                    *s = high_pass_step(&SPEAKER_HP, &mut st, *s);
                }
                *z = st;
            }
        }
        let mut b = self.limit(mix);
        for (f, frame) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let (l, r) = self.dither(mix[2 * f], mix[2 * f + 1]);
            let v = u32::from(l as u16) | u32::from(r as u16) << 16;
            frame.copy_from_slice(&v.to_le_bytes());
        }
        b.limited |= self.gain < UNITY;
        b
    }

    /// The limiter, then the backstop.
    fn limit(&mut self, mix: &mut [i32; 2 * BLOCK]) -> Block {
        // Each sign against its own full scale: -32768 is in range.
        let peak = mix
            .iter()
            .fold(0i32, |m, &s| m.max(if s < 0 { -(s + 256) } else { s }));
        self.limit_at(mix, peak)
    }

    /// The limiter and the backstop, given the block's peak ([`peak_of`]).
    fn limit_at(&mut self, mix: &mut [i32; 2 * BLOCK], peak: i32) -> Block {
        let peak = i64::from(peak);
        let want = if peak > i64::from(CEIL) {
            ((i64::from(TARGET) << 16) / peak) as i32
        } else {
            UNITY
        };
        let from = self.gain;
        let to = if want < from {
            // Attack: all the way within this block.
            self.hold = HOLD_BLOCKS;
            want
        } else if want < UNITY && want <= from + (from >> 6) {
            // Still near the level that needed it: hold.
            self.hold = HOLD_BLOCKS;
            from
        } else if self.hold > 0 {
            self.hold -= 1;
            from
        } else {
            // Release: a step of the way back; the last of it snaps.
            // Rounded up, so it never stalls just short of unity.
            let t = from + ((i64::from(want - from) * i64::from(RELEASE) + 0xFFFF) >> 16) as i32;
            if want - t < 8 { want } else { t }
        };
        self.gain = to;
        let mut b = Block {
            limited: from < UNITY,
            ..Block::default()
        };
        if from == UNITY && to == UNITY {
            // Nothing to ride down: the samples go through untouched (only
            // the backstop below looks at them).
        } else {
            for (i, s) in mix.iter_mut().enumerate() {
                let f = (i / 2 + 1) as i64;
                let g = i64::from(from) + (i64::from(to - from) * f) / BLOCK as i64;
                *s = ((i64::from(*s) * g) >> 16) as i32;
            }
        }
        if from == UNITY && to == UNITY && peak <= i64::from(CEIL) {
            // Untouched and in range: nothing for the backstop.
            return b;
        }
        for s in mix.iter_mut() {
            if *s > CEIL {
                *s = CEIL;
                b.clipped += 1;
            } else if *s < -CEIL - 256 {
                *s = -CEIL - 256;
                b.clipped += 1;
            }
        }
        b
    }

    /// A frame from Q8 to 16 bits: each sample exact if it already is a
    /// 16-bit value, else rounded with triangular dither of +-1 LSB (one
    /// draw of the generator serves both: 8 bits and 8 bits each).
    fn dither(&mut self, l: i32, r: i32) -> (i16, i16) {
        // The generator moves only when a sample needs it.
        let x = if (l | r) & 0xFF == 0 {
            0
        } else {
            let mut x = self.rng;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.rng = x;
            x
        };
        let one = |s: i32, x: u32| {
            if s & 0xFF == 0 {
                return (s >> 8) as i16;
            }
            let t = (x & 0xFF) as i32 + ((x >> 8) & 0xFF) as i32 - 255;
            ((s + 128 + t) >> 8).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        };
        (one(l, x), one(r, x >> 16))
    }
}

/// A sample's size against its own sign's full scale (the limiter's
/// measure: -32768 is in range), branch-free: `s` for `s >= 0`, else
/// `-(s + 256)` (`!s` is `-s - 1`, less 255 more).
#[inline(always)]
fn magnitude(s: i32) -> i32 {
    let sign = s >> 31;
    (s ^ sign) - (sign & 255)
}

/// The block's peak as the limiter measures it ([`magnitude`]'s largest,
/// at least 0).
fn peak_of(mix: &[i32; 2 * BLOCK]) -> i32 {
    mix.iter().fold(0i32, |m, &s| m.max(magnitude(s)))
}

/// [`State::dither`]'s frame with the generator in a local: the same
/// values, and the generator moves on exactly when that one's does. A
/// sample already on a 16-bit value takes `-128` in place of the dither,
/// so one rounding serves both cases (`(s + 128 - 128) >> 8` is `s >> 8`,
/// in range: the limiter's backstop or its peak test keeps every sample in
/// `[-CEIL - 256, CEIL]`, so the clamp leaves it).
#[inline(always)]
fn dither_fast(rng: &mut u32, l: i32, r: i32) -> (i16, i16) {
    let x = if (l | r) & 0xFF == 0 {
        0
    } else {
        let mut x = *rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *rng = x;
        x
    };
    let one = |s: i32, x: u32| {
        let t = if s & 0xFF == 0 {
            -128
        } else {
            (x & 0xFF) as i32 + ((x >> 8) & 0xFF) as i32 - 255
        };
        ((s + 128 + t) >> 8).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
    };
    (one(l, x), one(r, x >> 16))
}

/// The speaker filter's denominator coefficients negated (both fit: the
/// asserts), so a sample's sum is three multiply-accumulates.
const NEG_A1: i32 = -SPEAKER_HP[3];
const NEG_A2: i32 = -SPEAKER_HP[4];
const _: () = assert!(SPEAKER_HP[3] != i32::MIN && SPEAKER_HP[4] != i32::MIN);

/// [`high_pass_step`] on [`SPEAKER_HP`] as sums of products (the same
/// integers: `-(a * b)` is `(-a) * b` exactly in 64 bits); `c` is
/// `[b0, -a1, -a2]`.
#[inline(always)]
fn high_pass_mac(c: &[i32; 3], z: &mut [i32; 4], x: i32) -> i32 {
    let acc = i64::from(c[0]) * i64::from(x - 2 * z[0] + z[1])
        + i64::from(c[2]) * i64::from(z[3])
        + i64::from(c[1]) * i64::from(z[2]);
    let y = (acc >> 30) as i32;
    *z = [x, z[0], y, z[2]];
    y
}

/// The cookbook biquad for a high-pass whose numerator is `b0 (1, -2, 1)`
/// (the stored words, asserted above): `b0 x + b1 x1 + b2 x2` is
/// `b0 (x - 2 x1 + x2)` in exact integers (the sum fits 32 bits: Q8
/// samples are under 2^24), so three products a sample where the
/// cookbook's form takes five. The same output bit for bit (a test holds
/// them equal).
#[inline(always)]
fn high_pass_step(c: &Biquad, z: &mut [i32; 4], x: i32) -> i32 {
    let acc = i64::from(c[0]) * i64::from(x - 2 * z[0] + z[1])
        - i64::from(c[3]) * i64::from(z[2])
        - i64::from(c[4]) * i64::from(z[3]);
    let y = (acc >> 30) as i32;
    *z = [x, z[0], y, z[2]];
    y
}

/// The cookbook's direct form I: the reference for [`high_pass_step`].
#[cfg(test)]
fn biquad(c: &Biquad, z: &mut [i32; 4], x: i32) -> i32 {
    let acc = i64::from(c[0]) * i64::from(x)
        + i64::from(c[1]) * i64::from(z[0])
        + i64::from(c[2]) * i64::from(z[1])
        - i64::from(c[3]) * i64::from(z[2])
        - i64::from(c[4]) * i64::from(z[3]);
    let y = (acc >> 30) as i32;
    *z = [x, z[0], y, z[2]];
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn frames(out: &[u8]) -> Vec<(i16, i16)> {
        out.as_chunks::<4>()
            .0
            .iter()
            .map(|b| {
                (
                    i16::from_le_bytes([b[0], b[1]]),
                    i16::from_le_bytes([b[2], b[3]]),
                )
            })
            .collect()
    }

    /// Runs a signal (Q8, the same on both channels) through the chain.
    fn run(st: &mut State, x: &[i32], speaker: bool) -> (Vec<(i16, i16)>, Block) {
        let mut out = Vec::new();
        let mut total = Block::default();
        for chunk in x.chunks(BLOCK) {
            let mut mix = [0i32; 2 * BLOCK];
            for (i, &v) in chunk.iter().enumerate() {
                mix[2 * i] = v;
                mix[2 * i + 1] = v;
            }
            let mut o = [0u8; 4 * BLOCK];
            let b = st.run(&mut mix, speaker, &mut o);
            total.clipped += b.clipped;
            total.limited |= b.limited;
            out.extend(frames(&o));
        }
        (out, total)
    }

    fn tone(hz: f64, amp: f64, n: usize) -> Vec<i32> {
        (0..n)
            .map(|i| (amp * (2.0 * PI * hz * i as f64 / 44_100.0).sin() * 256.0).round() as i32)
            .collect()
    }

    fn rms(x: impl Iterator<Item = f64>) -> f64 {
        let (s, n) = x.fold((0.0, 0.0), |(s, n), v| (s + v * v, n + 1.0));
        (s / n).sqrt()
    }

    #[test]
    fn a_16_bit_signal_at_unity_comes_out_bit_exact() {
        let mut st = State::default();
        let x: Vec<i32> = (0..4_096)
            .map(|i| ((i * 7919) % 65_536 - 32_768) << 8)
            .collect();
        let (out, b) = run(&mut st, &x, false);
        for (i, (l, r)) in out.iter().enumerate() {
            assert_eq!(i32::from(*l) << 8, x[i], "frame {i}");
            assert_eq!(l, r);
        }
        assert_eq!(b, Block::default());
    }

    #[test]
    fn negative_full_scale_is_not_an_overload() {
        let mut st = State::default();
        let (out, b) = run(&mut st, &[i32::from(i16::MIN) << 8; 256], false);
        assert!(out.iter().all(|&f| f == (i16::MIN, i16::MIN)));
        assert_eq!(b, Block::default());
        assert_eq!(st.gain, UNITY);
    }

    #[test]
    fn silence_stays_silent() {
        let mut st = State::default();
        let (out, _) = run(&mut st, &[0; 1_024], false);
        assert!(out.iter().all(|&f| f == (0, 0)));
    }

    /// The speaker's three-product step gives the cookbook biquad's
    /// output bit for bit, across full scale, the edges and noise.
    #[test]
    fn the_high_pass_step_is_the_biquad_exactly() {
        let (mut a, mut b) = ([0i32; 4], [0i32; 4]);
        let mut r = 0x1234_5678u32;
        for k in 0..200_000u32 {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            let x = match k % 5 {
                0 => CEIL,
                1 => -CEIL - 256,
                _ => ((r as i32) >> 7).clamp(-CEIL - 256, CEIL),
            };
            assert_eq!(
                high_pass_step(&SPEAKER_HP, &mut a, x),
                biquad(&SPEAKER_HP, &mut b, x),
                "at {k}"
            );
        }
    }

    #[test]
    fn dither_is_triangular_one_lsb_and_unbiased() {
        let mut st = State::default();
        // Half an LSB: without dither every sample would round the same way.
        let (out, _) = run(&mut st, &[128; 32_768], false);
        let vals: Vec<i32> = out.iter().map(|f| i32::from(f.0)).collect();
        assert!(vals.iter().all(|v| (-1..=2).contains(v)));
        let mean = vals.iter().sum::<i32>() as f64 / vals.len() as f64;
        assert!((mean - 0.5).abs() < 0.02, "mean {mean}");
        // A quarter of an LSB averages to a quarter: the error is noise,
        // not a fixed offset.
        let (out, _) = run(&mut st, &[64; 32_768], false);
        let mean = out.iter().map(|f| f64::from(f.0)).sum::<f64>() / out.len() as f64;
        assert!((mean - 0.25).abs() < 0.02, "mean {mean}");
    }

    #[test]
    fn the_speaker_filter_cuts_the_bass_and_keeps_the_rest() {
        let gain = |hz: f64| {
            let mut st = State::default();
            let (out, _) = run(&mut st, &tone(hz, 10_000.0, 22_050), true);
            20.0 * (rms(out[11_025..].iter().map(|f| f64::from(f.0))) / (10_000.0 / 2f64.sqrt()))
                .log10()
        };
        assert!(gain(100.0) < -18.0, "100 Hz {:.1} dB", gain(100.0));
        assert!(
            (gain(300.0) + 3.0).abs() < 0.5,
            "300 Hz {:.1} dB",
            gain(300.0)
        );
        assert!(gain(440.0) > -1.6, "440 Hz {:.1} dB", gain(440.0));
        assert!(gain(2_000.0).abs() < 0.2, "2 kHz {:.1} dB", gain(2_000.0));
        // Only on the speaker: headphones get the bass.
        let mut st = State::default();
        let x = tone(100.0, 10_000.0, 4_410);
        let (out, _) = run(&mut st, &x, false);
        assert_eq!(i32::from(out[1_000].0), (x[1_000] + 128) >> 8);
    }

    #[test]
    fn an_overload_is_ridden_down_within_a_block_and_released_in_about_100_ms() {
        let mut st = State::default();
        // 6 dB over full scale for 0.2 s, then -12 dB.
        let mut x = tone(1_000.0, 65_534.0, 8_820);
        x.extend(tone(1_000.0, 8_000.0, 66_150));
        let (out, b) = run(&mut st, &x, false);
        assert!(b.limited);
        // Only the tone's first period (two blocks: until the detector has
        // seen its top) can reach the backstop.
        assert!(b.clipped <= 4 * BLOCK as u32, "clipped {}", b.clipped);
        // After that, nothing clips: the limiter alone holds it.
        let mut st3 = State::default();
        let _ = run(&mut st3, &x[..2 * BLOCK], false);
        let (_, rest) = run(&mut st3, &x[2 * BLOCK..], false);
        assert_eq!(rest.clipped, 0);
        // Held just under full scale (TARGET) while it lasts.
        let held = rms(out[4_410..8_820].iter().map(|f| f64::from(f.0)));
        let want = f64::from(TARGET >> 8) / 2f64.sqrt();
        assert!((held / want - 1.0).abs() < 0.05, "held at {held}");
        // Released: about 63% of the way back to unity 100 ms after.
        let g_at = |n: usize| {
            let mut st2 = State::default();
            let (o, _) = run(&mut st2, &x[..n], false);
            let _ = o;
            f64::from(st2.gain) / 65_536.0
        };
        let hold = HOLD_BLOCKS as usize * BLOCK;
        let at_end = g_at(8_832 + hold);
        let after = g_at(8_832 + hold + 4_416);
        let back = (after - at_end) / (1.0 - at_end);
        assert!(
            (back - 0.632).abs() < 0.05,
            "released {back:.3} of the way in 100 ms"
        );
        // And back at exactly unity in the end: bit-exact again.
        assert_eq!(st.gain, UNITY);
    }

    #[test]
    fn two_notes_beating_into_an_overload_are_held_without_clipping() {
        // The sum's peak grows a little from block to block as the two
        // notes drift into phase: the gain must follow it down without the
        // block's first samples reaching the backstop.
        let mut st = State::default();
        let a = tone(988.0, 30_000.0, 44_100);
        let b = tone(1_175.0, 30_000.0, 44_100);
        let x: Vec<i32> = a.iter().zip(&b).map(|(p, q)| p + q).collect();
        let _ = run(&mut st, &x[..2 * BLOCK], false);
        let (_, rest) = run(&mut st, &x[2 * BLOCK..], false);
        assert!(rest.limited);
        assert_eq!(rest.clipped, 0);
    }

    #[test]
    fn a_low_note_overloaded_is_held_between_its_peaks() {
        // 100 Hz peaks every 441 frames, 14 blocks apart. Without a hold the
        // gain would creep up between peaks and drop at each one: a ripple
        // at the note's own rate, which is distortion even without a clip.
        let mut st = State::default();
        let x = tone(100.0, 65_000.0, 44_100);
        let _ = run(&mut st, &x[..882], false);
        let mut gains = Vec::new();
        let mut clipped = 0;
        for chunk in x[882..].chunks(BLOCK) {
            let (_, b) = run(&mut st, chunk, false);
            clipped += b.clipped;
            gains.push(st.gain);
        }
        let (lo, hi) = (gains.iter().min().unwrap(), gains.iter().max().unwrap());
        assert!(
            f64::from(hi - lo) / f64::from(*hi) < 0.01,
            "the gain rippled {lo}..{hi}"
        );
        assert_eq!(clipped, 0);
    }

    /// R360: the fused pass gives the old way's bytes, block counts and
    /// state bit for bit, speaker on and off, over noise, full scale both
    /// ways, overloads that set the limiter off and its release, silence,
    /// samples already on 16-bit values, and outputs on and off a word.
    #[test]
    fn the_fused_pass_is_the_old_way_exactly() {
        let mut r = 0x9E37_79B9u32;
        let mut next = move || {
            r ^= r << 13;
            r ^= r >> 17;
            r ^= r << 5;
            r
        };
        for speaker in [false, true] {
            let (mut a, mut b) = (State::default(), State::default());
            // the host's allocator gives a word-aligned start (asserted)
            let mut store = vec![0u8; 4 * BLOCK + 4];
            assert!(rp_hal_simd::words_mut(&mut store[..4 * BLOCK]).is_some());
            for k in 0..20_000u32 {
                let mut mix = [0i32; 2 * BLOCK];
                let kind = (k / 37) % 7;
                for (i, s) in mix.iter_mut().enumerate() {
                    let n = next();
                    *s = match kind {
                        0 => ((n as i32) >> 7).clamp(-CEIL - 256, CEIL),
                        1 => (n as i32) >> 4, // overloads: the limiter's work
                        2 => 0,
                        3 => ((n as i32) >> 16) << 8, // on 16-bit values
                        4 => [CEIL, -CEIL - 256, CEIL * 3, -CEIL * 3][i % 4],
                        5 => ((n as i32) >> 9) & !0xFF | i32::from(i % 3 == 0),
                        _ => (n as i32) >> 12, // quiet: the release
                    };
                }
                let mut mix2 = mix;
                let (mut oa, mut ob) = ([0u8; 4 * BLOCK], [0u8; 4 * BLOCK]);
                let ra = a.run_reference(&mut mix, speaker, &mut oa);
                // the word path, and every third block the byte path
                // (an output one byte off a word)
                let off = if k % 3 == 0 { 1 } else { 0 };
                let rb = b.run(&mut mix2, speaker, &mut store[off..off + 4 * BLOCK]);
                ob.copy_from_slice(&store[off..off + 4 * BLOCK]);
                assert_eq!(oa, ob, "speaker {speaker} block {k}");
                assert_eq!(ra, rb, "speaker {speaker} block {k}");
                assert_eq!(a, b, "speaker {speaker} block {k}");
            }
        }
    }

    #[test]
    fn the_state_survives_its_bytes() {
        let mut st = State::default();
        let _ = run(&mut st, &tone(100.0, 70_000.0, 4_000), true);
        assert_eq!(State::load(&st.save()), st);
    }

    #[test]
    fn the_backstop_clips_both_ways() {
        let mut st = State::default();
        let mut mix = [0i32; 2 * BLOCK];
        mix[0] = CEIL * 4;
        mix[1] = -CEIL * 4;
        let mut o = [0u8; 4 * BLOCK];
        let b = st.run(&mut mix, false, &mut o);
        assert_eq!(frames(&o)[0], (i16::MAX, i16::MIN));
        assert!(b.clipped >= 2);
    }

    #[test]
    fn the_coefficients_are_the_cookbook_ones() {
        let w0 = 2.0 * PI * 300.0 / 44_100.0;
        let alpha = w0.sin() / (2.0 * 0.5f64.sqrt());
        let a0 = 1.0 + alpha;
        let q = f64::from(1u32 << 30);
        let want = [
            (1.0 + w0.cos()) / 2.0 / a0,
            -(1.0 + w0.cos()) / a0,
            (1.0 + w0.cos()) / 2.0 / a0,
            -2.0 * w0.cos() / a0,
            (1.0 - alpha) / a0,
        ];
        for (got, w) in SPEAKER_HP.iter().zip(want) {
            assert!((f64::from(*got) - w * q).abs() <= 1.0, "{got} vs {}", w * q);
        }
    }
}
