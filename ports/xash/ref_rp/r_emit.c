/*
r_emit.c - RetroPane OS (Dell Axim X51v): ref_rp's emitter, step 1
Copyright (C) 2026 the RetroPane OS authors

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.
*/

/*
ref_rp (docs/design/ref-rp.md) grows from the software renderer: its
scene code is kept and its rasterisers (the world's edges and spans, the
triangle rasteriser, the blit) are replaced by one emitter writing what
the 2700G's tile accelerator takes: a state block when the texture or
the blend changes, then each primitive's vertices as seven words (x, y,
z, 1/w, colour, u, v; floats but the colour).

Step 1 is the CPU's side alone: with `rp_emit 1` the words go into a
buffer in RAM and no further (no chip yet), so a run on the X51v measures
what the emitter and the scene code left around it cost, against the
frame with no 3D view at all (`r_norefresh 1`, run 8). Every five seconds
the emitter prints its counts in the run log, beside `-rpfps`'s frame rate.

Iterate, don't imitate (rule 0). `ref_gl` hands GL one call a vertex and
lets the GPU transform; the 2700G transforms nothing, so the vertices
leave the CPU already on the screen, and here they leave as the TA's own
words: no GL state machine, no call a vertex, a world face as one strip
in zigzag order (n vertices for n corners, where a triangle list takes
3(n - 2)), the state block only on a change, and the integers the
triangle path already holds turned into float bits with one `clz` (no
software-float conversion).
*/

/*
Step 3 (docs/design/ref-rp.md, "Step 3: the world on the 2700G"): with
`rp_emit 2` the world's faces go to the 2700G through the port layer
(`rp_gfx_*`, ports/include/retropane.h), each face's lit surface kept in
video memory: lit once into a scratch buffer, written straight into a
twiddled atlas page (512 x 512 RGB565) on shelves of whole 8 x 8
squares, and kept until its light changes; a full atlas drops its least
recently drawn page. The models, sprites, particles, the sky, water and
2D are step 4's and not drawn yet.
*/

#include <stdio.h>
#include <retropane.h>
#include "r_local.h"

/* The TA's words (drivers/mbx3d: `ta::STRIP`, `ta::END`; the state
   block's mask as P4's textured draw, `ta_quads::TEXTURED_MASK`). */
#define RP_TA_STATE_MASK 0x10000057u
#define RP_TA_STRIP      0x41000000u
#define RP_TA_END        0xF0000000u
#define RP_STATE_WORDS   8
#define RP_VERTEX_WORDS  7

/* A frame's words: 512 KiB (c1a0's opening view in the emulator: about
   83 Ki words, 4,200 triangles). Words past it are counted, not kept. */
#define RP_BUF_WORDS ( 128 * 1024 )

int rp_emit_on;
/* This frame is drawn both ways: the 2700G's and the software renderer's
   picture of one view, for the test's end shot (the engine's in_rp.c asks
   by the setting `rp_both` 1; 2 once both are written). `rp_gpu` says the
   2700G draws the frames. */
int rp_both;
static cvar_t *rp_both_var, *rp_gpu;
/* D_CacheSurface lights into the scratch, not the RAM cache (r_surf.c). */
int rp_scratch;

static cvar_t *rp_emit;
static uint32_t rp_buf[RP_BUF_WORDS];
static uint32_t rp_n;
static const void *rp_state; /* what the last state block named */
static uint32_t rp_lit_bytes, rp_block_bytes; /* this frame's lit surfaces (step 1b) */

/* Counted over the frames since the last report. */
static struct
{
	uint32_t frames, words, faces, tris, verts, states, nearclipped, dropped, built, relit;
	uint32_t lit_kib, block_kib, mip[4]; /* the view's lit surfaces (step 1b) */
	uint32_t uploads, upload_kib, evicted, skipped, refused, models; /* step 3 */
	uint64_t upload_us;
} rp_count;

void RP_EmitInit( void )
{
	rp_emit = gEngfuncs.Cvar_Get( "rp_emit", "0", 0, "ref_rp: 1 the TA's words into RAM, no pixels (step 1); 2 the world drawn by the 2700G (step 3)" );
	rp_both_var = gEngfuncs.Cvar_Get( "rp_both", "0", 0, "ref_rp: 1 asks for a frame drawn both ways (the test's end shot), 2 once written" );
	rp_gpu = gEngfuncs.Cvar_Get( "rp_gpu", "0", 0, "ref_rp: 1 while the 2700G draws the frames" );
}

/* The port layer's clock (ports/include/retropane.h; the runner's). */
uint64_t rp_time_us( void );

