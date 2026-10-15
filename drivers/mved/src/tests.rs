//! The register values against the survey (docs/hw/2700g.md, "The MVED1
//! video decoder"), the job checks, and the references against a direct
//! computation, so a change can't drift from the found format unnoticed.

use super::*;

const VRAM: usize = 16 << 20;

fn frame() -> Frame420 {
    Frame420::new(64, 64).unwrap()
}

fn job(dst_stride: u32) -> Convert {
    Convert {
        frame: frame(),
        src: 0x40_0000,
        dst: 0x41_0000,
        dst_stride,
    }
}

#[test]
fn only_dells_three_registers_are_readable() {
    for off in (0..0x1000).step_by(4) {
        assert_eq!(
            may_read(off),
            [0x50, 0x80, 0xA0].contains(&off),
            "0x{off:x}"
        );
    }
    assert!(!WINDOWS.contains(&(MVED + reg::STATUS)));
    assert_eq!(MVED + 0x0FFE_0000, 0x0FFE_3000);
    assert_eq!(WINDOWS.start + 0x0FFE_0000, 0x0FFF_0000);
}

#[test]
fn the_clock_goes_on_from_wms_idle_value() {
    assert_eq!(m24clk_on(0x2), 0x3, "WM idles at 0x2, on is 0x3");
    assert_eq!(m24clk_on(0x3), 0x3);
    assert_eq!(m24clk_on(0x4), 0x5, "the divider found is kept");
    assert_eq!(m24clk_on(0x0), 0x3, "never divide by 1");
    assert_eq!(m24clk_on(0x1), 0x3);
    assert_eq!(m24clk_on(0x100), 0x103, "higher bits kept");
}

#[test]
fn the_reset_pulses_then_acknowledges_both_bits() {
    assert_eq!(RESET_SEQUENCE, [(0x00, 1), (0x00, 0), (0x88, 3)]);
}

#[test]
fn fifo_levels_decode_from_dells_bit_fields() {
    assert!(Fifo::from_reg(0).empty());
    assert_eq!(Fifo::from_reg(0).command_room(), 62);
    let f = Fifo::from_reg(5 << 6 | 7 << 13 | 3 << 21);
    assert_eq!((f.command, f.residual, f.coefficient), (5, 7, 3));
    assert_eq!(f.command_room(), 52);
    // Bits outside the three fields are ignored.
    assert!(Fifo::from_reg(0x3F | 0xFC00_0000).empty());
    assert_eq!(
        Fifo::from_reg(0x7F << 6).command_room(),
        0,
        "never negative"
    );
}

#[test]
fn frames_round_the_stride_to_256() {
    let f = frame();
    assert_eq!(f.stride, 256);
    assert_eq!(f.bytes(), 256 * 64 * 3 / 2);
    assert_eq!(f.chroma(), 256 * 64);
    assert_eq!(Frame420::new(320, 240).unwrap().stride, 512);
    assert_eq!(Frame420::new(640, 480).unwrap().stride, 768);
    assert_eq!(Frame420::new(63, 64), None);
    assert_eq!(Frame420::new(0, 64), None);
    assert_eq!(Frame420::new(2048, 64), None);
}

#[test]
fn the_test_frame_is_laid_out_imc2_style() {
    let f = frame();
    assert_eq!(test_frame_byte(&f, 0), 16);
    assert_eq!(test_frame_byte(&f, 63), luma(63, 0));
    assert_eq!(test_frame_byte(&f, 64), 0, "padding");
    assert_eq!(test_frame_byte(&f, 256 * 5 + 7), luma(7, 5));
    let c = f.chroma();
    assert_eq!(test_frame_byte(&f, c), 0x40);
    assert_eq!(test_frame_byte(&f, c + 31), 0x7E);
    assert_eq!(test_frame_byte(&f, c + 32), 0, "padding");
    assert_eq!(
        test_frame_byte(&f, c + 128),
        0xC0,
        "second half at stride / 2"
    );
    assert_eq!(test_frame_byte(&f, c + 128 + 31), 0x82);
    assert_eq!(
        test_frame_byte(&f, c + 31 * 256 + 128),
        0xC0,
        "constant down"
    );
    assert_eq!(test_frame_byte(&f, f.bytes()), 0, "past the frame");
    assert_eq!(
        test_frame_word(&f, c + 128),
        u32::from_le_bytes([0xC0, 0xBE, 0xBC, 0xBA])
    );
}

