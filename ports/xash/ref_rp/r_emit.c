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
recently drawn page. The models are step 4a's (below); sprites,
particles, the sky, water and 2D are step 4's later sub-steps.
*/

#include <stdio.h>
#include <stdlib.h>
#include <retropane.h>
#include "r_local.h"
#include "rp_fixed.h"

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
static cvar_t *rp_both_var, *rp_gpu, *rp_fan, *rp_fixed, *rp_mip, *rp_cull, *rp_holes;
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
	uint32_t meshes, meshes_skipped, strips, fans, cmds_near, cmds_skipped, wraps; /* step 4a */
	uint32_t near_cut, near_gone; /* R299: model triangles cut at the near plane, wholly behind it */
	uint32_t culled; /* the models' triangles facing away or off the screen, not sent */
	uint32_t mipswitch; /* R297: faces drawn at a mip level other than their place's */
	uint32_t blended; /* step 4c: meshes drawn blended */
	uint32_t oldplace, unjams, coarser, deferred; /* the black walls: faces drawn from their old place, pages dropped to unjam, faces taken coarser (too large) */
	uint32_t fixed, mverts, mreuse; /* steps 4b, 4d */
	uint32_t bparts, bones, lit; /* 4d's rest: the bones and lighting in integers */
	uint32_t bfixed; /* brush entities' faces by the tables */
	uint64_t upload_us;
} rp_count;
static uint32_t rp_skins_total; /* step 4a: skins written since the start */

static void RP_FixedInit( void );

void RP_EmitInit( void )
{
	rp_emit = gEngfuncs.Cvar_Get( "rp_emit", "0", 0, "ref_rp: 1 the TA's words into RAM, no pixels (step 1); 2 the world drawn by the 2700G (step 3)" );
	rp_both_var = gEngfuncs.Cvar_Get( "rp_both", "0", 0, "ref_rp: 1 asks for a frame drawn both ways (the test's end shot), 2 once written" );
	rp_gpu = gEngfuncs.Cvar_Get( "rp_gpu", "0", 0, "ref_rp: 1 while the 2700G draws the frames" );
	rp_fan = gEngfuncs.Cvar_Get( "rp_fan", "1", 0, "ref_rp: 1 a model's fans as the TA's fans, 0 as strips" );
	rp_fixed = gEngfuncs.Cvar_Get( "rp_fixed", "1", 0, "ref_rp: 1 vertices in integers, each worked once a frame (steps 4b, 4d); 0 the float path" );
	rp_holes = gEngfuncs.Cvar_Get( "rp_holes", "0", 0, "ref_rp: 1 counts the clear colour showing at each report and keeps the first three frames with holes (a test: it waits for the render)" );
	rp_cull = gEngfuncs.Cvar_Get( "rp_cull", "1", 0, "ref_rp: 1 the models' triangles facing away or off the screen not sent (Half-Life's GL rule), 0 all sent, -1 the other winding" );
	rp_mip = gEngfuncs.Cvar_Get( "rp_mip", "-1", 0, "ref_rp: -1 each face's mip level from its nearest corner (the software renderer's); 0-3 every face at that level (R297's proof)" );
	RP_FixedInit();
}

/* The port layer's clock (ports/include/retropane.h; the runner's). */
uint64_t rp_time_us( void );

/*
The counts over the last five seconds, one plain line in the run log (the
same span as `-rpfps`'s frame rate, its own clock), then counted again.
*/
static int rp_mip_force, rp_cull_on, rp_chip;
static uint32_t RP_AtlasFill( void );
static void RP_HoleCheck( void );
static void RP_AtlasUnjam( void );
#define RP_UPLOAD_BUDGET ( 48 * 1024 ) /* bytes lit and written a frame: the hitches (RP_SurfacePlace) */
#define RP_FIRST_VIEW    45 /* frames after a map's tables are made with no budget: its first view whole */
static int rp_map_frames;
#define RP_PROVISIONAL 0x40000000u /* a place coarser than wanted (the upload budget) */
static int32_t rp_upload_left;

static void RP_EmitReport( void )
{
	static uint64_t since;
	uint64_t now = rp_time_us();
	uint32_t f = rp_count.frames;

	if( !since )
		since = now;
	/* every 30 s, not 5: each report is a dozen lines to the SD card, and
	   run 41's slowest card write took 108 ms (the owner: "still
	   hitching"); the runner's own fps line stays every 5 s */
	if( now - since < 30000000u || !f )
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
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: models a frame: %u meshes drawn (%u of them blended), %u not (no room), %u strips and %u fans; %u commands clipped at the near plane (%u triangles cut, %u wholly behind), %u too long; %u corners wrapped; %u skins written in all (fans as %s)\n",
			rp_count.meshes / f, rp_count.blended / f, rp_count.meshes_skipped / f, rp_count.strips / f, rp_count.fans / f,
			rp_count.cmds_near / f, rp_count.near_cut / f, rp_count.near_gone / f, rp_count.cmds_skipped, rp_count.wraps / f, rp_skins_total,
			rp_fan->value != 0.0f ? "the TA's fans" : "strips" );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: integers (steps 4b, 4d, rp_fixed %d): %u world faces a frame by the tables (%u of them brush entities'), %u by floats; model vertices %u worked and %u used again a frame\n",
			(int)rp_fixed->value, rp_count.fixed / f, rp_count.bfixed / f, ( rp_count.faces - rp_count.fixed ) / f,
			rp_count.mverts / f, rp_count.mreuse / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: atlas (R297, R299, rp_mip %d): %u faces a frame switched mip level (a pop each, the old place kept), the pages %u%% filled to their shelves; %u faces a frame drawn from their old place (no room), %u pages dropped to unjam, %u faces taken a level coarser (too large or no room), %u faces a frame waiting for the upload budget\n",
			rp_mip_force, rp_count.mipswitch / f, RP_AtlasFill(), rp_count.oldplace / f, rp_count.unjams, rp_count.coarser / f, rp_count.deferred / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: culling (rp_cull %d): %u model triangles a frame not sent (facing away or off the screen)\n",
			rp_cull_on, rp_count.culled / f );
	if( rp_emit_on == 2 && rp_chip > 0 && rp_holes->value != 0.0f )
		RP_HoleCheck();
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: bones in integers: %u model parts a frame, %u bones joined with the view, %u normals lit\n",
			rp_count.bparts / f, rp_count.bones / f, rp_count.lit / f );
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

/* rp_fixed, rp_fan and rp_mip as read at the frame's start. */
static int rp_fixed_on, rp_fan_on;

/* A new frame: the last one's words are let go (sent at its end by
   RP_EmitSubmit, or nowhere in step 1). */