/*
The counts over the last five seconds, one plain line in the run log (the
same span as `-rpfps`'s frame rate, its own clock), then counted again.
*/
static void RP_EmitReport( void )
{
	static uint64_t since;
	uint64_t now = rp_time_us();
	uint32_t f = rp_count.frames;

	if( !since )
		since = now;
	if( now - since < 5000000u || !f )
		return;
	fprintf( stderr, "rp-emit: a frame %u words, %u faces, %u triangles, %u vertices, %u state blocks, %u near-clipped; %u words past the buffer (%u frames)\n",
		rp_count.words / f, rp_count.faces / f, rp_count.tris / f, rp_count.verts / f,
		rp_count.states / f, rp_count.nearclipped / f, rp_count.dropped, f );
	fprintf( stderr, "rp-emit: the surface cache built %u surfaces a frame, %u of them relit in place (a light changed), the rest new or pushed out\n",
		rp_count.built / f, rp_count.relit / f );
	fprintf( stderr, "rp-emit: the view's lit surfaces %u KiB a frame (in power-of-two blocks %u KiB); faces by mip level 0-3: %u %u %u %u\n",
		rp_count.lit_kib / f, rp_count.block_kib / f,
		rp_count.mip[0] / f, rp_count.mip[1] / f, rp_count.mip[2] / f, rp_count.mip[3] / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: video memory: %u surfaces written a frame (%u KiB, %u us), %u pages dropped, %u faces not drawn (no room or too large), %u model triangles not drawn (step 4); %u frames refused by the port layer\n",
			rp_count.uploads / f, rp_count.upload_kib / f, (uint32_t)( rp_count.upload_us / f ),
			rp_count.evicted, rp_count.skipped / f, rp_count.models / f, rp_count.refused );
	memset( &rp_count, 0, sizeof( rp_count ));
	since = now;
}

/* A surface lit into the cache (r_surf.c's D_CacheSurface): `relit` when
   its block was still there and only its light changed. */
void RP_EmitSurfBuilt( qboolean relit )
{
	rp_count.built++;
	if( relit )
		rp_count.relit++;
}

/*
The cache's size from `sw_surfcacheoverride` whenever it changes: the
software renderer sizes it once at its start, before the command line's
`+` settings are run (run 10 on the X51v asked for 8 MiB and kept 3.2).
*/
static void RP_CacheSize( void )
{
	static float applied;

	if( sw_surfcacheoverride.value && sw_surfcacheoverride.value != applied )
	{
		applied = sw_surfcacheoverride.value;
		R_InitCaches();
	}
}

/* The 2700G through the port layer (step 3): opened at the first frame
   that asks for it; 1 open, -1 refused (the frame falls back to step 1). */
static struct rp_gfx rp_g;
static int rp_chip;
static void RP_AtlasInit( void );
/* The last word of a whole primitive in the buffer: what is sent. */
static uint32_t rp_good;

static int RP_ChipOpen( void )
{
	if( !rp_chip )
	{
		if( rp_gfx_open( &rp_g ) == 0 )
		{
			rp_chip = 1;
			RP_AtlasInit();
			fprintf( stderr, "rp-emit: the 2700G is ours: %u x %u, %u KiB of texture memory\n",
				rp_g.width, rp_g.height, rp_g.tex_bytes >> 10 );
		}
		else
		{
			rp_chip = -1;
			fprintf( stderr, "rp-emit: the 2700G was not given (rp_gfx_open): the words go nowhere, as step 1\n" );
		}
	}
	return rp_chip > 0;
}

/* A new frame: the last one's words are let go (sent at its end by
   RP_EmitSubmit, or nowhere in step 1). */
void RP_EmitFrame( void )
{
	int want = rp_emit ? (int)rp_emit->value : 0;

	if( want == 2 && !RP_ChipOpen( ))
		want = 1;
	rp_emit_on = want == 1 || want == 2 ? want : 0;
	rp_both = rp_emit_on == 2 && rp_both_var->value == 1.0f;
	if(( rp_emit_on == 2 ) != ( rp_gpu->value == 1.0f ))
		gEngfuncs.Cvar_SetValue( "rp_gpu", rp_emit_on == 2 ? 1.0f : 0.0f );
	if( !rp_emit_on )
		return;
	RP_CacheSize();
	if( rp_n )
	{
		rp_count.frames++;
		rp_count.words += rp_n;
		rp_count.lit_kib += rp_lit_bytes >> 10;
		rp_count.block_kib += rp_block_bytes >> 10;
		if( rp_n > RP_BUF_WORDS )
			rp_count.dropped += rp_n - RP_BUF_WORDS;
	}
	rp_n = 0;
	rp_good = 0;
	rp_lit_bytes = rp_block_bytes = 0;
	rp_state = NULL;
	RP_EmitReport();
}

static inline void RP_Put( uint32_t w )
{
	if( rp_n < RP_BUF_WORDS )
		rp_buf[rp_n] = w;
	rp_n++;
}

/* A primitive is whole: what the buffer holds up to here may be sent. */
static inline void RP_Whole( void )
{
	if( rp_n <= RP_BUF_WORDS )
		rp_good = rp_n;
}

