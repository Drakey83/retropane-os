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

/*
 * Input: the X51v's buttons and its stylus, as events the game polls.
 *
 * The stylus is what the player meant, in the game's own screen pixels:
 * readings pass the OS's touch filter (no stray position as it lands or
 * lifts) and the screen's calibration, and are turned with the picture. A
 * game that plays in landscape says which way the device is held; the
 * D-pad's four directions are turned to match (RP_KEY_UP is "up" as the
 * player sees it) and the stylus arrives in 640 x 480. The other buttons
 * keep their names: what they do is the game's to bind.
 *
 * The buttons and the stylus are read when the game polls and when it
 * reads the clock; a game polls until 0 once a frame.
 */

/* How the game holds the screen (rp_input_open). */
#define RP_TURN_NONE 0  /* upright, 480 x 640 */
#define RP_TURN_LEFT 1  /* landscape, the device's top at the player's left */
#define RP_TURN_RIGHT 2 /* landscape, the device's top at the player's right */

/* rp_event.type */
#define RP_EV_KEY 1
#define RP_EV_PEN 2

/* rp_event.code of a key */
#define RP_KEY_UP 1
#define RP_KEY_DOWN 2
#define RP_KEY_LEFT 3
#define RP_KEY_RIGHT 4
#define RP_KEY_ENTER 5    /* the D-pad's centre */
#define RP_KEY_CALENDAR 6 /* the four program buttons, as printed */
#define RP_KEY_CONTACTS 7
#define RP_KEY_MAIL 8
#define RP_KEY_HOME 9
#define RP_KEY_RECORD 10 /* the side button */
#define RP_KEY_WIFI 11   /* the wireless button */

/* rp_event.code of the stylus */
#define RP_PEN_DOWN 1 /* it landed */
#define RP_PEN_MOVE 2 /* it moved while down */
#define RP_PEN_UP 3   /* it lifted; x, y where it last was */

struct rp_event {
    uint8_t type;     /* RP_EV_KEY or RP_EV_PEN */
    uint8_t code;     /* RP_KEY_* or RP_PEN_* */
    uint8_t down;     /* a key: 1 pressed, 0 released; the stylus: 1 touching */
    uint8_t reserved; /* 0 */
    int16_t x, y;     /* the stylus, in the game's pixels; 0 for a key */
    uint64_t time_us; /* when it was read, on rp_time_us()'s clock */
};

/* Starts the input, the screen held as `turn` (RP_TURN_*). What is held
 * down already is not reported. 0, or -1 with errno (ENOSYS: no input on
 * this system; EBUSY: open already; EINVAL: no such turn). */
int rp_input_open(int turn);
/* The oldest event not yet read: 1 with it in *ev, 0 when there is none,
 * -1 with errno (EBADF: not open). The last 32 events are kept. */
int rp_input_poll(struct rp_event *ev);

/*
 * Modules: a big program's libraries (ADR-0039). A module is a file the
 * OS's build linked at the address it runs at, against this runner; it is
 * named by its file ("server.rpm") and found in the runner's own folder.
 * Loading one is a read, a check and its constructors: one built for
 * another runner, or damaged, is refused and never run.
 */
/* Loads the module; its handle, or NULL with errno (rp_module_error()
 * says why in plain words). Loading one that is loaded starts it afresh. */
void *rp_module_load(const char *name);
/* Forgets the module (its memory is its own for good: nothing else is
 * ever placed there). 0, or -1 with errno (EBADF: not a loaded module). */
int rp_module_free(void *module);
/* The exported function called `name`, or NULL. */
void *rp_module_find(void *module, const char *name);
/* The name of the exported function that starts at `function` (the
 * module's own string, good while it is loaded), or NULL. */
const char *rp_module_name(void *module, const void *function);
/* Why the last rp_module_load() was refused. */
const char *rp_module_error(void);