#[test]
fn the_pattern_tells_every_hypothesis_apart() {
    for cx in 0..32 {
        for cy in 0..32 {
            assert_ne!(chroma_first(cx), chroma_second(cy));
        }
    }
    let max = (0..64)
        .flat_map(|y| (0..64).map(move |x| luma(x, y)))
        .max()
        .unwrap();
    assert!(max <= 235, "studio range");
    // The YUY2 references differ in every chroma byte.
    for xb in [1u32, 3, 41, 127] {
        assert_ne!(
            yuy2_byte(ChromaOrder::UFirst, xb, 0),
            yuy2_byte(ChromaOrder::VFirst, xb, 0)
        );
    }
    // And the four RGB hypotheses all differ somewhere in the first row.
    for (i, a) in Rgb3::ALL.iter().enumerate() {
        for b in &Rgb3::ALL[i + 1..] {
            assert!(
                (0..192).any(|xb| rgb3_byte(*a, xb, 0).abs_diff(rgb3_byte(*b, xb, 0)) > 8),
                "{a:?} vs {b:?}"
            );
        }
    }
}

/// YUY2 straight from the frame bytes: the reference must agree with a
/// conversion that only reads `test_frame_byte`.
#[test]
fn the_yuy2_reference_matches_a_direct_conversion() {
    let f = frame();
    for y in 0..64u32 {
        for x in (0..64u32).step_by(2) {
            let l = |x: u32| test_frame_byte(&f, (y * 256 + x) as usize);
            let crow = f.chroma() + (y / 2 * 256) as usize;
            let a = test_frame_byte(&f, crow + (x / 2) as usize);
            let b = test_frame_byte(&f, crow + 128 + (x / 2) as usize);
            let want = [l(x), a, l(x + 1), b];
            for (k, &w) in want.iter().enumerate() {
                assert_eq!(yuy2_byte(ChromaOrder::UFirst, x * 2 + k as u32, y), w);
            }
            assert_eq!(yuy2_byte(ChromaOrder::VFirst, x * 2 + 1, y), b);
            assert_eq!(yuy2_byte(ChromaOrder::VFirst, x * 2 + 3, y), a);
        }
    }
}

#[test]
fn bt601_matches_the_mixing_rule() {
    assert_eq!(rgb601(16, 128, 128), [0, 0, 0]);
    assert_eq!(rgb601(235, 128, 128), [255, 255, 255]);
    assert_eq!(rgb601(126, 128, 128), [128, 128, 128]);
    // Red-ish (V high), blue-ish (U high), clamped.
    assert_eq!(rgb601(81, 90, 240), [255, 0, 0]);
    assert_eq!(rgb601(41, 240, 110), [0, 0, 255]);
    let h = Rgb3 {
        order: ChromaOrder::UFirst,
        bgr: false,
    };
    let p = rgb601(luma(0, 0), chroma_first(0), chroma_second(0));
    assert_eq!(
        [rgb3_byte(h, 0, 0), rgb3_byte(h, 1, 0), rgb3_byte(h, 2, 0)],
        p
    );
    let h = Rgb3 { bgr: true, ..h };
    assert_eq!(rgb3_byte(h, 0, 0), p[2]);
}