static inline uint32_t RP_Bits( float f )
{
	uint32_t u;

	memcpy( &u, &f, sizeof( u ));
	return u;
}

/* `v / 2^frac` as float bits, from the integer with one clz: exact below
   2^24, truncated above (screen and texture coordinates need far less). */
static inline uint32_t RP_FixBits( int v, int frac )
{
	uint32_t s = 0, a = (uint32_t)v, m;
	int e;

	if( !v )
		return 0;
	if( v < 0 )
	{
		s = 0x80000000u;
		a = 0u - a;
	}
	e = 31 - __builtin_clz( a );
	m = e <= 23 ? a << ( 23 - e ) : a >> ( e - 23 );
	return s | (uint32_t)( e - frac + 127 ) << 23 | ( m & 0x007FFFFFu );
}

/* A state block, when `what` (a texture, a page) is not the last one's. */
static void RP_State( const void *what, uint32_t tsp )
{
	if( what == rp_state )
		return;
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_TA_STATE_MASK );
	RP_Put( tsp );                                   /* TSP */
	RP_Put( 0xE0000000u );                           /* ISP */
	RP_Put( (uint32_t)(uintptr_t)what );             /* TEX0-2: the texture's */
	RP_Put( 0 );
	RP_Put( 0 );
	RP_Put( 0 );                                     /* clip */
	RP_Put( 0 );                                     /* the bit 6 word */
}

/*
A triangle from the models, sprites, beams and particles: the corners
`R_DrawTriangle` would have rasterised, already on the screen.
*/
void RP_EmitTriangle( void )
{
	if( rp_emit_on == 2 )
	{
		rp_count.models++; /* step 4: the models' skins in video memory */
		return;
	}

	const finalvert_t *v[3] = { aliastriangleparms.a, aliastriangleparms.b, aliastriangleparms.c };
	int sw = r_affinetridesc.skinwidth, sh = r_affinetridesc.skinheight;
	int su = sw > 0 ? 31 - __builtin_clz( (uint32_t)sw ) : 0;
	int tu = sh > 0 ? 31 - __builtin_clz( (uint32_t)sh ) : 0;

	RP_State( r_affinetridesc.pskin, (uint32_t)(uintptr_t)d_pdrawspans );
	RP_Put( RP_TA_STRIP );
	for( int i = 0; i < 3; i++ )
	{
		uint32_t l = (uint32_t)v[i]->l >> 8 & 0xFF;

		RP_Put( RP_FixBits( v[i]->u, 0 ));
		RP_Put( RP_FixBits( v[i]->v, 0 ));
		RP_Put( RP_FixBits( v[i]->zi, 31 ));         /* z: 1/z as the ISP sorts it */
		RP_Put( RP_FixBits( v[i]->zi, 31 ));         /* 1/w */
		RP_Put( 0xFF000000u | l * 0x010101u );
		/* s, t are 16.16 texels: over a power-of-two side the divide is a
		   shift (a skin of another size is a step-3 question) */
		RP_Put( RP_FixBits( v[i]->s, 16 + su ));
		RP_Put( RP_FixBits( v[i]->t, 16 + tu ));
	}
	RP_Put( RP_TA_END );
	rp_count.tris++;
	rp_count.verts += 3;
}

/* The smallest power of two at least `v` (v > 0). */
static inline uint32_t RP_Pow2( uint32_t v )
{
	return v <= 1 ? 1 : 1u << ( 32 - __builtin_clz( v - 1 ));
}

/*
Step 1b: what the view's lit surfaces take at their mip levels, as the
cache holds them (16-bit texels) and as power-of-two blocks of an atlas
(docs/design/ref-rp.md choice 2 A: a buddy allocator in 2700G memory),
summed in bytes and reported in KiB.
*/
static void RP_CountLit( const surfcache_t *c, int miplevel )
{
	if( c && c->width )
	{
		rp_lit_bytes += c->width * c->height * 2;
		rp_block_bytes += RP_Pow2( c->width ) * RP_Pow2( c->height ) * 2;
	}
	rp_count.mip[miplevel & 3]++;
}

/* A corner in view space, its texture's s and t. */
typedef struct
{
	float x, y, z, s, t;
} rp_corner_t;

#define RP_MAX_CORNERS 64

/*
==============================================================================
Step 3: the lit surfaces in video memory, the world to the 2700G
==============================================================================
*/

/* An atlas page: 512 x 512 RGB565 texels, twiddled (x in the odd bits of
   a texel's index, y in the even: drivers/mbx3d `tex::twiddle`), 64 x 64
   squares of 8 x 8 texels, each square 128 bytes in one run. */
#define RP_PAGE_SIDE    512
#define RP_PAGE_SQUARES 64
#define RP_PAGE_BYTES   ( RP_PAGE_SIDE * RP_PAGE_SIDE * 2 )
#define RP_MAX_PAGES    16
#define RP_MAX_SHELVES  64
/* The game's picture is 640 x 480; the 2700G's is upright, 480 x 640. */
#define RP_GAME_W 640
#define RP_GAME_H 480