void RP_EmitFrame( void )
{
	int want = rp_emit ? (int)rp_emit->value : 0;

	if( want == 2 && !RP_ChipOpen( ))
		want = 1;
	rp_emit_on = want == 1 || want == 2 ? want : 0;
	rp_both = rp_emit_on == 2 && rp_both_var->value == 1.0f;
	/* the switches as integers once a frame: read a vertex at a time
	   they were software-float compares (run 21's __aeabi_fcmpeq) */
	rp_fixed_on = rp_fixed->value != 0.0f;
	rp_fan_on = rp_fan->value != 0.0f;
	rp_mip_force = (int)rp_mip->value;
	rp_cull_on = (int)rp_cull->value;
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
	if( rp_emit_on == 2 )
		RP_AtlasUnjam();
	/* a map's first view goes in whole while the map comes up (the
	   owner, run 41: the opening door pixelated for seconds), then the
	   budget */
	rp_upload_left = rp_map_frames++ < RP_FIRST_VIEW ? 0x7FFFFFFF : RP_UPLOAD_BUDGET;
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

	__builtin_memcpy( &u, &f, sizeof( u )); /* inline: the build is -ffreestanding */
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
Steps 4b and 4d: vertices in integers, each worked once a frame
==============================================================================

Iterate, don't imitate (rule 0). `ref_soft` (and step 3 here) transforms
and projects a world corner in floats for every face it is in, and a model
vertex for every triangle command it is in; on the PXA270 (no FPU) every
float is a call into the software-float helpers (run 12: 59% of the frame).
`ref_gl` hands the floats to a GPU that transforms; the 2700G does not.
Ours: the view's rotation made once a frame as 2.30 integers, every vertex
as 16.16 (the world's at map load, a model's from its float with bit
operations), moved into view space with three 64-bit multiply-adds a row
(`smull`), and projected with one reciprocal (a 128-entry table and two
Newton steps of `umull`, no divide); the TA's floats made from the
integers by `clz` (RP_FixBits) and 1/z's float straight from the
reciprocal's bits. Each world vertex and each model vertex is worked once
a frame into a table that the faces and commands then only index; the
world's texture coordinates are made at map load. A vertex nearer than one
unit takes the float path (its face clipped at the near plane there).
*/

/* The game's picture is 640 x 480; the 2700G's is upright, 480 x 640. */
#define RP_GAME_W 640
#define RP_GAME_H 480
#define RP_ONE16 65536

static uint32_t rp_recip_tab[128];

/* 2^63 / zn for zn with its top bit set (zn in [2^31, 2^32)): the table
   by the top bits, then two Newton steps (y = y (2 - zn y)). */
static inline uint32_t RP_Recip( uint32_t zn )
{
	uint32_t y = rp_recip_tab[( zn >> 24 ) & 127];

	for( int i = 0; i < 2; i++ )
	{
		uint64_t d = 0u - (uint64_t)zn * y; /* 2^64 - zn y */
		uint64_t n = ( (uint64_t)y * (uint32_t)( d >> 32 )) >> 31;

		y = n > 0xFFFFFFFFu ? 0xFFFFFFFFu : (uint32_t)n;
	}
	return y;
}

static void RP_FixedInit( void )
{
	for( int i = 0; i < 128; i++ )
	{
		uint64_t n = ( 1ull << 63 ) / ((uint64_t)( 2 * ( 128 + i ) + 1 ) << 23 );

		rp_recip_tab[i] = n > 0xFFFFFFFFu ? 0xFFFFFFFFu : (uint32_t)n;
	}
}

/* A float as 16.16 by its bits (no helper call): |f| below 32768. */
static inline int32_t RP_Fix16( float f )
{
	uint32_t u, m;
	int      e, v;

	__builtin_memcpy( &u, &f, sizeof( u )); /* inline: the build is -ffreestanding */
	/* hidden from the optimiser: it turned the bit tests below back into
	   float compares, each a software call (run 39: __aeabi_fcmpeq 1.6%) */
	__asm__( "" : "+r"( u ));
	e = (int)( u >> 23 & 0xFF ) - 127;
	if( e < -17 )
		return 0;
	if( e > 14 )
		return u >> 31 ? -0x7FFFFFFF : 0x7FFFFFFF;
	m = ( u & 0x007FFFFFu ) | 0x00800000u;
	v = (int)( e >= 7 ? m << ( e - 7 ) : m >> ( 7 - e ));
	return u >> 31 ? -v : v;
}

/* The view this frame: rows right, up, forward as 2.30, the eye 16.16, the
   screen's centre and scale 16.16 (the turn and the 640 in RP_Project). */
static struct
{
	int     frame, serial;
	int32_t m[3][3], o[3];
	int32_t xc, yc, xs, ys;
	uint32_t mip[3]; /* d_scalemip / scale_for_mip as float bits */
	uint32_t mip_in[3], mip_out[3]; /* the hold's: x 1.25 to go finer, x 0.8 to go coarser */
} rp_view;

/* Each view the vertex tables are worked in has its own serial: the
   world's a frame, each brush entity's (its view turned into the entity's
   own space by R_RotateBmodel, the eye at tr.modelorg) its own. */
static int rp_vserial;
static struct
{
	int                frame, serial;
	const cl_entity_t *ent;
	int32_t            m[3][3], o[3];
} rp_bview;

static inline int32_t RP_Fix30( float f )
{
	/* |f| <= 1: 2.30 by the same bits as RP_Fix16, 14 more places */
	uint32_t u, m;
	int      e, v;

	__builtin_memcpy( &u, &f, sizeof( u )); /* inline: the build is -ffreestanding */
	/* hidden from the optimiser: it turned the bit tests below back into
	   float compares, each a software call (run 39: __aeabi_fcmpeq 1.6%) */
	__asm__( "" : "+r"( u ));
	e = (int)( u >> 23 & 0xFF ) - 127;
	if( e < -31 )
		return 0;
	if( e >= 1 )
		return u >> 31 ? -0x40000000 : 0x40000000;
	m = ( u & 0x007FFFFFu ) | 0x00800000u;
	v = (int)( e + 7 >= 0 ? m << ( e + 7 ) : m >> -( e + 7 ));
	return u >> 31 ? -v : v;
}

static void RP_ViewFixed( void )
{
	const float *row[3] = { RI.vright, RI.vup, RI.vforward };

	if( rp_view.frame == tr.framecount )
		return;
	rp_view.frame = tr.framecount;
	rp_view.serial = ++rp_vserial;
	for( int i = 0; i < 3; i++ )
	{
		for( int j = 0; j < 3; j++ )
			rp_view.m[i][j] = RP_Fix30( row[i][j] );
		rp_view.o[i] = RP_Fix16( RI.rvp.vieworigin[i] );
	}
	rp_view.xc = RP_Fix16( xcenter );
	rp_view.yc = RP_Fix16( ycenter );
	rp_view.xs = RP_Fix16( xscale );
	rp_view.ys = RP_Fix16( yscale );
	for( int k = 0; k < 3; k++ )
	{
		float t = d_scalemip[k] / scale_for_mip;

		rp_view.mip[k] = RP_Bits( t );
		rp_view.mip_in[k] = RP_Bits( t * 1.25f );
		rp_view.mip_out[k] = RP_Bits( t * 0.8f );
	}
}

/* Step 2 of the brief (brush entities by the tables): the current brush
   entity's view, once an entity a frame: the rows R_RotateBmodel turned
   into its space, the eye there (tr.modelorg). */
static void RP_BrushViewFixed( void )
{
	const float *row[3] = { RI.vright, RI.vup, RI.vforward };

	if( rp_bview.frame == tr.framecount && rp_bview.ent == RI.currententity )
		return;
	rp_bview.frame = tr.framecount;
	rp_bview.ent = RI.currententity;
	rp_bview.serial = ++rp_vserial;
	for( int i = 0; i < 3; i++ )
	{
		for( int j = 0; j < 3; j++ )
			rp_bview.m[i][j] = RP_Fix30( row[i][j] );
		rp_bview.o[i] = RP_Fix16( tr.modelorg[i] );
	}
}

/* A 16.16 point into view space (16.16) by the rows `m` (2.30), the eye `o`. */
static inline void RP_ViewPoint( const int32_t m[3][3], const int32_t o[3], const int32_t p[3], int32_t v[3] )
{
	int32_t d0 = p[0] - o[0], d1 = p[1] - o[1], d2 = p[2] - o[2];

	for( int i = 0; i < 3; i++ )
		v[i] = (int32_t)(( (int64_t)m[i][0] * d0 + (int64_t)m[i][1] * d1
			+ (int64_t)m[i][2] * d2 ) >> 30 );
}

/*
A view-space point (z at least one unit) to the TA's x, y (turned into
the upright picture: x = the game's y, y = 640 - the game's x) and 1/z,
the words RP_ChipFace's floats make, from integers.
*/
static inline void RP_ProjectS( const int32_t v[3], uint32_t w[3], int32_t *st );

static inline void RP_Project( const int32_t v[3], uint32_t w[3] )
{
	RP_ProjectS( v, w, NULL );
}

/* As RP_Project, and the TA's x, y as 28.4 integers into `st` (if not
   NULL): the models' culling reads them. */
static inline void RP_ProjectS( const int32_t v[3], uint32_t w[3], int32_t *st )
{
	int      n = __builtin_clz( (uint32_t)v[2] );
	uint32_t r = RP_Recip( (uint32_t)v[2] << n );
	/* x / z = x r 2^(n - 63): as 16.16 */
	int64_t  xz = ( (int64_t)v[0] * r ) >> ( 47 - n );
	int64_t  yz = ( (int64_t)v[1] * r ) >> ( 47 - n );
	/* the screen in 28.4 */
	int32_t  sx = ( rp_view.xc >> 12 ) + (int32_t)(( xz * rp_view.xs ) >> 28 );
	int32_t  sy = ( rp_view.yc >> 12 ) - (int32_t)(( yz * rp_view.ys ) >> 28 );

	w[0] = RP_FixBits( sy, 4 );
	w[1] = RP_FixBits(( RP_GAME_W << 4 ) - sx, 4 );
	if( st )
	{
		st[0] = sy;
		st[1] = ( RP_GAME_W << 4 ) - sx;
	}
	/* 1/z = r 2^(n - 47): r has its top bit set, so its float is direct */
	w[2] = (uint32_t)( n - 16 + 127 ) << 23 | ( r >> 8 & 0x007FFFFFu );
}


/*
The mip hold (the owner, run 39: "be clever"; R297). The software
renderer picks a face's level from its nearest corner against fixed
distances, so a face near a line flips level as the player moves or turns,
and each flip lights the face again on the CPU, writes it to video memory,
leaves its old place taken (the atlas's pressure, the black walls) and
shows as a pop. Here a face keeps the level it has until its nearest
corner is a quarter past the line (1.25 x nearer to go finer, 0.8 x to go
coarser): one change a pass, none while standing or strafing. `zi` is
1/z's float bits (positive floats compare as their bits).
*/
static uint32_t RP_Loc( msurface_t *fa );

static int RP_MipHold( msurface_t *fa, int level, uint32_t zi )
{
	uint32_t loc = RP_Loc( fa );
	int      p;

	if( !loc || ( loc & RP_PROVISIONAL ))
		return level;
	p = (int)( loc >> 24 & 3 );
	if( level < p )
		return zi >= rp_view.mip_in[p - 1] ? level : p;
	if( level > p )
		return zi < rp_view.mip_out[p] ? level : p;
	return level;
}

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
/* A model's: Gouraud (TSP bit 22, rp-mbx3d run 6); a skin with see-through
   texels ARGB1555 (format 0) drawn with the alpha test "greater than 0"
   (ISP bits 31-29 the compare in GL order, 28-21 the reference:
   rp-glesfeat A). */
#define RP_TSP_GOURAUD    ( 1u << 22 )
#define RP_ISP_ALPHA      0x80000000u
#define RP_TEX0_PAGE_1555 ( 0xA0000000u | 6u << 24 | 6u << 20 | 0u << 13 )

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
	uint32_t   live;         /* bytes drawn from it this frame (the unjam's choice) */
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

/* The face's place if it is still valid (its page not dropped since), or 0. */
static uint32_t RP_Loc( msurface_t *fa )
{
	uint32_t loc = (uint32_t)RP_LOC( fa );
	int      pg = loc >> 16 & 0xFF;

	if( !( loc & RP_VALID ) || pg >= rp_pages || (int)( RP_GEN( fa ) >> 1 ) != rp_page[pg].gen )
		return 0;
	return loc;
}

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

/* How full the pages are to their shelves' tops, in percent. */
static uint32_t RP_AtlasFill( void )
{
	uint32_t rows = 0;

	for( int pg = 0; pg < rp_pages; pg++ )
		rows += rp_page[pg].top;
	return rp_pages ? rows * 100 / ( rp_pages * RP_PAGE_SQUARES ) : 0;
}

/* A page emptied: its faces' places stop being valid (their generation). */
static void RP_PageDrop( rp_page_t *p )
{
	p->shelves = 0;
	p->top = 0;
	p->gen++;
}

/*
The black walls (the owner, runs 37-38; run 37's log: 41 faces a frame
switched mip level, 41 not drawn, 0 pages dropped, the pages 99% full).
Each page held a face drawn that frame, so the allocator could drop none,
and a face needing a new place (its level changed: the old place stays
taken until its page drops) got none, frame after frame: the cleared
screen showed through. Two cures, both kept: a face with no new place is
drawn from the place it has (RP_SurfacePlace), and a frame that ran out
of room drops, before the next frame's words, the page the frame drew
least from (its places come back; the faces on it are placed again).
*/
static int rp_atlas_short;

static void RP_AtlasUnjam( void )
{
	int least = -1;

	if( rp_atlas_short && rp_pages > 1 )
	{
		for( int pg = 0; pg < rp_pages; pg++ )
			if( least < 0 || rp_page[pg].live < rp_page[least].live )
				least = pg;
		/* the last frame's render may still read it */
		if( rp_page[least].used >= tr.framecount - 1 )
			rp_gfx_wait();
		RP_PageDrop( &rp_page[least] );
		rp_count.evicted++;
		rp_count.unjams++;
	}
	rp_atlas_short = 0;
	for( int pg = 0; pg < rp_pages; pg++ )
		rp_page[pg].live = 0;
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
	{
		rp_atlas_short = 1;
		return -1;
	}
	/* the render in flight (the last frame's) may read the page: waited for
	   only then (it is up to a whole render: the hitches, run 41); the
	   least recently drawn page is mostly older than that */
	if( rp_page[oldest].used >= tr.framecount - 1 )
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
/*
A brush entity's render mode (the owner, run 45: "I cant see through the
microwave window"): glass and other see-through brushes are drawn in the
engine's translucent pass with `tr.blend` (CL_FxBlend / 255). Rule 0:
`ref_soft` blends them on the CPU through its tables (a few fixed
levels), `ref_gl` makes a GL blend state; ours gives the 2700G the same
words the models' blends proved (step 4c: source alpha over one minus it,
or one plus one for the additive modes, depth compared and not written),
the alpha in each corner's colour (TEX2 takes the vertex's alpha), a state
block only when the page or the blend changes. The world and kRenderNormal
brushes stay opaque; kRenderTransAlpha (the '{' grates) is R303's.
*/
static int      rp_face_blend;                  /* 0 opaque, 1 alpha, 2 additive */
static uint32_t rp_face_colour = 0xFFFFFFFFu;    /* each corner's colour word */

static void RP_FaceBlend( void )
{
	const cl_entity_t *e = RI.currententity;
	int                mode = e && RI.currentmodel != WORLDMODEL ? e->curstate.rendermode : kRenderNormal;
	int32_t            a;

	rp_face_blend = 0;
	rp_face_colour = 0xFFFFFFFFu;
	if( mode == kRenderNormal || mode == kRenderTransAlpha )
		return;
	a = rp_fixq( tr.blend, 8 );
	a = a < 0 ? 0 : a > 255 ? 255 : a;
	if( mode == kRenderTransAdd || mode == kRenderGlow )
	{
		rp_face_blend = 2;
		rp_face_colour = 0xFF000000u | (uint32_t)a * 0x010101u;
	}
	else
	{
		rp_face_blend = 1;
		rp_face_colour = (uint32_t)a << 24 | 0x00FFFFFFu;
	}
}

static void RP_PageState( int pg )
{
	/* each blend its own key on the page (models use +1 to +6) */
	const void *what = (const uint8_t *)&rp_page[pg] + ( rp_face_blend ? 8 + rp_face_blend : 0 );
	uint32_t    tsp = RP_TSP_WORLD;

	if( what == rp_state )
		return;
	if( rp_face_blend )
	{
		tsp = ( tsp & 0x03FFFFFFu ) | ( rp_face_blend == 2 ? 1u << 29 | 1u << 26 : 4u << 29 | 5u << 26 )
			| 1u << 13 | 1u << 7;
		rp_count.blended++;
	}
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_GFX_STATE );
	RP_Put( tsp );
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
/*
The hitches (the owner, run 40: "it kept hitching"; its log: a dropped
page's faces lit and written again in one frame, up to 94 KiB, 6.5 ms a
frame averaged and far more in the frame it landed). Iterate, don't
imitate: the software renderer and ref_gl build what the view needs in
the frame it needs it; texture streaming in today's engines spends a
budget a frame and shows a coarser level until the full one is in. Here
each frame may light and write RP_UPLOAD_BUDGET bytes: a face over it keeps
its old texels (its light a frame late), or its old place at another
level, or goes in at a coarser level marked provisional (RP_PROVISIONAL:
the mip hold lets it sharpen the next frame there is room); the coarsest
level always goes in (1/64 of the face). A map's first view builds over
a few frames, coarse first, instead of one long stall.
*/

static int RP_SurfacePlace( msurface_t *fa, int miplevel, int w, int h, int *x, int *y, int *oldmip )
{
	uint32_t     loc = (uint32_t)RP_LOC( fa );
	image_t     *image = R_GetTexture( R_TextureAnimation( fa )->gl_texturenum );
	int          pg = loc >> 16 & 0xFF, stale, placed;
	surfcache_t *cache;
	uint64_t     t0;
	int          over;

	placed = ( loc & RP_VALID ) && (int)( loc >> 24 & 3 ) == miplevel && pg < rp_pages
		&& (int)( RP_GEN( fa ) >> 1 ) == rp_page[pg].gen;
	/* R297: a face still placed at another level (its old place stays
	   taken until its page drops) */
	if(( loc & RP_VALID ) && (int)( loc >> 24 & 3 ) != miplevel && pg < rp_pages
		&& (int)( RP_GEN( fa ) >> 1 ) == rp_page[pg].gen )
		rp_count.mipswitch++;
	stale = !placed || ( RP_GEN( fa ) & 1 ) || fa->dlightframe == tr.framecount
		|| (image_t *)RP_IMAGE( fa ) != image;
	for( int maps = 0; !stale && maps < MAXLIGHTMAPS && fa->styles[maps] != 255; maps++ )
		if( g_lightstylevalue[fa->styles[maps]] != fa->cached_light[maps] )
			stale = 1;
	over = stale && miplevel < 3 && w * h * 2 > rp_upload_left;

	if( placed )
	{
		*x = loc & 0xFF;
		*y = loc >> 8 & 0xFF;
		if( over )
		{
			/* its old texels this frame, relit when there is room */
			rp_count.deferred++;
			stale = 0;
		}
	}
	else
	{
		int npg = over ? -1 : RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, x, y );

		if( npg < 0 )
		{
			if( over )
				rp_count.deferred++;
			/* no room: drawn from the place it has at its old level,
			   never left out (the caller takes the level back) */
			if(( loc & RP_VALID ) && pg < rp_pages && (int)( RP_GEN( fa ) >> 1 ) == rp_page[pg].gen )
			{
				rp_count.oldplace++;
				rp_page[pg].used = tr.framecount;
				rp_page[pg].live += w * h * 2;
				*x = loc & 0xFF;
				*y = loc >> 8 & 0xFF;
				*oldmip = (int)( loc >> 24 & 3 );
				return pg;
			}
			return -1;
		}
		pg = npg;
	}
	rp_page[pg].used = tr.framecount;
	rp_page[pg].live += w * h * 2;
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
	rp_upload_left -= w * h * 2;
	RP_LOC( fa ) = (intptr_t)( RP_VALID | (uint32_t)miplevel << 24 | (uint32_t)pg << 16 | (uint32_t)*y << 8 | (uint32_t)*x );
	RP_IMAGE( fa ) = (intptr_t)image;
	RP_GEN( fa ) = (intptr_t)( rp_page[pg].gen << 1 | ( cache->dlight ? 1 : 0 ));
	rp_count.uploads++;
	rp_count.upload_kib += ( w * h * 2 ) >> 10;
	rp_count.upload_us += rp_time_us() - t0;
	return pg;
}

/*
The world's tables, made when a map is first drawn: each vertex as 16.16;
each face corner's s, t (mip 0 texels from the surface's corner, the same
sums RP_ChipFace's float path makes) as 24.8, by surfedge; each vertex's
words a frame (`frame` this frame's: `w[2]` 0 when nearer than a unit).
*/
typedef struct
{
	int      frame;
	uint32_t w[3];
	int32_t  xy[2]; /* the TA's x, y as 28.4: the cracks' cure reads them */
} rp_wvert_t;

static const model_t *rp_wmodel;
static int32_t     ( *rp_wpos )[3];
static int32_t     ( *rp_wst )[2];
static rp_wvert_t  *rp_wv;

static int RP_WorldTables( void )
{
	const model_t *mod = WORLDMODEL;

	if( mod == rp_wmodel )
		return rp_wst != NULL;
	rp_wmodel = mod;
	rp_map_frames = 0;
	free( rp_wpos );
	free( rp_wst );
	free( rp_wv );
	rp_wpos = NULL;
	rp_wst = NULL;
	rp_wv = NULL;
	if( !mod || !mod->numvertexes || !mod->numsurfedges )
		return 0;
	rp_wpos = malloc( sizeof( *rp_wpos ) * mod->numvertexes );
	rp_wv = calloc( mod->numvertexes, sizeof( *rp_wv ));
	rp_wst = malloc( sizeof( *rp_wst ) * mod->numsurfedges );
	if( !rp_wpos || !rp_wv || !rp_wst )
	{
		free( rp_wpos );
		free( rp_wv );
		free( rp_wst );
		rp_wpos = NULL;
		rp_wv = NULL;
		rp_wst = NULL;
		fprintf( stderr, "rp-emit: no room for the world's tables: the float path\n" );
		return 0;
	}
	for( int i = 0; i < mod->numvertexes; i++ )
	{
		rp_wv[i].frame = -1;
		for( int j = 0; j < 3; j++ )
			rp_wpos[i][j] = RP_Fix16( mod->vertexes[i].position[j] );
	}
	for( int f = 0; f < mod->numsurfaces; f++ )
	{
		const msurface_t  *fa = &mod->surfaces[f];
		const mtexinfo_t  *tex = fa->texinfo;
		int                luxels;
		const float       *sv, *tv;
		float              smin, tmin;

		if( !tex || !fa->info || fa->firstedge < 0 || fa->firstedge + fa->numedges > mod->numsurfedges )
			continue;
		luxels = ( tex->flags & TEX_WORLD_LUXELS ) != 0;
		sv = luxels ? tex->vecs[0] : fa->info->lmvecs[0];
		tv = luxels ? tex->vecs[1] : fa->info->lmvecs[1];
		smin = luxels ? fa->texturemins[0] : fa->info->lightmapmins[0];
		tmin = luxels ? fa->texturemins[1] : fa->info->lightmapmins[1];
		for( int i = 0; i < fa->numedges; i++ )
		{
			int lindex = mod->surfedges[fa->firstedge + i];
			const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
			const float *world = mod->vertexes[lindex > 0 ? e->v[0] : e->v[1]].position;

			rp_wst[fa->firstedge + i][0] = (int32_t)(( DotProduct( world, sv ) + sv[3] - smin ) * 256.0f );
			rp_wst[fa->firstedge + i][1] = (int32_t)(( DotProduct( world, tv ) + tv[3] - tmin ) * 256.0f );
		}
	}
	fprintf( stderr, "rp-emit: the world's tables: %d vertices, %d face corners (%u KiB)\n",
		mod->numvertexes, mod->numsurfedges,
		(uint32_t)(( sizeof( *rp_wpos ) + sizeof( *rp_wv )) * mod->numvertexes + sizeof( *rp_wst ) * mod->numsurfedges ) >> 10 );
	return 1;
}

/*
A world face (the world entity's: brush entities turn the view) by the
tables: each corner's words from its vertex's this frame (worked once), its
texture coordinates from the corner's s, t, all in integers. 0 if a corner
is nearer than a unit (the float path clips it).
*/
static int RP_ChipFaceFixed( msurface_t *fa, const int32_t m[3][3], const int32_t o[3], int serial )
{
	const model_t    *mod = RI.currentmodel;
	const mtexinfo_t *tex = fa->texinfo;
	const uint32_t   *cw[RP_MAX_CORNERS];
	const int32_t    *st[RP_MAX_CORNERS], *cxy[RP_MAX_CORNERS];
	int64_t           sx = 0, sy = 0; /* the face's centre on the screen, 28.4 */
	uint32_t          nearzi = 0;
	int               n = fa->numedges, miplevel, w, h, qx, qy, pg, luxels, lo, hi, kw, kh, oldmip = -1, wanted;

	for( int i = 0; i < n; i++ )
	{
		int         lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
		int         vi = lindex > 0 ? e->v[0] : e->v[1];
		rp_wvert_t *c = &rp_wv[vi];

		if( c->frame != serial )
		{
			int32_t v[3];

			c->frame = serial;
			RP_ViewPoint( m, o, rp_wpos[vi], v );
			if( v[2] < RP_ONE16 )
				c->w[2] = 0;
			else
				RP_ProjectS( v, c->w, c->xy );
		}
		if( !c->w[2] )
			return 0;
		if( c->w[2] > nearzi )
			nearzi = c->w[2];
		cw[i] = c->w;
		cxy[i] = c->xy;
		sx += c->xy[0];
		sy += c->xy[1];
		st[i] = rp_wst[fa->firstedge + i];
	}
	sx /= n;
	sy /= n;

	/* the software renderer's mip level for the nearest corner (positive
	   floats compare as their bits) */
	luxels = ( tex->flags & TEX_WORLD_LUXELS ) != 0;
	miplevel = nearzi >= rp_view.mip[0] ? 0 : nearzi >= rp_view.mip[1] ? 1 : nearzi >= rp_view.mip[2] ? 2 : 3;
	miplevel = RP_MipHold( fa, miplevel, nearzi );
	if( miplevel < d_minmip )
		miplevel = d_minmip;
	if( rp_mip_force >= 0 && rp_mip_force <= 3 )
		miplevel = rp_mip_force;
	while( 1 << miplevel > gEngfuncs.Mod_SampleSizeForFace( fa ))
		miplevel--;
	w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
	h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
	/* too large at its level: the next coarser one, never left out */
	while(( w > RP_PAGE_SIDE || h > RP_PAGE_SIDE || w * h > RP_SCRATCH_TEXELS ) && miplevel < 3
		&& 2 << miplevel <= gEngfuncs.Mod_SampleSizeForFace( fa ))
	{
		miplevel++;
		w >>= 1;
		h >>= 1;
		rp_count.coarser++;
	}
	if( w < 1 || h < 1 || w > RP_PAGE_SIDE || h > RP_PAGE_SIDE || w * h > RP_SCRATCH_TEXELS )
	{
		rp_count.skipped++;
		return 1;
	}
	rp_count.mip[miplevel & 3]++;
	rp_lit_bytes += w * h * 2;
	rp_block_bytes += RP_Pow2( w ) * RP_Pow2( h ) * 2;
	pg = RP_SurfacePlace( fa, miplevel, w, h, &qx, &qy, &oldmip );
	/* no room at its level: each smaller level (a quarter the room each)
	   before a face is ever left out (the black walls) */
	wanted = miplevel;
	while( pg < 0 && miplevel < 3 && 2 << miplevel <= gEngfuncs.Mod_SampleSizeForFace( fa ))
	{
		miplevel++;
		w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
		h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
		if( w < 1 || h < 1 )
			break;
		rp_count.coarser++;
		pg = RP_SurfacePlace( fa, miplevel, w, h, &qx, &qy, &oldmip );
	}
	if( pg < 0 )
	{
		rp_count.skipped++;
		return 1;
	}
	if( miplevel > wanted && oldmip < 0 )
		RP_LOC( fa ) |= RP_PROVISIONAL; /* coarser than its distance asks: sharpens when there is room */
	if( oldmip >= 0 )
	{
		/* drawn from its old place, at that place's level */
		miplevel = oldmip;
		w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
		h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
	}
	RP_PageState( pg );

	/* texels at the mip level, a 32nd of a texel inside the far edges:
	   x (w - 1/32) / w as 0.16 */
	kw = RP_ONE16 - ( 2048 + ( w >> 1 )) / w;
	kh = RP_ONE16 - ( 2048 + ( h >> 1 )) / h;

	RP_Put( RP_GFX_STRIP( n ));
	lo = 1;
	hi = n - 1;
	for( int k = 0; k < n; k++ )
	{
		int c = k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--;
		/* page texels as 24.8 */
		int32_t u = ( qx << 11 ) + (int32_t)(( (int64_t)( st[c][0] >> miplevel ) * kw ) >> 16 );
		int32_t v = ( qy << 11 ) + (int32_t)(( (int64_t)( st[c][1] >> miplevel ) * kh ) >> 16 );

		/* the cracks (emulator run of kit 40: the clear colour along the
		   seams where a face's corner lies on its neighbour's edge, the
		   T-junctions the map keeps): each corner half a pixel out from
		   the face's centre in x and in y, so a seam's gap of up to a
		   pixel is covered by the faces on both sides. The map compiler's
		   cure adds corners (words); GL lives with the sparkles; this
		   adds none. */
		int32_t x = cxy[c][0] + ( cxy[c][0] > sx ? 8 : cxy[c][0] < sx ? -8 : 0 );
		int32_t y = cxy[c][1] + ( cxy[c][1] > sy ? 8 : cxy[c][1] < sy ? -8 : 0 );

		RP_Put( RP_FixBits( x, 4 ));
		RP_Put( RP_FixBits( y, 4 ));
		RP_Put( cw[c][2] );
		RP_Put( cw[c][2] );
		RP_Put( rp_face_colour );
		RP_Put( RP_FixBits( u, 8 + 9 ));
		RP_Put( RP_FixBits( v, 8 + 9 ));
	}
	RP_Whole();
	rp_count.faces++;
	rp_count.fixed++;
	rp_count.tris += n - 2;
	rp_count.verts += n;
	return 1;
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
	int n = 0, m = 0, lo, hi, miplevel, k, w, h, qx, qy, pg, luxels, oldmip = -1, wanted;
	float nearzi = 0.0f, smin, tmin, scale, ks, kt, u0, v0;

	if( fa->flags & ( SURF_DRAWSKY | SURF_DRAWTURB | SURF_CONVEYOR ))
		return; /* the sky, water and conveyors: step 4 */
	if( fa->numedges < 3 || fa->numedges > RP_MAX_CORNERS )
		return;
	RP_FaceBlend();
	/* the world's faces by the integer tables (steps 4b, 4d), and a brush
	   entity's (a submodel: the world's own vertices and corners, its view
	   turned into the entity's space) by the same tables in its own view */
	if( rp_fixed_on && RP_WorldTables( ))
	{
		if( mod == WORLDMODEL )
		{
			RP_ViewFixed();
			if( RP_ChipFaceFixed( fa, (const int32_t( * )[3] )rp_view.m, rp_view.o, rp_view.serial ))
				return;
		}
		/* (the screen's numbers from the world's view, made unturned
		   before any entity: R_RotateBmodel turns RI's rows) */
		else if( rp_view.frame == tr.framecount && mod->vertexes == WORLDMODEL->vertexes && mod->surfedges == WORLDMODEL->surfedges
			&& mod->edges16 == WORLDMODEL->edges16 && mod->edges16 )
		{
			RP_BrushViewFixed();
			if( RP_ChipFaceFixed( fa, (const int32_t( * )[3] )rp_bview.m, rp_bview.o, rp_bview.serial ))
			{
				rp_count.bfixed++;
				return;
			}
		}
	}

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
	RP_ViewFixed();
	miplevel = RP_MipHold( fa, miplevel, RP_Bits( nearzi ));
	if( miplevel < d_minmip )
		miplevel = d_minmip;
	if( rp_mip_force >= 0 && rp_mip_force <= 3 )
		miplevel = rp_mip_force;
	while( 1 << miplevel > gEngfuncs.Mod_SampleSizeForFace( fa ))
		miplevel--;
	w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
	h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
	/* too large at its level: the next coarser one, never left out */
	while(( w > RP_PAGE_SIDE || h > RP_PAGE_SIDE || w * h > RP_SCRATCH_TEXELS ) && miplevel < 3
		&& 2 << miplevel <= gEngfuncs.Mod_SampleSizeForFace( fa ))
	{
		miplevel++;
		w >>= 1;
		h >>= 1;
		rp_count.coarser++;
	}
	if( w < 1 || h < 1 || w > RP_PAGE_SIDE || h > RP_PAGE_SIDE || w * h > RP_SCRATCH_TEXELS )
	{
		rp_count.skipped++;
		return;
	}
	rp_count.mip[miplevel & 3]++;
	rp_lit_bytes += w * h * 2;
	rp_block_bytes += RP_Pow2( w ) * RP_Pow2( h ) * 2;
	pg = RP_SurfacePlace( fa, miplevel, w, h, &qx, &qy, &oldmip );
	/* no room at its level: each smaller level (a quarter the room each)
	   before a face is ever left out (the black walls) */
	wanted = miplevel;
	while( pg < 0 && miplevel < 3 && 2 << miplevel <= gEngfuncs.Mod_SampleSizeForFace( fa ))
	{
		miplevel++;
		w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
		h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
		if( w < 1 || h < 1 )
			break;
		rp_count.coarser++;
		pg = RP_SurfacePlace( fa, miplevel, w, h, &qx, &qy, &oldmip );
	}
	if( pg < 0 )
	{
		rp_count.skipped++;
		return;
	}
	if( miplevel > wanted && oldmip < 0 )
		RP_LOC( fa ) |= RP_PROVISIONAL; /* coarser than its distance asks: sharpens when there is room */
	if( oldmip >= 0 )
	{
		/* drawn from its old place, at that place's level */
		miplevel = oldmip;
		w = ( luxels ? fa->extents[0] : fa->info->lightextents[0] ) >> miplevel;
		h = ( luxels ? fa->extents[1] : fa->info->lightextents[1] ) >> miplevel;
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
		RP_Put( rp_face_colour );
		RP_Put( RP_Bits( u0 + c->s * ks ));
		RP_Put( RP_Bits( v0 + c->t * kt ));
	}
	RP_Whole();
	rp_count.faces++;
	rp_count.tris += m - 2;
	rp_count.verts += m;
}

/*
==============================================================================
Step 4a: the models on the 2700G (docs/design/ref-rp.md "Step 4")
==============================================================================

Iterate, don't imitate (rule 0). `ref_soft` hands each of a model's stored
strips and fans to its triangle interface a vertex at a time and draws
them as loose triangles (three corners each); `ref_gl` makes a GL call a
vertex. Here each stored command goes to the TA as one strip or one fan
(the TA's own fan, primitive word 0x5): a corner a vertex, the triangles
the chip's. Each skin is written once, at its first use, into the same
video-memory atlas as the world's lit surfaces (its own size rounded to
whole 8 x 8 squares, no power-of-two waste, no resampling), RGB565 through
the renderer's own table; a skin with see-through texels as ARGB1555 drawn
with the alpha test, the software renderer's skipped texel. The light is
the vertex colour, Gouraud shaded (TSP bit 22, measured: rp-mbx3d run 6)
and multiplied into the texel (TEX2, the GLES MODULATE word): the
lighting's own colour, where the software renderer kept a grey in 32 steps.
*/

/* Each vertex of the model's part being drawn, worked once (step 4b):
   `serial` the part's (RP_StudioVerts), `w[2]` 0 when nearer than a unit. */
typedef struct
{
	int      serial;
	uint32_t w[3];
	int32_t  q[3]; /* in view space, 16.16: the near plane's clip reads it */
	int32_t  xy[2]; /* the TA's x, y as 28.4: the culling reads them */
} rp_mvert_t;

static rp_mvert_t rp_mv[RP_STUDIO_VERTS];
static int        rp_studio_serial = 1;

/* A model's part has new vertices (R_StudioDrawPoints moved them). */
void RP_StudioVerts( void )
{
	rp_studio_serial++;
}

int RP_StudioSerial( void )
{
	return rp_studio_serial;
}

/*
4d's rest: the bones and the lighting in integers (rp_fixed.h, its rule 0).
R_StudioDrawPoints asks RP_StudioFixedBegin for a part; when on, each
vertex goes from the model's file through its bone joined with the view
straight into rp_mv (RP_StudioCmd then only indexes it), and each normal is
lit into its colour bytes by RP_StudioLightFixed. Bones joined at their
first use in the part, the light vector made integers once a bone.
*/
#define RP_BONES 128 /* MAXSTUDIOBONES */

static rp_fbone_t rp_fbone[RP_BONES];
static int32_t    rp_flight[RP_BONES][3];
static int        rp_fbone_serial[RP_BONES], rp_flight_serial[RP_BONES];
static struct
{
	int32_t g16[3], a16, s16;
	int     flat;
} rp_flit;

int RP_StudioFixedBegin( int numverts )
{
	if( rp_emit_on != 2 || rp_both || !rp_fixed_on || numverts > RP_STUDIO_VERTS )
		return 0;
	RP_ViewFixed();
	rp_studio_serial++;
	rp_count.bparts++;
	return 1;
}

void RP_StudioVertFixed( int idx, int bone, const float ( *bm )[4], const float *p )
{
	rp_mvert_t *mv = &rp_mv[idx];
	int32_t     a[3], q[3];

	if(( unsigned )bone >= RP_BONES )
		bone = 0;
	if( rp_fbone_serial[bone] != rp_studio_serial )
	{
		rp_fbone_serial[bone] = rp_studio_serial;
		rp_bone_join( (const int32_t( * )[3] )rp_view.m, rp_view.o, bm, &rp_fbone[bone] );
		rp_count.bones++;
	}
	for( int j = 0; j < 3; j++ )
		a[j] = rp_fixq( p[j], 16 );
	rp_bone_point( &rp_fbone[bone], a, q );
	mv->serial = rp_studio_serial;
	mv->q[0] = q[0];
	mv->q[1] = q[1];
	mv->q[2] = q[2];
	if( q[2] < RP_ONE16 )
		mv->w[2] = 0;
	else
		RP_ProjectS( q, mv->w, mv->xy );
	rp_count.mverts++;
}

/*
A chrome mesh's texels (R299: barney's helmet top; the owner, 2026-11-16:
every mesh drawn). R_StudioSetupChrome makes each bone's two vectors in
floats once an entity a frame, then two float dot products a normal; here
the vectors are made 2.30 once a bone and each normal's texels come from
two integer dot products, (n + 1) x 32 texels as 27.5.
*/
static int32_t rp_fchrome[RP_BONES][2][3];

void RP_StudioChromeBone( int bone, const float *right, const float *up )
{
	if(( unsigned )bone >= RP_BONES )
		return;
	for( int j = 0; j < 3; j++ )
	{
		rp_fchrome[bone][0][j] = rp_fixq( right[j], 30 );
		rp_fchrome[bone][1][j] = rp_fixq( up[j], 30 );
	}
}

void RP_StudioChromeFixed( int bone, const float *normal, int st[2] )
{
	int32_t n[3];

	if(( unsigned )bone >= RP_BONES )
		bone = 0;
	for( int j = 0; j < 3; j++ )
		n[j] = rp_fixq( normal[j], 30 );
	for( int c = 0; c < 2; c++ )
	{
		const int32_t *v = rp_fchrome[bone][c];
		int64_t        d = ( (int64_t)n[0] * v[0] + (int64_t)n[1] * v[1] + (int64_t)n[2] * v[2] ) >> 30;

		/* (d + 1) x 32 texels, as 27.5: (d30 + 2^30) x 1024 / 2^30 */
		st[c] = (int)(( d + ( 1 << 30 )) >> 20 );
	}
}

/* A mesh not drawn on the 2700G (none now: kept for counting). */
void RP_StudioSkipped( int tris )
{
	rp_count.models += tris;
}

void RP_StudioLightBegin( const float *color, float ambient, float shade )
{
	for( int c = 0; c < 3; c++ )
		rp_flit.g16[c] = rp_fixq( color[c], 16 ) * 255 / 1023;
	rp_flit.a16 = rp_fixq( ambient, 16 );
	rp_flit.s16 = rp_fixq( shade, 16 );
	/* STUDIO_NF_FLATSHADE: a + s x 0.8 */
	rp_flit.flat = rp_illum_index( rp_flit.a16 + rp_flit.s16 / 5 * 4 );
}

void RP_StudioLightFixed( int bone, const float *blight, int flags, const float *normal, byte *out )
{
	uint32_t gamma;

	if( FBitSet( flags, STUDIO_NF_FULLBRIGHT ))
		gamma = 1023;
	else
	{
		int index;

		if( FBitSet( flags, STUDIO_NF_FLATSHADE ))
			index = rp_flit.flat;
		else
		{
			int32_t n[3];

			if(( unsigned )bone >= RP_BONES )
				bone = 0;
			if( rp_flight_serial[bone] != rp_studio_serial )
			{
				rp_flight_serial[bone] = rp_studio_serial;
				for( int j = 0; j < 3; j++ )
					rp_flight[bone][j] = rp_fixq( blight[j], 28 );
			}
			for( int j = 0; j < 3; j++ )
				n[j] = rp_fixq( normal[j], 30 );
			index = rp_light_index( rp_dot30( n, rp_flight[bone] ), rp_flit.a16, rp_flit.s16 );
		}
		gamma = LightToTexGamma( (uint)index );
	}
	for( int c = 0; c < 3; c++ )
		out[c] = rp_light_byte( rp_flit.g16[c], gamma );
	rp_count.lit++;
}

/* The skins in the atlas, found by their texels' address. */
#define RP_SKINS 512

typedef struct
{
	const pixel_t *pixels;
	uint16_t       w, h;
	uint8_t        pg, qx, qy, masked;
	int            gen;
} rp_skin_t;

static rp_skin_t   rp_skin[RP_SKINS];
static const void *rp_skin_world; /* the map the table was filled under */
static const rp_skin_t *rp_cur_skin;

/* `rp_fan` 0 sends a fan as a strip (the TA's fan not trusted: ref-rp.md
   "What would overturn it"). */

/* A skin's texel to 16 bits: RGB565 through the renderer's table, or
   ARGB1555 with the see-through texel's alpha 0. */
static inline uint32_t RP_SkinTexel( pixel_t p, int masked )
{
	uint32_t c;

	if( !masked )
		return vid.screen[p];
	if( p == TRANSPARENT_COLOR )
		return 0;
	c = vid.screen[p];
	return 0x8000u | ( c >> 1 & 0x7FE0u ) | ( c & 0x1Fu );
}

/* A skin of `w` x `h` texels written into page `pg` at square (`qx`, `qy`)
   as RP_Upload writes a lit surface (past its edges the last row and
   column repeat). */
static void RP_SkinUpload( const pixel_t *src, int w, int h, int pg, int qx, int qy, int masked )
{
	uint32_t *page = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES );
	int       w8 = ( w + 7 ) >> 3, h8 = ( h + 7 ) >> 3;

	for( int sy = 0; sy < h8; sy++ )
	{
		for( int sx = 0; sx < w8; sx++ )
		{
			uint32_t *dst = page + RP_Interleave( qx + sx, qy + sy ) * 32;
			int       x0 = sx << 3, y0 = sy << 3;

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
					t[j] = RP_SkinTexel( src[y * w + x], masked );
				}
				dst[k] = t[0] | t[1] << 16;
			}
		}
	}
}

