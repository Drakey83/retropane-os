/*
config.h - mpg123's settings for RetroPane OS (Dell Axim X51v)

mpg123's own build writes this file by probing the PC it runs on; the
engine's build (3rdparty/mpg123/wscript) writes it from a list. This is
that list as RetroPane wants it, written by hand: the names are mpg123's
(src/config.h.in), the choices are ours. RetroPane's own file
(GPL-2.0-or-later), holding settings only.

What differs from the engine's list, and why: no IEEE_FLOAT. The engine
builds the float decoder; the PXA270 has no FPU, so this build is mpg123's
fixed-point one (REAL_IS_FIXED and OPT_ARM, given on the command line by
xtask/src/xash.rs, as mpg123's own "arm_nofpu" build does), whose tables
are computed beforehand (no RUNTIME_TABLES): decoding uses no float.
*/
#ifndef RP_MPG123_CONFIG_H
#define RP_MPG123_CONFIG_H

/* Only the int64_t API; the engine does all the reading itself. */
#define PORTABLE_API 1

#define GAPLESS 1
#define FRAME_INDEX 1
#define INDEX_SIZE 1000
#define USE_NEW_HUFFTABLE 1

/* Layer III, 16-bit output at the file's own rate, nothing else. */
#define NO_LAYER1 1
#define NO_LAYER2 1
#define NO_8BIT 1
#define NO_32BIT 1
#define NO_REAL 1
#define NO_SYNTH32 1
#define NO_NTOM 1
#define NO_DOWNSAMPLE 1
#define NO_EQUALIZER 1
#define NO_ICY 1
#define NO_STRING 1
#define NO_ID3V2 1
#define NO_MOREINFO 1

/* Nothing printed: the engine reports a file that does not decode. */
#define NO_WARNING 1
#define NO_ERRORMSG 1
#define NO_ERETURN 1

/* How this assembler aligns (synth_arm.S): `.balign`, in bytes. */
#define ASMALIGN_BALIGN 1

/* What picolibc has. */
#define HAVE_STRERROR 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_UNISTD_H 1
#define HAVE_STRTOK_R 1
#define HAVE_INTTYPES_H 1
#define HAVE_STDINT_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRING_H 1
#define HAVE_STRINGS_H 1
#define HAVE_LIMITS_H 1

#endif