/* The 2700G's words for a page's state block (drivers/mbx3d, test P's
   textured draw): TSP Dell's textured one with depth "greater or equal"
   and written (z is 1/w: nearer is larger), ISP the alpha test "always",
   TEX0 512 x 512 (size code 6) RGB565 (2) clamped, TEX1 Dell's level of
   detail and point sampling with the address / 128, TEX2 test P's
   combine, the clip word for 480 x 640 (`ta::clip`). */
#define RP_TSP_WORLD  0x22220C00u
#define RP_ISP_ALWAYS 0xE0000000u
#define RP_TEX0_PAGE  ( 0xA0000000u | 6u << 24 | 6u << 20 | 2u << 13 )
#define RP_TEX1( addr ) ( 0x0E000000u | (( addr ) >> 7 ))
#define RP_TEX2       0x86086610u
#define RP_CLIP       ( 1u << 29 | 29u << 14 | 39u )

typedef struct
{
	uint8_t y, h, x; /* in squares: the shelf's row, height, first free column */
} rp_shelf_t;

typedef struct
{
	rp_shelf_t shelf[RP_MAX_SHELVES];
	int        shelves, top; /* the first row no shelf has */
	int        gen;          /* counts the times the page was dropped */
	int        used;         /* the last frame a face was drawn from it */
} rp_page_t;

static rp_page_t rp_page[RP_MAX_PAGES];
static int       rp_pages;

/* A face's place in the atlas, kept in its extra data's spare words
   (com_model.h `mextrasurf_t.reserved`; 0-3 are the RAM cache's spots):
   4: valid (bit 31), mip << 24, page << 16, square row << 8, column;
   5: the texture (animation frame) it was lit with; 6: its page's
   generation << 1, and 1 if a dynamic light was in it. */
#define RP_LOC( fa )   ( (fa)->info->reserved[4] )
#define RP_IMAGE( fa ) ( (fa)->info->reserved[5] )
#define RP_GEN( fa )   ( (fa)->info->reserved[6] )
#define RP_VALID       0x80000000u

/* A square's 64 texels in storage order: each one's y << 3 | x. */
static uint8_t rp_square[64];

/* The scratch the surface is lit into (D_CacheSurface, `rp_scratch`):
   512 x 256 texels at most. */
#define RP_SCRATCH_TEXELS ( 512 * 256 )
static struct
{
	surfcache_t head;
	pixel_t     more[RP_SCRATCH_TEXELS];
} rp_scratch_buf;

surfcache_t *RP_SurfScratch( int width, int size )
{
	surfcache_t *c = &rp_scratch_buf.head;

	memset( c, 0, sizeof( *c ));
	c->width = width;
	c->height = width ? size / ( width * 2 ) : 0;
	c->size = size;
	return c;
}

static uint32_t RP_Interleave( uint32_t x, uint32_t y )
{
	uint32_t r = 0;

	for( int b = 0; b < 8; b++ )
		r |= ( y >> b & 1 ) << ( 2 * b ) | ( x >> b & 1 ) << ( 2 * b + 1 );
	return r;
}

static void RP_AtlasInit( void )
{
	rp_pages = rp_g.tex_bytes / RP_PAGE_BYTES;
	if( rp_pages > RP_MAX_PAGES )
		rp_pages = RP_MAX_PAGES;
	memset( rp_page, 0, sizeof( rp_page ));
	for( int k = 0; k < 64; k++ )
	{
		uint32_t x = ( k >> 1 & 1 ) | ( k >> 3 & 1 ) << 1 | ( k >> 5 & 1 ) << 2;
		uint32_t y = ( k & 1 ) | ( k >> 2 & 1 ) << 1 | ( k >> 4 & 1 ) << 2;

		rp_square[k] = (uint8_t)( y << 3 | x );
	}
}

/* A page emptied: its faces' places stop being valid (their generation). */
static void RP_PageDrop( rp_page_t *p )
{
	p->shelves = 0;
	p->top = 0;
	p->gen++;
}

/* Room for `w` x `h` squares on a shelf of `p`: the column and row. */
static int RP_ShelfFit( rp_page_t *p, int w, int h, int *x, int *y )
{
	rp_shelf_t *best = NULL;

	for( int i = 0; i < p->shelves; i++ )
	{
		rp_shelf_t *s = &p->shelf[i];

		/* a shelf as tall or up to a quarter taller (whole squares) */
		if( s->h >= h && s->h <= h + ( h >> 2 ) && RP_PAGE_SQUARES - s->x >= w )
			if( !best || s->h < best->h )
				best = s;
	}
	if( !best && p->shelves < RP_MAX_SHELVES && RP_PAGE_SQUARES - p->top >= h )
	{
		best = &p->shelf[p->shelves++];
		best->y = (uint8_t)p->top;
		best->h = (uint8_t)h;
		best->x = 0;
		p->top += h;
	}
	if( !best )
		return 0;
	*x = best->x;
	*y = best->y;
	best->x = (uint8_t)( best->x + w );
	return 1;
}