/*
The skin the triangle interface has bound (GL_Bind: `r_affinetridesc`) in
video memory, written there at its first use, and its state block: 1, or
0 when the mesh is not drawn (no room).

Step 4c, the blended models (the owner, 2026-11-16: no mesh skipped; they
were, 12 a frame in run 35's busy view): the render mode the triangle
interface was given (`vid.rendermode`, GL_SetRenderMode's) and the mesh's
additive bit pick the 2700G's blend in the state block (drivers/mbx3d
`blend`, `depth_test`: measured, rp-mbx3d version 23): a see-through mode
the source's alpha over 1 minus it, the alpha `tr.blend` in every corner's
colour (TEX2's alpha from the vertex); the additive ones (TransAdd, Glow,
an additive texture) 1 plus 1, their colours already scaled by the
engine; both with the depth compared and not written. Iterate, don't
imitate: `ref_soft` blends each pixel on the CPU through its tables and
`ref_gl` makes a GL state change a mesh; here the chip blends while it
draws, and the state block goes only when the blend or page changes.
*/
#define RP_BLEND_OPAQUE 0
#define RP_BLEND_ALPHA  1
#define RP_BLEND_ADD    2
static int      rp_mesh_flags;
static uint32_t rp_vert_alpha = 0xFF000000u;

