/*
 * RetroPane's build of picolibc 1.8.12 (docs/design/half-life-readiness.md
 * line 3). picolibc's own build writes this file from its options; ours is
 * written out here, so the choices are in the repo and the same on every PC
 * (`cargo xtask cc` puts it on the include path as <picolibc.h>).
 *
 * Iterate, don't imitate (rule 0): each choice is made for the X51v (a
 * PXA270: no FPU, one core, 64 MB), not taken from picolibc's defaults.
 * Sizes and speeds are to be measured in the evaluation (line 1) before any
 * of these is called final.
 */
#pragma once

/* picolibc's small stdio, not newlib's. */
#define __TINY_STDIO
/* stdin, stdout and stderr go through read() and write() on handles 0-2,
 * so the console and the card's files are one path (rp-hal-cabi). */
#define POSIX_CONSOLE
/* One core and one C thread: no locks, one errno, no thread-local storage
 * (the PXA270 has no register for it). */
#define __SINGLE_THREAD
#define __GLOBAL_ERRNO
/* Half-Life prints floats ("%f" in the console, in saves): double printf
 * and scanf by default, exact (the Ryu tables) so a save's numbers read
 * back as written. */
#define __IO_DEFAULT 'd'
#define __IO_FLOAT_EXACT
#define __IO_C99_FORMATS
#define __IO_LONG_LONG
/* printf's integer digits without a software divide (the PXA270 has no
 * divide instruction). */
#define __IO_SMALL_ULTOA
/* The maths library made for CPUs without a floating-point unit: float
 * functions computed in float (the newer code computes them in double,
 * twice the software arithmetic), and no errno from maths (Quake-family
 * engines never read it). The hot ones get replaced after the evaluation
 * (readiness line 9). */
#define __OBSOLETE_MATH_FLOAT 1
#define __OBSOLETE_MATH_DOUBLE 1
#define __IEEE_LIBM
/* strcmp made for speed, not size. */
#define __FAST_STRCMP
/* What the compiler has. */
#define __HAVE_BITFIELDS_IN_PACKED_STRUCTS
#define __HAVE_COMPLEX
#define __INIT_FINI_ARRAY
/* An assert says where it failed. */
#define __ASSERT_VERBOSE
#define __ELIX_LEVEL 4

/* Versions (picolibc 1.8.12, which carries newlib 4.3.0's headers). */
#define _NEWLIB_VERSION "4.3.0"
#define __NEWLIB__ 4
#define __NEWLIB_MINOR__ 3
#define __NEWLIB_PATCHLEVEL__ 0
#define _PICOLIBC__ 1
#define _PICOLIBC_MINOR__ 8
#define _PICOLIBC_VERSION "1.8.12"
#define __PICOLIBC__ 1
#define __PICOLIBC_MINOR__ 8
#define __PICOLIBC_PATCHLEVEL__ 12
#define __PICOLIBC_VERSION__ "1.8.12"