/* A place for `w` x `h` squares: any page's shelves first, then the least
   recently drawn page not drawn from this frame, dropped. The page, or
   -1 (every page in this frame's view: the face waits a frame). */
static int RP_AtlasAlloc( int w, int h, int *x, int *y )
{
	int oldest = -1;

	for( int pg = 0; pg < rp_pages; pg++ )
		if( RP_ShelfFit( &rp_page[pg], w, h, x, y ))
			return pg;
	for( int pg = 0; pg < rp_pages; pg++ )
		if( rp_page[pg].used != tr.framecount && ( oldest < 0 || rp_page[pg].used < rp_page[oldest].used ))
			oldest = pg;
	if( oldest < 0 )
		return -1;
	/* the last frame's render may still read the page */
	rp_gfx_wait();
	RP_PageDrop( &rp_page[oldest] );
	rp_count.evicted++;
	return RP_ShelfFit( &rp_page[oldest], w, h, x, y ) ? oldest : -1;
}

/*
The lit surface (`w` x `h` texels of the software renderer's 16-bit
pixels) written into page `pg` at square (`qx`, `qy`): each 8 x 8 square
in one run of 32 words, each texel through the renderer's own table to
RGB565 (`vid.screen`, as its blit does) and to its twiddled place in the
same pass. Texels past the surface's edge repeat its last row and column.
*/
static void RP_Upload( const pixel_t *src, int w, int h, int pg, int qx, int qy )
{
	uint32_t *page = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES );
	int       w8 = ( w + 7 ) >> 3, h8 = ( h + 7 ) >> 3;

	for( int sy = 0; sy < h8; sy++ )
	{
		for( int sx = 0; sx < w8; sx++ )
		{
			uint32_t *dst = page + RP_Interleave( qx + sx, qy + sy ) * 32;
			int       x0 = sx << 3, y0 = sy << 3;

			if( x0 + 8 <= w && y0 + 8 <= h )
			{
				const pixel_t *at = src + y0 * w + x0;

				for( int k = 0; k < 32; k++ )
				{
					int a = rp_square[2 * k], b = rp_square[2 * k + 1];

					dst[k] = vid.screen[at[( a >> 3 ) * w + ( a & 7 )]]
						| (uint32_t)vid.screen[at[( b >> 3 ) * w + ( b & 7 )]] << 16;
				}
			}
			else
			{
				for( int k = 0; k < 32; k++ )
				{
					uint32_t t[2];

					for( int j = 0; j < 2; j++ )
					{
						int c = rp_square[2 * k + j];
						int x = x0 + ( c & 7 ), y = y0 + ( c >> 3 );

						if( x >= w )
							x = w - 1;
						if( y >= h )
							y = h - 1;
						t[j] = vid.screen[src[y * w + x]];
					}
					dst[k] = t[0] | t[1] << 16;
				}
			}
		}
	}
}

/* A page's state block, when it is not the last one's. */
static void RP_PageState( int pg )
{
	const void *what = &rp_page[pg];

	if( what == rp_state )
		return;
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_GFX_STATE );
	RP_Put( RP_TSP_WORLD );
	RP_Put( RP_ISP_ALWAYS );
	RP_Put( RP_TEX0_PAGE );
	RP_Put( RP_TEX1( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ));
	RP_Put( RP_TEX2 );
	RP_Put( RP_CLIP );
	RP_Put( 0 );
	RP_Whole();
}