void RP_StudioMeshFlags( int flags )
{
	rp_mesh_flags = flags;
}

int RP_StudioSkin( void )
{
	const pixel_t *px = r_affinetridesc.pskin;
	int            w = r_affinetridesc.skinwidth, h = r_affinetridesc.skinheight;
	uint32_t       slot;
	rp_skin_t     *e;
	const void    *what;

	int            blend = RP_BLEND_OPAQUE;
	uint32_t       tsp;

	rp_cur_skin = NULL;
	if( !px || w < 1 || h < 1 || w > RP_PAGE_SIDE || h > RP_PAGE_SIDE )
	{
		rp_count.meshes_skipped++;
		return 0;
	}
	if(( rp_mesh_flags & STUDIO_NF_ADDITIVE ) || vid.rendermode == kRenderTransAdd || vid.rendermode == kRenderGlow )
		blend = RP_BLEND_ADD;
	else if( vid.rendermode != kRenderNormal )
		blend = RP_BLEND_ALPHA;
	rp_vert_alpha = 0xFF000000u;
	if( blend == RP_BLEND_ALPHA )
	{
		int32_t a = rp_fixq( tr.blend, 8 );

		rp_vert_alpha = (uint32_t)( a < 0 ? 0 : a > 255 ? 255 : a ) << 24;
	}
	if( rp_skin_world != WORLDMODEL )
	{
		memset( rp_skin, 0, sizeof( rp_skin ));
		rp_skin_world = WORLDMODEL;
	}
	slot = (uint32_t)( (uintptr_t)px >> 4 ) % RP_SKINS;
	for( int i = 0;; i++ )
	{
		e = &rp_skin[( slot + i ) % RP_SKINS];
		if( !e->pixels || ( e->pixels == px && e->w == w && e->h == h ))
			break;
		if( i == RP_SKINS - 1 )
		{
			rp_count.meshes_skipped++;
			return 0;
		}
	}

	if( !e->pixels || e->gen != rp_page[e->pg].gen )
	{
		int x, y, pg, masked = 0;
		uint64_t t0 = rp_time_us();

		pg = RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, &x, &y );
		if( pg < 0 )
		{
			rp_count.meshes_skipped++;
			return 0;
		}
		for( int i = 0; i < w * h && !masked; i++ )
			masked = px[i] == TRANSPARENT_COLOR;
		RP_SkinUpload( px, w, h, pg, x, y, masked );
		e->pixels = px;
		e->w = (uint16_t)w;
		e->h = (uint16_t)h;
		e->pg = (uint8_t)pg;
		e->qx = (uint8_t)x;
		e->qy = (uint8_t)y;
		e->masked = (uint8_t)masked;
		e->gen = rp_page[pg].gen;
		rp_skins_total++;
		rp_count.upload_kib += ( w * h * 2 ) >> 10;
		rp_count.upload_us += rp_time_us() - t0;
	}
	rp_page[e->pg].used = tr.framecount;
	rp_page[e->pg].live += (uint32_t)e->w * e->h * 2;
	rp_cur_skin = e;

	/* a model's state block differs from the world's on its page (Gouraud,
	   the format, the alpha test): its own key beside the page's */
	what = (const uint8_t *)&rp_page[e->pg] + 1 + e->masked + 2 * blend;
	tsp = RP_TSP_WORLD | RP_TSP_GOURAUD;
	if( blend != RP_BLEND_OPAQUE )
	{
		/* the factors (bits 31-29 source, 28-26 destination: 4 the
		   source's alpha, 5 one minus it, 1 one), blending on (bit 13),
		   depth not written (bit 7) */
		tsp = ( tsp & 0x03FFFFFFu ) | ( blend == RP_BLEND_ADD ? 1u << 29 | 1u << 26 : 4u << 29 | 5u << 26 )
			| 1u << 13 | 1u << 7;
		rp_count.blended++;
	}
	if( what != rp_state )
	{
		rp_state = what;
		rp_count.states++;
		RP_Put( RP_GFX_STATE );
		RP_Put( tsp );
		RP_Put( e->masked ? RP_ISP_ALPHA : RP_ISP_ALWAYS );
		RP_Put( e->masked ? RP_TEX0_PAGE_1555 : RP_TEX0_PAGE );
		RP_Put( RP_TEX1( rp_g.tex_at + (uint32_t)e->pg * RP_PAGE_BYTES ));
		RP_Put( RP_TEX2 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Whole();
	}
	rp_count.meshes++;
	return 1;
}