/*
 * The screen through the 2700G (docs/design/ref-rp.md step 3): a frame is
 * the tile accelerator's own words, handed over in one call, checked by
 * the port layer before any reaches the chip, drawn into the screen behind
 * an opaque black background while the program makes its next frame.
 *
 * The picture is upright (480 x 640): a game in landscape turns its
 * vertices itself (RP_TURN_RIGHT: x = its y, y = 640 - its x).
 *
 * A frame's words, in any number and order:
 *  - a state block: RP_GFX_STATE, then TSP, ISP, TEX0, TEX1, TEX2, the
 *    clip word and 0 (the 2700G's words: drivers/mbx3d). Its texture
 *    (16-bit texels, twiddled) lies wholly inside the texture memory;
 *  - a strip: RP_GFX_STRIP(n), then n corners of RP_GFX_CORNER_WORDS
 *    words each (x, y, z, 1/w, colour 0xAARRGGBB, u, v; floats but the
 *    colour), after a state block;
 *  - a fan: RP_GFX_FAN(n), then n corners as a strip's (every triangle
 *    shares the first corner);
 *  - either with RP_GFX_NO_Z in its header: corners without the z word.
 * Anything else refuses the whole frame (EINVAL), nothing of it drawn.
 */
struct rp_gfx {
    uint16_t width, height; /* the picture, upright */
    uint32_t tex_at;        /* the texture memory as TEX1 names it */
    uint32_t tex_bytes;     /* its size */
    void *tex;              /* the same memory as the program writes it */
    const void *screen;     /* the picture as drawn (width x height RGB565,
                               rows of width x 2 bytes): read only, after
                               rp_gfx_wait(), for a test's eye */
    uint32_t flags;         /* RP_GFX_KEEP, ... */
};

/* A frame's words are read after rp_gfx_frame_parts returns (the 2700G fed
 * by DMA while the program goes on), up to the next rp_gfx_frame_parts or
 * rp_gfx_wait returning: keep them (write the next frame elsewhere). */
#define RP_GFX_KEEP 1u

#define RP_GFX_STATE 0x10000057u
#define RP_GFX_STRIP(n) (0x40000000u | (uint32_t)(n)) /* 3 to 255 corners */
#define RP_GFX_FAN(n) (0x50000000u | (uint32_t)(n))   /* 3 to 255 corners */
#define RP_GFX_CORNER_WORDS 7
/* A strip's or fan's header with RP_GFX_NO_Z (ADR-0040 step 7g7, shown on
 * the X51v, run 144): its corners have no z word, RP_GFX_CORNER_WORDS_NO_Z
 * words each (x, y, 1/w, colour, u, v), the depth taken from 1/w. */
#define RP_GFX_NO_Z (1u << 21)
#define RP_GFX_CORNER_WORDS_NO_Z 6

/* Takes the screen: 0 with *g filled in, or -1 with errno. */
int rp_gfx_open(struct rp_gfx *g);
/* One frame's `count` words: 0, or -1 with errno. */
int rp_gfx_frame(const uint32_t *words, size_t count);
/* One frame in parts, drawn in their order, as if their words were one
 * run (each part must start with a state block): no copy to join them
 * (ref_rp's second pass, its sky first). At most RP_GFX_PARTS parts:
 * 0, or -1 with errno. */
#define RP_GFX_PARTS 32
struct rp_gfx_part {
    const uint32_t *words;
    size_t count;
};
int rp_gfx_frame_parts(const struct rp_gfx_part *parts, size_t n);

/* R310: large blocks from the heap's top region, apart from malloc's (a
   map's multi-megabyte blocks never share holes with small ones). NULL:
   no room, use malloc. rp_big_free: 1 if p was one (now free), else 0.
   rp_big_size: its size, 0 if p is not one. */
void  *rp_big_alloc(size_t size);
int    rp_big_free(void *p);
size_t rp_big_size(const void *p);
/* Waits until the last frame is drawn (before texture memory it may read
 * is written over): 0, or -1 with errno. */
int rp_gfx_wait(void);
/* Gives the screen back: 0, or -1 with errno. */
int rp_gfx_close(void);
/* R324: the program's hint that its next frames run much code a frame (1:
 * a game's many models) or not (0): the runner may keep its hottest code
 * locked in the I-cache meanwhile. Call it on a change, not every frame. */
void rp_code_hint(int hot);

#ifdef __cplusplus
}
#endif

#endif /* RETROPANE_H */