/*
The face's lit surface in video memory at `miplevel`, built if it is not
there or its light or texture frame changed (D_CacheSurface's own tests);
its page, and its place in `*x`, `*y` (squares), or -1 (not drawn).
*/
static int RP_SurfacePlace( msurface_t *fa, int miplevel, int w, int h, int *x, int *y )
{
	uint32_t     loc = (uint32_t)RP_LOC( fa );
	image_t     *image = R_GetTexture( R_TextureAnimation( fa )->gl_texturenum );
	int          pg = loc >> 16 & 0xFF, stale, placed;
	surfcache_t *cache;
	uint64_t     t0;

	placed = ( loc & RP_VALID ) && (int)( loc >> 24 & 3 ) == miplevel && pg < rp_pages
		&& (int)( RP_GEN( fa ) >> 1 ) == rp_page[pg].gen;
	stale = !placed || ( RP_GEN( fa ) & 1 ) || fa->dlightframe == tr.framecount
		|| (image_t *)RP_IMAGE( fa ) != image;
	for( int maps = 0; !stale && maps < MAXLIGHTMAPS && fa->styles[maps] != 255; maps++ )
		if( g_lightstylevalue[fa->styles[maps]] != fa->cached_light[maps] )
			stale = 1;

	if( placed )
	{
		*x = loc & 0xFF;
		*y = loc >> 8 & 0xFF;
	}
	else
	{
		pg = RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, x, y );
		if( pg < 0 )
			return -1;
	}
	rp_page[pg].used = tr.framecount;
	if( !stale )
		return pg;

	/* lit into the scratch by the software renderer's own code */
	t0 = rp_time_us();
	CACHESPOT( fa )[miplevel] = NULL;
	rp_scratch = 1;
	cache = D_CacheSurface( fa, miplevel );
	rp_scratch = 0;
	CACHESPOT( fa )[miplevel] = NULL;
	if( (int)cache->width != w || (int)cache->height != h )
	{
		RP_LOC( fa ) = 0;
		return -1;
	}
	RP_Upload((const pixel_t *)cache->data, w, h, pg, *x, *y );
	RP_LOC( fa ) = (intptr_t)( RP_VALID | (uint32_t)miplevel << 24 | (uint32_t)pg << 16 | (uint32_t)*y << 8 | (uint32_t)*x );
	RP_IMAGE( fa ) = (intptr_t)image;
	RP_GEN( fa ) = (intptr_t)( rp_page[pg].gen << 1 | ( cache->dlight ? 1 : 0 ));
	rp_count.uploads++;
	rp_count.upload_kib += ( w * h * 2 ) >> 10;
	rp_count.upload_us += rp_time_us() - t0;
	return pg;
}

/*
A world (or brush model) face to the 2700G: its corners transformed and
projected, clipped against the near plane only, its lit surface placed in
video memory, written as one strip in zigzag order, each corner turned
into the upright picture (x = the game's y, y = 640 - the game's x) with
z and 1/w both 1/z, u and v in the page.
*/
static void RP_ChipFace( msurface_t *fa )
{
	rp_corner_t in[RP_MAX_CORNERS], out[RP_MAX_CORNERS + 1];
	const model_t *mod = RI.currentmodel;
	const mtexinfo_t *tex = fa->texinfo;
	const float *sv, *tv;
	int n = 0, m = 0, lo, hi, miplevel, k, w, h, qx, qy, pg, luxels;
	float nearzi = 0.0f, smin, tmin, scale, ks, kt, u0, v0;

	if( fa->flags & ( SURF_DRAWSKY | SURF_DRAWTURB | SURF_CONVEYOR ))
		return; /* the sky, water and conveyors: step 4 */
	if( fa->numedges < 3 || fa->numedges > RP_MAX_CORNERS )
		return;

	luxels = ( tex->flags & TEX_WORLD_LUXELS ) != 0;
	sv = luxels ? tex->vecs[0] : fa->info->lmvecs[0];
	tv = luxels ? tex->vecs[1] : fa->info->lmvecs[1];
	smin = luxels ? fa->texturemins[0] : fa->info->lightmapmins[0];
	tmin = luxels ? fa->texturemins[1] : fa->info->lightmapmins[1];

	for( int i = 0; i < fa->numedges; i++ )
	{
		int lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
		const float *world = r_pcurrentvertbase[lindex > 0 ? e->v[0] : e->v[1]].position;
		vec3_t local, t;

		VectorSubtract( world, tr.modelorg, local );
		TransformVector( local, t );
		in[n].x = t[0];
		in[n].y = t[1];
		in[n].z = t[2];
		in[n].s = DotProduct( world, sv ) + sv[3] - smin;
		in[n].t = DotProduct( world, tv ) + tv[3] - tmin;
		n++;
	}

	for( int i = 0; i < n; i++ )
	{
		const rp_corner_t *a = &in[i], *b = &in[( i + 1 ) % n];
		int ain = a->z >= NEAR_CLIP, bin = b->z >= NEAR_CLIP;

		if( ain )
			out[m++] = *a;
		if( ain != bin )
		{
			float f = ( NEAR_CLIP - a->z ) / ( b->z - a->z );

			out[m].x = a->x + f * ( b->x - a->x );
			out[m].y = a->y + f * ( b->y - a->y );
			out[m].z = NEAR_CLIP;
			out[m].s = a->s + f * ( b->s - a->s );
			out[m].t = a->t + f * ( b->t - a->t );
			m++;
		}
	}
	if( m < 3 )
		return;
	if( m != n )
		rp_count.nearclipped++;
	for( int i = 0; i < m; i++ )
	{
		float zi = 1.0f / out[i].z;

		if( zi > nearzi )
			nearzi = zi;
		out[i].z = zi;
	}

	/* the software renderer's mip level for the face's nearest corner */
	scale = nearzi * scale_for_mip;
	miplevel = scale >= d_scalemip[0] ? 0 : scale >= d_scalemip[1] ? 1 : scale >= d_scalemip[2] ? 2 : 3;
	if( miplevel < d_minmip )
		miplevel = d_minmip;
	while( 1 << miplevel > gEngfuncs.Mod_SampleSizeForFace( fa ))
		miplevel--;
	w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
	h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
	if( w < 1 || h < 1 || w > RP_PAGE_SIDE || h > RP_PAGE_SIDE || w * h > RP_SCRATCH_TEXELS )
	{
		rp_count.skipped++;
		return;
	}
	rp_count.mip[miplevel & 3]++;
	rp_lit_bytes += w * h * 2;
	rp_block_bytes += RP_Pow2( w ) * RP_Pow2( h ) * 2;
	pg = RP_SurfacePlace( fa, miplevel, w, h, &qx, &qy );
	if( pg < 0 )
	{
		rp_count.skipped++;
		return;
	}
	RP_PageState( pg );

	/* s, t (texels at mip 0 from the surface's corner) to the page: at the
	   mip level, kept a 32nd of a texel inside the surface's far edges
	   (point sampling there would read the next square) */
	u0 = qx * 8 * ( 1.0f / RP_PAGE_SIDE );
	v0 = qy * 8 * ( 1.0f / RP_PAGE_SIDE );
	ks = ( w - 1.0f / 32.0f ) / ( (float)w * (float)( 1 << miplevel ) * RP_PAGE_SIDE );
	kt = ( h - 1.0f / 32.0f ) / ( (float)h * (float)( 1 << miplevel ) * RP_PAGE_SIDE );

	RP_Put( RP_GFX_STRIP( m ));
	lo = 1;
	hi = m - 1;
	for( k = 0; k < m; k++ )
	{
		const rp_corner_t *c = &out[k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--];
		float sx = xcenter + xscale * c->z * c->x;
		float sy = ycenter - yscale * c->z * c->y;

		RP_Put( RP_Bits( sy ));
		RP_Put( RP_Bits( (float)RP_GAME_W - sx ));
		RP_Put( RP_Bits( c->z ));
		RP_Put( RP_Bits( c->z ));
		RP_Put( 0xFFFFFFFFu );
		RP_Put( RP_Bits( u0 + c->s * ks ));
		RP_Put( RP_Bits( v0 + c->t * kt ));
	}
	RP_Whole();
	rp_count.faces++;
	rp_count.tris += m - 2;
	rp_count.verts += m;
}