/* The words of `n` (3 to 255) corners from `c`, in the order `idx`. */
static void RP_StudioWords( uint32_t prim, const uint32_t ( *c )[RP_VERTEX_WORDS], const uint16_t *idx, int n )
{
	RP_Put( prim | (uint32_t)n );
	for( int k = 0; k < n; k++ )
		for( int j = 0; j < RP_VERTEX_WORDS; j++ )
			RP_Put( c[idx[k]][j] );
	RP_Whole();
}

/*
R299, the near plane for the models (run 35: 394 commands a frame lost up
close, "the top half of a scientist"). Iterate, don't imitate (rule 0):
GL clips every triangle in clip space with floats, the software renderer
(R_AliasClip) every triangle a vertex at a time. Here a command clips only
where it must: its triangles wholly in front go on as runs of the same
strip or fan (one header a run), a triangle wholly behind is dropped, and
only a triangle across the plane is cut, in 16.16 integers (the new
corner's x, y, texels and colour by one 0.16 fraction), into a strip of 3
or 4 corners. The plane is the fixed path's own (z one unit), so no corner
is ever projected nearer.
*/
static inline int32_t RP_Lerp16( int32_t a, int32_t b, int32_t f )
{
	return a + (int32_t)(( (int64_t)( b - a ) * f ) >> 16 );
}

