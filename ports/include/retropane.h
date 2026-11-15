/*
 * RetroPane's port layer: what a game written in C or C++ calls beyond the
 * C library (docs/design/half-life-readiness.md, line 4). One header for
 * every port; `cargo xtask cc` puts it on the include path as
 * <retropane.h>.
 *
 * Iterate, don't imitate (rule 0): ports to Linux handhelds and consoles
 * sit on SDL (a desktop's model: windows, an event queue, float audio
 * formats), Windows Mobile ports on GAPI and waveOut. This is a handful of
 * calls shaped to what the X51v has, each landing in checked Rust
 * (hal/cabi, then rp_port): integer time, the audio service's mixer, the
 * device's own keys and stylus, the 2700G's display lists.
 *
 * The C library's own calls (fopen, malloc, printf, time) work as C says;
 * the card's files are reached with plain paths ("valve/maps/c0a0.bsp",
 * either slash, any case).
 */
#ifndef RETROPANE_H
#define RETROPANE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * The clock: whole microseconds since the program started, exact (the
 * PXA270's 3.25 MHz counter: 13 ticks are 4 microseconds), never going
 * back, good for any length of run. A frame's time is the difference of
 * two reads, in integers; gettimeofday() and time() give the same clock
 * (counted from 1970: the date is not the port layer's to know yet).
 */
uint64_t rp_time_us(void);

#ifdef __cplusplus
}
#endif

#endif /* RETROPANE_H */