/* The 2700G's picture of the frame drawn both ways, turned back to the
   game's 640 x 480, to the card beside the software renderer's. */
#define RP_GPU_SHOT "/rp-xash/frame-gpu.565"

static void RP_ChipShot( void )
{
	static uint16_t row[RP_GAME_W];
	const uint16_t *screen = (const uint16_t *)rp_g.screen;
	FILE *f;

	if( rp_gfx_wait( ) != 0 || !screen || !( f = fopen( RP_GPU_SHOT, "wb" )))
	{
		fprintf( stderr, "rp-emit: the 2700G's picture was not written\n" );
		return;
	}
	for( int gy = 0; gy < RP_GAME_H; gy++ )
	{
		for( int gx = 0; gx < RP_GAME_W; gx++ )
			row[gx] = screen[( RP_GAME_W - 1 - gx ) * rp_g.width + gy];
		fwrite( row, sizeof( row ), 1, f );
	}
	fclose( f );
	fprintf( stderr, "rp-emit: the 2700G's picture of the frame drawn both ways written to %s\n", RP_GPU_SHOT );
}

/* The software renderer's picture of the frame drawn both ways, as its
   blit makes it (`vid.screen` a pixel), where the test's end shot goes. */
#define RP_SOFT_SHOT "/rp-xash/frame.565"

static void RP_SoftShot( void )
{
	static uint16_t row[RP_GAME_W];
	FILE *f = fopen( RP_SOFT_SHOT, "wb" );

	if( !f )
	{
		fprintf( stderr, "rp-emit: the software renderer's picture was not written\n" );
		return;
	}
	for( int y = 0; y < RP_GAME_H; y++ )
	{
		const pixel_t *src = vid.buffer + y * vid.rowbytes;

		for( int x = 0; x < RP_GAME_W; x++ )
			row[x] = vid.screen[src[x]];
		fwrite( row, sizeof( row ), 1, f );
	}
	fclose( f );
	fprintf( stderr, "rp-emit: the software renderer's picture of the frame drawn both ways, the last, written to %s\n", RP_SOFT_SHOT );
}

/* The frame's end (R_BlitScreen): its whole primitives to the 2700G. */
void RP_EmitSubmit( void )
{
	if( rp_emit_on != 2 )
		return;
	if( rp_gfx_frame( rp_buf, rp_good ) != 0 )
		rp_count.refused++;
	if( rp_both )
	{
		RP_ChipShot();
		RP_SoftShot();
		rp_both = 0;
		gEngfuncs.Cvar_SetValue( "rp_both", 2.0f );
	}
}