/* The corner where the edge a-b (a in front, b behind) meets the plane. */
static void RP_NearCorner( const rp_svert_t *v, const int32_t ( *cq )[3], const int32_t ( *cst )[2],
	int a, int b, uint32_t out[RP_VERTEX_WORDS], int32_t xy[2] )
{
	/* f = (z_a - 1) / (z_a - z_b) as 0.16: z_a >= 1 > z_b, so 0 <= f < 1 */
	int64_t  num = (int64_t)( cq[a][2] - RP_ONE16 ) << 16;
	int64_t  den = (int64_t)cq[a][2] - cq[b][2];
	int32_t  f = (int32_t)( num / den ), p[3];
	uint32_t rgb = rp_vert_alpha;

	p[0] = RP_Lerp16( cq[a][0], cq[b][0], f );
	p[1] = RP_Lerp16( cq[a][1], cq[b][1], f );
	p[2] = RP_ONE16;
	RP_ProjectS( p, out, xy );
	out[3] = out[2];
	for( int j = 0; j < 3; j++ )
		rgb |= (uint32_t)( v[a].rgb[j] + ((( v[b].rgb[j] - v[a].rgb[j] ) * f ) >> 16 )) << ( 16 - 8 * j );
	out[4] = rgb;
	out[5] = RP_FixBits( RP_Lerp16( cst[a][0], cst[b][0], f ), 5 + 9 );
	out[6] = RP_FixBits( RP_Lerp16( cst[a][1], cst[b][1], f ), 5 + 9 );
}