#[test]
fn the_yuy2_job_writes_the_found_fields_start_last() {
    let w = convert_yuy2(&job(128), VRAM).unwrap();
    assert_eq!(
        w,
        [
            (0x30, 0x40_0000 / 8),
            (0x34, 0x41_0000 / 8),
            (0x38, 64 | 64 << 11),
            (0x3C, ((128 / 4) << 9) | (256 / 8)),
            (0x48, 1),
        ]
    );
    assert_eq!(job(128).dst_bytes(2), 63 * 128 + 128);
}

#[test]
fn the_rgb3_job_writes_the_found_fields_start_last() {
    let w = convert_rgb3(&job(192), VRAM).unwrap();
    assert_eq!(
        w,
        [
            (0x30, 0x40_0000 / 8),
            (0x40, 0x41_0000 / 8),
            (0x38, 64 | 64 << 11),
            (0x3C, 256 / 8),
            (0x44, 192 / 8),
            (0x48, 3),
        ]
    );
}

#[test]
fn jobs_aimed_outside_their_frames_are_refused() {
    let ok = job(128);
    assert_eq!(
        convert_yuy2(
            &Convert {
                src: ok.src + 4,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Unaligned)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                dst: ok.dst + 4,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Unaligned)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                dst_stride: 96,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Stride)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                dst_stride: 130,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Stride)
    );
    assert_eq!(
        convert_rgb3(&ok, VRAM),
        Err(JobError::Stride),
        "3 bytes a pixel"
    );
    let bad = Frame420 {
        stride: 200,
        ..ok.frame
    };
    assert_eq!(
        convert_yuy2(&Convert { frame: bad, ..ok }, VRAM),
        Err(JobError::Stride)
    );
    let odd = Frame420 {
        width: 63,
        ..ok.frame
    };
    assert_eq!(
        convert_yuy2(&Convert { frame: odd, ..ok }, VRAM),
        Err(JobError::Field)
    );
    // The end of memory, exactly and one row past.
    let end = VRAM - ok.dst_bytes(2);
    assert!(convert_yuy2(&Convert { dst: end, ..ok }, VRAM).is_ok());
    assert_eq!(
        convert_yuy2(&Convert { dst: end + 8, ..ok }, VRAM),
        Err(JobError::OutOfMemory)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                src: VRAM - 8,
                ..ok
            },
            VRAM
        ),
        Err(JobError::OutOfMemory)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                dst: usize::MAX & !7,
                ..ok
            },
            VRAM
        ),
        Err(JobError::OutOfMemory),
        "no overflow"
    );
    // Overlap either way; touching is fine.
    assert_eq!(
        convert_yuy2(
            &Convert {
                dst: ok.src + 256,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Overlap)
    );
    assert_eq!(
        convert_yuy2(
            &Convert {
                src: ok.src + 0x1000,
                dst: ok.src,
                ..ok
            },
            VRAM
        ),
        Err(JobError::Overlap)
    );
    assert!(
        convert_yuy2(
            &Convert {
                dst: ok.src + ok.frame.bytes(),
                ..ok
            },
            VRAM
        )
        .is_ok()
    );
    // A source stride past the 9-bit field.
    let wide = Frame420 {
        width: 2046,
        height: 2,
        stride: 4096,
    };
    assert_eq!(
        convert_yuy2(
            &Convert {
                frame: wide,
                src: 0,
                dst: 0x80_0000,
                dst_stride: 4096
            },
            VRAM
        ),
        Err(JobError::Field)
    );
}

#[test]
fn diffs_count_past_the_tolerance() {
    let mut d = Diff::default();
    assert!(!d.matches(), "nothing compared");
    d.add(0, 0, 10, 11, 1);
    assert!(d.matches());
    d.add(5, 2, 10, 13, 1);
    d.add(6, 2, 0, 255, 1);
    assert_eq!((d.bytes, d.wrong, d.max), (3, 2, 255));
    assert_eq!(d.first, Some((5, 2, 10, 13)));
}