/*
A world (or brush model) face the edge renderer would have clipped into
edges: its corners transformed and projected, clipped against the near
plane only (the 2700G clips the screen's sides itself), its surface lit
into the cache as the software renderer does (the texture it is drawn
with: docs/design/ref-rp.md choice 2 A), written as one strip.
*/
void RP_EmitFace( msurface_t *fa )
{
	if( rp_emit_on == 2 )
	{
		RP_ChipFace( fa );
		return;
	}

	rp_corner_t in[RP_MAX_CORNERS], out[RP_MAX_CORNERS + 1];
	const model_t *mod = RI.currentmodel;
	const mtexinfo_t *tex = fa->texinfo;
	int n = 0, m = 0, lo, hi, miplevel, k;
	float nearzi = 0.0f, smin, tmin, sinv, tinv;
	surfcache_t *cache;

	if( fa->flags & ( SURF_DRAWSKY | SURF_DRAWTURB ))
		return; /* the sky and water: a later step */
	if( fa->numedges < 3 || fa->numedges > RP_MAX_CORNERS )
		return;

	for( int i = 0; i < fa->numedges; i++ )
	{
		int lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
		const float *world = r_pcurrentvertbase[lindex > 0 ? e->v[0] : e->v[1]].position;
		vec3_t local, t;

		VectorSubtract( world, tr.modelorg, local );
		TransformVector( local, t );
		in[n].x = t[0];
		in[n].y = t[1];
		in[n].z = t[2];
		in[n].s = DotProduct( world, tex->vecs[0] ) + tex->vecs[0][3];
		in[n].t = DotProduct( world, tex->vecs[1] ) + tex->vecs[1][3];
		n++;
	}

	/* the near plane */
	for( int i = 0; i < n; i++ )
	{
		const rp_corner_t *a = &in[i], *b = &in[( i + 1 ) % n];
		int ain = a->z >= NEAR_CLIP, bin = b->z >= NEAR_CLIP;

		if( ain )
			out[m++] = *a;
		if( ain != bin )
		{
			float f = ( NEAR_CLIP - a->z ) / ( b->z - a->z );

			out[m].x = a->x + f * ( b->x - a->x );
			out[m].y = a->y + f * ( b->y - a->y );
			out[m].z = NEAR_CLIP;
			out[m].s = a->s + f * ( b->s - a->s );
			out[m].t = a->t + f * ( b->t - a->t );
			m++;
		}
	}
	if( m < 3 )
		return;
	if( m != n )
		rp_count.nearclipped++;

	for( int i = 0; i < m; i++ )
	{
		float zi = 1.0f / out[i].z;

		if( zi > nearzi )
			nearzi = zi;
		out[i].z = zi;
	}

	/* the surface lit into the cache at the software renderer's mip level */
	if( fa->flags & SURF_CONVEYOR )
		miplevel = 1;
	else
	{
		float scale = nearzi * scale_for_mip;

		miplevel = scale >= d_scalemip[0] ? 0 : scale >= d_scalemip[1] ? 1 : scale >= d_scalemip[2] ? 2 : 3;
		if( miplevel < d_minmip )
			miplevel = d_minmip;
	}
	while( 1 << miplevel > gEngfuncs.Mod_SampleSizeForFace( fa ))
		miplevel--;
	cache = D_CacheSurface( fa, miplevel );
	RP_CountLit( cache, miplevel );

	/* the cache's surfaces as pages of an atlas (128 KiB: a 256 x 256 page
	   of 16-bit texels): faces on one page share a state block */
	RP_State( (const void *)( (uintptr_t)cache >> 17 ), 0 );

	smin = fa->texturemins[0];
	tmin = fa->texturemins[1];
	sinv = fa->extents[0] ? 1.0f / fa->extents[0] : 0.0f;
	tinv = fa->extents[1] ? 1.0f / fa->extents[1] : 0.0f;

	/* one strip, zigzag: 0, 1, m-1, 2, m-2, ... */
	RP_Put( RP_TA_STRIP );
	lo = 1;
	hi = m - 1;
	for( k = 0; k < m; k++ )
	{
		const rp_corner_t *c = &out[k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--];
		float sx = xcenter + xscale * c->z * c->x;
		float sy = ycenter - yscale * c->z * c->y;

		RP_Put( RP_Bits( sx ));
		RP_Put( RP_Bits( sy ));
		RP_Put( RP_Bits( c->z ));
		RP_Put( RP_Bits( c->z ));
		RP_Put( 0xFFFFFFFFu );
		RP_Put( RP_Bits(( c->s - smin ) * sinv ));
		RP_Put( RP_Bits(( c->t - tmin ) * tinv ));
	}
	RP_Put( RP_TA_END );
	rp_count.faces++;
	rp_count.tris += m - 2;
	rp_count.verts += m;
}