/* The words of a run of corners `idx[0..m)`, as one primitive in pieces
   of 255 (a fan's later pieces start with its first corner again). */
static void RP_StudioRun( int fan, const uint32_t ( *c )[RP_VERTEX_WORDS], uint16_t *idx, int m )
{
	uint16_t piece[255];

	if( !fan )
	{
		for( int at = 0; at + 2 < m; at += 253 )
			RP_StudioWords( RP_GFX_STRIP( 0 ), c, idx + at, m - at < 255 ? m - at : 255 );
		return;
	}
	if( !rp_fan_on )
	{
		/* fans off (a test switch): the run's triangles one by one */
		for( int k = 1; k + 1 < m; k++ )
		{
			piece[0] = idx[0];
			piece[1] = idx[k];
			piece[2] = idx[k + 1];
			RP_StudioWords( RP_GFX_STRIP( 0 ), c, piece, 3 );
		}
		return;
	}
	for( int at = 1; at + 1 < m; at += 253 )
	{
		int p = m - at < 254 ? m - at : 254;

		piece[0] = idx[0];
		for( int k = 0; k < p; k++ )
			piece[1 + k] = idx[at + k];
		RP_StudioWords( RP_GFX_FAN( 0 ), c, piece, p + 1 );
	}
}

/*
The models' culling (the owner, run 39: "get us to 45-60 fps ... be
clever"). Models are 85% of the frame's words (run 37's heavy view: 36 of
42 thousand), and about half their triangles face away. Iterate, don't
imitate: ref_gl leaves them to the GPU (GL_FRONT culled: Half-Life's own
rule), and the software renderer draws them all; the 2700G culls only
after the bus has carried them (2700g.md), and the bus is the frame's
cost (about 133 ns a word). Here each triangle's facing comes from the
TA's own integer x, y (28.4: one cross product), its winding Half-Life's
GL rule (front counter-clockwise in GL's window, strips flipping every
other triangle; our turn into the upright picture keeps winding), and a
triangle wholly off the screen goes too. A gap of one or two dropped
triangles inside a run is sent anyway: a strip's restart costs two
corners, its header and end (16 words), a triangle in it 7. `rp_cull` 0
sends them all (the check), -1 the other winding; off while the view
model is mirrored (GL turns culling off then).
*/
#define RP_TRI_KEEP 0
#define RP_TRI_BACK 1
#define RP_TRI_CUT  2
#define RP_TRI_GONE 3

static inline int RP_Facing( const int32_t a[2], const int32_t b[2], const int32_t c[2] )
{
	int64_t x = (int64_t)( b[0] - a[0] ) * ( c[1] - a[1] ) - (int64_t)( b[1] - a[1] ) * ( c[0] - a[0] );

	return x > 0 ? 1 : x < 0 ? -1 : 0;
}

static inline int RP_OffScreen( const int32_t a[2], const int32_t b[2], const int32_t c[2] )
{
	const int32_t w = 480 << 4, h = 640 << 4;

	return ( a[0] < 0 && b[0] < 0 && c[0] < 0 ) || ( a[0] > w && b[0] > w && c[0] > w )
		|| ( a[1] < 0 && b[1] < 0 && c[1] < 0 ) || ( a[1] > h && b[1] > h && c[1] > h );
}

