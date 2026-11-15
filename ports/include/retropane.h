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

#include <stddef.h>
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

/*
 * Sound: the game's own mixed samples, 16-bit signed, at the game's rate
 * (4,000 to 96,000 Hz; the OS converts to the hardware's), mono or stereo
 * (interleaved, left first). One stream of the OS's mixer (ADR-0032), so
 * system sounds play over it and it pauses when the headphones come out.
 *
 * There is no callback and no audio thread: each frame the game asks how
 * much has played, and writes enough to keep its own lead queued (a tenth
 * of a second is usual; more if its frames can take longer than that). A
 * stall longer than the lead (a map loading) is heard as silence, never
 * as a stutter. The mixer is woken by these calls and by rp_time_us(), so
 * a game calls one of them at least once a frame.
 *
 * Each returns -1 with errno set on failure (ENOSYS: no sound on this
 * system; EBADF: not open; EBUSY: open already; EINVAL: a rate or channel
 * count that is not taken).
 */

/* Opens the sound: `channels` 1 or 2. 0 on success. */
int rp_snd_open(uint32_t rate, int channels);
/* Frames that can be written now (at least a tenth of a second's worth
 * when nothing is queued). */
int rp_snd_room(void);
/* Queues `count` samples (frames times channels): the number of whole
 * frames taken, fewer than given when the room ran out. */
int rp_snd_write(const int16_t *samples, size_t count);
/* The game's frames that have really left the device (the DMA engine's
 * own count): written minus this is what is queued, and this is the
 * clock to sync pictures to. The last few dozen frames written wait for
 * the samples after them (the rate converter reads ahead). */
int64_t rp_snd_played(void);
/* Closes the sound; what was queued and not yet played is dropped. */
int rp_snd_close(void);

#ifdef __cplusplus
}
#endif

#endif /* RETROPANE_H */