static void RP_StudioClipCmd( int fan, int n, const rp_svert_t *v, uint32_t ( *c )[RP_VERTEX_WORDS],
	const int32_t ( *cq )[3], const int32_t ( *cst )[2], int32_t ( *cxy )[2] )
{
	static uint16_t idx[RP_STUDIO_CMD + 1];
	static uint8_t  state[RP_STUDIO_CMD];
	int run = -1, tris = n - 2, cull = tr.fFlipViewModel ? 0 : rp_cull_on;

	/* each triangle's state */
	for( int i = 0; i < tris; i++ )
	{
		int t0 = fan ? 0 : i, t1 = i + 1, t2 = i + 2;
		int ins = ( cq[t0][2] >= RP_ONE16 ) + ( cq[t1][2] >= RP_ONE16 ) + ( cq[t2][2] >= RP_ONE16 );

		if( ins == 0 )
			state[i] = RP_TRI_GONE;
		else if( ins < 3 )
			state[i] = RP_TRI_CUT;
		else if( !cull )
			state[i] = RP_TRI_KEEP;
		else
		{
			/* GL's strip order: an odd triangle's first two corners swap */
			int f = RP_Facing( cxy[t0], cxy[t1], cxy[t2] );

			if( !fan && ( i & 1 ))
				f = -f;
			state[i] = f * cull > 0 && !RP_OffScreen( cxy[t0], cxy[t1], cxy[t2] ) ? RP_TRI_KEEP : RP_TRI_BACK;
		}
	}
	/* a gap of one or two dropped triangles between kept ones is sent */
	for( int i = 1; i < tris; i++ )
	{
		if( state[i] != RP_TRI_BACK || state[i - 1] != RP_TRI_KEEP )
			continue;
		if( i + 1 < tris && state[i + 1] == RP_TRI_KEEP )
			state[i] = RP_TRI_KEEP;
		else if( i + 2 < tris && state[i + 1] == RP_TRI_BACK && state[i + 2] == RP_TRI_KEEP )
			state[i] = state[i + 1] = RP_TRI_KEEP;
	}

	for( int i = 0; i <= tris; i++ )
	{
		int st = i < tris ? state[i] : -1; /* -1: the end, close the last run */

		if( st == RP_TRI_KEEP )
		{
			if( run < 0 )
				run = i;
			continue;
		}
		if( run >= 0 )
		{
			/* triangles run .. i - 1 */
			int m = 0;

			if( fan )
				idx[m++] = 0;
			for( int k = fan ? run + 1 : run; k <= i + 1; k++ )
				idx[m++] = (uint16_t)k;
			RP_StudioRun( fan, (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx, m );
			run = -1;
		}
		if( st == RP_TRI_BACK )
			rp_count.culled++;
		else if( st == RP_TRI_GONE )
			rp_count.near_gone++;
		else if( st == RP_TRI_CUT )
		{
			/* the triangle cut: its polygon in order (corners in front
			   kept, a new corner on each edge across), sent as a strip
			   in zigzag order, if it faces the eye */
			int      t[3] = { fan ? 0 : i, i + 1, i + 2 }, in[3];
			uint16_t poly[4], sidx[4];
			int      pn = 0, slot = n, f;

			for( int j = 0; j < 3; j++ )
				in[j] = cq[t[j]][2] >= RP_ONE16;
			for( int j = 0; j < 3; j++ )
			{
				int a = t[j], b = t[( j + 1 ) % 3], bin = in[( j + 1 ) % 3];

				if( in[j] )
					poly[pn++] = (uint16_t)a;
				if( in[j] != bin )
				{
					if( in[j] )
						RP_NearCorner( v, cq, cst, a, b, c[slot], cxy[slot] );
					else
						RP_NearCorner( v, cq, cst, b, a, c[slot], cxy[slot] );
					poly[pn++] = (uint16_t)slot++;
				}
			}
			f = RP_Facing( cxy[poly[0]], cxy[poly[1]], cxy[poly[2]] );
			if( !fan && ( i & 1 ))
				f = -f;
			if( cull && f * cull <= 0 )
			{
				rp_count.culled++;
				continue;
			}
			sidx[0] = poly[0];
			sidx[1] = poly[1];
			sidx[2] = poly[pn - 1];
			if( pn == 4 )
				sidx[3] = poly[2];
			RP_StudioWords( RP_GFX_STRIP( 0 ), (const uint32_t( * )[RP_VERTEX_WORDS] )c, sidx, pn );
			rp_count.near_cut++;
		}
	}
	rp_count.tris += n - 2;
	rp_count.verts += n;
}

/*
One stored command of the bound skin's mesh (`n` corners, a fan if `fan`):
each corner moved into view space, projected and turned as RP_ChipFace's,
its light the vertex colour, its texels into the skin's place on its page;
sent as one TA strip or fan (pieces of 255 corners at most); with a corner
behind the near plane, by RP_StudioClipCmd.
*/
void RP_StudioCmd( int fan, int n, const rp_svert_t *v )
{
	static uint32_t c[RP_STUDIO_CMD + 4][RP_VERTEX_WORDS];
	static uint16_t idx[RP_STUDIO_CMD * 2];
	static int32_t  cq[RP_STUDIO_CMD][3], cst[RP_STUDIO_CMD][2], cxy[RP_STUDIO_CMD + 4][2];
	const rp_skin_t *e = rp_cur_skin;
	int32_t u0, v0, smax, tmax;
	int     m, behind = 0;

	if( !e || n < 3 )
		return;
	if( n > RP_STUDIO_CMD )
	{
		rp_count.cmds_skipped++;
		return;
	}
	/* page texels as 27.5; kept a 32nd of a texel inside the skin's far
	   edges, as the world's */
	u0 = e->qx << 8;
	v0 = e->qy << 8;
	smax = ( e->w << 5 ) - 1;
	tmax = ( e->h << 5 ) - 1;
	if( rp_fixed_on )
		RP_ViewFixed();

	for( int k = 0; k < n; k++ )
	{
		int32_t s = v[k].s, tt = v[k].t;

		if( rp_fixed_on && (unsigned)v[k].idx < RP_STUDIO_VERTS )
		{
			/* each vertex of the model's part worked once (step 4b) */
			rp_mvert_t *mv = &rp_mv[v[k].idx];

			if( mv->serial != rp_studio_serial )
			{
				int32_t p[3], q[3];

				mv->serial = rp_studio_serial;
				for( int j = 0; j < 3; j++ )
					p[j] = RP_Fix16( v[k].xyz[j] );
				RP_ViewPoint( (const int32_t( * )[3] )rp_view.m, rp_view.o, p, q );
				mv->q[0] = q[0];
				mv->q[1] = q[1];
				mv->q[2] = q[2];
				if( q[2] < RP_ONE16 )
					mv->w[2] = 0;
				else
					RP_ProjectS( q, mv->w, mv->xy );
				rp_count.mverts++;
			}
			else
				rp_count.mreuse++;
			cxy[k][0] = mv->xy[0];
			cxy[k][1] = mv->xy[1];
			cq[k][0] = mv->q[0];
			cq[k][1] = mv->q[1];
			cq[k][2] = mv->q[2];
			if( !mv->w[2] )
				behind++;
			c[k][0] = mv->w[0];
			c[k][1] = mv->w[1];
			c[k][2] = mv->w[2];
			c[k][3] = mv->w[2];
		}
		else
		{
			vec3_t local, t;
			float  zi, sx, sy;

			VectorSubtract( v[k].xyz, RI.rvp.vieworigin, local );
			TransformVector( local, t );
			for( int j = 0; j < 3; j++ )
				cq[k][j] = RP_Fix16( t[j] );
			if( cq[k][2] < RP_ONE16 )
			{
				/* behind the near plane: its words come from the clip */
				behind++;
				c[k][2] = 0;
				goto texels;
			}
			zi = 1.0f / t[2];
			sx = xcenter + xscale * zi * t[0];
			sy = ycenter - yscale * zi * t[1];
			c[k][0] = RP_Bits( sy );
			c[k][1] = RP_Bits( (float)RP_GAME_W - sx );
			c[k][2] = RP_Bits( zi );
			c[k][3] = RP_Bits( zi );
			cxy[k][0] = RP_Fix16( sy ) >> 12;
			cxy[k][1] = RP_Fix16( (float)RP_GAME_W - sx ) >> 12;
		}
texels:
		/* a skin's texels past its edges wrap (counted: ref-rp.md "What
		   would overturn it"), kept inside it as the software renderer
		   keeps each vertex */
		if( s < 0 || s > ( e->w << 5 ) || tt < 0 || tt > ( e->h << 5 ))
		{
			rp_count.wraps++;
			s %= e->w << 5;
			tt %= e->h << 5;
			if( s < 0 )
				s += e->w << 5;
			if( tt < 0 )
				tt += e->h << 5;
		}
		if( s > smax )
			s = smax;
		if( tt > tmax )
			tt = tmax;
		c[k][4] = rp_vert_alpha | (uint32_t)v[k].rgb[0] << 16 | (uint32_t)v[k].rgb[1] << 8 | v[k].rgb[2];
		c[k][5] = RP_FixBits( u0 + s, 5 + 9 );
		c[k][6] = RP_FixBits( v0 + tt, 5 + 9 );
		cst[k][0] = u0 + s;
		cst[k][1] = v0 + tt;
	}
	if( behind || rp_cull_on )
	{
		/* each triangle sorted (R299's near plane, the culling): the ones
		   kept in runs, a triangle across the near plane cut */
		if( behind )
			rp_count.cmds_near++;
		RP_StudioClipCmd( fan, n, v, c, (const int32_t( * )[3] )cq, (const int32_t( * )[2] )cst, cxy );
		return;
	}

	if( !fan )
	{
		/* a strip in pieces of 255, each the last two corners again */
		for( int at = 0; at + 2 < n; at += 253 )
		{
			m = n - at < 255 ? n - at : 255;
			for( int k = 0; k < m; k++ )
				idx[k] = (uint16_t)( at + k );
			RP_StudioWords( RP_GFX_STRIP( 0 ), (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx, m );
		}
		rp_count.strips++;
	}
	else if( rp_fan_on )
	{
		/* a fan in pieces of 255, each the first corner and the last again */
		for( int at = 1; at + 1 < n; at += 253 )
		{
			m = n - at < 254 ? n - at : 254;
			idx[0] = 0;
			for( int k = 0; k < m; k++ )
				idx[1 + k] = (uint16_t)( at + k );
			RP_StudioWords( RP_GFX_FAN( 0 ), (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx, m + 1 );
		}
		rp_count.fans++;
	}
	else
	{
		/* a fan as a strip: 1, 0, 2, 3, 0, 4, 5, 0, 6 ... (each third
		   triangle the one before again; every fan triangle drawn) */
		m = 0;
		idx[m++] = 1;
		idx[m++] = 0;
		idx[m++] = 2;
		for( int i = 3; i < n; i++ )
		{
			idx[m++] = (uint16_t)i;
			if(( i & 1 ) && i + 1 < n )
				idx[m++] = 0;
		}
		for( int at = 0; at + 2 < m; at += 252 )
		{
			int p = m - at < 255 ? m - at : 254;

			/* pieces of an even count of triangles keep the pattern */
			RP_StudioWords( RP_GFX_STRIP( 0 ), (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx + at, p );
		}
		rp_count.fans++;
	}
	rp_count.tris += n - 2;
	rp_count.verts += n;
}

/* The 2700G's picture of the frame drawn both ways, turned back to the
   game's 640 x 480, to the card beside the software renderer's. */
#define RP_GPU_SHOT "/rp-xash/frame-gpu.565"

static void RP_ChipShotTo( const char *path );

static void RP_ChipShot( void )
{
	RP_ChipShotTo( RP_GPU_SHOT );
}

/*
The black walls' witness (the owner, runs 37-39: "still saw it a little
bit"; every counted path was 0 in run 39). The runner clears to RGB565
0x0001 (black to the eye); a pixel still that colour after the render is
one nothing drew over. Each report samples the picture on an 8-pixel grid
(about 4,700 reads), logs the holes, and keeps the first three frames with
any as pictures on the card (`/rp-xash/hole-N.565`, the game's 640 x 480)
to show where and what they are.
*/
static void RP_HoleCheck( void )
{
	static int shots;
	const uint16_t *screen = (const uint16_t *)rp_g.screen;
	uint32_t holes = 0, samples = 0;

	if( !screen || rp_gfx_wait( ) != 0 )
		return;
	/* the screen's last 16 columns hold the game's bottom rows: the fps bar */
	for( uint32_t y = 0; y < rp_g.height; y += 8 )
		for( uint32_t x = 0; x + 16 < rp_g.width; x += 8 )
		{
			samples++;
			holes += screen[y * rp_g.width + x] == 0x0001;
		}
	fprintf( stderr, "rp-emit: holes (the clear colour still showing): %u of %u samples\n", holes, samples );
	if( holes && shots < 3 )
	{
		char path[32];

		snprintf( path, sizeof( path ), "/rp-xash/hole-%d.565", ++shots );
		RP_ChipShotTo( path );
	}
}

static void RP_ChipShotTo( const char *path )
{
	static uint16_t row[RP_GAME_W];
	const uint16_t *screen = (const uint16_t *)rp_g.screen;
	FILE *f;

	if( rp_gfx_wait( ) != 0 || !screen || !( f = fopen( path, "wb" )))
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
	fprintf( stderr, "rp-emit: the 2700G's picture written to %s\n", path );
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


/*
A new map (R_NewMap, the engine's own word for it). The world is always
`mod_known[0]` (Mod_LoadWorld asserts it), so a pointer never tells one
map from the next: run 45's level change into c1a0d drew c1a0d's faces
from c1a0's tables (the walls black down its first hallway). Everything
kept per map is let go here, once, from the engine's call: the world's
tables, the skins' table, the atlas's places. Rule 0: ref_gl and ref_soft
rebuild their per-map state here, in the load's own pause; ours builds the
integer tables here too (not in the new map's first frame, which then
pays nothing) and logs what that costs.
*/
void RP_EmitNewMap( void )
{
	uint64_t t0 = rp_time_us( );

	rp_wmodel = NULL;
	rp_skin_world = NULL;
	rp_state = NULL;
	for( int pg = 0; pg < rp_pages; pg++ )
		RP_PageDrop( &rp_page[pg] );
	if( rp_emit_on == 2 && rp_fixed_on )
		RP_WorldTables( );
	rp_map_frames = 0;
	fprintf( stderr, "rp-emit: new map: per-map state reset, tables in %u us\n", (uint32_t)( rp_time_us( ) - t0 ));
}
