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

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <malloc.h>
#include <retropane.h>
#include "r_local.h"
#include "crclib.h"
#include "rp_fixed.h"

/* kit 89 (ref_light.c): the light pre-pass's misses, by reason */
extern unsigned rp_pre_counts[2];
extern unsigned rp_ln_counts[2]; /* kit 90: light nodes met, surfaces tried (ref_light.c) */

/* The TA's words (drivers/mbx3d: `ta::STRIP`, `ta::END`; the state
   block's mask as P4's textured draw, `ta_quads::TEXTURED_MASK`). */
#define RP_TA_STATE_MASK 0x10000057u
#define RP_TA_STRIP      0x41000000u
#define RP_TA_END        0xF0000000u
#define RP_STATE_WORDS   8
#define RP_VERTEX_WORDS  7
/* ADR-0040 step 7g7 (shown on the X51v, run 144): strips and fans whose
   corners leave out the z word (RP_GFX_NO_Z: depth from 1/w, which every
   corner here carries as the same word as z, the sky's own depth word
   apart). RP_GFX_NO_Z by default, 0 with -rpz7 (the control). */
static uint32_t rp_noz = RP_GFX_NO_Z;
/* Kit 116 (a probe, off by default): `-rphwcull1` / `-rphwcull2` send a
   model command with no corner behind the near plane whole, its header's
   bits 19-18 that value (Dell's D3D driver's cull field, a lead:
   local/re/7g6-gles-findings.md), with no facing test on the CPU */
static uint32_t rp_hwcull;
/* kit 103 (ADR-0040 step 7c): the cooked map, live when rp_ck_live */
static rp_cooked_t rp_ck;
static int         rp_ck_live;

/* A frame's words: 512 KiB (c1a0's opening view in the emulator: about
   83 Ki words, 4,200 triangles). Words past it are counted, not kept. */
#define RP_BUF_WORDS ( 128 * 1024 )

int rp_emit_on;
/* This frame is drawn both ways: the 2700G's and the software renderer's
   picture of one view, for the test's end shot (the engine's in_rp.c asks
   by the setting `rp_both` 1; 2 once both are written). `rp_gpu` says the
   2700G draws the frames. */
int rp_both;
static cvar_t *rp_near; /* R347: the near plane (1 ref_rp, 4 Half-Life GL) */
static cvar_t *rp_picture; /* R343: the picture's brightness, 0 to 8 */
/* R347 (the owner's ladder smear, run 280): the near plane in 16.16 view
   units, `rp_near` (1 by default, ref_rp's since step 4b; Half-Life's GL
   draws at 4): the world's faces, the water's cells and the models' and
   effects' corners cut against it. Read once a frame (RP_EmitFrame). */
static int32_t rp_near16 = 65536 /* RP_ONE16 */;

/* The owner, run 316 (2026-11-21, his clip): "when i am holding a shotgun and
   moving around it looks like a shotgun shell pops up huge in front of my
   face as I walk": a pale faceted shape at the lower left for single frames.
   The run's count: 68 of the models' commands a frame clipped at the near
   plane, 211 triangles wholly behind it: the weapon in the hands keeps
   parts at the eye (the shells in the left hand, for the reload). Half-Life
   draws with the near plane at 4 units, so its weapon models were made to
   hide those parts inside it; ours is 1 (the world seen closer to a wall),
   and the walk's bob brought a shell between 1 and 4 into view, as large as
   anything that near. Rule 0: one near plane for everything is the vendor's
   way (GL's 4) or ours (1); here each takes the one it was made for: the
   world ours, the weapon in the hands Half-Life's 4. */
#define RP_VIEWMODEL_NEAR16 ( 4 * 65536 )
static int32_t rp_near16_world;

void RP_ViewModelNear( int on )
{
	if( on )
	{
		rp_near16_world = rp_near16;
		if( rp_near16 < RP_VIEWMODEL_NEAR16 )
			rp_near16 = RP_VIEWMODEL_NEAR16;
	}
	else if( rp_near16_world )
		rp_near16 = rp_near16_world;
}
static cvar_t *rp_report; /* kit 80: a report at the next frame (rp_report 1) */
static cvar_t *rp_both_var, *rp_gpu, *rp_fan, *rp_fixed, *rp_mip, *rp_cull, *rp_holes, *rp_lm, *rp_lit;
static void RP_LmFlush( void );
static void RP_LmFrame( void );
/* D_CacheSurface lights into the scratch, not the RAM cache (r_surf.c). */
int rp_scratch;

static cvar_t *rp_emit;
/* Step 4f (RP_GFX_KEEP): the port layer reads a frame's words after
   rp_gfx_frame_parts returns, while the 2700G is fed by DMA; so two of
   each buffer, the next frame written into the other (rp_keep 1). The
   second pair comes from the heap only when the port asks; if it cannot,
   each frame waits for the last instead (rp_keep -1). */
static uint32_t rp_buf_a[RP_BUF_WORDS];
static uint32_t *rp_buf = rp_buf_a, *rp_buf_b;
#define RP_LM_WORDS  ( 32 * 1024 ) /* the second pass's words (c1a0's view: ~19 Ki) */
static uint32_t rp_lm_buf_a[RP_LM_WORDS];
static uint32_t *rp_lm_buf = rp_lm_buf_a, *rp_lm_buf_b;
static int rp_keep;
static uint32_t rp_n;
static const void *rp_state; /* what the last state block named */
static uint32_t rp_lit_bytes, rp_block_bytes; /* this frame's lit surfaces (step 1b) */
static int64_t rp_time_ms; /* the client's time this frame, ms (R309's water and conveyors) */

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
	uint32_t dlit, dlit_kib; /* R305: faces relit a frame for a dynamic light (never waiting) */
	uint32_t oldplace, unjams, coarser, deferred; /* the black walls: faces drawn from their old place, pages dropped to unjam, faces taken coarser (too large) */
	uint32_t fixed, mverts, mreuse; /* steps 4b, 4d */
	uint32_t bparts, bones, lit; /* 4d's rest: the bones and lighting in integers */
	uint32_t bfixed; /* brush entities' faces by the tables */
	uint32_t pics, pics_skipped, masked; /* the 2D layer's quads, tiles not drawn (no room); R303's see-through faces */
	uint32_t mask_texels, mask_keys; /* R321: see-through faces' texels uploaded, the key among them */
	uint32_t beams_fixed, beams_culled, beams_off; /* kit 78: beams walked in 16.16, out of the visible set, off the screen */
	uint32_t hwcull; /* kit 116: model commands sent whole for the TA to cull */
	uint32_t fx_fixed, fx_cut, fx_off, fx_behind; /* kit 114: fx_behind, wholly behind the near plane, dropped in integers; kit 77: the effects' triangles by the fixed-point corners, across the near plane (the software's way), off the screen */
	uint32_t fx_tris, fx_plain, fx_skipped; /* kit 74: the effects' triangles, drawn plainly (no picture), not drawn (no room) */
	uint32_t lm_faces, lm_built, lm_dlit, lm_words, lm_full, tex_built, tex_kib, tex_resets; /* R297 option B */
	uint32_t evict_waits; /* R352: pages dropped that a frame in flight could still read (waited for) */
	uint32_t staged, stage_flushes, stage_full; /* R352 after run 330: rewrites in place held to the frame's end (squares), the frames that had any, early flushes */
	uint32_t decals, decal_cut, decals_lit, decals_built, decal_arena_resets; /* R352: decals drawn over their faces; those cut at the near plane; those lit by their wall's lightmap */
	uint32_t path_flips, why_decal, why_tex, why_lm; /* R352: faces drawn the other way than the frame before (texture and lightmap against the lit surface), and why a face left option B */
	uint32_t sky_frames, sky_sides, sky_missed, sky_faces, water_faces, water_cells, water_missed, conveyors; /* R309 */
	uint32_t coplanar; /* R352 after run 333: brush entities' faces in a world face's plane, drawn just behind it */
	uint64_t upload_us;
} rp_count;
static uint32_t rp_skins_total; /* step 4a: skins written since the start */

static void RP_FixedInit( void );
static int      rp_lm_on;
/* rp_lit as read at the frame's start; its check (2): the one-pass words
   kept (1), then the staged ones compared with them (2); corners compared
   and corners that differed */
static int      rp_lit_on, rp_lit_cap, rp_lit_capn;
static uint32_t rp_lit_corners, rp_lit_differ;
/* kit 68's guard: corners sent that RP_LitFill had not filled (must be 0) */
static uint32_t rp_lit_unfilled;
/* kit 72: normals recorded for lighting when read, and lit (a report's) */
static uint32_t rp_nm_lazy, rp_nm_made;
/* rp_lit 2 also checks kit 61's one-call meshes and parts against the call a
   normal (a vertex): items made both ways, items that differed */
static uint32_t rp_b61_items, rp_b61_differ;
static uint32_t rp_tex_used, rp_tex_room; /* R297 B: the texture pool's bytes */

static int rp_tex2_plain; /* -rptex2plain (R340's probe): the ARGB4444 pictures under the plain TEX2 word */
/* Run 305 (the owner's play with the decals drawn as polygons): 14 stalls
   and 4,440 bit 9 events where run 297 had 6 and 1,329, and his video of
   faces drawn with other pictures. Until R340's cause is found the
   polygons are off by default (the old way: a decal face by its lit
   surface, which flashes when a decal lands but adds no small blended
   primitive to the frame); -rpdecalpoly turns them on.
   On by default again (2026-11-22, runs 324-326 under R340's strict chip
   order: decal polygons in four ordinary fight frames, a bullet hole drawn
   right on the floor in run 326's pictures, no fault, no stall; the red
   patch of runs 320 and 323 only in the test's end frame drawn both ways,
   which play never draws). With the engine now keeping every decal (its
   clip test reads the map's edges), the old way would send each marked
   wall through a surface rebuild; -rpdecalsoft keeps it for a side by side. */
static int rp_decal_on = 1;
static int rp_skyface_off; /* R352 after run 331: -rpskybox, the sky faces not drawn (R309's way) */
static int rp_nostage; /* R360: -rpnostage, rewrites in place written at once (before a68545bb: no wait, the hazard back) */
static int rp_coplanar_off; /* R352 after run 333: -rpcoplanar, brush entities' faces in a world plane at their own depth (the old way) */
static uint8_t *rp_cop; /* R352 after run 333: a bit a face, set for a brush entity's face in a world face's plane and side (RP_CoplanarMark) */
static uint32_t rp_cop_marked; /* how many the map has */
/* R352: decals lit by their wall's lightmap (Half-Life's GL look); -rpdecalunlit the unlit way (ref_soft's) */
static int rp_decal_lit = 1;
static cvar_t *rp_decal; /* 1 turns them on while a run plays (-rpcombo's second half) */
/* R352's eye on ordinary frames (-rpshots): rp_shot 1 asks for this frame's
   picture from the 2700G (shot-N.565 on the card, N the frame) and a line
   for each decal polygon drawn in it */
static cvar_t *rp_shot;
static int     rp_shot_now;

void RP_EmitInit( void )
{
	rp_emit = gEngfuncs.Cvar_Get( "rp_emit", "0", 0, "ref_rp: 1 the TA's words into RAM, no pixels (step 1); 2 the world drawn by the 2700G (step 3)" );
	rp_both_var = gEngfuncs.Cvar_Get( "rp_both", "0", 0, "ref_rp: 1 asks for a frame drawn both ways (the test's end shot), 2 once written" );
	rp_gpu = gEngfuncs.Cvar_Get( "rp_gpu", "0", 0, "ref_rp: 1 while the 2700G draws the frames" );
	rp_fan = gEngfuncs.Cvar_Get( "rp_fan", "1", 0, "ref_rp: 1 a model's fans as the TA's fans, 0 as strips" );
	rp_fixed = gEngfuncs.Cvar_Get( "rp_fixed", "1", 0, "ref_rp: 1 vertices in integers, each worked once a frame (steps 4b, 4d); 0 the float path" );
	if( gEngfuncs.Sys_CheckParm( "-rptex2plain" ))
		rp_tex2_plain = 1;
	if( gEngfuncs.Sys_CheckParm( "-rpskybox" ))
		rp_skyface_off = 1;
	if( gEngfuncs.Sys_CheckParm( "-rpnostage" ))
		rp_nostage = 1;
	if( gEngfuncs.Sys_CheckParm( "-rpcoplanar" ))
		rp_coplanar_off = 1;
	if( gEngfuncs.Sys_CheckParm( "-rpdecalsoft" )) /* R352: decals baked into the lit surface, the old way */
		rp_decal_on = 0;
	if( gEngfuncs.Sys_CheckParm( "-rpdecalunlit" ))
		rp_decal_lit = 0;
	if( gEngfuncs.Sys_CheckParm( "-rpfx0" )) /* R338's bisection: the float path, one word on the 31-word line */
		gEngfuncs.Cvar_Set( "rp_fixed", "0" );
	rp_holes = gEngfuncs.Cvar_Get( "rp_holes", "0", 0, "ref_rp: 1 counts the clear colour showing at each report and keeps the first three frames with holes (a test: it waits for the render)" );
	rp_cull = gEngfuncs.Cvar_Get( "rp_cull", "1", 0, "ref_rp: 1 the models' triangles facing away or off the screen not sent (Half-Life's GL rule), 0 all sent, -1 the other winding" );
	rp_mip = gEngfuncs.Cvar_Get( "rp_mip", "-1", 0, "ref_rp: -1 each face's mip level from its nearest corner (the software renderer's); 0-3 every face at that level (R297's proof)" );
	if( gEngfuncs.Sys_CheckParm( "-rpz7" ))
		rp_noz = 0; /* the control: seven-word corners */
	if( gEngfuncs.Sys_CheckParm( "-rphwcull1" ))
		rp_hwcull = 1u << 18;
	if( gEngfuncs.Sys_CheckParm( "-rphwcull2" ))
		rp_hwcull = 2u << 18;
	if( gEngfuncs.Sys_CheckParm( "-rpnocook" ))
		rp_cook_on = 0; /* kit 103's control: kit 102's walk, no cooked map */
	if( gEngfuncs.Sys_CheckParm( "-rpnofacecull" ))
		rp_facecull = 0; /* kit 104's control: off-screen faces sent as kit 103 did */
	rp_lit = gEngfuncs.Cvar_Get( "rp_lit", "1", 0, "ref_rp: 1 a lit part's corners in one pass (kit 53); 0 staged a corner at a time; 2 both, every corner's words compared (a check: the staged words sent)" );
	rp_lm = gEngfuncs.Cvar_Get( "rp_lm", "1", 0, "ref_rp: 1 the world as its textures and lightmaps, two passes on the 2700G (R297 option B); 0 each face's lit surface (choice 2 A)" );
	rp_decal = gEngfuncs.Cvar_Get( "rp_decal", "0", 0, "ref_rp: R352: 1 draws decals as polygons over their faces from now on" );
	rp_shot = gEngfuncs.Cvar_Get( "rp_shot", "0", 0, "ref_rp: R352: 1 writes this frame's 2700G picture to the card (shot-N.565) and logs its decals" );
	rp_picture = gEngfuncs.Cvar_Get( "rp_picture", "4", FCVAR_ARCHIVE, "ref_rp: R343: the picture's brightness, 0 (the game's own dark look) to 8 (daylight), 4 the default: the screen's curve and the light together" );
	rp_near = gEngfuncs.Cvar_Get( "rp_near", "1", 0, "ref_rp: R347: the near plane in units, 1 to 8 (1 ref_rp's own, 4 Half-Life GL's); a ladder's look" );
	rp_report = gEngfuncs.Cvar_Get( "rp_report", "0", 0, "ref_rp: 1 the counts reported at the next frame (a yardstick's window; kit 80)" );
	RP_FixedInit();
}

/* The port layer's clock (ports/include/retropane.h; the runner's). */
uint64_t rp_time_us( void );
/* R335: the frame-by-frame record (RP_HitchFrame) */
extern uint32_t rp_tex_rebuilds, rp_tex_rebuild_us_max;
static struct
{
	uint32_t worst_us, over50, over50_tex, over100, rebuilds;
} rp_hitch;


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

/* kit 80: the cvar rp_report, set by a yardstick (in_rp.c's -rpcascade,
   in the runner's module: a cvar crosses it) for a report at once, so one
   covers exactly the frames it scores (declared above RP_EmitInit) */

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
	if(( now - since < 30000000u && !( rp_report && rp_report->value != 0.0f )) || !f )
		return;
	if( rp_report && rp_report->value != 0.0f )
		gEngfuncs.Cvar_SetValue( "rp_report", 0.0f );
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
	if( rp_emit_on == 2 && rp_lit_on == 2 )
		fprintf( stderr, "rp-emit: rp_lit 2 (kit 53's check): %u corners built both ways, %u differed in a word\n",
			rp_lit_corners, rp_lit_differ );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: kit 68's lazy corners: %u sent unfilled (must be 0); kit 72's normals: %u a frame recorded, %u of them lit\n",
			rp_lit_unfilled, rp_nm_lazy / f, rp_nm_made / f );
	if( rp_emit_on == 2 && rp_lit_on >= 2 )
		fprintf( stderr, "rp-emit: rp_lit 2 (kit 61's check): %u normals and vertices made both ways (one call a mesh, one a normal), %u differed\n",
			rp_b61_items, rp_b61_differ );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: integers (steps 4b, 4d, rp_fixed %d): %u world faces a frame by the tables (%u of them brush entities'), %u by floats; model vertices %u worked and %u used again a frame\n",
			(int)rp_fixed->value, rp_count.fixed / f, rp_count.bfixed / f, ( rp_count.faces - rp_count.fixed ) / f,
			rp_count.mverts / f, rp_count.mreuse / f );
	if( rp_emit_on == 2 )
	{
		/* kit 103: the cooked walk's PVS arrays made (a cluster change each) */
		extern unsigned rp_vn_counts[4];

		fprintf( stderr, "rp-emit: kit 103's cooked map: %s; %u PVS arrays made since the last report (%u records each)\n",
			!rp_cook_on ? "off (-rpnocook)" : rp_ck_live ? "the walk on it" : "none (the engine's walk)",
			rp_vn_counts[0], rp_vn_counts[0] ? rp_vn_counts[1] / rp_vn_counts[0] : 0 );
		fprintf( stderr, "rp-emit: kit 103's screen test (rp_facecull %d): %u world faces a frame wholly off the screen not sent, %u sent\n",
			rp_facecull, rp_vn_counts[2] / f, rp_vn_counts[3] / f );
{
			extern int      rp_light_check;
			extern unsigned rp_light_checks[2];

			if( rp_light_check > 0 )
				fprintf( stderr, "rp-emit: kit 107's light lists checked against the scans: %u checks, %u differed (must be 0)%c", rp_light_checks[0], rp_light_checks[1], 10 );
			rp_light_checks[0] = rp_light_checks[1] = 0;
		}
		{
			extern unsigned rp_debris_counts[2];

			if( rp_debris_counts[0] )
				fprintf( stderr, "rp-emit: debris pieces a frame %u, %u of them not drawn (-rpdebrisN, 35 by default; -rpdebrisall)%c", rp_debris_counts[0] / f, rp_debris_counts[1] / f, 10 );
			rp_debris_counts[0] = rp_debris_counts[1] = 0;
			{
				extern unsigned rp_probe_counts[2];

				if( rp_probe_counts[0] )
					fprintf( stderr, "rp-emit: far pieces drawn plainly a frame %u, their cells sampled %u (-rpdebrisplainN)%c",
						rp_probe_counts[0] / f, rp_probe_counts[1] / f, 10 );
				rp_probe_counts[0] = rp_probe_counts[1] = 0;
			}
			{
				/* kit 109: the traces' sloped nodes settled by the cooked integers */
				extern unsigned rp_trace_counts[2];
				extern int      rp_trace_nnodes;

				fprintf( stderr, "rp-emit: the traces (kit 109, R358 every hull): %u sloped nodes a frame by the integers, %u by the floats (the world's table %s)%c",
					rp_trace_counts[0] / f, rp_trace_counts[1] / f, rp_trace_nnodes ? "on" : "off: no cooked map", 10 );
				rp_trace_counts[0] = rp_trace_counts[1] = 0;
			}
			{
				/* R358 after run 330: the planes' cache (RP_PlaneGet) */
				extern unsigned rp_pcache_counts[2];

				fprintf( stderr, "rp-emit: R358: planes taken from the cache %u a frame, taken into integers %u a frame%c",
					rp_pcache_counts[0] / f, rp_pcache_counts[1] / f, 10 );
				rp_pcache_counts[0] = rp_pcache_counts[1] = 0;
			}
			{
				/* R358: every hull's sloped nodes (the line above now counts them all) and -rphullcheck's differences */
				extern int      rp_hull_int, rp_hull_check;
				extern unsigned rp_hull_counts[8];

				if( rp_hull_check )
					fprintf( stderr, "rp-hull: -rphullcheck since the last report %u traces, %u points, %u boxes; differences in the run: %u, %u, %u (all must be 0; integers %s)%c",
						rp_hull_counts[0], rp_hull_counts[2], rp_hull_counts[4], rp_hull_counts[1], rp_hull_counts[3], rp_hull_counts[5], rp_hull_int ? "on" : "off", 10 );
				rp_hull_counts[0] = rp_hull_counts[2] = rp_hull_counts[4] = 0;
			}
			{
				/* the owner's "Yes, try both": -rpdebrishalf, -rpdebrissoundsN */
				extern unsigned rp_tent_half_counts[2], rp_tent_sound_counts[2];

				if( rp_tent_half_counts[1] )
					fprintf( stderr, "rp-emit: -rpdebrishalf: pieces moved %u a frame, held %u%c", rp_tent_half_counts[0] / f, rp_tent_half_counts[1] / f, 10 );
				if( rp_tent_sound_counts[0] || rp_tent_sound_counts[1] )
					fprintf( stderr, "rp-emit: debris sounds since the last report: %u played, %u skipped (-rpdebrissoundsN)%c", rp_tent_sound_counts[0], rp_tent_sound_counts[1], 10 );
				rp_tent_half_counts[0] = rp_tent_half_counts[1] = 0;
				rp_tent_sound_counts[0] = rp_tent_sound_counts[1] = 0;
				{
					extern unsigned rp_debris_list_counts[2];

					if( rp_debris_list_counts[0] )
						fprintf( stderr, "rp-emit: kit 110: debris candidates %u a frame, %u of them listed%c", rp_debris_list_counts[0] / f, rp_debris_list_counts[1] / f, 10 );
					rp_debris_list_counts[0] = rp_debris_list_counts[1] = 0;
				}
			}
		}
				rp_vn_counts[0] = rp_vn_counts[1] = rp_vn_counts[2] = rp_vn_counts[3] = 0;
	}
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: atlas (R297, R299, rp_mip %d): %u faces a frame switched mip level (a pop each, the old place kept), the pages %u%% filled to their shelves; %u faces a frame drawn from their old place (no room), %u pages dropped to unjam, %u faces taken a level coarser (too large or no room), %u faces a frame waiting for the upload budget; %u faces a frame relit for a dynamic light (%u KiB, never waiting: R305)\n",
			rp_mip_force, rp_count.mipswitch / f, RP_AtlasFill(), rp_count.oldplace / f, rp_count.unjams, rp_count.coarser / f, rp_count.deferred / f,
			rp_count.dlit / f, rp_count.dlit_kib / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: culling (rp_cull %d): %u model triangles a frame not sent (facing away or off the screen); kit 116: %u commands a frame sent whole for the TA to cull (field %u)\n",
			rp_cull_on, rp_count.culled / f, rp_count.hwcull / f, rp_hwcull >> 18 );
	if( rp_emit_on == 2 && rp_chip > 0 && rp_holes->value != 0.0f )
		RP_HoleCheck();
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: bones in integers: %u model parts a frame, %u bones joined with the view, %u normals lit\n",
			rp_count.bparts / f, rp_count.bones / f, rp_count.lit / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: 2D: %u quads a frame (the HUD, text, fills), %u tiles not drawn (no room); %u see-through faces a frame (R303)\n",
			rp_count.pics / f, rp_count.pics_skipped / f, rp_count.masked / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: kit 74's effects (sprites, beams, particles): %u triangles a frame, %u of them plain (no picture), %u not drawn (no room)\n",
			rp_count.fx_tris / f, rp_count.fx_plain / f, rp_count.fx_skipped / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: kit 77's effect corners in fixed point: %u triangles a frame, %u across the near plane (the software's way), %u wholly behind it (kit 114: dropped in integers), %u off the screen\n",
			rp_count.fx_fixed / f, rp_count.fx_cut / f, rp_count.fx_behind / f, rp_count.fx_off / f );
	if( rp_emit_on == 2 )
	{
		/* kit 80: the engine's own counts (engine/common/pm_trace.c), a frame */
		extern unsigned int rp_prof[10];

		fprintf( stderr, "rp-emit: kit 80's counts a frame: %u model light setups (%u kept), %u light traces walking %u brush models; %u physics traces testing %u entities; %u tempents; %u entity-by-name searches scanning %u edicts; %u model lights from kit 89's pre-pass (%u not in it, %u changed since)\n",
			rp_prof[0] / f, rp_prof[1] / f, rp_prof[2] / f, rp_prof[3] / f, rp_prof[4] / f, rp_prof[5] / f, rp_prof[6] / f, rp_prof[7] / f, rp_prof[8] / f, rp_prof[9] / f, rp_pre_counts[0] / f, rp_pre_counts[1] / f );
		fprintf( stderr, "rp-emit: kit 90's light walk a frame: %u light nodes met, %u surfaces tried\n", rp_ln_counts[0] / f, rp_ln_counts[1] / f );
		rp_ln_counts[0] = rp_ln_counts[1] = 0;
		{
			extern unsigned rp_grid_counts[2]; /* option C (ref_light.c) */

			if( rp_grid_counts[0] + rp_grid_counts[1] )
				fprintf( stderr, "rp-emit: option C's light grid a frame: %u points made, %u reused\n", rp_grid_counts[0] / f, rp_grid_counts[1] / f );
			rp_grid_counts[0] = rp_grid_counts[1] = 0;
		}
		{
			/* kit 115: the matrix concats a frame and the server's hitbox
			   bones among them (public/matrixlib.c, mod_studio.c) */
			extern unsigned int rp_concat_n[3];

			fprintf( stderr, "rp-emit: kit 115's matrix concats a frame: %u in all, %u of them the server's hitbox bones\n",
				rp_concat_n[0] / f, rp_concat_n[1] / f );
			rp_concat_n[0] = rp_concat_n[1] = rp_concat_n[2] = 0;
		}
		rp_pre_counts[0] = rp_pre_counts[1] = 0;
		for( int i = 0; i < 10; i++ )
			rp_prof[i] = 0;
	}
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: kit 78's beams: %u a frame walked in 16.16, %u a frame culled (out of the visible set), %u off the screen\n",
			rp_count.beams_fixed / f, rp_count.beams_culled / f, rp_count.beams_off / f );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: R321: see-through faces' texels uploaded %u, %u of them the see-through key\n",
			rp_count.mask_texels, rp_count.mask_keys );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: R297 B (rp_lm %d): %u faces a frame as texture and lightmap, %u lightmaps written a frame (%u for a dynamic light), %u second-pass words a frame, %u faces with no second-pass room; %u textures written (%u KiB), the texture pool %u of %u KiB, %u pool resets\n",
			rp_lm_on, rp_count.lm_faces / f, rp_count.lm_built / f, rp_count.lm_dlit / f, rp_count.lm_words / f, rp_count.lm_full / f,
			rp_count.tex_built, rp_count.tex_kib, rp_tex_used >> 10, rp_tex_room >> 10, rp_count.tex_resets );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: R352: %u faces drawn the other way than the frame before (option B against the lit surface: a flash); faces left option B for a decal %u, no texture room %u, no lightmap room %u; %u pages dropped in all, %u of them after a wait for the frames in flight; %u decals drawn over their faces (%u of them cut at the near plane, %u lit by their wall's lightmap; %u built, the rest kept from before; %u arena restarts)\n",
			rp_count.path_flips, rp_count.why_decal, rp_count.why_tex, rp_count.why_lm, rp_count.evicted, rp_count.evict_waits, rp_count.decals, rp_count.decal_cut, rp_count.decals_lit, rp_count.decals_built, rp_count.decal_arena_resets );
		fprintf( stderr, "rp-emit: R352: rewrites in place held to the frame's end: %u squares (%u KiB) in %u frames, %u times copied early (the 64 KiB full)\n",
			rp_count.staged, rp_count.staged >> 3, rp_count.stage_flushes, rp_count.stage_full );
	if( rp_emit_on == 2 )
	{
		char line[512];

		R_TexRingStats( line, sizeof( line ), true ); /* R335: the 8-bit textures and their ring */
		fprintf( stderr, "rp-emit: %s\n", line );
		fprintf( stderr, "rp-emit: R335 frame by frame: the slowest %u.%u ms, %u over 50 ms (%u of them rebuilt a texture), %u over 100 ms; %u rebuilds, the slowest %u us\n",
			rp_hitch.worst_us / 1000u, rp_hitch.worst_us / 100u % 10u, rp_hitch.over50, rp_hitch.over50_tex, rp_hitch.over100,
			rp_hitch.rebuilds, rp_tex_rebuild_us_max );
		memset( &rp_hitch, 0, sizeof( rp_hitch ));
		rp_tex_rebuild_us_max = 0;
	}
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: R309: the sky in %u of %u frames (%u sides a frame, %u not drawn: no room), %u water faces a frame in %u cells (%u not drawn), %u conveyor faces a frame\n",
			rp_count.sky_frames, f, rp_count.sky_sides / f, rp_count.sky_missed, rp_count.water_faces / f, rp_count.water_cells / f,
			rp_count.water_missed, rp_count.conveyors / f );
	if( rp_emit_on == 2 && rp_count.sky_frames )
		fprintf( stderr, "rp-emit: R352: sky faces drawn at their own depth %u a frame (pieces, one a cube side), hiding what the map keeps behind its sky (-rpskybox: not drawn)%c",
			rp_count.sky_faces / f, 10 );
	if( rp_emit_on == 2 )
		fprintf( stderr, "rp-emit: R352: %u brush entity faces a frame in a world face's plane drawn just behind it (%u marked in the map%s)%c",
			rp_count.coplanar / f, rp_cop_marked, rp_coplanar_off ? ", -rpcoplanar: not moved" : "", 10 );
	memset( &rp_count, 0, sizeof( rp_count ));
	rp_nm_lazy = rp_nm_made = 0;
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
			if( rp_g.flags & RP_GFX_KEEP )
			{
				rp_buf_b = malloc( sizeof( rp_buf_a ));
				rp_lm_buf_b = malloc( sizeof( rp_lm_buf_a ));
				rp_keep = rp_buf_b && rp_lm_buf_b ? 1 : -1;
				fprintf( stderr, "rp-emit: the frame's words are read after the call (step 4f): %s\n",
					rp_keep > 0 ? "two buffers, the next frame written into the other" : "no room for a second buffer, each frame waits for the last" );
			}
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

/* rp_fixed, rp_fan and rp_mip as read at the frame's start (rp_lit's
   with the report, above). */
static int rp_fixed_on, rp_fan_on;

/* A new frame: the last one's words are let go (sent at its end by
   RP_EmitSubmit, or nowhere in step 1). */
/*
Kit 88 (R324, runs 90-93 on the X51v, 2026-11-18): the cascade's heavy
frames lose 44.5% of their cycles to the I-cache (each of ~110 debris
pieces runs the whole model path, far past 32 KiB), and the runner locking
the path's hottest 16 KiB (ports/xash/icache-lock.txt) made them 10%
faster but the calm frames, whose code then has half the cache, 2.6%
slower. Iterate (rule 0): the engine leaves the cache to the hardware;
ours tells the runner when a frame is model-heavy, so the lock is held
only while it pays: on from 40 models drawn, off below 24 (a margin, so a
count near one line doesn't lock and unlock every frame). One loop over
the entities a frame; the hint every frame (the runner acts on a change).
*/
#define RP_HOT_ON  40
#define RP_HOT_OFF 24
static int rp_code_hot;

/*
The models the last scene drew, counted while its list is the scene's own
(run 301, 2026-11-21: a data abort a frame after "disconnect"). The count
used to be made at the next frame's start, from the last frame's list; but
leaving a map frees the entities the list points at before any frame
clears it, and the menu's frames draw no scene to refill it. Now it is
taken as a scene is handed over (RP_EmitSubmit), only for a frame that
drew one (tr.framecount moved), and a frame with no scene counts none.
*/
static int rp_hint_n, rp_hint_frame = -1;

static void RP_HintCount( void )
{
	int n = 0;

	if( !tr.draw_list || tr.framecount == rp_hint_frame )
	{
		rp_hint_n = 0; /* no scene this frame (the menu, the console) */
		return;
	}
	rp_hint_frame = tr.framecount;
	for( uint i = 0; i < tr.draw_list->num_solid_entities; i++ )
	{
		const cl_entity_t *e = tr.draw_list->solid_entities[i];

		n += e && e->model && e->model->type == mod_studio;
	}
	for( uint i = 0; i < tr.draw_list->num_trans_entities; i++ )
	{
		const cl_entity_t *e = tr.draw_list->trans_entities[i];

		n += e && e->model && e->model->type == mod_studio;
	}
	rp_hint_n = n;
}

static void RP_CodeHint( void )
{
	int n = rp_hint_n;

	int hot = rp_code_hot ? n >= RP_HOT_OFF : n >= RP_HOT_ON;

	/* every frame (ADR-0040 step 7b): the runner also puts the hot code
	   on the on-chip SRAM on the hint once the program plays, and again
	   after a module load took it off; once there, the call is a check */
	rp_code_hot = hot;
	rp_code_hint( hot );
}

/* R335 (the owner: "are there hitches in the fight scene?"): each frame's
   time from one RP_EmitFrame to the next, the slowest and those over 50
   and 100 ms, and which of the slow ones rebuilt a texture (r_image.c). */
static void RP_HitchFrame( void )
{
	static uint64_t last;
	static uint32_t seen;
	uint64_t        now = rp_time_us();

	if( last )
	{
		uint32_t dt = (uint32_t)( now - last );

		if( dt > rp_hitch.worst_us )
			rp_hitch.worst_us = dt;
		if( dt > 50000u )
		{
			rp_hitch.over50++;
			rp_hitch.over50_tex += rp_tex_rebuilds != seen;
		}
		rp_hitch.over100 += dt > 100000u;
	}
	rp_hitch.rebuilds += rp_tex_rebuilds - seen;
	last = now;
	seen = rp_tex_rebuilds;
}

void RP_EmitFrame( void )
{
	int want = rp_emit ? (int)rp_emit->value : 0;

	RP_HitchFrame( );

	/* step 4f: the last frame's words are still being read */
	if( rp_keep > 0 )
	{
		uint32_t *b = rp_buf, *l = rp_lm_buf;

		rp_buf = rp_buf_b;
		rp_buf_b = b;
		rp_lm_buf = rp_lm_buf_b;
		rp_lm_buf_b = l;
	}
	else if( rp_keep < 0 )
		rp_gfx_wait();

	if( want == 2 && !RP_ChipOpen( ))
		want = 1;
	rp_emit_on = want == 1 || want == 2 ? want : 0;
	rp_both = rp_emit_on == 2 && rp_both_var->value == 1.0f;
	/* the switches as integers once a frame: read a vertex at a time
	   they were software-float compares (run 21's __aeabi_fcmpeq) */
	rp_fixed_on = rp_fixed->value != 0.0f;
	rp_fan_on = rp_fan->value != 0.0f;
	rp_lit_on = (int)rp_lit->value;
	rp_mip_force = (int)rp_mip->value;
	rp_cull_on = (int)rp_cull->value;
	if( rp_decal && rp_decal->value != 0.0f )
		rp_decal_on = 1;
	rp_shot_now = rp_shot && rp_shot->value != 0.0f && rp_emit_on == 2;
	if( rp_shot_now )
		gEngfuncs.Cvar_SetValue( "rp_shot", 0.0f );
	{
		/* R347: 1 to 8 units, in 1/16ths */
		float n = rp_near->value;

		rp_near16 = n >= 1.0f && n <= 8.0f ? (int32_t)( n * 16.0f ) << 12 : 65536;
	}
	{
		/* R343, the owner's ruling ("do the smart slider"): `rp_picture`,
		   the one picture-brightness setting (0 to 8, 4 the default; the
		   menu's Video options slider; saved with the game's options).
		   A step moves two things together: the screen's curve (the
		   runner's path, rp_gfx_look: the scan-out's table, at once) and
		   the light (Half-Life's own `brightness` on the lightmaps: 0 at
		   the dark end, 1 at the default as the owner picked in run 292,
		   2 at the bright end), so no step is a muddy or washed-out pair.
		   Set when it changes and at the first frame (a config read later
		   may have put `brightness` elsewhere).
		   The curve is set at once (it costs nothing); the light once the
		   step has rested for a quarter of a second, since it has every
		   lightmap lit again (run 316: the owner dragging Valve's own
		   brightness slider, each move 0.12 s, the menu at 8 frames a
		   second). */
		static int      rp_picture_set = -1, rp_light_set = -1;
		static uint64_t rp_picture_at;
		static cvar_t  *rp_world;
		int             level = (int)( rp_picture->value + 0.5f );

		if( level < 0 )
			level = 0;
		if( level > 8 )
			level = 8;
		if( level != rp_picture_set )
		{
			int first = rp_picture_set < 0;

			rp_picture_set = level;
			rp_picture_at = first ? 0 : rp_time_us( );
			rp_gfx_look( level );
		}
		if( level != rp_light_set && ( !rp_picture_at || rp_time_us( ) - rp_picture_at > 250000u ))
		{
			rp_light_set = level;
			gEngfuncs.Cvar_SetValue( "brightness", (float)level * 0.25f );
		}
		/* behind a menu opened in a game: the agreed picture (the owner,
		   run 317: "i told you to make menu look like this, like the real
		   game"), so no world is drawn under the menu. Run 317's kit set
		   this setting to 1 and the engine saves it: put back to 0. */
		if( !rp_world )
			rp_world = gEngfuncs.Cvar_Get( "ui_renderworld", "0", 0, "render world when UI is visible" );
		if( rp_world && rp_world->value != 0.0f )
			gEngfuncs.Cvar_SetValue( "ui_renderworld", 0.0f );
	}
	RP_LmFrame();
	RP_CodeHint();
	rp_time_ms = (int64_t)( gp_cl->time * 1000.0 );
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
/* always inline (run 49: once kit 49 added callers the compiler made it a
   call, 828 samples of call overhead) */
static inline __attribute__(( always_inline )) uint32_t RP_FixBits( int v, int frac )
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
	RP_LmFlush(); /* the lit walls before the sprites, beams and particles */
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
static void RP_FxTriangle( void ); /* kit 74, by RP_EmitFill */

void RP_EmitTriangle( void )
{
	if( rp_emit_on == 2 )
	{
		RP_FxTriangle( ); /* kit 74: drawn on the 2700G */
		return;
	}

	const finalvert_t *v[3] = { aliastriangleparms.a, aliastriangleparms.b, aliastriangleparms.c };
	int sw = r_affinetridesc.skinwidth, sh = r_affinetridesc.skinheight;
	int su = sw > 0 ? 31 - __builtin_clz( (uint32_t)sw ) : 0;
	int tu = sh > 0 ? 31 - __builtin_clz( (uint32_t)sh ) : 0;

	RP_State( r_skin_img, (uint32_t)(uintptr_t)d_pdrawspans ); /* R335: the bound image, not its pixels */
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

/*
A face's divides by tables (FPS work after run 47: the PXA270 has no divide
instruction; RP_ChipFaceFixed's four a face, the centre's and the far
edges', were a third of __aeabi_idiv's 0.65% of the play). Exact: the
centre's |x| / n as |x| ceil(2^32 / n) / 2^32 (exact for |x| below 2^26 and
n up to 64: the sum of 64 corners' quarter pixels is under 2^24), its sign
put back (C's division truncates); the far edges' (2048 + w / 2) / w made
once for each width a page can hold.
*/
static uint32_t rp_recip_n[RP_MAX_CORNERS + 1];
static int32_t  rp_kedge[512 + 1];

static inline int32_t RP_DivN( int32_t x, int n )
{
	uint32_t a = x < 0 ? 0u - (uint32_t)x : (uint32_t)x;
	int32_t  q = (int32_t)(( (uint64_t)a * rp_recip_n[n] ) >> 32 );

	return x < 0 ? -q : q;
}

static void RP_FixedInit( void )
{
	for( int n = 1; n <= RP_MAX_CORNERS; n++ )
		rp_recip_n[n] = (uint32_t)(( 0xFFFFFFFFull + n ) / n ); /* ceil( 2^32 / n ) */
	for( int w = 1; w <= 512; w++ )
		rp_kedge[w] = RP_ONE16 - ( 2048 + ( w >> 1 )) / w;
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
	int                still; /* the entity unmoved and unturned: its planes are the world's (R352 after run 333) */
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
	rp_bview.still = RI.currententity && VectorIsNull( RI.currententity->origin ) && VectorIsNull( RI.currententity->angles );
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
/* The punch-through object kind (TSP bit 14): the alpha test acts only on
   it (2700g.md "Alpha test"; R321) */
#define RP_TSP_PUNCH      ( 1u << 14 )
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

/* R297 option B's own (the texture pool, a texel's twiddle) */
#define RP_TEX_PAGES 6           /* 3 MiB for the map's textures */
#define RP_LMAP      0x20000000u /* RP_LOC: the place holds the face's lightmap (B), not its lit surface */
static int      rp_tex_pages;
static uint16_t rp_spread[256];  /* a byte's bits to the even bits */
static int      rp_upload_raw;   /* RP_Upload's texels are RGB565 already */

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

	if( !( loc & RP_VALID ) || ( loc & RP_LMAP ) || pg >= rp_pages || (int)( RP_GEN( fa ) >> 1 ) != rp_page[pg].gen )
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
	/* R297 B: the top pages hold the map's textures (with their mip
	   chains), the rest stay the atlas (lightmaps, skins, the faces still
	   lit on the CPU) */
	rp_tex_pages = rp_pages >= RP_TEX_PAGES + 4 ? RP_TEX_PAGES : 0;
	rp_pages -= rp_tex_pages;
	rp_tex_room = (uint32_t)rp_tex_pages * RP_PAGE_BYTES;
	for( int b = 0; b < 256; b++ )
	{
		uint32_t v = 0;

		for( int k = 0; k < 8; k++ )
			v |= ( (uint32_t)b >> k & 1 ) << ( 2 * k );
		rp_spread[b] = (uint16_t)v;
	}
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

/*
R352 (the owner, runs 293-298: textures flashing; "it kinda reminds me of
what the throwable weapon models were doing"): how many frames back a page
may still be read by the chip. The rule here was written for one frame in
flight (a page drawn from in the last frame: wait for its render before
the page is dropped and written over). Kit 113's overlap keeps two: the
frame before this one (its TA running or its render waiting) and the one
before that (its render still drawing). A page last drawn from two frames
ago was dropped and given to other faces while that render read it: a
surface drawn for a frame with another surface's texels. Rule 0: a
desktop driver copies or renames a texture that is in use (memory we do
not have: 64 MB); ours keeps the cheap rule (the least recently drawn
page is nearly always far older, so the wait is rare) and counts the
frames the chip really holds.
*/
#define RP_IN_FLIGHT 2

static void RP_AtlasUnjam( void )
{
	int least = -1;

	if( rp_atlas_short && rp_pages > 1 )
	{
		for( int pg = 0; pg < rp_pages; pg++ )
			if( least < 0 || rp_page[pg].live < rp_page[least].live )
				least = pg;
		/* frames in flight may still read it (RP_IN_FLIGHT) */
		if( rp_page[least].used >= tr.framecount - RP_IN_FLIGHT )
		{
			rp_gfx_wait();
			rp_count.evict_waits++;
		}
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
	/* a frame in flight may read the page: waited for only then (it is up
	   to a whole render: the hitches, run 41); the least recently drawn
	   page is mostly older than that */
	if( rp_page[oldest].used >= tr.framecount - RP_IN_FLIGHT )
	{
		rp_gfx_wait();
		rp_count.evict_waits++;
	}
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
/* R303 (the owner, run 46: "the glass half wall is bright blue"): a face
   with a see-through texture (`{`: SURF_TRANSPARENT) keeps ref_soft's key
   texel (TRANSPARENT_COLOR) unlit in its surface; ours writes it as
   ARGB1555 with that texel's alpha 0, drawn with the alpha test (the
   models' masked skins' words, proven). Set per face by RP_ChipFace. */
static int rp_face_masked;

static inline uint32_t RP_FaceTexel( pixel_t p )
{
	uint32_t c;

	if( rp_upload_raw )
		return p;
	if( !rp_face_masked )
		return vid.screen[p];
	rp_count.mask_texels++; /* R321's check */
	if( p == TRANSPARENT_COLOR )
	{
		rp_count.mask_keys++;
		return 0;
	}
	c = vid.screen[p];
	return 0x8000u | ( c >> 1 & 0x7FE0u ) | ( c & 0x1Fu );
}

/*
R340 (run 297): asked before every write of ours into the 2700G's texture
memory. Rule 0: a desktop driver hands texels to the vendor's upload call
and never learns the chip died under it; Dell's GLES copies them with the
chip idle (one scene at a time). Ours writes the chip's memory itself
while it draws, so it asks the screen first (one call, no chip register
read once the chip is lost): 0 means the 2700G stopped answering, and
every word written now would wait out the bus time-out (4 to 9 ms a word:
a face's lightmap seconds), so the write is skipped; the next frame call
says ESTALE and RP_ChipLost places everything again. The same call is
the owner's flashlight lead's lever (the runner's -rptexhold): the write
held while the TA and a render are both at work.
*/
static inline int RP_ChipWrite( void )
{
	return rp_gfx_quiet( 1 );
}

/*
R352 after run 330 (the owner: texture glitching again, in fights; its log:
two bursts of chip events in two fights). A face whose light changed (a
muzzle flash, the flashlight, a flickering style) has its lightmap, or its
lit surface, written again at the SAME place in video memory (RP_LmPlace,
RP_SurfacePlace) while the CPU builds the next frame, and with kit 113's
overlap the frame before may still be rendering from those texels: for
that frame the face shows half the old light and half the new, the
one-frame flash of a fight. The vendor way (GL drivers) renames a texture
being written while in use, a copy in video memory we do not have (the
atlas is 98% full in fights); the textbook way waits for the render
before each write, which stalls the CPU every frame there is a light.
Ours: a rewrite in place is held in RAM, square by square (each 8 x 8
square is 128 bytes in a row of the twiddled page), and copied in at the
frame's end, once the render before has ended (rp_gfx_wait there: in a
CPU-bound fight it has long ended; in a chip-bound frame the frame call
would have waited for it anyway). New places are written at once as
before (no frame reads them yet). 64 KiB of held squares; past it the
squares so far are copied in early (after the same wait).
*/
#define RP_STAGE_SQUARES 512
static uint32_t rp_stage_words[RP_STAGE_SQUARES][32];
static uint32_t rp_stage_at[RP_STAGE_SQUARES]; /* the square's first word, from rp_g.tex */
static int      rp_stage_n;
static int      rp_upload_stage; /* RP_Upload: hold the squares (a rewrite in place) */

static void RP_StageFlush( void )
{
	if( !rp_stage_n )
		return;
	rp_gfx_wait( );
	if( RP_ChipWrite( ))
		for( int i = 0; i < rp_stage_n; i++ )
			memcpy((uint32_t *)rp_g.tex + rp_stage_at[i], rp_stage_words[i], sizeof( rp_stage_words[i] ));
	rp_stage_n = 0;
	rp_count.stage_flushes++;
}

static void RP_Upload( const pixel_t *src, int w, int h, int pg, int qx, int qy )
{
	uint32_t *page = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES );
	int       w8 = ( w + 7 ) >> 3, h8 = ( h + 7 ) >> 3;

	if( !RP_ChipWrite( ))
		return;

	for( int sy = 0; sy < h8; sy++ )
	{
		for( int sx = 0; sx < w8; sx++ )
		{
			uint32_t *dst = page + RP_Interleave( qx + sx, qy + sy ) * 32;
			int       x0 = sx << 3, y0 = sy << 3;

			if( rp_upload_stage )
			{
				if( rp_stage_n == RP_STAGE_SQUARES )
				{
					rp_count.stage_full++;
					RP_StageFlush( );
				}
				rp_stage_at[rp_stage_n] = (uint32_t)( dst - (uint32_t *)rp_g.tex );
				dst = rp_stage_words[rp_stage_n++];
				rp_count.staged++;
			}

			if( x0 + 8 <= w && y0 + 8 <= h )
			{
				const pixel_t *at = src + y0 * w + x0;

				for( int k = 0; k < 32; k++ )
				{
					int a = rp_square[2 * k], b = rp_square[2 * k + 1];

					dst[k] = RP_FaceTexel( at[( a >> 3 ) * w + ( a & 7 )] )
						| RP_FaceTexel( at[( b >> 3 ) * w + ( b & 7 )] ) << 16;
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
						t[j] = RP_FaceTexel( src[y * w + x] );
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
	const void *what = (const uint8_t *)&rp_page[pg] + ( rp_face_blend ? 8 + rp_face_blend : 0 ) + ( rp_face_masked ? 16 : 0 );
	uint32_t    tsp = RP_TSP_WORLD;

	if( what == rp_state )
		return;
	if( rp_face_blend )
		RP_LmFlush(); /* the lit walls before what blends over them */
	if( rp_face_blend )
	{
		tsp = ( tsp & 0x03FFFFFFu ) | ( rp_face_blend == 2 ? 1u << 29 | 1u << 26 : 4u << 29 | 5u << 26 )
			| 1u << 13 | 1u << 7;
		rp_count.blended++;
	}
	/* R321 (c1a0e's ladder drawn as a black strip): the alpha test works
	   only on a punch-through object, TSP bit 14 (2700g.md: Dell's GLES
	   sets it with GL_ALPHA_TEST); without it the holes drew black */
	if( rp_face_masked )
		tsp |= RP_TSP_PUNCH;
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_GFX_STATE );
	RP_Put( tsp );
	RP_Put( rp_face_masked ? RP_ISP_ALPHA : RP_ISP_ALWAYS );
	RP_Put( rp_face_masked ? RP_TEX0_PAGE_1555 : RP_TEX0_PAGE );
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

R305 (the owner, run 47: the flashlight "doesnt shine in the correct
location"; the log: 3-17 faces a frame waiting for the budget, the whole
run). A placed face larger than the budget (48 KiB: a wall of 160 x 160
texels at mip 0; pages hold 512 x 512) waited every frame, forever: the
flashlight's spot stayed where it was when the face was last lit. Xash's
ref_soft lights every face in the frame it is drawn (no budget: the
hitches); ref_gl uploads each dlit lightmap every frame (a lightmap is
1/256 a surface's texels). Ours: a face whose dynamic light changed (lit
by one this frame, or by one when it was last lit) never waits, and the
frame's first stale face always goes whatever its size, so no face starves;
the misses and new places stay under the budget (no hitches). With R297's
option B the dlit work shrinks to the lightmap, as ref_gl's.
*/

static int RP_SurfacePlace( msurface_t *fa, int miplevel, int w, int h, int *x, int *y, int *oldmip )
{
	uint32_t     loc = (uint32_t)RP_LOC( fa );
	image_t     *image = R_GetTexture( R_TextureAnimation( fa )->gl_texturenum );
	int          pg = loc >> 16 & 0xFF, stale, placed;
	surfcache_t *cache;
	uint64_t     t0;
	int          over, dlit;

	if( loc & RP_LMAP )
		loc = 0; /* its lightmap's place (B): its lit surface gets its own */
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
	/* R305: a dynamic light's face never waits; the frame's first face always goes */
	dlit = fa->dlightframe == tr.framecount || ( placed && ( RP_GEN( fa ) & 1 ));
	over = stale && !dlit && miplevel < 3 && w * h * 2 > rp_upload_left && rp_upload_left < RP_UPLOAD_BUDGET;

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
	if( fa->dlightframe == tr.framecount )
	{
		/* R305: the light into the face's own space, as D_DrawSurfaces sets it
		   per surface (ref_rp lights at emit time, where the flag and the
		   matrix were left by whichever entity set them last): the world's
		   faces as they are, a brush entity's moved by its origin and angles */
		if( RI.currentmodel == WORLDMODEL || !RI.currententity )
			tr.modelviewIdentity = true;
		else
		{
			Matrix4x4_CreateFromEntity( RI.objectMatrix, RI.currententity->angles, RI.currententity->origin, 1 );
			tr.modelviewIdentity = false;
		}
		rp_count.dlit++;
		rp_count.dlit_kib += ( w * h * 2 ) >> 10;
	}
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
	rp_upload_stage = placed && !rp_nostage; /* R352: a rewrite in place held to the frame's end */
	RP_Upload((const pixel_t *)cache->data, w, h, pg, *x, *y );
	rp_upload_stage = 0;
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
R297 option B (the owner's pick, 2026-11-16: docs/research/sheets/r297-mip-popping.md).
The world's faces as the map's own textures and their lightmaps, two
passes on the 2700G. Iterate, don't imitate (rule 0): `ref_soft` lights
each face's texels on the CPU at one mip level a face (the pops, the
atlas's churn, the flashlight's relights: runs 34-48); `ref_gl` draws the
texture and the lightmap with GL's mips and filtering, its textures
resampled to powers of two at load. Ours keeps ref_gl's split (the chip
picks a level a pixel and filters: no pops, no CPU lighting of texels),
and beats it on this machine: each texture goes into video memory once,
at its first use, in the chip's own twiddled order with its chain down to
1 x 1 (layout B, rp-mbx3d T12b), from the map's four stored levels where
they fit (no resampling of a power-of-two texture); the lightmaps are
ref_soft's own light levels (R_BuildLightMap's scale and gamma, a channel
at a time) in the atlas at one texel a sample, and only those are written
again when a light changes (a lightmap is a 256th of its face's texels);
every face's second pass waits in its own buffer and goes after the
opaque world in a few state blocks (the TBDR multiplies whole tiles), so
the walls' words go twice but no texel is lit on the CPU.
Faces with decals, see-through or blended textures, the conveyors and
any face whose texture or lightmap has no room stay on choice 2 A (their
lit surface), so nothing is ever left out.
*/
#define RP_TEX_SLOTS 512
#define RP_LM_TEXELS 1024          /* a lightmap's samples at most (HL's: 18 x 18) */
/* pass 1: the texture, opaque, depth written; pass 2: the lightmap,
   the destination's colour times the texel (the 2D layer's case 3),
   depth "greater or equal" and not written; TEX1 bilinear with the
   nearest level (T12b's mip bit, ref_gl's GL_LINEAR_MIPMAP_NEAREST), LOD 7 */
#define RP_TSP_LMAP ( ( RP_TSP_WORLD & 0x03FFFFFFu ) | 2u << 29 | 0u << 26 | 1u << 13 | 1u << 7 )
#define RP_TEX1_LINEAR( addr ) ( 0x8E000000u | (( addr ) >> 7 ))
#define RP_TEX1_MIP( addr )    ( 0xAE000000u | (( addr ) >> 7 ))

typedef struct
{
	uint32_t       serial; /* the image's upload (R335: never reused, unlike its slot) */
	uint32_t       tex0, tex1;
	int            gen; /* rp_tex_gen when it was written */
	int            sky; /* a skybox side: one level, at most 256 a side */
} rp_tex_t;

static rp_tex_t rp_tex[RP_TEX_SLOTS];
static int      rp_tex_gen = 1, rp_tex_full;
static uint32_t rp_lm_n;
/*
Kit 51 (R306: the second pass's memcpy into the frame, ~1-2% of the play
in run 49): the frame goes to the port layer as parts that lie where they
were made (rp_gfx_frame_parts), the first pass's stretches of rp_buf and
the second pass's of rp_lm_buf in their order, the sky first. ref_gl's GL
driver and the software renderer have no such copy only because they draw
as they go; the 2700G takes a frame whole, so a list of parts is ours:
no word moves twice. With no room left in the list, the old copy.
*/
static struct rp_gfx_part rp_parts[RP_GFX_PARTS];
static int      rp_nparts;
static uint32_t rp_seg_from, rp_lm_from; /* the parts not yet listed start here */
static int      rp_sky_seen; /* a sky face was in the view this frame */
static int      rp_water_left; /* R309: water cells the frame may still take (RP_WATER_FRAME) */
static void RP_SkyBox( void );
static int      rp_lm_page = -1;
static uint16_t rp_lm_texels[RP_LM_TEXELS];

static void RP_TexReset( void )
{
	rp_tex_gen++; /* (RP_BorrowReturn: what was borrowed from this room is not put back) */
	rp_tex_used = 0;
	rp_tex_full = 0;
}

/*
R344 (runs 276-278: a 2700G whose memory stopped taking writes, cleared
only by a reset): the runner resets and rebuilds the chip and the frame
call says ESTALE once: the chip's memory is lost. Rule 0: a desktop driver
says the same (a lost context, GL_ARB_robustness) and the game reloads
every texture from disk; ours keeps every texture's source in RAM already
(the atlas's pages and the texture pool are rebuilt on demand by their
generations), so dropping them all is the whole reload, done over the next
frames as faces are drawn, with no pause.
*/
static uint32_t rp_chip_lost;

static void RP_ChipLost( void )
{
	for( int pg = 0; pg < rp_pages; pg++ )
		RP_PageDrop( &rp_page[pg] );
	RP_TexReset( );
	rp_state = NULL;
	if( ++rp_chip_lost <= 5 )
		fprintf( stderr, "rp-emit: R344: the 2700G was reset (its memory lost, %u times): every texture placed again\n", rp_chip_lost );
}

/* Once a frame, before any face: the switch, and a pool that filled last
   frame emptied (the render in flight may read it: waited for). */
static void RP_LmFrame( void )
{
	rp_lm_on = rp_lm->value != 0.0f && rp_tex_pages > 0;
	rp_lm_n = 0;
	rp_lm_page = -1;
	rp_nparts = 0;
	rp_seg_from = rp_lm_from = 0;
	rp_sky_seen = 0;
	rp_water_left = 1024; /* RP_WATER_FRAME */
	if( rp_tex_full )
	{
		rp_gfx_wait();
		RP_TexReset();
		rp_count.tex_resets++;
	}
}

/* The second pass's words into the frame, after what was drawn so far. */
static void RP_AddPart( const uint32_t *w, uint32_t n )
{
	if( n )
	{
		rp_parts[rp_nparts].words = w;
		rp_parts[rp_nparts].count = n;
		rp_nparts++;
	}
}

static void RP_LmFlush( void )
{
	uint32_t n = rp_lm_n - rp_lm_from;

	if( !n )
		return;
	/* room kept for the sky and the last stretch */
	if( rp_nparts + 4 <= RP_GFX_PARTS )
	{
		uint32_t end = rp_n <= RP_BUF_WORDS ? rp_n : rp_good;

		if( end > rp_seg_from )
			RP_AddPart( &rp_buf[rp_seg_from], end - rp_seg_from );
		RP_AddPart( &rp_lm_buf[rp_lm_from], n );
		rp_seg_from = end;
	}
	else
	{
		if( rp_n + n <= RP_BUF_WORDS )
			memcpy( &rp_buf[rp_n], &rp_lm_buf[rp_lm_from], n * sizeof( uint32_t ));
		rp_n += n;
		RP_Whole();
	}
	rp_count.lm_words += n;
	rp_lm_from = rp_lm_n;
	rp_lm_page = -1;
	rp_state = NULL; /* the main stream's state changed */
}

static inline void RP_LmPut( uint32_t w )
{
	rp_lm_buf[rp_lm_n++] = w; /* room checked a face at a time */
}

/* A texel's place in a w x h (powers of two) twiddled texture: squares of
   the short side along the long one, x in a square's odd bits (2700g.md
   "Texture shapes", tex::twiddle_rect). */
static inline uint32_t RP_TexAt( uint32_t x, uint32_t y, int lw, int lh, int sb )
{
	uint32_t m = ( 1u << sb ) - 1u, sq = lw > lh ? x >> sb : y >> sb;
	uint32_t in = rp_spread[y & m & 0xFF] | (uint32_t)rp_spread[x & m & 0xFF] << 1;

	if( sb > 8 )
		in |= ( (uint32_t)rp_spread[( y & m ) >> 8] | (uint32_t)rp_spread[( x & m ) >> 8] << 1 ) << 16;
	return ( sq << ( 2 * sb )) + in;
}

static inline uint32_t RP_565Avg4( uint32_t a, uint32_t b, uint32_t c, uint32_t d )
{
	uint32_t r = (( a >> 11 ) + ( b >> 11 ) + ( c >> 11 ) + ( d >> 11 ) + 2 ) >> 2;
	uint32_t g = (( a >> 5 & 63 ) + ( b >> 5 & 63 ) + ( c >> 5 & 63 ) + ( d >> 5 & 63 ) + 2 ) >> 2;
	uint32_t bl = (( a & 31 ) + ( b & 31 ) + ( c & 31 ) + ( d & 31 ) + 2 ) >> 2;

	return r << 11 | g << 5 | bl;
}

/* One texel of a source level (`sw` x `sh` pixel_t), sampled for texel
   (x, y) of a `lw` x `lh` level: direct at the same size, else bilinear
   across the texture's wrap (a size that isn't a power of two). */
static uint32_t RP_TexSample( const pixel_t *src, int sw, int sh, int x, int y, int lw, int lh )
{
	int32_t  fx, fy, x0, y0, x1, y1, ax, ay;
	uint32_t c[4], r = 0, g = 0, b = 0;

	if( sw == lw && sh == lh )
		return vid.screen[src[y * sw + x]];
	/* the source position of the texel's centre, 16.16 */
	fx = (int32_t)((( (int64_t)( 2 * x + 1 ) * sw ) << 15 ) / lw ) - 32768;
	fy = (int32_t)((( (int64_t)( 2 * y + 1 ) * sh ) << 15 ) / lh ) - 32768;
	x0 = fx >> 16;
	y0 = fy >> 16;
	ax = fx >> 8 & 255;
	ay = fy >> 8 & 255;
	x0 = ( x0 % sw + sw ) % sw;
	y0 = ( y0 % sh + sh ) % sh;
	x1 = ( x0 + 1 ) % sw;
	y1 = ( y0 + 1 ) % sh;
	c[0] = vid.screen[src[y0 * sw + x0]];
	c[1] = vid.screen[src[y0 * sw + x1]];
	c[2] = vid.screen[src[y1 * sw + x0]];
	c[3] = vid.screen[src[y1 * sw + x1]];
	{
		const uint32_t wgt[4] = { ( 256 - ax ) * ( 256 - ay ), ax * ( 256 - ay ), ( 256 - ax ) * ay, ax * ay };

		for( int k = 0; k < 4; k++ )
		{
			r += ( c[k] >> 11 ) * wgt[k];
			g += ( c[k] >> 5 & 63 ) * wgt[k];
			b += ( c[k] & 31 ) * wgt[k];
		}
	}
	return ( r + 32768 ) >> 16 << 11 | ( g + 32768 ) >> 16 << 5 | ( b + 32768 ) >> 16;
}

/* RP_TexSample's source column (or row) for each of a level's `l` columns
   of a source `s` wide: the two texels blended and the second's weight,
   by RP_TexSample's own formulas. */
static void RP_TexAxis( int l, int s, int16_t *i0, int16_t *i1, uint8_t *a )
{
	for( int i = 0; i < l; i++ )
	{
		int32_t f = (int32_t)((( (int64_t)( 2 * i + 1 ) * s ) << 15 ) / l ) - 32768;
		int32_t k = f >> 16;

		a[i] = (uint8_t)( f >> 8 & 255 );
		k = ( k % s + s ) % s;
		i0[i] = (int16_t)k;
		i1[i] = (int16_t)(( k + 1 ) % s );
	}
}

/* RP_TexSample's blend of the four texels, their places from RP_TexAxis. */
static inline uint32_t RP_TexBlend( const pixel_t *src, int sw, int x0, int x1, int ax, int y0, int y1, int ay )
{
	const uint32_t c[4] = { vid.screen[src[y0 * sw + x0]], vid.screen[src[y0 * sw + x1]], vid.screen[src[y1 * sw + x0]], vid.screen[src[y1 * sw + x1]] };
	const uint32_t wgt[4] = { (uint32_t)(( 256 - ax ) * ( 256 - ay )), (uint32_t)( ax * ( 256 - ay )), (uint32_t)(( 256 - ax ) * ay ), (uint32_t)( ax * ay ) };
	uint32_t       r = 0, g = 0, b = 0;

	for( int k = 0; k < 4; k++ )
	{
		r += ( c[k] >> 11 ) * wgt[k];
		g += ( c[k] >> 5 & 63 ) * wgt[k];
		b += ( c[k] & 31 ) * wgt[k];
	}
	return ( r + 32768 ) >> 16 << 11 | ( g + 32768 ) >> 16 << 5 | ( b + 32768 ) >> 16;
}

/* The texture in the pool (written at its first use), or NULL: no room
   (the pool empties at the next frame) or a shape it doesn't take. */
#define RP_TEX0_CLAMP 0xA0000000u /* clamped both ways (RP_TEX0_PAGE's) */

static int  rp_borrowed; /* the menu's picture has two pages of this room (RP_BorrowPicture) */
static void RP_BorrowReturn( void );

static rp_tex_t *RP_TexPlace( const image_t *image, int sky )
{
	uint32_t  h = (uint32_t)(( image ? image->rp_serial : 0u ) * 2654435761u ) >> 23;
	rp_tex_t *t = NULL;
	pixel_t *const *src;
	uint16_t *chain = (uint16_t *)rp_scratch_buf.more;
	int       W, H, levels = 0, sb, plw = 1, plh = 1;
	uint32_t  bytes = 0, at, off = 0, prev = 0;

	if( rp_borrowed )
		RP_BorrowReturn( ); /* the world's textures asked for again: theirs back first */

	for( int i = 0; i < 16; i++ )
	{
		rp_tex_t *e = &rp_tex[( h + i ) & ( RP_TEX_SLOTS - 1 )];

		if( e->gen == rp_tex_gen && image && e->serial == image->rp_serial && e->sky == sky )
			return e;
		if( e->gen != rp_tex_gen )
		{
			t = e;
			break;
		}
	}
	if( !t || rp_tex_full || !image || !R_TexLoaded( image ) || image->width < 1 || image->height < 1 )
		return NULL;
	W = RP_Pow2( image->width );
	H = RP_Pow2( image->height );
	if( W < 8 )
		W = 8;
	if( H < 8 )
		H = 8;
	if( sky )
	{
		/* always magnified: no chain; a larger side sampled down to 256 */
		W = W > 256 ? 256 : W;
		H = H > 256 ? 256 : H;
	}
	/* the whole chain is built in the scratch first (a third more) */
	if( W > 1024 || H > 1024 || W * H > 65536 )
		return NULL;
	for( int lw = W, lh = H;; lw = lw > 1 ? lw >> 1 : 1, lh = lh > 1 ? lh >> 1 : 1 )
	{
		bytes += (uint32_t)( lw * lh * 2 );
		levels++;
		if(( lw == 1 && lh == 1 ) || sky )
			break;
	}
	at = ( rp_tex_used + 127u ) & ~127u;
	if( at + bytes > rp_tex_room )
	{
		rp_tex_full = 1;
		return NULL;
	}
	/* R335: its four levels, rebuilt from its 8 bits when it keeps those
	   (read here and let go: r_image.c R_TexLevels) */
	src = R_TexLevels( (image_t *)image, 1 );

	for( int l = 0, lw = W, lh = H; l < levels; l++ )
	{
		uint16_t *lv = chain + off / 2;
		/* A texel's place in the level (RP_TexAt) is its column's part plus
		   its row's part: the two sets of bits never meet. Each is worked
		   once a column and once a row here, not once a texel (the owner,
		   2026-11-22: "close all opened things": a texture placed in play
		   still cost its frame 27% to 42% in this function in run 317).
		   -rpplacecheck compares every place with RP_TexAt's own. */
		static uint32_t cx[1024], ry[1024];

		sb = 31 - __builtin_clz((uint32_t)( lw < lh ? lw : lh ));
		{
			const uint32_t m = ( 1u << sb ) - 1u;

			for( int x = 0; x < lw; x++ )
			{
				uint32_t in = (uint32_t)rp_spread[x & m & 0xFF] << 1;

				if( sb > 8 )
					in |= ( (uint32_t)rp_spread[( x & m ) >> 8] << 1 ) << 16;
				cx[x] = ( lw > lh ? ( (uint32_t)x >> sb ) << ( 2 * sb ) : 0u ) + in;
			}
			for( int y = 0; y < lh; y++ )
			{
				uint32_t in = rp_spread[y & m & 0xFF];

				if( sb > 8 )
					in |= (uint32_t)rp_spread[( y & m ) >> 8] << 16;
				ry[y] = ( lw > lh ? 0u : ( (uint32_t)y >> sb ) << ( 2 * sb )) + in;
			}
			{
				static int      check = -1;
				static uint32_t places, differ, said;

				if( check < 0 )
					check = gEngfuncs.Sys_CheckParm( "-rpplacecheck" ) != 0;
				if( check )
				{
					for( int y = 0; y < lh; y++ )
						for( int x = 0; x < lw; x++ )
						{
							places++;
							differ += cx[x] + ry[y] != RP_TexAt( x, y, lw, lh, sb );
						}
					if(( ++said & 255 ) == 1 )
						fprintf( stderr, "rp-emit: -rpplacecheck: %u texel places against RP_TexAt, %u differ\n", places, differ );
				}
			}
		}
		if( l < 4 && src[l] )
		{
			/* the map's own level (a quarter of the area each) */
			int sw = image->width >> l, sh = image->height >> l;

			if( sw < 1 )
				sw = 1;
			if( sh < 1 )
				sh = 1;
			if( sw == lw && sh == lh )
			{
				/* the same size: RP_TexSample's first line, a texel a lookup,
				   without its call (run 317's frames that placed a texture:
				   23% in RP_TexSample, a call of seven arguments a texel) */
				const pixel_t *row = src[l];

				for( int y = 0; y < lh; y++, row += sw )
					for( int x = 0; x < lw; x++ )
						lv[cx[x] + ry[y]] = (uint16_t)vid.screen[row[x]];
			}
			else
			{
				/* Run 316's samples, frame by frame: placing a texture whose
				   size is not a power of two took frames of 106 and 161 ms, up
				   to 45% of them in the division helpers: RP_TexSample works a
				   texel's source column and row with two 64-bit divisions and
				   four remainders, and the X51v's CPU has no divide. A texel's
				   column depends on x alone and its row on y alone, so each is
				   worked once (lw + lh times, not lw x lh) by the same formulas,
				   and a texel is four lookups and the blend: the same bits. */
				static int16_t tx0[1024], tx1[1024], ty0[1024], ty1[1024];
				static uint8_t tax[1024], tay[1024];

				RP_TexAxis( lw, sw, tx0, tx1, tax );
				RP_TexAxis( lh, sh, ty0, ty1, tay );
				for( int y = 0; y < lh; y++ )
					for( int x = 0; x < lw; x++ )
						lv[cx[x] + ry[y]] = (uint16_t)RP_TexBlend( src[l], sw, tx0[x], tx1[x], tax[x], ty0[y], ty1[y], tay[y] );
				/* -rpplacecheck (the emulator's check of the above): every texel
				   against RP_TexSample's own answer */
				{
					static int      check = -1;
					static uint32_t texels, differ, levels_checked;

					if( check < 0 )
						check = gEngfuncs.Sys_CheckParm( "-rpplacecheck" ) != 0;
					if( check )
					{
						for( int y = 0; y < lh; y++ )
							for( int x = 0; x < lw; x++ )
							{
								texels++;
								differ += lv[RP_TexAt( x, y, lw, lh, sb )] != (uint16_t)RP_TexSample( src[l], sw, sh, x, y, lw, lh );
							}
						if(( ++levels_checked & 15 ) == 1 )
							fprintf( stderr, "rp-emit: -rpplacecheck: %u resampled levels, %u texels against RP_TexSample, %u differ\n", levels_checked, texels, differ );
					}
				}
			}
		}
		else
		{
			/* past the stored four: each texel the mean of the larger level's two by two */
			const uint16_t *pv = chain + prev / 2;
			int             pw = plw, ph = plh;
			int             psb = 31 - __builtin_clz((uint32_t)( pw < ph ? pw : ph ));
			for( int y = 0; y < lh; y++ )
				for( int x = 0; x < lw; x++ )
				{
					int x0 = pw > lw ? 2 * x : x, y0 = ph > lh ? 2 * y : y;
					int x1 = pw > lw ? x0 + 1 : x0, y1 = ph > lh ? y0 + 1 : y0;

					lv[cx[x] + ry[y]] = (uint16_t)RP_565Avg4(
						pv[RP_TexAt( x0, y0, pw, ph, psb )], pv[RP_TexAt( x1, y0, pw, ph, psb )],
						pv[RP_TexAt( x0, y1, pw, ph, psb )], pv[RP_TexAt( x1, y1, pw, ph, psb )] );
				}
		}
		prev = off;
		off += (uint32_t)( lw * lh * 2 );
		plw = lw;
		plh = lh;
		lw = lw > 1 ? lw >> 1 : 1;
		lh = lh > 1 ? lh >> 1 : 1;
	}
	/* one copy of the whole chain (whole words: the last texel padded) */
	if( RP_ChipWrite( ))
		memcpy((uint8_t *)rp_g.tex + (size_t)rp_pages * RP_PAGE_BYTES + at, chain, ( bytes + 3u ) & ~3u );
	rp_tex_used = at + (( bytes + 3u ) & ~3u );
	t->serial = image->rp_serial;
	t->gen = rp_tex_gen;
	t->sky = sky;
	/* wrap (repeat 0) both ways, the sizes, RGB565, the chain's last level */
	t->tex0 = (uint32_t)( 31 - __builtin_clz((uint32_t)W ) - 3 ) << 24 | (uint32_t)( 31 - __builtin_clz((uint32_t)H ) - 3 ) << 20
		| 2u << 13 | (uint32_t)( levels - 1 ) << 9;
	at += rp_g.tex_at + (uint32_t)rp_pages * RP_PAGE_BYTES;
	t->tex1 = levels > 1 ? RP_TEX1_MIP( at ) : RP_TEX1_LINEAR( at );
	rp_count.tex_built++;
	rp_count.tex_kib += bytes >> 10;
	return t;
}

/* The face's lightmap in the atlas, written when it is new or its light
   changed (D_CacheSurface's tests); its page and place, or -1. */
static int RP_LmPlace( msurface_t *fa, int *x, int *y )
{
	uint32_t loc = (uint32_t)RP_LOC( fa );
	int      pg = loc >> 16 & 0xFF, placed, stale, w, h, r;

	placed = ( loc & RP_VALID ) && ( loc & RP_LMAP ) && pg < rp_pages && (int)( RP_GEN( fa ) >> 1 ) == rp_page[pg].gen;
	stale = !placed || ( RP_GEN( fa ) & 1 ) || fa->dlightframe == tr.framecount;
	for( int maps = 0; !stale && maps < MAXLIGHTMAPS && fa->styles[maps] != 255; maps++ )
		if( g_lightstylevalue[fa->styles[maps]] != fa->cached_light[maps] )
			stale = 1;
	if( placed )
	{
		*x = loc & 0xFF;
		*y = loc >> 8 & 0xFF;
		rp_page[pg].used = tr.framecount;
		rp_page[pg].live += 128;
		if( !stale )
			return pg;
	}
	if( fa->dlightframe == tr.framecount )
	{
		/* R305: the light in the face's own space (RP_SurfacePlace's) */
		if( RI.currentmodel == WORLDMODEL || !RI.currententity )
			tr.modelviewIdentity = true;
		else
		{
			Matrix4x4_CreateFromEntity( RI.objectMatrix, RI.currententity->angles, RI.currententity->origin, 1 );
			tr.modelviewIdentity = false;
		}
	}
	r = RP_SurfLightmap( fa, rp_lm_texels, RP_LM_TEXELS, &w, &h );
	if( !r )
		return -1;
	if( !placed )
	{
		pg = RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, x, y );
		if( pg < 0 )
			return -1;
		rp_page[pg].used = tr.framecount;
		rp_page[pg].live += 128;
	}
	rp_upload_raw = 1;
	rp_upload_stage = placed && !rp_nostage; /* R352: a rewrite in place held to the frame's end */
	RP_Upload((const pixel_t *)rp_lm_texels, w, h, pg, *x, *y );
	rp_upload_stage = 0;
	rp_upload_raw = 0;
	RP_LOC( fa ) = (intptr_t)( RP_VALID | RP_LMAP | (uint32_t)pg << 16 | (uint32_t)*y << 8 | (uint32_t)*x );
	RP_GEN( fa ) = (intptr_t)( rp_page[pg].gen << 1 | ( r & 2 ? 1 : 0 ));
	rp_count.lm_built++;
	if( r & 2 )
		rp_count.lm_dlit++;
	return pg;
}

/*
A world face (corners as RP_ChipFaceFixed made them) by option B: the
texture's pass into the frame, the lightmap's into the second pass's
buffer. 0: not this face (it goes the lit surface's way).
*/
/* kit 112: one pass unless -rplm2pass, read once; the state the two-unit blocks leave (any
   one-unit draw after them sends its own block again) */
static rp_tex_t rp_lm1_mark;
/* Unit 1's MODULATE for 16-bit colour textures, read from Dell's combine
   table (libGLES_CL.dll 0x0225D3A0 + 100, kind 1, mode 0): unit 0's
   0x86086610 with bits 27 and 11; run 224 without bit 27 drew unit 1's
   texture alone (the previous unit's colour not taken) */
#define RP_TEX2_UNIT1 0x8E086E10u

static __attribute__(( noinline )) int RP_Lm1PassRead( void )
{
	/* kit 112: one pass by default (runs 222-225); -rplm2pass the control */
	return gEngfuncs.Sys_CheckParm( "-rplm2pass" ) ? 0 : 1;
}

static inline int RP_Lm1Pass( void )
{
	static int on = -1;

	if( on < 0 )
		on = RP_Lm1PassRead( );
	return on;
}

/* kit 112's one-pass face (out of the SRAM's hot code while it is tried) */
static __attribute__(( noinline )) int RP_ChipFaceLm1( int n, const uint32_t *zis, const int32_t *const *cxy, const int32_t *const *st,
	const rp_tex_t *t, int pg, int qx, int qy, int32_t ks, int32_t kt, int64_t iw, int64_t ih )
{
	int lo, hi;

	/*
	Kit 112 (the default, `-rplm2pass` the control; the owner: "Yes, go
	after it"): the face in
	ONE pass, its texture on unit 0 times its lightmap on the 2700G's
	second texture unit, as Dell's GLES driver builds a two-unit draw
	(the state block's mask bit 3, TSP bit 16 and 2 coordinate sets in
	bits 19-17, unit 1's MODULATE from his combine table). Run 221: the
	second pass is the cascade's limit (35.0 / 30.5 fps without it).
	Test M hung the chip this way under our own register table; the
	runner runs Dell's full table and it draws (run 222, no hang). Run
	225: 101 of 307,200 pixels from the two passes' picture, cascade
	31.6 / 29.7 fps against 29.8 / 28.8.
	*/
	static const rp_tex_t *last_t;
	static int             last_pg = -1;

	if( rp_state != &rp_lm1_mark || last_t != t || last_pg != pg )
	{
		rp_state = &rp_lm1_mark;
		last_t = t;
		last_pg = pg;
		rp_count.states++;
		RP_Put( RP_GFX_STATE2 );
		RP_Put(( RP_TSP_WORLD & ~0x000F0000u ) | 1u << 16 | 2u << 17 );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put( t->tex0 );
		RP_Put( t->tex1 );
		RP_Put( RP_TEX2 );
		RP_Put( RP_TEX0_PAGE );
		RP_Put( RP_TEX1_LINEAR( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ));
		RP_Put( RP_TEX2_UNIT1 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Whole();
	}
	RP_Put( RP_GFX_STRIP( n ) | rp_noz );
	lo = 1;
	hi = n - 1;
	for( int k = 0; k < n; k++ )
	{
		int      c = k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--;
		uint32_t fx = RP_FixBits( cxy[c][0], 4 ), fy = RP_FixBits( cxy[c][1], 4 );
		int32_t  u = (int32_t)(( (int64_t)( st[c][0] + ks ) * iw ) >> 16 );
		int32_t  v = (int32_t)(( (int64_t)( st[c][1] + kt ) * ih ) >> 16 );
		int32_t  lu = ( qx << 11 ) + ( st[c][0] >> 4 ) + 128;
		int32_t  lv = ( qy << 11 ) + ( st[c][1] >> 4 ) + 128;

		RP_Put( fx );
		RP_Put( fy );
		if( !rp_noz ) RP_Put( zis[c] );
		RP_Put( zis[c] );
		RP_Put( 0xFFFFFFFFu );
		RP_Put( RP_FixBits( u, 16 ));
		RP_Put( RP_FixBits( v, 16 ));
		RP_Put( RP_FixBits( lu, 8 + 9 ));
		RP_Put( RP_FixBits( lv, 8 + 9 ));
	}
	RP_Whole();
	rp_count.faces++;
	rp_count.fixed++;
	rp_count.lm_faces++;
	rp_count.tris += n - 2;
	rp_count.verts += n;
	return 1;
}

static int RP_ChipFaceLmTry( msurface_t *fa, int n, const uint32_t *zis, const int32_t *const *cxy, const int32_t *const *st );
/* (out of line: RP_ChipFaceFixed is hot code kept in the on-chip SRAM, and
   the decals' code inlined into it took the hot set past the SRAM's size) */
static void RP_FaceDecals( msurface_t *fa, const int32_t m[3][3], const int32_t o[3] ) __attribute__(( noinline ));

/*
R352 (the owner, run 293: a patch on a ledge flashing bright and back):
a face drawn by option B one frame (texture, then its lightmap in a
second pass) and by its lit surface the next (choice 2 A, ref_soft's own
lighting) looks different each time it changes way. Each face's way and
frame are kept (16 bits a face: the frame's low 15, the way), and a
change between two frames in a row counted.
*/
#define RP_FACES_KEPT 32768
static uint16_t rp_face_way[RP_FACES_KEPT];

static int RP_ChipFaceLm( msurface_t *fa, int n, const uint32_t *zis, const int32_t *const *cxy, const int32_t *const *st )
{
	int        b = RP_ChipFaceLmTry( fa, n, zis, cxy, st );
	uintptr_t  fi = (uintptr_t)( fa - WORLDMODEL->surfaces );

	if( fi < RP_FACES_KEPT )
	{
		uint16_t was = rp_face_way[fi], now = (uint16_t)(( rp_view.serial & 0x7FFF ) << 1 | ( b != 0 ));

		if(( was >> 1 ) == (( rp_view.serial - 1 ) & 0x7FFF ) && ( was & 1 ) != ( now & 1 ))
			rp_count.path_flips++;
		rp_face_way[fi] = now;
	}
	return b;
}

static int RP_ChipFaceLmTry( msurface_t *fa, int n, const uint32_t *zis, const int32_t *const *cxy, const int32_t *const *st )
{
	texture_t *tt;
	rp_tex_t  *t;
	int        pg, qx, qy, lo, hi, tw, th;
	int32_t    ks, kt;
	int64_t    iw, ih;

	uintptr_t  fi = (uintptr_t)( fa - WORLDMODEL->surfaces );

	/* R352: a face with decals stays on this path (RP_FaceDecals draws
	   them over it); -rpdecalsoft sends it the lit surface's way as
	   before, counted */
	if( rp_face_blend || rp_face_masked )
		return 0;
	if( fa->pdecals && !rp_decal_on )
	{
		rp_count.why_decal++;
		return 0;
	}
	/* kit 103: the face's fixed facts from the cooked map (one byte), not
	   three structures and a call into the engine */
	if( rp_ck_live && fi < (uintptr_t)rp_ck.nfaces )
	{
		if( !( rp_ck.faces[fi] & RP_CK_LMOK ))
			return 0;
	}
	else if(( fa->texinfo->flags & TEX_WORLD_LUXELS ) || gEngfuncs.Mod_SampleSizeForFace( fa ) != 16
		|| fa->info->lightmapmins[0] != fa->texturemins[0] || fa->info->lightmapmins[1] != fa->texturemins[1] )
		return 0;
	if( rp_lm_n + RP_STATE_WORDS + 1 + (uint32_t)n * RP_VERTEX_WORDS > RP_LM_WORDS )
	{
		rp_count.lm_full++;
		return 0;
	}
	tt = R_TextureAnimation( fa );
	if( !tt || tt->width < 1 || tt->height < 1 )
		return 0;
	t = RP_TexPlace( R_GetTexture( tt->gl_texturenum ), 0 );
	if( !t )
	{
		rp_count.why_tex++;
		return 0;
	}
	pg = RP_LmPlace( fa, &qx, &qy );
	if( pg < 0 )
	{
		rp_count.why_lm++;
		return 0;
	}

	/* the texture's repeats: s from the face's corner, moved by whole
	   texture widths to start inside the first (the floats stay small) */
	tw = tt->width;
	th = tt->height;
	ks = ( fa->texturemins[0] % tw + tw ) % tw * 256;
	kt = ( fa->texturemins[1] % th + th ) % th * 256;
	if(( fa->flags & SURF_CONVEYOR ) && RI.currententity )
	{
		/* R309: ref_gl's scroll (DrawGLPoly): the entity's speed in 16ths
		   of a texel a second (rendercolor g, b; r turns it), s moved back
		   by speed x time, here in 24.8 texels within one repeat */
		const cl_entity_t *e = RI.currententity;
		int64_t            sp = e->curstate.rendercolor.g << 8 | e->curstate.rendercolor.b;

		if( e->curstate.rendercolor.r )
			sp = -sp;
		ks = (int32_t)((( ks - rp_time_ms * sp * 16 / 1000 ) % ( tw * 256 ) + tw * 256 ) % ( tw * 256 ));
		rp_count.conveyors++;
	}
	iw = ( (int64_t)1 << 24 ) / tw; /* (24.8 texels x 2^24 / w) >> 16: the repeat as 16.16 */
	ih = ( (int64_t)1 << 24 ) / th;

	if( RP_Lm1Pass( ))
		return RP_ChipFaceLm1( n, zis, cxy, st, t, pg, qx, qy, ks, kt, iw, ih );
	if( rp_state != t )
	{
		rp_state = t;
		rp_count.states++;
		RP_Put( RP_GFX_STATE );
		RP_Put( RP_TSP_WORLD );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put( t->tex0 );
		RP_Put( t->tex1 );
		RP_Put( RP_TEX2 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Whole();
	}
	if( rp_lm_page != pg )
	{
		rp_lm_page = pg;
		RP_LmPut( RP_GFX_STATE );
		RP_LmPut( RP_TSP_LMAP );
		RP_LmPut( RP_ISP_ALWAYS );
		RP_LmPut( RP_TEX0_PAGE );
		RP_LmPut( RP_TEX1_LINEAR( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ));
		RP_LmPut( RP_TEX2 );
		RP_LmPut( RP_CLIP );
		RP_LmPut( 0 );
	}

	RP_Put( RP_GFX_STRIP( n ) | rp_noz);
	RP_LmPut( RP_GFX_STRIP( n ) | rp_noz);
	lo = 1;
	hi = n - 1;
	for( int k = 0; k < n; k++ )
	{
		int c = k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--;
		/* the corners as they are, not moved out for the cracks as
		   RP_ChipFaceFixed's: two faces overlapping at a seam would each
		   multiply its pixels in the second pass (the light twice: a dark
		   seam); a T-junction's crack shows as GL's (ref_gl's sparkles,
		   counted by rp_holes) */
		uint32_t fx = RP_FixBits( cxy[c][0], 4 ), fy = RP_FixBits( cxy[c][1], 4 );
		/* the texture in repeats (16.16); the lightmap a sample a 16
		   texels, at the samples' centres, in page texels (24.8) */
		int32_t  u = (int32_t)(( (int64_t)( st[c][0] + ks ) * iw ) >> 16 );
		int32_t  v = (int32_t)(( (int64_t)( st[c][1] + kt ) * ih ) >> 16 );
		int32_t  lu = ( qx << 11 ) + ( st[c][0] >> 4 ) + 128;
		int32_t  lv = ( qy << 11 ) + ( st[c][1] >> 4 ) + 128;

		RP_Put( fx );
		RP_Put( fy );
		if( !rp_noz ) RP_Put( zis[c] );
		RP_Put( zis[c] );
		RP_Put( 0xFFFFFFFFu );
		RP_Put( RP_FixBits( u, 16 ));
		RP_Put( RP_FixBits( v, 16 ));
		RP_LmPut( fx );
		RP_LmPut( fy );
		if( !rp_noz ) RP_LmPut( zis[c] );
		RP_LmPut( zis[c] );
		RP_LmPut( 0xFFFFFFFFu );
		RP_LmPut( RP_FixBits( lu, 8 + 9 ));
		RP_LmPut( RP_FixBits( lv, 8 + 9 ));
	}
	RP_Whole();
	rp_count.faces++;
	rp_count.fixed++;
	rp_count.lm_faces++;
	rp_count.tris += 2 * ( n - 2 );
	rp_count.verts += 2 * n;
	return 1;
}

/*
The world's tables, made when a map is first drawn: each vertex as 16.16;
each face corner's s, t (mip 0 texels from the surface's corner, the same
sums RP_ChipFace's float path makes) as 24.8, by surfedge; each vertex's
words a frame (`frame` this frame's: `zi` 0 when nearer than a unit).
*/
/*
The corner stream (FPS work after run 47: RP_ChipFaceFixed 6.4% of the
play, most of it waiting on memory: a corner's surfedge, its edge, its
vertex's words and its s, t, four loads from four tables, each a cache
miss the next waits on). The software renderer and ref_gl walk the same
chain (surfedge, edge, vertex) for every face every frame. Ours: a face's
corners as one run of 12-byte records (vertex number and s, t) in the
face's own order, read straight through; a vertex's position and its words
this frame in one 32-byte record, one cache line (the PXA270's), aligned.
*/
typedef struct
{
	int      frame;
	uint32_t zi;    /* 1/z's float bits this frame, 0 when nearer than a unit */
	int32_t  xy[2]; /* the TA's x, y as 28.4: the cracks' cure reads them */
	int32_t  pos[3]; /* 16.16 */
	int32_t  pad;
} rp_wvert_t;

typedef struct
{
	int32_t st[2]; /* the corner's s, t as 24.8 (mip 0 texels from the surface's corner) */
	int32_t vi;    /* its vertex */
} rp_wcorner_t;

static const model_t *rp_wmodel;
static rp_wcorner_t *rp_wc;
static rp_wvert_t   *rp_wv;
static void         *rp_wv_block; /* rp_wv's allocation (rp_wv on a cache line in it) */

/* once a map, out of line: RP_WorldTables is on the SRAM (ports/xash/sram-hot.txt) */
static int  RP_CookRead( const model_t *mod ) __attribute__(( noinline ));
static void RP_CookMake( const model_t *mod ) __attribute__(( noinline ));
static void RP_CookFree( void );

/*
R352 after run 333 (the owner's clip IMG_0666: "all of the top beams on
the top of the building flash in and out"). c2a5f's steel ribs are a
func_wall (model *140, `out_w4b`) whose front faces stand in plane 2362
(y = -376 facing +y) from z -1568 to -1120; below z -1240 the world's wall
is 8 units behind them (y = -384), but in the top band (z -1240 to -1120)
the world's own wall (`c3a1_w5b`, faces 3277, 3278, 4255, 4256) is in the
same plane and side. Two faces at one depth with different corners: the
2700G's depth test settles each pixel by the rounding of the two faces'
interpolated 1/z, and that changes as the view moves, so the ribs in the
band appear and vanish frame to frame (the clip: frames 0-10 ribs up to
the top, 12-23 none, the view all but still; run 333's chip picture of
the same view with none and thin dark seams at the ribs' edges). The map
has 666 such faces (of 3,477 brush entity faces, 147 entities).

Iterate, don't imitate (rule 0). GoldSrc's and Quake's GL renderers draw
the world, then the entities, with depth "less or equal": the same fight,
a known look of those renderers. ref_soft's span sorter settles a tie by
the surfaces' keys, the world's first, so the software picture (the look
Half-Life ships, and ours elsewhere) shows the world's wall in the band,
every frame. Ours: the ties found once, at the map's first draw, from the
map's own planes (a brush entity's face whose plane and side a world face
uses: the compiler gives coplanar faces one plane), a bit a face; such a
face, when its entity is unmoved and unturned, is drawn 2^-9 of its depth
behind itself (its 1/z words down by the decals' step, the other way), so
the world wins every pixel of a tie, every frame, as ref_soft draws it,
for one compare a brush face and nothing on the world's faces. 2^-9 of the
depth is under a unit at 500 units (a rib stands 8 proud), and the chip
keeps the decals' same step apart without speckle (runs 326-328).
-rpcoplanar keeps the old way for a side by side.
*/
#define RP_COPLANAR_BIAS 0x4000u /* float bits: 2^14 of the mantissa's 2^23 steps, as RP_DECAL_BIAS */
#define RP_COPLANAR( fa ) ( rp_cop && ( rp_cop[( fa ) - rp_wmodel->surfaces >> 3] >> (( ( fa ) - rp_wmodel->surfaces ) & 7 ) & 1 ))

static void RP_CoplanarMark( const model_t *mod )
{
	uint8_t *seen;
	int      world;

	free( rp_cop );
	rp_cop = NULL;
	rp_cop_marked = 0;
	world = mod->nummodelsurfaces;
	if( !mod->numplanes || world <= 0 || world >= mod->numsurfaces )
		return;
	seen = calloc( mod->numplanes, 1 );
	rp_cop = calloc(( mod->numsurfaces + 7 ) >> 3, 1 );
	if( !seen || !rp_cop )
	{
		free( seen );
		free( rp_cop );
		rp_cop = NULL;
		return;
	}
	/* each plane's sides the world's faces use: bit 0 the front, bit 1 the back */
	for( int f = 0; f < world; f++ )
	{
		const msurface_t *fa = &mod->surfaces[f];
		ptrdiff_t         pl = fa->plane - mod->planes;

		if( fa->plane && pl >= 0 && pl < mod->numplanes )
			seen[pl] |= ( fa->flags & SURF_PLANEBACK ) ? 2 : 1;
	}
	for( int f = world; f < mod->numsurfaces; f++ )
	{
		const msurface_t *fa = &mod->surfaces[f];
		ptrdiff_t         pl = fa->plane - mod->planes;

		if( fa->plane && pl >= 0 && pl < mod->numplanes
			&& ( seen[pl] & (( fa->flags & SURF_PLANEBACK ) ? 2 : 1 )))
		{
			rp_cop[f >> 3] |= (uint8_t)( 1u << ( f & 7 ));
			rp_cop_marked++;
		}
	}
	free( seen );
	fprintf( stderr, "rp-emit: R352: %u brush entity faces share a world face's plane and side (drawn just behind it%s)\n",
		rp_cop_marked, rp_coplanar_off ? "; -rpcoplanar: not" : "" );
}

static int RP_WorldTables( void )
{
	const model_t *mod = WORLDMODEL;
	uint64_t       t0;

	if( mod == rp_wmodel )
		return rp_wc != NULL;
	rp_wmodel = mod;
	rp_map_frames = 0;
	RP_TexReset(); /* a new map's textures (image slots are used again) */
	RP_CookFree();
	free( rp_wc );
	free( rp_wv_block );
	rp_wc = NULL;
	rp_wv = NULL;
	rp_wv_block = NULL;
	if( !mod || !mod->numvertexes || !mod->numsurfedges )
		return 0;
	RP_CoplanarMark( mod );
	rp_wv_block = calloc( 1, sizeof( *rp_wv ) * mod->numvertexes + 31 );
	rp_wc = calloc( mod->numsurfedges, sizeof( *rp_wc ));
	if( !rp_wv_block || !rp_wc )
	{
		free( rp_wv_block );
		free( rp_wc );
		rp_wv_block = NULL;
		rp_wc = NULL;
		fprintf( stderr, "rp-emit: no room for the world's tables: the float path\n" );
		return 0;
	}
	rp_wv = (rp_wvert_t *)(((uintptr_t)rp_wv_block + 31 ) & ~(uintptr_t)31 );
	t0 = rp_time_us( );
	if( rp_cook_on && RP_CookRead( mod ))
	{
		fprintf( stderr, "rp-emit: the cooked map read and checked in %u us (kit 103)\n", (uint32_t)( rp_time_us( ) - t0 ));
		return 1;
	}
	for( int i = 0; i < mod->numvertexes; i++ )
	{
		rp_wv[i].frame = -1;
		for( int j = 0; j < 3; j++ )
			rp_wv[i].pos[j] = RP_Fix16( mod->vertexes[i].position[j] );
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

			rp_wc[fa->firstedge + i].st[0] = (int32_t)(( DotProduct( world, sv ) + sv[3] - smin ) * 256.0f );
			rp_wc[fa->firstedge + i].st[1] = (int32_t)(( DotProduct( world, tv ) + tv[3] - tmin ) * 256.0f );
			rp_wc[fa->firstedge + i].vi = lindex > 0 ? e->v[0] : e->v[1];
		}
	}
	fprintf( stderr, "rp-emit: the world's tables: %d vertices, %d face corners (%u KiB)\n",
		mod->numvertexes, mod->numsurfedges,
		(uint32_t)( sizeof( *rp_wv ) * mod->numvertexes + sizeof( *rp_wc ) * mod->numsurfedges ) >> 10 );
	if( rp_cook_on )
		RP_CookMake( mod );
	return 1;
}

/*
==============================================================================
ADR-0040 step 7c (kit 103): the map cooked once on the X51v
==============================================================================

Iterate, don't imitate (rule 0). GoldSrc and Xash load the BSP as the file
stores it and, every frame, walk its float nodes (Xash's 52-byte mnode_t,
floats turned into integers again a node a frame here since kit 52's
RP_WalkPlane), then read three structures a face (msurface_t, its texinfo,
its extra data) to decide what the face needs. The PS2's Half-Life and the
fixed-point Quake ports converted maps on a PC into the console's own
format; modern engines cook in a studio's build. Ours cooks on the X51v at
the map's first load, from the user's own copy (any map, any mod, nothing
to prepare on a PC), and keeps it on the card beside the game:

- the planes as integers (the walk's own rp_fixq values, once), the nodes
  in preorder as one 32-byte record each (a PXA270 cache line: the box as
  whole units, the plane, the faces, the children), the leaves and their
  faces as indexes, and the world's vertex and corner tables;
- each face's fixed facts as one byte (its side, whether its texture and
  lightmap line up for R297 B), so the frame does not ask the engine;
- checked against the BSP's MD5 (taken from the buffer the engine already
  hands the renderer: no second read) and its counts, and remade when
  either differs. The card is untrusted (ADR-0039 rule 10): every index is
  bounds-checked, a child must come after its parent (no cycle can be
  written), and a walk that would visit more records than the map has
  stops and goes back to the engine's walk.

At each frame the walk (r_bsp.c, RP_CookWalk) runs over a compact copy of
only the nodes and leaves in the current PVS, made again only when the
view enters another cluster (tr.visframecount): the same tests in the same
order with the same integers, so the same faces go out in the same order,
word for word. `-rpnocook` is the control (kit 102's way, nothing read or
written).
*/
#define RP_COOK_MAGIC   0x31575052u /* "RPW1" */
#define RP_COOK_VERSION 3           /* a change in what is cooked: a new number */

typedef struct
{
	uint32_t magic, version, head_bytes, bsp_len;
	uint8_t  md5[16];
	uint32_t nverts, ncorners, nfaces, nplanes, nnodes, nleafs, nmarks, total;
	uint32_t reserved[4];
} rp_cook_head_t;

int                rp_cook_on = 1;
int                rp_facecull = 1; /* kit 104, the owner 2026-11-18 ("Yes, turn it on", R331); -rpnofacecull the control */
static uint8_t     rp_bsp_md5[16];
static uint32_t    rp_bsp_len;
static char        rp_bsp_name[64]; /* whose buffer was hashed */
static void       *rp_ck_block;
static uint32_t    rp_ck_serial;

void RP_EmitWorldBuffer( const model_t *mod, const byte *buf, size_t len )
{
	MD5Context_t c;
	uint64_t     t0 = rp_time_us( );

	MD5Init( &c );
	MD5Update( &c, buf, (uint)len );
	MD5Final( rp_bsp_md5, &c );
	rp_bsp_len = (uint32_t)len;
	Q_strncpy( rp_bsp_name, mod->name, sizeof( rp_bsp_name ));
	fprintf( stderr, "rp-emit: %s's MD5 for its cooked file in %u us (%u KiB)\n", mod->name,
		(uint32_t)( rp_time_us( ) - t0 ), (uint32_t)( len >> 10 ));
}

/*
Kit 109: the engine's point traces (pm_trace.c RP_TraceSide) read the cooked
planes for the world's hull 0, which is the BSP's nodes by number: a table
engine node -> cooked plane (0xFFFF for nodes not in the world's tree, the
brush entities'), 2 bytes a node, made on each cook or read.
*/
extern const void     *rp_trace_clip;
extern const uint16_t *rp_trace_np;
extern const void     *rp_trace_pl;
extern int             rp_trace_nnodes;
extern void            RP_TraceForget( void );
static uint16_t       *rp_trace_table;

static void RP_TraceTable( const model_t *mod )
{
	RP_TraceForget( );
	free( rp_trace_table );
	rp_trace_table = NULL;
	if( !rp_ck_live || mod->numnodes <= 0 || ( !mod->hulls[0].clipnodes16 && !mod->hulls[0].clipnodes32 ))
		return;
	rp_trace_table = (uint16_t *)malloc( sizeof( uint16_t ) * (size_t)mod->numnodes );
	if( !rp_trace_table )
		return; /* the floats, as before */
	memset( rp_trace_table, 0xFF, sizeof( uint16_t ) * (size_t)mod->numnodes );
	for( int i = 0; i < rp_ck.nnodes; i++ )
		if( rp_ck.nodes[i].orig < (uint32_t)mod->numnodes && rp_ck.nodes[i].plane < (uint32_t)rp_ck.nplanes )
			rp_trace_table[rp_ck.nodes[i].orig] = rp_ck.nodes[i].plane;
	rp_trace_np = rp_trace_table;
	rp_trace_pl = rp_ck.planes;
	rp_trace_nnodes = mod->numnodes;
	rp_trace_clip = mod->hulls[0].clipnodes16 ? (const void *)mod->hulls[0].clipnodes16 : (const void *)mod->hulls[0].clipnodes32;
}

static void RP_CookFree( void )
{
	RP_TraceForget( ); /* the engine's traces stop reading the planes first */
	free( rp_trace_table );
	rp_trace_table = NULL;
	free( rp_ck_block );
	rp_ck_block = NULL;
	rp_ck_live = 0;
	memset( &rp_ck, 0, sizeof( rp_ck ));
}

const rp_cooked_t *RP_Cooked( void )
{
	if( !rp_cook_on || rp_emit_on != 2 || !rp_fixed_on || !RP_WorldTables( ) || !rp_ck_live )
		return NULL;
	return &rp_ck;
}

/* "maps/c1a0e.bsp" to "maps/c1a0e.rpw": 0 if the hash is not this map's */
static int RP_CookPath( const model_t *mod, char *path, size_t size )
{
	size_t n = Q_strlen( mod->name );

	if( !rp_bsp_len || Q_strcmp( rp_bsp_name, mod->name ) || n < 5 || n + 1 > size )
		return 0;
	Q_strncpy( path, mod->name, size );
	Q_strncpy( path + n - 4, ".rpw", size - ( n - 4 ));
	return 1;
}

/* the cooked arrays' sizes, in the file's order */
typedef struct
{
	uint32_t verts, corners, planes, nodes, leafs, marks, faces, fbox, total;
} rp_cook_sizes_t;

static int RP_CookSizes( const rp_cook_head_t *h, rp_cook_sizes_t *s )
{
	uint64_t t;

	/* each count below 2^24 (a BSP's own limits are far under it), so no
	   product or sum here can overflow 64 bits; faces below 2^16 (a leaf's
	   faces are 16-bit numbers) */
	if( h->nverts >= 1u << 24 || h->ncorners >= 1u << 24 || h->nfaces > 0xFFFFu || h->nplanes >= 1u << 24
		|| h->nnodes >= 1u << 24 || h->nleafs >= 1u << 24 || h->nmarks >= 1u << 24 )
		return 0;
	s->verts = h->nverts * 12u;
	s->corners = h->ncorners * (uint32_t)sizeof( rp_wcorner_t );
	s->planes = h->nplanes * (uint32_t)sizeof( rp_cplane_t );
	s->nodes = h->nnodes * (uint32_t)sizeof( rp_cnode_t );
	s->leafs = h->nleafs * (uint32_t)sizeof( rp_cleaf_t );
	s->marks = ( h->nmarks * 2u + 3u ) & ~3u;
	s->faces = ( h->nfaces + 3u ) & ~3u;
	s->fbox = h->nfaces * 12u;
	t = (uint64_t)sizeof( rp_cook_head_t ) + s->verts + s->corners + s->planes + s->nodes + s->leafs + s->marks + s->faces + s->fbox;
	if( t >= 1u << 30 )
		return 0;
	s->total = (uint32_t)t;
	return 1;
}

/* the cooked arrays in one block, the face boxes only for -rpfacecull
   (run 153: the fight's map ran out of memory with every array kept) */
static int RP_CookAlloc( const rp_cook_sizes_t *s, const rp_cook_head_t *h )
{
	uint8_t *p;
	uint32_t fbox = rp_facecull ? s->fbox : 0;

	rp_ck_block = malloc( s->planes + s->nodes + s->leafs + s->marks + s->faces + fbox + 4 );
	if( !rp_ck_block )
		return 0;
	p = rp_ck_block;
	rp_ck.planes = (const rp_cplane_t *)p;
	p += s->planes;
	rp_ck.nodes = (const rp_cnode_t *)p;
	p += s->nodes;
	rp_ck.leafs = (const rp_cleaf_t *)p;
	p += s->leafs;
	rp_ck.marks = (const uint16_t *)p;
	p += s->marks;
	rp_ck.faces = p;
	p += s->faces;
	rp_ck.fbox = fbox ? (const int16_t ( * )[6] )p : NULL;
	rp_ck.nplanes = (int)h->nplanes;
	rp_ck.nnodes = (int)h->nnodes;
	rp_ck.nleafs = (int)h->nleafs;
	rp_ck.nmarks = (int)h->nmarks;
	rp_ck.nfaces = (int)h->nfaces;
	return 1;
}

static void RP_CookHead( const model_t *mod, rp_cook_head_t *h )
{
	memset( h, 0, sizeof( *h ));
	h->magic = RP_COOK_MAGIC;
	h->version = RP_COOK_VERSION;
	h->head_bytes = sizeof( *h );
	h->bsp_len = rp_bsp_len;
	memcpy( h->md5, rp_bsp_md5, sizeof( h->md5 ));
	h->nverts = (uint32_t)mod->numvertexes;
	h->ncorners = (uint32_t)mod->numsurfedges;
	h->nfaces = (uint32_t)mod->numsurfaces;
	h->nleafs = (uint32_t)mod->numleafs + 1; /* leaf 0 (solid) and the visible ones */
	h->nmarks = (uint32_t)mod->nummarksurfaces;
	/* nplanes and nnodes: the ones the world's own tree uses (RP_CookMake) */
}

/* a box (minmaxs) as whole units: exactly rp_fixq's 16.16 >> 16, or 0 */
static int RP_CookBox( const float *mm, int16_t box[6] )
{
	for( int i = 0; i < 6; i++ )
	{
		int32_t v = rp_fixq( mm[i], 16 );

		if( v & 0xFFFF )
			return 0; /* not a whole unit (never in a compiled map): the engine's walk */
		box[i] = (int16_t)( v >> 16 );
		if( ((int32_t)box[i] << 16 ) != v )
			return 0;
	}
	return 1;
}

/* RP_CookNode's work: the nodes in preorder, the planes they use by their
   new numbers (the engine's plane number to ours, -1 not yet) */
static struct
{
	rp_cnode_t  *nodes;
	rp_cplane_t *planes;
	int32_t     *remap;
	int          nn, np;
} rp_cw;

/* the node and everything under it, in preorder: its place, or -1 */
static int32_t RP_CookNode( const model_t *mod, const mnode_t *node, int depth )
{
	rp_cnode_t *out;
	int32_t     k, c, pi;

	if( depth > 1024 || rp_cw.nn >= mod->numnodes )
		return -1;
	if( node < mod->nodes || node >= mod->nodes + mod->numnodes || !node->plane
		|| node->plane < mod->planes || node->plane >= mod->planes + mod->numplanes )
		return -1;
	if( node_numsurfaces( node, mod ) > 0xFFFF
		|| node_firstsurface( node, mod ) + node_numsurfaces( node, mod ) > mod->numsurfaces )
		return -1;
	k = rp_cw.nn++;
	out = &rp_cw.nodes[k];
	if( !RP_CookBox( node->minmaxs, out->box ))
		return -1;
	pi = (int32_t)( node->plane - mod->planes );
	if( rp_cw.remap[pi] < 0 )
	{
		/* the walk's own integers for the plane (rp_fixq), once */
		rp_cplane_t *p = &rp_cw.planes[rp_cw.np];

		for( int j = 0; j < 3; j++ )
			p->n[j] = rp_fixq( node->plane->normal[j], 30 );
		p->d = rp_fixq( node->plane->dist, 16 );
		p->type = node->plane->type;
		rp_cw.remap[pi] = rp_cw.np++;
	}
	if( rp_cw.remap[pi] > 0xFFFF )
		return -1;
	out->plane = (uint16_t)rp_cw.remap[pi];
	out->nfaces = (uint16_t)node_numsurfaces( node, mod );
	out->firstface = (uint32_t)node_firstsurface( node, mod );
	out->orig = (uint32_t)( node - mod->nodes );
	for( int side = 0; side < 2; side++ )
	{
		const mnode_t *ch = node_child( node, side, mod );

		if( !ch )
			return -1;
		if( ch->contents < 0 )
		{
			const mleaf_t *l = (const mleaf_t *)ch;

			if( l < mod->leafs || l > mod->leafs + mod->numleafs )
				return -1;
			c = -1 - (int32_t)( l - mod->leafs );
		}
		else if(( c = RP_CookNode( mod, ch, depth + 1 )) < 0 )
			return -1;
		out->child[side] = c;
	}
	return k;
}

/* a face's box in whole units, around its corners (floor and ceiling of
   their 16.16): what the walk's screen test (-rpfacecull) reads */
static void RP_CookFaceBox( const model_t *mod, const msurface_t *fa, int16_t fb[6] )
{
	fb[0] = fb[1] = fb[2] = 32767;
	fb[3] = fb[4] = fb[5] = -32768;
	for( int e = 0; e < fa->numedges && fa->firstedge >= 0 && fa->firstedge + e < mod->numsurfedges; e++ )
	{
		int              lindex = mod->surfedges[fa->firstedge + e];
		const medge16_t *ed = &mod->edges16[lindex > 0 ? lindex : -lindex];
		int              vi = lindex > 0 ? ed->v[0] : ed->v[1];

		if( vi < 0 || vi >= mod->numvertexes )
			continue;
		for( int j = 0; j < 3; j++ )
		{
			int32_t lo = rp_wv[vi].pos[j] >> 16, hi = (int32_t)(((int64_t)rp_wv[vi].pos[j] + 0xFFFF ) >> 16 );

			if( lo < fb[j] ) fb[j] = (int16_t)( lo < -32768 ? -32768 : lo );
			if( hi > fb[3 + j] ) fb[3 + j] = (int16_t)( hi > 32767 ? 32767 : hi );
		}
	}
	if( fb[0] > fb[3] )
	{
		/* no corner: never culled by its box */
		fb[0] = fb[1] = fb[2] = -32768;
		fb[3] = fb[4] = fb[5] = 32767;
	}
}

static void RP_CookWorkFree( void )
{
	free( rp_cw.nodes );
	free( rp_cw.planes );
	free( rp_cw.remap );
	memset( &rp_cw, 0, sizeof( rp_cw ));
}

/* the cooked arrays from the live model (the world's tables made just
   before), then the file written: rp_ck_live, or the engine's walk */
static void RP_CookMake( const model_t *mod )
{
	rp_cook_head_t  h;
	rp_cook_sizes_t s;
	char            path[80];
	uint64_t        t0 = rp_time_us( );
	file_t         *f;
	int             ok = 1;

	RP_CookHead( mod, &h );
	if( mod->numnodes < 1 || mod->numplanes < 1 || mod->numsurfaces > 0xFFFF )
		return;
	/* the world's tree first, into work arrays (the brush entities' nodes
	   share the engine's array and are left out: run 149, c1a0e's world
	   reaches 1,323 of its 2,169 nodes; and only the planes it uses: 2,000
	   or so of c2a5f's 18,972) */
	rp_cw.nodes = malloc( sizeof( *rp_cw.nodes ) * mod->numnodes );
	rp_cw.planes = malloc( sizeof( *rp_cw.planes ) * mod->numnodes );
	rp_cw.remap = malloc( sizeof( *rp_cw.remap ) * mod->numplanes );
	if( !rp_cw.nodes || !rp_cw.planes || !rp_cw.remap )
	{
		RP_CookWorkFree( );
		return;
	}
	for( int i = 0; i < mod->numplanes; i++ )
		rp_cw.remap[i] = -1;
	if( RP_CookNode( mod, mod->nodes, 0 ) != 0 )
	{
		fprintf( stderr, "rp-emit: the map could not be cooked (a node's box not in whole units, or a node out of range): the engine's walk\n" );
		RP_CookWorkFree( );
		return;
	}
	h.nnodes = (uint32_t)rp_cw.nn;
	h.nplanes = (uint32_t)rp_cw.np;
	if( !RP_CookSizes( &h, &s ) || !RP_CookAlloc( &s, &h ))
	{
		RP_CookWorkFree( );
		return;
	}
	memcpy( (void *)rp_ck.planes, rp_cw.planes, s.planes );
	memcpy( (void *)rp_ck.nodes, rp_cw.nodes, s.nodes );
	RP_CookWorkFree( );
	for( int i = 0; ok && i < (int)h.nleafs; i++ )
	{
		const mleaf_t *l = &mod->leafs[i];
		rp_cleaf_t    *o = (rp_cleaf_t *)&rp_ck.leafs[i];

		ok = RP_CookBox( l->minmaxs, o->box );
		o->contents = l->contents;
		o->firstmark = 0;
		o->nmarks = 0;
		if( l->nummarksurfaces > 0 )
		{
			if( l->firstmarksurface < mod->marksurfaces
				|| l->firstmarksurface + l->nummarksurfaces > mod->marksurfaces + mod->nummarksurfaces )
				ok = 0;
			else
			{
				o->firstmark = (uint32_t)( l->firstmarksurface - mod->marksurfaces );
				o->nmarks = (uint32_t)l->nummarksurfaces;
			}
		}
	}
	for( int i = 0; ok && i < mod->nummarksurfaces; i++ )
	{
		const msurface_t *m = mod->marksurfaces[i];

		ok = m >= mod->surfaces && m < mod->surfaces + mod->numsurfaces;
		if( ok )
			((uint16_t *)rp_ck.marks)[i] = (uint16_t)( m - mod->surfaces );
	}
	for( int i = 0; ok && i < mod->numsurfaces; i++ )
	{
		msurface_t *fa = &mod->surfaces[i];
		uint8_t     b = ( fa->flags & SURF_PLANEBACK ) ? RP_CK_PLANEBACK : 0;

		if( fa->texinfo && fa->info && !( fa->texinfo->flags & TEX_WORLD_LUXELS )
			&& gEngfuncs.Mod_SampleSizeForFace( fa ) == 16
			&& fa->info->lightmapmins[0] == fa->texturemins[0] && fa->info->lightmapmins[1] == fa->texturemins[1] )
			b |= RP_CK_LMOK;
		((uint8_t *)rp_ck.faces)[i] = b;
		if( rp_ck.fbox )
			RP_CookFaceBox( mod, fa, ((int16_t ( * )[6] )rp_ck.fbox)[i] );
	}
	if( !ok )
	{
		fprintf( stderr, "rp-emit: the map could not be cooked (a leaf's box or faces out of range): the engine's walk\n" );
		RP_CookFree( );
		return;
	}
	h.total = s.total;
	rp_ck.serial = ++rp_ck_serial;
	rp_ck_live = 1;
	RP_TraceTable( mod ); /* kit 109: the engine's traces on the cooked planes */
	fprintf( stderr, "rp-emit: the map cooked in %u us (%u of %d nodes, %u of %d planes; %u KiB kept in RAM)\n",
		(uint32_t)( rp_time_us( ) - t0 ), h.nnodes, mod->numnodes, h.nplanes, mod->numplanes,
		( s.planes + s.nodes + s.leafs + s.marks + s.faces + ( rp_ck.fbox ? s.fbox : 0 )) >> 10 );

	/* kept on the card for the next load */
	if( !RP_CookPath( mod, path, sizeof( path )))
	{
		fprintf( stderr, "rp-emit: no hash of %s's file: the cooked map not kept\n", mod->name );
		return;
	}
	t0 = rp_time_us( );
	f = gEngfuncs.fsapi->Open( path, "wb", true );
	if( !f )
	{
		fprintf( stderr, "rp-emit: %s could not be written\n", path );
		return;
	}
	ok = gEngfuncs.fsapi->Write( f, &h, sizeof( h )) == sizeof( h );
	for( int i = 0; ok && i < mod->numvertexes; i++ )
		ok = gEngfuncs.fsapi->Write( f, rp_wv[i].pos, 12 ) == 12;
	if( ok ) ok = gEngfuncs.fsapi->Write( f, rp_wc, s.corners ) == s.corners;
	if( ok ) ok = gEngfuncs.fsapi->Write( f, rp_ck.planes, s.planes + s.nodes + s.leafs + s.marks + s.faces )
		== s.planes + s.nodes + s.leafs + s.marks + s.faces;
	/* the face boxes: made here a face at a time when not kept in RAM */
	for( int i = 0; ok && i < mod->numsurfaces; i++ )
	{
		int16_t fb[6];

		if( rp_ck.fbox )
			memcpy( fb, rp_ck.fbox[i], sizeof( fb ));
		else
			RP_CookFaceBox( mod, &mod->surfaces[i], fb );
		ok = gEngfuncs.fsapi->Write( f, fb, sizeof( fb )) == sizeof( fb );
	}
	gEngfuncs.fsapi->Close( f );
	if( !ok )
	{
		gEngfuncs.fsapi->Delete( path );
		fprintf( stderr, "rp-emit: %s not written whole: removed\n", path );
		return;
	}
	fprintf( stderr, "rp-emit: the cooked map kept as %s in %u us (%u KiB)\n", path, (uint32_t)( rp_time_us( ) - t0 ), s.total >> 10 );
}

/* `n` bytes of the open file into `to`: 1 if all came */
static int RP_CookGet( file_t *f, void *to, uint32_t n )
{
	return gEngfuncs.fsapi->Read( f, to, n ) == (fs_offset_t)n;
}

/* the cooked file, read and checked: 1 with the world's tables and the
   cooked arrays made from it, 0 (nothing kept) to cook again. Read a
   section at a time straight into its place (run 153: a 1.2 MiB file read
   whole into the engine's pool was a hole the fight's map then lacked). */
static int RP_CookRead( const model_t *mod )
{
	rp_cook_head_t  h, want;
	rp_cook_sizes_t s;
	char            path[80];
	file_t         *f;
	const char     *why = NULL;

	if( !RP_CookPath( mod, path, sizeof( path )))
		return 0;
	f = gEngfuncs.fsapi->Open( path, "rb", true );
	if( !f )
	{
		fprintf( stderr, "rp-emit: %s not on the card yet: cooked at this load\n", path );
		return 0;
	}
	RP_CookHead( mod, &want );
	if( !RP_CookGet( f, &h, sizeof( h )))
		why = "too short";
	else if( h.magic != RP_COOK_MAGIC || h.version != RP_COOK_VERSION || h.head_bytes != sizeof( h ))
		why = "another version";
	else if( h.bsp_len != want.bsp_len || memcmp( h.md5, want.md5, sizeof( h.md5 )))
		why = "the map's file changed";
	else if( h.nverts != want.nverts || h.ncorners != want.ncorners || h.nfaces != want.nfaces || h.nleafs != want.nleafs
		|| h.nmarks != want.nmarks || h.nnodes < 1 || h.nnodes > (uint32_t)mod->numnodes
		|| h.nplanes < 1 || h.nplanes > h.nnodes )
		why = "its counts are not the map's";
	else if( !RP_CookSizes( &h, &s ) || h.total != s.total || gEngfuncs.fsapi->FileLength( f ) != (fs_offset_t)s.total )
		why = "its size is wrong";
	else if( !RP_CookAlloc( &s, &h ))
		why = "no room";
	else
	{
		uint8_t buf[64 * 12];

		for( uint32_t i = 0; !why && i < h.nverts; i += 64 )
		{
			uint32_t n = h.nverts - i < 64 ? h.nverts - i : 64;

			if( !RP_CookGet( f, buf, n * 12u ))
				why = "short";
			for( uint32_t j = 0; !why && j < n; j++ )
			{
				rp_wv[i + j].frame = -1;
				memcpy( rp_wv[i + j].pos, buf + j * 12u, 12 );
			}
		}
		if( !why && !( RP_CookGet( f, rp_wc, s.corners )
			&& RP_CookGet( f, (void *)rp_ck.planes, s.planes + s.nodes + s.leafs + s.marks + s.faces )
			&& ( rp_ck.fbox ? RP_CookGet( f, (void *)rp_ck.fbox, s.fbox ) : 1 )))
			why = "short";
	}
	gEngfuncs.fsapi->Close( f );
	if( !why )
	{
		/* the checks on the file's own numbers, before anything is used */
		for( uint32_t i = 0; !why && i < h.ncorners; i++ )
			if( rp_wc[i].vi < 0 || (uint32_t)rp_wc[i].vi >= h.nverts )
				why = "a corner's vertex out of range";
		for( uint32_t i = 0; !why && i < h.nnodes; i++ )
		{
			const rp_cnode_t *nd = &rp_ck.nodes[i];

			if( nd->plane >= h.nplanes || nd->orig >= (uint32_t)mod->numnodes
				|| (uint64_t)nd->firstface + nd->nfaces > h.nfaces )
				why = "a node out of range";
			for( int side = 0; !why && side < 2; side++ )
			{
				int32_t c = nd->child[side];

				if( c >= 0 ? ( (uint32_t)c <= i || (uint32_t)c >= h.nnodes ) : ( (uint32_t)( -1 - c ) >= h.nleafs ))
					why = "a node's child out of range or before it";
			}
		}
		for( uint32_t i = 0; !why && i < h.nleafs; i++ )
			if( (uint64_t)rp_ck.leafs[i].firstmark + rp_ck.leafs[i].nmarks > h.nmarks )
				why = "a leaf's faces out of range";
		for( uint32_t i = 0; !why && i < h.nmarks; i++ )
			if( rp_ck.marks[i] >= h.nfaces )
				why = "a leaf's face out of range";
	}
	if( why )
	{
		fprintf( stderr, "rp-emit: %s not used (%s): cooked again\n", path, why );
		RP_CookFree( );
		/* the vertices and corners are made again by the caller; nothing
		   of the file's corners left for the faces it skips */
		memset( rp_wc, 0, sizeof( *rp_wc ) * (size_t)mod->numsurfedges );
		return 0;
	}
	rp_ck.serial = ++rp_ck_serial;
	rp_ck_live = 1;
	RP_TraceTable( mod ); /* kit 109: the engine's traces on the cooked planes */
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
	const mtexinfo_t   *tex = fa->texinfo;
	const rp_wcorner_t *wc = &rp_wc[fa->firstedge];
	uint32_t           zis[RP_MAX_CORNERS];
	const int32_t      *st[RP_MAX_CORNERS], *cxy[RP_MAX_CORNERS];
	/* the face's centre on the screen, 28.4: 32 bits hold it (a corner within
	   +-2^20 of the screen x 64 corners), and an int64 divide is a library
	   call on the PXA270 (run 46: RP_ChipFaceFixed 4.0% of the play) */
	int32_t           sx = 0, sy = 0;
	uint32_t          nearzi = 0;
	int               n = fa->numedges, miplevel, w, h, qx, qy, pg, luxels, lo, hi, kw, kh, oldmip = -1, wanted, behind = 0;
	/* a face across the near plane: its corners cut in integers (below) */
	int32_t           kxy[RP_MAX_CORNERS][2], kst[RP_MAX_CORNERS][2];

	/* kit 64 (run 60: ~450 of this function's 2,008 samples waiting on
	   each corner's first load of its vertex record, the misses one after
	   another): every corner's record asked for first (PLD, a hint), so
	   the PXA270's four line fills overlap */
	for( int i = 0; i < n; i++ )
		__builtin_prefetch( &rp_wv[wc[i].vi] );
	for( int i = 0; i < n; i++ )
	{
		rp_wvert_t *c = &rp_wv[wc[i].vi];

		if( c->frame != serial )
		{
			int32_t  v[3];
			uint32_t w[3];

			c->frame = serial;
			RP_ViewPoint( m, o, c->pos, v );
			if( v[2] < rp_near16 )
				c->zi = 0;
			else
			{
				RP_ProjectS( v, w, c->xy );
				c->zi = w[2];
			}
		}
		if( !c->zi )
			behind = 1;
		zis[i] = c->zi;
		cxy[i] = c->xy;
		st[i] = wc[i].st;
	}
	if( behind )
	{
		/*
		Kit 50 (run 49: RP_EmitFace's float path ~2.5% of the play, ~13
		faces a frame, the tram's own walls around the eye): ref_soft and
		ref_gl clip such a face in floats (R_ClipEdge, GL's clip space);
		here the face is cut at the near plane in 16.16 view space, as
		the models' RP_StudioClipCmd cuts theirs: each corner's view point
		made again (integers, cheap), a new corner on each edge across
		(one divide), its texture coordinates carried the same fraction
		along, then projected as any corner. Never a float.
		*/
		int32_t q[RP_MAX_CORNERS][3];
		int     m2 = 0;

		if( n >= RP_MAX_CORNERS )
			return 0;
		for( int i = 0; i < n; i++ )
			RP_ViewPoint( m, o, rp_wv[wc[i].vi].pos, q[i] );
		for( int i = 0; i < n; i++ )
		{
			int     j = i + 1 == n ? 0 : i + 1;
			int     ini = q[i][2] >= rp_near16, inj = q[j][2] >= rp_near16;

			if( ini )
			{
				zis[m2] = rp_wv[wc[i].vi].zi;
				kxy[m2][0] = rp_wv[wc[i].vi].xy[0];
				kxy[m2][1] = rp_wv[wc[i].vi].xy[1];
				kst[m2][0] = wc[i].st[0];
				kst[m2][1] = wc[i].st[1];
				m2++;
			}
			if( ini != inj )
			{
				/* t = (1 - z_i) / (z_j - z_i), 0.16 */
				int64_t  den = (int64_t)q[j][2] - q[i][2];
				int32_t  t = (int32_t)(( (int64_t)( rp_near16 - q[i][2] ) << 16 ) / den );
				int32_t  p[3];
				uint32_t wd[3];

				for( int k = 0; k < 2; k++ )
					p[k] = q[i][k] + (int32_t)(( (int64_t)( q[j][k] - q[i][k] ) * t ) >> 16 );
				p[2] = rp_near16;
				RP_ProjectS( p, wd, kxy[m2] );
				zis[m2] = wd[2];
				kst[m2][0] = wc[i].st[0] + (int32_t)(( (int64_t)( wc[j].st[0] - wc[i].st[0] ) * t ) >> 16 );
				kst[m2][1] = wc[i].st[1] + (int32_t)(( (int64_t)( wc[j].st[1] - wc[i].st[1] ) * t ) >> 16 );
				m2++;
			}
		}
		if( m2 < 3 )
			return 1; /* wholly behind: nothing to draw */
		n = m2;
		for( int i = 0; i < n; i++ )
		{
			cxy[i] = kxy[i];
			st[i] = kst[i];
		}
		rp_count.nearclipped++;
	}
	for( int i = 0; i < n; i++ )
	{
		if( zis[i] > nearzi )
			nearzi = zis[i];
		sx += cxy[i][0] >> 2; /* quarter pixels: 64 far corners can't overflow */
		sy += cxy[i][1] >> 2;
	}
	sx = RP_DivN( sx, n ) * 4;
	sy = RP_DivN( sy, n ) * 4;
	/* R352 after run 333: a still brush entity's face in a world face's
	   plane drawn just behind it, so the world wins the tie every frame
	   (RP_CoplanarMark); nearzi (the mip level) taken before, unmoved */
	if( m == (const int32_t( * )[3])rp_bview.m && rp_bview.still && !rp_coplanar_off && RP_COPLANAR( fa ))
	{
		for( int i = 0; i < n; i++ )
			zis[i] -= RP_COPLANAR_BIAS;
		rp_count.coplanar++;
	}
	if( rp_lm_on && RP_ChipFaceLm( fa, n, zis, cxy, st ))
	{
		if( fa->pdecals )
			RP_FaceDecals( fa, m, o );
		return 1;
	}

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
	kw = rp_kedge[w]; /* w, h at most RP_PAGE_SIDE (512): checked above */
	kh = rp_kedge[h];

	RP_Put( RP_GFX_STRIP( n ) | rp_noz);
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
		if( !rp_noz ) RP_Put( zis[c] );
		RP_Put( zis[c] );
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
==============================================================================
R309: the sky, water and conveyor belts on the 2700G (kit 51)
==============================================================================

Iterate, don't imitate (rule 0).

The sky. ref_soft fills each sky span by sampling the six skybox images as
a cube for every pixel (several float multiplies a pixel: hopeless without
an FPU); ref_gl clips every sky polygon into the box's six sides in floats
(ClipSkyPolygon), keeps each side's bounds and draws those rectangles of
the box at the far clip, the sky faces themselves not drawn. Ours: the
six sides as six textured quads around the eye (the box's corners turned
by the view's rows once a frame, integers, the eye at its centre), their
depth word below anything the world writes, so the 2700G's hidden-surface
removal textures only the pixels where nothing nearer is: the per-pixel
work is the chip's, the CPU's is eight corners. Only in a frame with a sky
face in the view; the sides behind the eye not sent; each side's image in
the texture pool once, no mip chain (the sky is always magnified),
clamped and bilinear (no seam where two sides meet). Sent as the frame's
first part, before anything a blend could put over it.

Water. ref_soft warps each pixel's texel (Turbulent8: a 128-texel wave of
8 texels each way); ref_gl cuts the face into 64-unit polygons and moves
only their corners' texture coordinates (a 50-texel wave sampled every 64:
it aliases into a slow swim). Ours: the face cut on the fly, in integers,
into a grid of 16-texel cells (8 corners a wave: the warp within 0.6 texel
of ref_soft's), coarser for a large face (at most 256 cells), each corner's
texture coordinates warped by ref_soft's own wave (its phase moving
smoothly, not in ref_soft's 1/20 s steps; on the absolute texel, so two
faces of one pool meet without a seam), the inner cells sent as one strip
a row, the cells on the face's edge cut to it; the texture from the pool
with its mip chain and the chip's bilinear filtering. A func_water's
blend as the other brushes'; its waves (the entity's scale) as ref_gl's.

Conveyors. ref_gl scrolls the texture coordinates by the entity's speed;
ref_soft scrolls the lit surface. Ours: option B's texture pass with its
s moved by the same speed (one multiply a face), the lightmap still.
*/

/* the box's half side: 4096 units (16.16), the depth word of its sides */
#define RP_SKY_R    ( 1 << 28 )
#define RP_SKY_NEAR ( RP_SKY_R >> 4 ) /* off the screen at any field of view up to 172 degrees */
#define RP_SKY_Z    ( 107u << 23 )    /* 2^-20: behind the farthest world point (2^-14) */

/* ref_gl's st_to_vec (gl_warp.c): a side's s, t, out to the world's axes */
static const int rp_st_to_vec[6][3] =
{
	{  3, -1,  2 },
	{ -3,  1,  2 },
	{  1,  3,  2 },
	{ -1, -3,  2 },
	{ -2, -1,  3 },
	{  2, -1, -3 }
};

/*
A convex polygon in view space (16.16), its texture coordinates (16.16
repeats), cut at the plane z = `near` and sent as one strip: `zword` the
depth word (0: 1/z), `colour` each corner's colour word.
*/
static void RP_PolyEmit( int n, const int32_t ( *q )[3], const int32_t ( *uv )[2], int32_t near, uint32_t zword, uint32_t colour )
{
	int32_t  o[RP_MAX_CORNERS][3], ouv[RP_MAX_CORNERS][2];
	uint32_t w[RP_MAX_CORNERS][3];
	int      m = 0, lo, hi;

	if( n < 3 || n > RP_MAX_CORNERS - 2 )
		return;
	for( int i = 0; i < n; i++ )
	{
		int j = i + 1 == n ? 0 : i + 1;
		int ini = q[i][2] >= near, inj = q[j][2] >= near;

		if( ini )
		{
			o[m][0] = q[i][0];
			o[m][1] = q[i][1];
			o[m][2] = q[i][2];
			ouv[m][0] = uv[i][0];
			ouv[m][1] = uv[i][1];
			m++;
		}
		if( ini != inj )
		{
			int64_t den = (int64_t)q[j][2] - q[i][2];
			int32_t t = (int32_t)(( (int64_t)( near - q[i][2] ) << 16 ) / den );

			for( int k = 0; k < 2; k++ )
			{
				o[m][k] = q[i][k] + (int32_t)(( (int64_t)( q[j][k] - q[i][k] ) * t ) >> 16 );
				ouv[m][k] = uv[i][k] + (int32_t)(( (int64_t)( uv[j][k] - uv[i][k] ) * t ) >> 16 );
			}
			o[m][2] = near;
			m++;
		}
	}
	if( m < 3 )
		return;
	if( m != n )
		rp_count.nearclipped++;
	for( int i = 0; i < m; i++ )
		RP_ProjectS( o[i], w[i], NULL );
	RP_Put( RP_GFX_STRIP( m ) | ( zword ? 0 : rp_noz ));
	lo = 1;
	hi = m - 1;
	for( int k = 0; k < m; k++ )
	{
		int c = k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--;

		RP_Put( w[c][0] );
		RP_Put( w[c][1] );
		if( zword || !rp_noz ) RP_Put( zword ? zword : w[c][2] );
		RP_Put( w[c][2] );
		RP_Put( colour );
		RP_Put( RP_FixBits( ouv[c][0], 16 ));
		RP_Put( RP_FixBits( ouv[c][1], 16 ));
	}
	RP_Whole();
	rp_count.tris += m - 2;
	rp_count.verts += m;
}

/* The sky's six sides (RP_EmitSubmit, when a sky face was in the view). */
static void RP_SkyBox( void )
{
	static const int corner[4][2] = { { -1, -1 }, { -1, 1 }, { 1, 1 }, { 1, -1 } };

	if( rp_view.frame != tr.framecount )
		return;
	for( int i = 0; i < 6; i++ )
		if( !tr.skyboxTextures[i] )
			return; /* none loaded: the clear colour, as ref_soft's flat fill */
	for( int i = 0; i < 6; i++ )
	{
		int32_t  q[4][3], uv[4][2];
		int      behind = 0;
		rp_tex_t *t;

		for( int c = 0; c < 4; c++ )
		{
			int32_t b[3] = { corner[c][0] * RP_SKY_R, corner[c][1] * RP_SKY_R, RP_SKY_R }, d[3];
			static const int32_t zero[3] = { 0, 0, 0 };

			for( int j = 0; j < 3; j++ )
			{
				int k = rp_st_to_vec[i][j];

				d[j] = k < 0 ? -b[-k - 1] : b[k - 1];
			}
			RP_ViewPoint( (const int32_t( * )[3] )rp_view.m, zero, d, q[c] );
			behind += q[c][2] < RP_SKY_NEAR;
			/* ref_gl's MakeSkyVec: s, t from -1..1 to 0..1, t upside down */
			uv[c][0] = corner[c][0] > 0 ? RP_ONE16 : 0;
			uv[c][1] = corner[c][1] > 0 ? 0 : RP_ONE16;
		}
		if( behind == 4 )
			continue;
		t = RP_TexPlace( R_GetTexture( tr.skyboxTextures[r_skyTexOrder[i]] ), 1 );
		if( !t )
		{
			rp_count.sky_missed++;
			continue;
		}
		rp_state = NULL;
		rp_count.states++;
		RP_Put( RP_GFX_STATE );
		RP_Put( RP_TSP_WORLD );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put( t->tex0 | RP_TEX0_CLAMP );
		RP_Put( t->tex1 );
		RP_Put( RP_TEX2 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_PolyEmit( 4, (const int32_t( * )[3] )q, (const int32_t( * )[2] )uv, RP_SKY_NEAR, RP_SKY_Z, 0xFFFFFFFFu );
		rp_count.sky_sides++;
	}
}

/*
R352 after run 331 (the owner: "flashing or glitching textures at the top
of the building in the outside area"). R309 drew the sky box first, behind
everything, and no sky face at all: whatever a map keeps behind its sky
brushes (a building's upper part past the sky, the far side of a wall
topped with sky) was drawn over the sky, and came and went as its leaves
fell in and out of the view: the flashing at the buildings' tops. GoldSrc
hides all of it: ref_soft draws each sky face as a surface of its own,
ref_gl draws the box only where sky faces are and they cover what lies
behind. Rule 0: each sky face drawn on the 2700G at its own depth (it
hides what is behind it, as Half-Life's), with the sky box's picture
mapped onto it exactly: the face cut into the cube's six pyramids, each
piece's corners given the box's s, t of their direction, and the chip's
texture perspective word set to the direction's distance along that side's
axis over the corner's depth, while the depth word stays the corner's own
(the separate words the box itself already uses), so the chip's
perspective-correct interpolation lands each pixel on the box's own texel:
no seam with the box behind, no CPU per pixel, a few corners a face. The
box stays drawn first, behind all, for any crack between faces.
*/
typedef struct { float d[3], v[3]; } rp_skyc_t; /* direction from the eye (world axes), view space */

static int RP_SkyClip( const rp_skyc_t *in, int n, rp_skyc_t *out, int axis, float sign, int on_d, int dist_axis, float dist_sign )
{
	int m = 0;

	for( int i = 0; i < n; i++ )
	{
		const rp_skyc_t *a = &in[i], *b = &in[i + 1 == n ? 0 : i + 1];
		/* inside: dist + sign * component >= 0 (on_d), or view depth >= NEAR_CLIP */
		float fa = on_d ? dist_sign * a->d[dist_axis] + sign * a->d[axis] : a->v[2] - NEAR_CLIP;
		float fb = on_d ? dist_sign * b->d[dist_axis] + sign * b->d[axis] : b->v[2] - NEAR_CLIP;

		if( fa >= 0.0f )
			out[m++] = *a;
		if(( fa >= 0.0f ) != ( fb >= 0.0f ))
		{
			float f = fa / ( fa - fb );

			for( int k = 0; k < 3; k++ )
			{
				out[m].d[k] = a->d[k] + f * ( b->d[k] - a->d[k] );
				out[m].v[k] = a->v[k] + f * ( b->v[k] - a->v[k] );
			}
			m++;
		}
		if( m >= RP_MAX_CORNERS )
			return 0;
	}
	return m;
}

static void RP_SkyFace( msurface_t *fa )
{
	rp_skyc_t poly[RP_MAX_CORNERS], a[RP_MAX_CORNERS], b[RP_MAX_CORNERS];
	const model_t *mod = RI.currentmodel;
	int n = 0;

	if( mod != WORLDMODEL || fa->numedges < 3 || fa->numedges > RP_MAX_CORNERS - 8 || rp_skyface_off )
		return;
	for( int i = 0; i < 6; i++ )
		if( !tr.skyboxTextures[i] )
			return;
	for( int i = 0; i < fa->numedges; i++ )
	{
		int lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
		const float *world = r_pcurrentvertbase[lindex > 0 ? e->v[0] : e->v[1]].position;

		VectorSubtract( world, tr.modelorg, poly[n].d );
		TransformVector( poly[n].d, poly[n].v );
		n++;
	}
	n = RP_SkyClip( poly, n, a, 0, 0.0f, 0, 0, 0.0f ); /* the near plane */
	if( n < 3 )
		return;
	for( int side = 0; side < 6; side++ )
	{
		int        js = 0, jt = 0, jd = 0, m;
		float      ss = 1.0f, st = 1.0f, sd = 1.0f;
		rp_tex_t  *t;

		for( int j = 0; j < 3; j++ )
		{
			int k = rp_st_to_vec[side][j], ak = k < 0 ? -k : k;
			float sg = k < 0 ? -1.0f : 1.0f;

			if( ak == 1 ) { js = j; ss = sg; }
			else if( ak == 2 ) { jt = j; st = sg; }
			else { jd = j; sd = sg; }
		}
		/* the side's pyramid: dist >= |s| and dist >= |t| (four cuts) */
		m = RP_SkyClip( a, n, b, js, -ss, 1, jd, sd );
		m = m >= 3 ? RP_SkyClip( b, m, poly, js, ss, 1, jd, sd ) : 0;
		m = m >= 3 ? RP_SkyClip( poly, m, b, jt, -st, 1, jd, sd ) : 0;
		m = m >= 3 ? RP_SkyClip( b, m, poly, jt, st, 1, jd, sd ) : 0;
		if( m < 3 )
			continue;
		t = RP_TexPlace( R_GetTexture( tr.skyboxTextures[r_skyTexOrder[side]] ), 1 );
		if( !t )
		{
			rp_count.sky_missed++;
			continue;
		}
		rp_state = NULL;
		rp_count.states++;
		RP_Put( RP_GFX_STATE );
		RP_Put( RP_TSP_WORLD );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put( t->tex0 | RP_TEX0_CLAMP );
		RP_Put( t->tex1 );
		RP_Put( RP_TEX2 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Put( RP_GFX_STRIP( m )); /* with its depth word: the depth is not the texture's 1/w here */
		{
			int lo = 1, hi = m - 1;

			for( int k = 0; k < m; k++ )
			{
				const rp_skyc_t *c = &poly[k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--];
				float zi = 1.0f / c->v[2];
				float dist = sd * c->d[jd];
				float sx = xcenter + xscale * zi * c->v[0];
				float sy = ycenter - yscale * zi * c->v[1];
				float u = 0.5f + 0.5f * ss * c->d[js] / dist;
				float v = 0.5f - 0.5f * st * c->d[jt] / dist;

				RP_Put( RP_Bits( sy ));
				RP_Put( RP_Bits( (float)RP_GAME_W - sx ));
				RP_Put( RP_Bits( zi ));        /* depth: the face's own */
				RP_Put( RP_Bits( dist * zi )); /* the texture's perspective: the box's side */
				RP_Put( 0xFFFFFFFFu );
				RP_Put( RP_Bits( u ));
				RP_Put( RP_Bits( v ));
			}
		}
		RP_Whole();
		rp_count.sky_faces++;
		rp_count.tris += m - 2;
		rp_count.verts += m;
	}
}

/*
A water face's shape, made at its first draw (floats, once): the point of
texel (s0, t0) in the model's space and the steps a texel along s and t
(16.16 units), its corners in texels from (s0, t0) as 24.8, its bounds.
*/
typedef struct
{
	int32_t p0[3], a[3], b[3];
	int32_t s0, t0;            /* whole texels */
	int32_t smax, tmax;        /* whole texels from (s0, t0), rounded up */
	int     n, ccw;
	int32_t st[RP_MAX_CORNERS][2];
} rp_turb_t;

static rp_turb_t **rp_turb; /* by face, for the map (RP_WorldTables) */
static int         rp_turb_faces;

static void RP_TurbFree( void )
{
	if( rp_turb )
		for( int i = 0; i < rp_turb_faces; i++ )
			free( rp_turb[i] );
	free( rp_turb );
	rp_turb = NULL;
	rp_turb_faces = 0;
}

static rp_turb_t *RP_TurbShape( msurface_t *fa )
{
	const model_t *wm = WORLDMODEL;
	const model_t *mod = RI.currentmodel;
	int            idx, n = fa->numedges, i1 = 1, i2 = 2;
	float          s[RP_MAX_CORNERS], t[RP_MAX_CORNERS], smin = 1e9f, tmin = 1e9f, smax = -1e9f, tmax = -1e9f, best = 0.0f;
	const float   *p[RP_MAX_CORNERS];
	const float   *sv = fa->texinfo->vecs[0], *tv = fa->texinfo->vecs[1];
	rp_turb_t     *w;
	float          det, a[3], b[3], area = 0.0f;

	if( !wm || fa < wm->surfaces || fa >= wm->surfaces + wm->numsurfaces || n < 3 || n > RP_MAX_CORNERS - 6 )
		return NULL;
	idx = (int)( fa - wm->surfaces );
	if( !rp_turb || rp_turb_faces != wm->numsurfaces )
	{
		RP_TurbFree();
		rp_turb = calloc( wm->numsurfaces, sizeof( *rp_turb ));
		if( !rp_turb )
			return NULL;
		rp_turb_faces = wm->numsurfaces;
	}
	if( rp_turb[idx] )
		return rp_turb[idx];
	for( int i = 0; i < n; i++ )
	{
		int lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];

		p[i] = mod->vertexes[lindex > 0 ? e->v[0] : e->v[1]].position;
		s[i] = DotProduct( p[i], sv ) + sv[3];
		t[i] = DotProduct( p[i], tv ) + tv[3];
		smin = s[i] < smin ? s[i] : smin;
		tmin = t[i] < tmin ? t[i] : tmin;
		smax = s[i] > smax ? s[i] : smax;
		tmax = t[i] > tmax ? t[i] : tmax;
	}
	/* the two corners making the largest triangle with corner 0 */
	for( int i = 1; i < n; i++ )
		for( int j = i + 1; j < n; j++ )
		{
			float d = ( s[i] - s[0] ) * ( t[j] - t[0] ) - ( s[j] - s[0] ) * ( t[i] - t[0] );

			if( fabsf( d ) > best )
			{
				best = fabsf( d );
				i1 = i;
				i2 = j;
			}
		}
	det = ( s[i1] - s[0] ) * ( t[i2] - t[0] ) - ( s[i2] - s[0] ) * ( t[i1] - t[0] );
	if( fabsf( det ) < 1e-3f || smax - smin > 32768.0f || tmax - tmin > 32768.0f )
		return NULL;
	w = calloc( 1, sizeof( *w ));
	if( !w )
		return NULL;
	for( int k = 0; k < 3; k++ )
	{
		float d1 = p[i1][k] - p[0][k], d2 = p[i2][k] - p[0][k];

		a[k] = (( t[i2] - t[0] ) * d1 - ( t[i1] - t[0] ) * d2 ) / det;
		b[k] = (( s[i1] - s[0] ) * d2 - ( s[i2] - s[0] ) * d1 ) / det;
	}
	w->s0 = (int32_t)floorf( smin );
	w->t0 = (int32_t)floorf( tmin );
	w->smax = (int32_t)ceilf( smax ) - w->s0;
	w->tmax = (int32_t)ceilf( tmax ) - w->t0;
	for( int k = 0; k < 3; k++ )
	{
		w->p0[k] = RP_Fix16( p[0][k] + ( w->s0 - s[0] ) * a[k] + ( w->t0 - t[0] ) * b[k] );
		w->a[k] = RP_Fix16( a[k] );
		w->b[k] = RP_Fix16( b[k] );
	}
	w->n = n;
	for( int i = 0; i < n; i++ )
	{
		int j = i + 1 == n ? 0 : i + 1;

		w->st[i][0] = (int32_t)(( s[i] - w->s0 ) * 256.0f );
		w->st[i][1] = (int32_t)(( t[i] - w->t0 ) * 256.0f );
		area += s[i] * t[j] - s[j] * t[i];
	}
	w->ccw = area > 0.0f;
	rp_turb[idx] = w;
	return w;
}

/* ref_soft's wave (r_main.c sintable: AMP + AMP sin, a 128-texel cycle)
   as 24.8 texels, a table entry a texel */
static int32_t rp_wave[128];
static int     rp_wave_ready;

/* the wave's offset at texel position `x` (24.8), its phase `ph` in 1/256
   of a table step: linear between the table's whole texels */
static inline int32_t RP_Wave( int32_t x, int32_t ph )
{
	int32_t  at = ( x + ph ) & ( 128 * 256 - 1 );
	int      i = at >> 8, f = at & 255;
	int32_t  a = rp_wave[i], b = rp_wave[( i + 1 ) & 127];

	return a + ((( b - a ) * f ) >> 8 );
}

/* ref_gl's r_turbsin (8 sin) for the waves of a func_water, 16.16 */
static int32_t rp_tsin[256];

#define RP_WATER_CELLS 256
/*
Run 51 (c2a5, 33-49 water faces a frame): every face cut at 16 texels made
5,000-7,400 cells a frame, 60-120 Ki words, past the frame's buffer (its
end not drawn) and 6-12 fps. A cell's size now follows the face's distance,
as a texture's mip level does: the warp sampled about every 32 pixels on
the screen (8 corners a wave where the wave is 256 pixels or more, never
finer than 16 texels), and the frame holds at most RP_WATER_FRAME cells
(a face past it cut coarser still).
*/
#define RP_WATER_PIXELS 32
#define RP_WATER_FRAME  1024
#define RP_WATER_GRID  ( RP_WATER_CELLS * 2 + 64 )

/* the face's edges each grid point is outside of, by bits (st 24.8) */
static inline uint32_t RP_TurbOut( const rp_turb_t *w, int32_t s, int32_t t )
{
	uint32_t out = 0;

	for( int i = 0; i < w->n && i < 32; i++ )
	{
		int     j = i + 1 == w->n ? 0 : i + 1;
		int64_t c = (int64_t)( w->st[j][0] - w->st[i][0] ) * ( t - w->st[i][1] )
			- (int64_t)( w->st[j][1] - w->st[i][1] ) * ( s - w->st[i][0] );

		if( w->ccw ? c < 0 : c > 0 )
			out |= 1u << i;
	}
	return out;
}

/* A point of the face (st 24.8 from s0, t0) in view space and its warped
   texture coordinates (16.16 repeats). */
typedef struct
{
	const rp_turb_t *w;
	int32_t          vo[3], va[3], vb[3]; /* (s0, t0)'s view point; a texel's step in view space */
	int32_t          wz[3];               /* a unit of the world's z in view space (the waves) */
	int32_t          wave;                /* the waves' height, 16.16 (0: none) */
	int32_t          ph, wph1, wph2;      /* the warp's phase; the waves' */
	int32_t          iw, ih;              /* 2^24 / the texture's width, height */
	int32_t          tw, th;
} rp_wctx_t;

static void RP_TurbPoint( const rp_wctx_t *x, int32_t s, int32_t t, int32_t v[3], int32_t uv[2] )
{
	const rp_turb_t *w = x->w;
	/* absolute texels (24.8) for the warp: the same wave across faces */
	int32_t          as = s + ( w->s0 << 8 ), at = t + ( w->t0 << 8 );
	int32_t          ws = as + RP_Wave( at, x->ph ), wt = at + RP_Wave( as, x->ph );

	for( int k = 0; k < 3; k++ )
		v[k] = x->vo[k] + (int32_t)(( (int64_t)s * x->va[k] + (int64_t)t * x->vb[k] ) >> 8 );
	if( x->wave )
	{
		/* ref_gl's EmitWaterPolys: the point's height moved by two waves
		   along the world's x and y */
		int32_t px = ( w->p0[0] + (int32_t)(( (int64_t)s * w->a[0] + (int64_t)t * w->b[0] ) >> 8 )) >> 16;
		int32_t py = ( w->p0[1] + (int32_t)(( (int64_t)s * w->a[1] + (int64_t)t * w->b[1] ) >> 8 )) >> 16;
		int32_t nv = rp_tsin[( x->wph1 + px + py ) & 255] + ( 8 << 16 );
		int32_t dz;

		nv += (int32_t)(( (int64_t)( rp_tsin[( px * 5 + x->wph2 - py ) & 255] + ( 8 << 16 )) * 52429 ) >> 16 ); /* x 0.8 */
		dz = (int32_t)(( (int64_t)nv * x->wave ) >> 16 );
		for( int k = 0; k < 3; k++ )
			v[k] += (int32_t)(( (int64_t)x->wz[k] * dz ) >> 30 );
	}
	/* texels (24.8) to repeats (16.16): x 2^24 / w >> 16 */
	uv[0] = (int32_t)(( (int64_t)( ws - ( w->s0 << 8 ) + (( w->s0 % x->tw + x->tw ) % x->tw << 8 )) * x->iw ) >> 16 );
	uv[1] = (int32_t)(( (int64_t)( wt - ( w->t0 << 8 ) + (( w->t0 % x->th + x->th ) % x->th << 8 )) * x->ih ) >> 16 );
}

/* Clips the polygon `in` (st 24.8, `n` corners) to the face's edges. */
static int RP_TurbClip( const rp_turb_t *w, int n, int32_t ( *in )[2], int32_t ( *tmp )[2] )
{
	int32_t ( *src )[2] = in, ( *dst )[2] = tmp;

	for( int e = 0; e < w->n && n >= 3; e++ )
	{
		int     f = e + 1 == w->n ? 0 : e + 1;
		int64_t ex = w->st[f][0] - w->st[e][0], ey = w->st[f][1] - w->st[e][1];
		int     m = 0;

		for( int i = 0; i < n; i++ )
		{
			int     j = i + 1 == n ? 0 : i + 1;
			int64_t ci = ex * ( src[i][1] - w->st[e][1] ) - ey * ( src[i][0] - w->st[e][0] );
			int64_t cj = ex * ( src[j][1] - w->st[e][1] ) - ey * ( src[j][0] - w->st[e][0] );
			int     ini, inj;

			if( !w->ccw )
			{
				ci = -ci;
				cj = -cj;
			}
			ini = ci >= 0;
			inj = cj >= 0;
			if( m >= RP_MAX_CORNERS - 2 )
				return 0;
			if( ini )
			{
				dst[m][0] = src[i][0];
				dst[m][1] = src[i][1];
				m++;
			}
			if( ini != inj )
			{
				/* where the edge crosses: ci / (ci - cj) of the way */
				int64_t d = ci - cj;

				dst[m][0] = src[i][0] + (int32_t)(( src[j][0] - src[i][0] ) * ci / d );
				dst[m][1] = src[i][1] + (int32_t)(( src[j][1] - src[i][1] ) * ci / d );
				m++;
			}
		}
		n = m;
		{
			int32_t ( *sw )[2] = src;

			src = dst;
			dst = sw;
		}
	}
	if( src != in )
		for( int i = 0; i < n; i++ )
		{
			in[i][0] = src[i][0];
			in[i][1] = src[i][1];
		}
	return n;
}

/* A water face (SURF_DRAWTURB) to the 2700G. */
static void RP_ChipWater( msurface_t *fa )
{
	const model_t *mod = RI.currentmodel;
	const int32_t ( *m )[3];
	const int32_t *o;
	rp_turb_t     *w;
	texture_t     *tt;
	rp_tex_t      *t;
	rp_wctx_t      x;
	int            c = 16, nx, ny, gs0, gt0;
	static int32_t gv[RP_WATER_GRID][3], guv[RP_WATER_GRID][2];
	static uint32_t gout[RP_WATER_GRID], gw[RP_WATER_GRID][3];
	uint32_t       colour = rp_face_blend ? rp_face_colour : 0xFFFFFFFFu;
	const void    *key;

	if( !rp_wave_ready )
	{
		/* the tables in floats, once */
		for( int i = 0; i < 128; i++ )
			rp_wave[i] = (int32_t)(( 8.0f + 8.0f * sinf( i * 2.0f * M_PI_F / 128.0f )) * 256.0f );
		for( int i = 0; i < 256; i++ )
			rp_tsin[i] = (int32_t)( 8.0f * sinf( i * 2.0f * M_PI_F / 256.0f ) * 65536.0f );
		rp_wave_ready = 1;
	}
	if( mod == WORLDMODEL )
	{
		RP_ViewFixed();
		m = (const int32_t( * )[3] )rp_view.m;
		o = rp_view.o;
	}
	else if( rp_view.frame == tr.framecount )
	{
		RP_BrushViewFixed();
		m = (const int32_t( * )[3] )rp_bview.m;
		o = rp_bview.o;
	}
	else
	{
		rp_count.water_missed++;
		return;
	}
	w = RP_TurbShape( fa );
	tt = R_TextureAnimation( fa );
	t = w && tt ? RP_TexPlace( R_GetTexture( tt->gl_texturenum ), 0 ) : NULL;
	if( !t || tt->width < 1 || tt->height < 1 )
	{
		rp_count.water_missed++;
		return;
	}

	x.w = w;
	RP_ViewPoint( m, o, w->p0, x.vo );
	{
		static const int32_t zero[3] = { 0, 0, 0 };

		RP_ViewPoint( m, zero, w->a, x.va );
		RP_ViewPoint( m, zero, w->b, x.vb );
		for( int k = 0; k < 3; k++ )
			x.wz[k] = m[k][2]; /* the world's z in view space, 2.30 */
	}
	/* ref_soft's phase: SPEED (20) table steps a second, here smooth */
	x.ph = (int32_t)(( rp_time_ms * 20 * 256 / 1000 ) & ( 128 * 256 - 1 ));
	x.wph1 = (int32_t)( rp_time_ms * 160 / 1000 );
	x.wph2 = (int32_t)( rp_time_ms * 171 / 1000 );
	x.wave = 0;
	if( RI.currententity && RI.currententity->curstate.scale != 0.0f && mod != WORLDMODEL )
	{
		/* below the eye the waves go up, above it down (ref_gl's sign) */
		int32_t sc = RP_Fix16( RI.currententity->curstate.scale );

		x.wave = w->p0[2] >= RP_Fix16( tr.modelorg[2] ) ? -sc : sc;
	}
	x.tw = tt->width;
	x.th = tt->height;
	x.iw = (int32_t)(( 1 << 24 ) / tt->width );
	x.ih = (int32_t)(( 1 << 24 ) / tt->height );

	/* the cell's size from the face's nearest grid corner: c texels
	   cover about RP_WATER_PIXELS on the screen there */
	{
		int32_t zmin = 0x7FFFFFFF, step = 0;

		for( int k = 0; k < 3; k++ )
		{
			int32_t a = x.va[k] < 0 ? -x.va[k] : x.va[k], b = x.vb[k] < 0 ? -x.vb[k] : x.vb[k];

			step = a > step ? a : step;
			step = b > step ? b : step;
		}
		for( int k = 0; k < 4; k++ )
		{
			int32_t ss = k & 1 ? w->smax : 0, tt8 = k & 2 ? w->tmax : 0;
			int32_t z = x.vo[2] + (int32_t)( (int64_t)ss * x.va[2] + (int64_t)tt8 * x.vb[2] );

			zmin = z < zmin ? z : zmin;
		}
		/* pixels a texel at zmin: step (16.16 units) x xs / zmin */
		if( zmin > 16 * RP_ONE16 && step > 0 )
		{
			int64_t px = ( (int64_t)step * ( rp_view.xs >> 8 )) / ( zmin >> 8 ); /* 16.16 pixels a texel */

			while( c < 1024 && (( px * c ) >> 16 ) < RP_WATER_PIXELS )
				c <<= 1;
		}
	}
	/* the grid: lines on whole multiples of c texels (absolute: the
	   cells of two faces of one pool line up) */
	for( ;; )
	{
		gs0 = (( w->s0 & ~( c - 1 )) - w->s0 );
		gt0 = (( w->t0 & ~( c - 1 )) - w->t0 );
		nx = ( w->smax - gs0 + c - 1 ) / c;
		ny = ( w->tmax - gt0 + c - 1 ) / c;
		if(( nx * ny <= RP_WATER_CELLS && ( nx * ny <= rp_water_left || nx * ny <= 4 )) || c >= 1024 )
			break;
		c <<= 1;
	}
	rp_water_left -= nx * ny;
	if( nx < 1 || ny < 1 || ( nx + 1 ) * ( ny + 1 ) > RP_WATER_GRID )
	{
		rp_count.water_missed++;
		return;
	}

	/* the state block: the texture, opaque or the entity's blend */
	key = (const uint8_t *)t + ( rp_face_blend ? 1 + rp_face_blend : 0 );
	if( rp_state != key )
	{
		uint32_t tsp = RP_TSP_WORLD;

		if( rp_face_blend )
		{
			RP_LmFlush(); /* the lit walls before what blends over them */
			tsp = ( tsp & 0x03FFFFFFu ) | ( rp_face_blend == 2 ? 1u << 29 | 1u << 26 : 4u << 29 | 5u << 26 )
				| 1u << 13 | 1u << 7;
			rp_count.blended++;
		}
		rp_state = key;
		rp_count.states++;
		RP_Put( RP_GFX_STATE );
		RP_Put( tsp );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put( t->tex0 );
		RP_Put( t->tex1 );
		RP_Put( RP_TEX2 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Whole();
	}

	/* every grid point: in view space, warped, which edges it is outside,
	   projected when in front of the near plane */
	for( int j = 0; j <= ny; j++ )
		for( int i = 0; i <= nx; i++ )
		{
			int k = j * ( nx + 1 ) + i;
			int32_t s = ( gs0 + i * c ) << 8, tt8 = ( gt0 + j * c ) << 8;

			RP_TurbPoint( &x, s, tt8, gv[k], guv[k] );
			gout[k] = RP_TurbOut( w, s, tt8 );
			if( gv[k][2] >= rp_near16 )
				RP_ProjectS( gv[k], gw[k], NULL );
			else
				gw[k][2] = 0;
		}

	/* each row: the runs of whole cells in front of the near plane as one
	   strip; the cells on the face's edge or across the near plane cut */
	for( int j = 0; j < ny; j++ )
	{
		int i = 0;

		while( i < nx )
		{
			int k0 = j * ( nx + 1 ) + i, k1 = k0 + nx + 1;
			int whole = !( gout[k0] | gout[k0 + 1] | gout[k1] | gout[k1 + 1] )
				&& gw[k0][2] && gw[k0 + 1][2] && gw[k1][2] && gw[k1 + 1][2];

			if( whole )
			{
				int run = 1;

				while( i + run < nx && run < 126 )
				{
					int a = k0 + run + 1, b = k1 + run + 1;

					if( gout[a] | gout[b] || !gw[a][2] || !gw[b][2] )
						break;
					run++;
				}
				RP_Put( RP_GFX_STRIP( 2 * ( run + 1 )) | rp_noz);
				for( int r = 0; r <= run; r++ )
				{
					for( int e = 0; e < 2; e++ )
					{
						int g = ( e ? k1 : k0 ) + r;

						RP_Put( gw[g][0] );
						RP_Put( gw[g][1] );
						if( !rp_noz ) RP_Put( gw[g][2] );
						RP_Put( gw[g][2] );
						RP_Put( colour );
						RP_Put( RP_FixBits( guv[g][0], 16 ));
						RP_Put( RP_FixBits( guv[g][1], 16 ));
					}
				}
				RP_Whole();
				rp_count.tris += 2 * run;
				rp_count.verts += 2 * ( run + 1 );
				rp_count.water_cells += run;
				i += run;
				continue;
			}
			if( !( gout[k0] & gout[k0 + 1] & gout[k1] & gout[k1 + 1] ))
			{
				/* on the edge (or across the near plane): the cell cut to
				   the face, each corner worked on its own */
				int32_t sq[RP_MAX_CORNERS][2], tmp[RP_MAX_CORNERS][2];
				int32_t q[RP_MAX_CORNERS][3], uv[RP_MAX_CORNERS][2];
				int     n;

				sq[0][0] = ( gs0 + i * c ) << 8;
				sq[0][1] = ( gt0 + j * c ) << 8;
				sq[1][0] = sq[0][0] + ( c << 8 );
				sq[1][1] = sq[0][1];
				sq[2][0] = sq[1][0];
				sq[2][1] = sq[0][1] + ( c << 8 );
				sq[3][0] = sq[0][0];
				sq[3][1] = sq[2][1];
				n = ( gout[k0] | gout[k0 + 1] | gout[k1] | gout[k1 + 1] ) ? RP_TurbClip( w, 4, sq, tmp ) : 4;
				for( int v = 0; v < n; v++ )
					RP_TurbPoint( &x, sq[v][0], sq[v][1], q[v], uv[v] );
				RP_PolyEmit( n, (const int32_t( * )[3] )q, (const int32_t( * )[2] )uv, rp_near16, 0, colour );
				rp_count.water_cells++;
			}
			i++;
		}
	}
	rp_count.faces++;
	rp_count.water_faces++;
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

	if( fa->flags & SURF_DRAWSKY )
	{
		/* R309: the sky box behind everything, once at the frame's end */
		if( mod == WORLDMODEL )
			RP_ViewFixed();
		if( !rp_sky_seen )
			rp_count.sky_frames++;
		rp_sky_seen = 1;
		RP_SkyFace( fa ); /* R352 after run 331: at its own depth, hiding what is behind */
		return;
	}
	if( fa->numedges < 3 || fa->numedges > RP_MAX_CORNERS )
		return;
	RP_FaceBlend();
	if( fa->flags & SURF_DRAWTURB )
	{
		RP_ChipWater( fa );
		return;
	}
	rp_face_masked = ( fa->flags & SURF_TRANSPARENT ) != 0;
	if( rp_face_masked )
		rp_count.masked++;
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

	RP_Put( RP_GFX_STRIP( m ) | rp_noz);
	lo = 1;
	hi = m - 1;
	for( k = 0; k < m; k++ )
	{
		const rp_corner_t *c = &out[k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--];
		float sx = xcenter + xscale * c->z * c->x;
		float sy = ycenter - yscale * c->z * c->y;

		RP_Put( RP_Bits( sy ));
		RP_Put( RP_Bits( (float)RP_GAME_W - sx ));
		if( !rp_noz ) RP_Put( RP_Bits( c->z ));
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
   `serial` the part's (RP_StudioVerts), `zi` 0 when nearer than a unit.
   One 32-byte record, one cache line (FPS work after run 47: the record was
   36 bytes, so every vertex spanned two lines; RP_StudioCmd 6.5% of the
   play, most of it waiting on them).
   Kit 49 (run 48: RP_StudioCmd 3802 -> 4527 samples, 898 of them on the
   compare after the record's first load, ~20% of the function): the
   PXA270's D-cache allocates on reads only, so RP_StudioVertFixed's
   stores never bring the lines in and every corner's first load is an
   SDRAM miss (~506 ns, docs/perf.md). ref_soft and ref_gl walk the same
   kind of transformed-vertex table and leave the miss to the CPU's
   hardware prefetcher, which the PXA270 has not: here RP_StudioCmd asks
   for the record RP_MV_AHEAD corners ahead with PLD (the corner's index
   is known from the command), so the miss overlaps the corners between.
   The record now holds x and y's float words themselves (made once a
   vertex, not twice a corner: RP_FixBits was ~11% of RP_StudioCmd) and
   the view-space point moved to rp_mq, read only by a command with a
   corner behind the near plane. */
typedef struct
{
	int      serial;
	uint32_t zi;    /* 1/z's float bits, 0 behind the near plane */
	uint32_t f[2];  /* the TA's x, y float words */
	int32_t  xy[2]; /* the TA's x, y as 28.4: the culling reads them */
	int32_t  pad[2];
} __attribute__(( aligned( 32 ))) rp_mvert_t;

static rp_mvert_t rp_mv[RP_STUDIO_VERTS];
/* each vertex in view space, 16.16: the near plane's clip reads it */
static int32_t    rp_mq[RP_STUDIO_VERTS][3];
/* kit 71 (run 68: RP_StudioCmdLit's hottest load, 218 samples, the record's
   first use: kit 68 left its loop a third as long, so four corners ahead is
   no longer a miss's time ahead): eight there */
#define RP_MV_AHEAD_LIT 8
#define RP_MV_AHEAD 4 /* corners: ~300 cycles a miss against ~100 a corner;
                         the PXA270 keeps four line fills going at once */
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

/* A vertex's record from its view-space point. */
static inline void RP_StudioVertSet( int idx, rp_mvert_t *mv, const int32_t q[3] )
{
	mv->serial = rp_studio_serial;
	rp_mq[idx][0] = q[0];
	rp_mq[idx][1] = q[1];
	rp_mq[idx][2] = q[2];
	if( q[2] < rp_near16 )
		mv->zi = 0;
	else
	{
		uint32_t w[3];

		RP_ProjectS( q, w, mv->xy );
		mv->f[0] = w[0];
		mv->f[1] = w[1];
		mv->zi = w[2];
	}
	rp_count.mverts++;
}

int RP_StudioFixedBegin( int numverts )
{
	if( rp_emit_on != 2 || rp_both || !rp_fixed_on || numverts > RP_STUDIO_VERTS )
		return 0;
	RP_ViewFixed();
	rp_studio_serial++;
	rp_count.bparts++;
	/* the first records' lines (RP_StudioVertFixed asks four on) */
	for( int k = 0; k < 4 && k < numverts; k++ )
		__builtin_prefetch( &rp_mv[k] );
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
	/* the model's vertices are read in order and miss (read-allocate,
	   no hardware prefetch): ask for the ones eight vertices on */
	__builtin_prefetch( p + 24 );
	/* kit 53 (run 51: RP_StudioCmd's first read of each record a miss,
	   ~1,500 a frame): the PXA270's cache allocates on reads only, so the
	   record's stores below went past it to memory and the corners read
	   them back from there. Its line asked for four vertices ahead (one
	   record a line), the stores land in the cache and the corners find
	   them there; the line is filled meanwhile, the CPU not waiting. */
	__builtin_prefetch( &rp_mv[( idx + 4 ) & ( RP_STUDIO_VERTS - 1 )] );
	for( int j = 0; j < 3; j++ )
		a[j] = rp_fixq( p[j], 16 );
	rp_bone_point( &rp_fbone[bone], a, q );
	RP_StudioVertSet( idx, mv, q );
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
			/* the model's normals are read in order and miss as its
			   vertices do (run 48: ~400 of RP_StudioLightFixed's 1863
			   samples on these loads): ask for eight normals on */
			__builtin_prefetch( normal + 24 );
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

/*
Kit 61 (run 56: RP_StudioLightFixed 3.0% and RP_StudioVertFixed 2.1% of the
play, ~320 cycles a normal and ~340 a vertex): ref_soft and ref_gl light
and move a model's normals and vertices with a call each (R_StudioLighting,
Matrix3x4_VectorTransform), and kit 49's integer path kept that shape, so
every normal paid a call, the flags' tests, the light's three globals
loaded from memory and the counter's store. Here one call a mesh lights
all its normals (and one a part moves all its vertices): the flags tested
once (full-bright and flat-shade meshes made once and copied), the light
kept in registers, the same integer maths a normal, so the same bytes and
the same records as RP_StudioLightFixed and RP_StudioVertFixed.
*/
/*
Kit 72 (run 69: RP_StudioLightMesh 1,486 samples, 2.6% of the play, while
~60% of the model triangles are dropped before drawing, kit 68): a mesh's
normals are lit when a sent corner first reads one (RP_StudioNCol), not
all before the draw; full-bright and flat meshes (one colour) stay made
at once, and rp_lit 2 lights every normal both ways as before. The same
integer maths a normal (RP_LitNormal, shared by both ways), the same light
and bone tables (they hold for the whole part: rp_studio_serial), so the
same bytes for every normal read. Iterate (rule 0): GL and ref_soft light
every vertex before culling; only what is drawn needs a colour.
*/
#define RP_LM_MESHES 64
static struct
{
	const byte *bones;
	const float ( *blight )[3];
	const float ( *norms )[3];
	int         first, count, flags;
} rp_nm[RP_LM_MESHES];
static int        rp_nm_n;          /* meshes recorded this part */
static byte ( *rp_nm_base )[3];     /* the part's colour table */
static int        rp_nm_serial[RP_STUDIO_VERTS]; /* lit for this serial */

/* The part's colour table (R_StudioDrawPoints' rp_ncol): none recorded. */
void RP_StudioLightTable( byte ( *table )[3] )
{
	rp_nm_base = table;
	rp_nm_n = 0;
}

/* Normal i of a mesh lit, as kit 62's loop lit it. */
static inline void RP_LitNormal( const byte *bones, const float ( *blight )[3], const float ( *norms )[3],
	int i, int serial, const uint16_t *gtab, int small, byte *out )
{
	const int32_t a16 = rp_flit.a16, s16 = rp_flit.s16;
	const int32_t g0 = rp_flit.g16[0], g1 = rp_flit.g16[1], g2 = rp_flit.g16[2];
	unsigned bone = bones[i];
	int32_t  n[3];
	uint32_t gamma;
	int      index;

	if( bone >= RP_BONES )
		bone = 0;
	if( rp_flight_serial[bone] != serial )
	{
		rp_flight_serial[bone] = serial;
		for( int j = 0; j < 3; j++ )
			rp_flight[bone][j] = rp_fixq( blight[bones[i]][j], 28 );
	}
	if( !rp_fixq3_fast( norms[i], 30, n ))
		for( int j = 0; j < 3; j++ )
			n[j] = rp_fixq( norms[i][j], 30 );
	index = rp_light_index( rp_dot30( n, rp_flight[bone] ), a16, s16 );
	gamma = gtab ? gtab[index] : (uint32_t)index;
	if( small )
	{
		uint32_t c0 = (uint32_t)g0 * gamma >> 16, c1 = (uint32_t)g1 * gamma >> 16, c2 = (uint32_t)g2 * gamma >> 16;

		out[0] = c0 > 255 ? 255 : (byte)c0;
		out[1] = c1 > 255 ? 255 : (byte)c1;
		out[2] = c2 > 255 ? 255 : (byte)c2;
	}
	else
	{
		out[0] = rp_light_byte( g0, gamma );
		out[1] = rp_light_byte( g1, gamma );
		out[2] = rp_light_byte( g2, gamma );
	}
}

/* kit 62's choices a mesh: the gamma table, the 32-bit products */
static inline const uint16_t *RP_LitGamma( void )
{
	return FBitSet( gp_host->features, ENGINE_LINEAR_GAMMA_SPACE ) ? NULL : tr.lightgammatable;
}

static inline int RP_LitSmall( void )
{
	return (uint32_t)rp_flit.g16[0] < ( 1u << 21 ) && (uint32_t)rp_flit.g16[1] < ( 1u << 21 )
		&& (uint32_t)rp_flit.g16[2] < ( 1u << 21 );
}

/* The part's normal ni's colour, lit now if a recorded mesh holds it. */
const byte *RP_StudioNCol( int ni )
{
	const int serial = rp_studio_serial;

	if( rp_nm_serial[ni] != serial )
	{
		rp_nm_serial[ni] = serial;
		for( int m = 0; m < rp_nm_n; m++ )
			if( ni >= rp_nm[m].first && ni < rp_nm[m].first + rp_nm[m].count )
			{
				RP_LitNormal( rp_nm[m].bones, rp_nm[m].blight, rp_nm[m].norms, ni - rp_nm[m].first, serial,
					RP_LitGamma( ), RP_LitSmall( ), rp_nm_base[ni] );
				rp_nm_made++;
				if( rp_lit_on == 3 )
				{
					/* rp_lit 3: each normal lit when read also made one at a
					   time, the old way, and compared */
					const int i = ni - rp_nm[m].first;
					const int bone = rp_nm[m].bones[i];
					byte b[3];

					RP_StudioLightFixed( bone, rp_nm[m].blight[bone], rp_nm[m].flags, rp_nm[m].norms[i], b );
					rp_b61_items++;
					rp_b61_differ += b[0] != rp_nm_base[ni][0] || b[1] != rp_nm_base[ni][1] || b[2] != rp_nm_base[ni][2];
				}
				break;
			}
	}
	return rp_nm_base[ni];
}

void RP_StudioLightMesh( const byte *bones, const float ( *blight )[3], int flags, const float ( *norms )[3], int count, byte ( *out )[3] )
{
	const int32_t g0 = rp_flit.g16[0], g1 = rp_flit.g16[1], g2 = rp_flit.g16[2];
	const int     serial = rp_studio_serial;

	if( count <= 0 )
		return;
	rp_count.lit += (uint32_t)count;
	if( FBitSet( flags, STUDIO_NF_FULLBRIGHT | STUDIO_NF_FLATSHADE ))
	{
		uint32_t gamma = FBitSet( flags, STUDIO_NF_FULLBRIGHT ) ? 1023 : LightToTexGamma( (uint)rp_flit.flat );
		byte     c0 = rp_light_byte( g0, gamma ), c1 = rp_light_byte( g1, gamma ), c2 = rp_light_byte( g2, gamma );

		for( int i = 0; i < count; i++ )
		{
			out[i][0] = c0;
			out[i][1] = c1;
			out[i][2] = c2;
		}
		if( rp_lit_on == 2 )
			for( int i = 0; i < count; i++ )
			{
				byte b[3];

				RP_StudioLightFixed( bones[i], blight[bones[i]], flags, norms[i], b );
				rp_b61_items++;
				rp_b61_differ += b[0] != out[i][0] || b[1] != out[i][1] || b[2] != out[i][2];
			}
		return;
	}
	/* kit 72: recorded, lit when read (rp_lit 2: all now, and checked) */
	if( rp_lit_on != 2 && rp_nm_base && rp_nm_n < RP_LM_MESHES && out >= rp_nm_base
		&& out - rp_nm_base + count <= RP_STUDIO_VERTS )
	{
		rp_nm[rp_nm_n].bones = bones;
		rp_nm[rp_nm_n].blight = blight;
		rp_nm[rp_nm_n].norms = norms;
		rp_nm[rp_nm_n].first = (int)( out - rp_nm_base );
		rp_nm[rp_nm_n].count = count;
		rp_nm[rp_nm_n].flags = flags;
		rp_nm_n++;
		rp_nm_lazy += (uint32_t)count;
		return;
	}
	const uint16_t *gtab = RP_LitGamma( );
	const int       small = RP_LitSmall( );

	for( int i = 0; i < count; i++ )
	{
		/* the normals are read in order and miss: eight on asked for; the
		   colour bytes' line asked for before it is written (kit 62: the
		   PXA270 allocates on reads only, so RP_StudioCmdLit's reads of
		   them missed: run 57) */
		__builtin_prefetch( norms[i] + 24 );
		if( !( i & 7 ))
			__builtin_prefetch( out[i + 16] );
		RP_LitNormal( bones, blight, norms, i, serial, gtab, small, out[i] );
	}
	if( rp_nm_base && out >= rp_nm_base && out - rp_nm_base + count <= RP_STUDIO_VERTS )
		for( int i = 0; i < count; i++ )
			rp_nm_serial[out - rp_nm_base + i] = serial; /* made: not again */
	if( rp_lit_on == 2 )
		for( int i = 0; i < count; i++ )
		{
			byte b[3];

			RP_StudioLightFixed( bones[i], blight[bones[i]], flags, norms[i], b );
			rp_b61_items++;
			rp_b61_differ += b[0] != out[i][0] || b[1] != out[i][1] || b[2] != out[i][2];
		}
}

void RP_StudioVertsFixed( int count, const byte *bones, const float ( *bm )[3][4], const float ( *verts )[3] )
{
	const int serial = rp_studio_serial;

	for( int i = 0; i < count; i++ )
	{
		unsigned    bone = bones[i];
		rp_mvert_t *mv = &rp_mv[i];
		int32_t     a[3], q[3];

		if( bone >= RP_BONES )
			bone = 0;
		if( rp_fbone_serial[bone] != serial )
		{
			rp_fbone_serial[bone] = serial;
			rp_bone_join( (const int32_t( * )[3] )rp_view.m, rp_view.o, bm[bones[i]], &rp_fbone[bone] );
			rp_count.bones++;
		}
		/* as RP_StudioVertFixed: the model's vertices eight on, the
		   record four on (its stores then land in the cache) */
		__builtin_prefetch( verts[i] + 24 );
		__builtin_prefetch( &rp_mv[( i + 4 ) & ( RP_STUDIO_VERTS - 1 )] );
		if( !rp_fixq3_fast( verts[i], 16, a ))
			for( int j = 0; j < 3; j++ )
				a[j] = rp_fixq( verts[i][j], 16 );
		rp_bone_point( &rp_fbone[bone], a, q );
		RP_StudioVertSet( i, mv, q );
		if( rp_lit_on == 2 )
		{
			/* the record and the view-space point made again the old way */
			rp_mvert_t kept = *mv;
			int32_t    kq[3] = { rp_mq[i][0], rp_mq[i][1], rp_mq[i][2] };

			RP_StudioVertFixed( i, bones[i], (const float( * )[4] )bm[bones[i]], verts[i] );
			rp_b61_items++;
			rp_b61_differ += memcmp( &kept, mv, sizeof( kept )) != 0 || memcmp( kq, rp_mq[i], sizeof( kq )) != 0;
		}
	}
}

/* The skins in the atlas, found by their image's upload and place
   (R335: the texels' address was the key, but the 8-bit textures' 16-bit
   pixels move when rebuilt, and a freed address could come back as
   another texture's). */
#define RP_SKINS 512

typedef struct
{
	uint32_t       key; /* RP_SkinKey's; 0 an empty slot */
	uint32_t       off; /* a picture tile's first texel in the image */
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

	if( !RP_ChipWrite( ))
		return;

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

/* An image's upload, what it is drawn as (0 a skin, 1 a picture) and which
   of its copies (1 the alpha one), as one number never 0. */
static inline uint32_t RP_SkinKey( const image_t *img, int kind, int alpha )
{
	return img->rp_serial << 2 | (uint32_t)kind << 1 | (uint32_t)( alpha != 0 );
}

static inline uint32_t RP_SkinSlot( uint32_t key, uint32_t off )
{
	return ( key * 2654435761u ^ off * 40503u ) % RP_SKINS;
}

int RP_StudioSkin( void )
{
	image_t       *img = r_skin_img;
	int            w = r_affinetridesc.skinwidth, h = r_affinetridesc.skinheight;
	uint32_t       key;
	uint32_t       slot;
	rp_skin_t     *e;
	const void    *what;

	int            blend = RP_BLEND_OPAQUE;
	uint32_t       tsp;

	rp_cur_skin = NULL;
	if( !img || !R_TexLoaded( img ) || w < 1 || h < 1 || w > RP_PAGE_SIDE || h > RP_PAGE_SIDE )
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
	key = RP_SkinKey( img, 0, r_skin_alpha );
	slot = RP_SkinSlot( key, 0 );
	for( int i = 0;; i++ )
	{
		e = &rp_skin[( slot + i ) % RP_SKINS];
		if( !e->key || ( e->key == key && e->off == 0 && e->w == w && e->h == h ))
			break;
		if( i == RP_SKINS - 1 )
		{
			rp_count.meshes_skipped++;
			return 0;
		}
	}

	if( !e->key || e->gen != rp_page[e->pg].gen )
	{
		int x, y, pg, masked = 0;
		uint64_t t0 = rp_time_us();
		const pixel_t *px;

		pg = RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, &x, &y );
		if( pg < 0 )
		{
			rp_count.meshes_skipped++;
			return 0;
		}
		px = R_SkinPix( ); /* R335: its pixels only now, to write them */
		if( !px )
		{
			rp_count.meshes_skipped++;
			return 0;
		}
		for( int i = 0; i < w * h && !masked; i++ )
			masked = px[i] == TRANSPARENT_COLOR;
		RP_SkinUpload( px, w, h, pg, x, y, masked );
		e->key = key;
		e->off = 0;
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
	if( blend != RP_BLEND_OPAQUE && what != rp_state )
		RP_LmFlush(); /* the lit walls before the blended meshes */
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
		RP_Put( e->masked ? tsp | RP_TSP_PUNCH : tsp ); /* R321: the alpha test's object kind */
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
	/* R306 after run 51: word by word (RP_Put) this was ~8 instructions a
	   word, the corner's index and address made again and the room checked
	   for each of its 7; the room is checked once and each corner copied
	   as one 28-byte block (ldm/stm). The same words. */
	typedef struct
	{
		uint32_t w[RP_VERTEX_WORDS];
	} rp_corner_t;
	uint32_t need = 1u + (uint32_t)n * RP_VERTEX_WORDS;

	if( rp_noz )
	{
		/* six words a corner: the record's z (its word 2, the same as its
		   1/w, word 3) left out */
		need = 1u + (uint32_t)n * ( RP_VERTEX_WORDS - 1 );
		if( rp_n + need <= RP_BUF_WORDS )
		{
			uint32_t *o = &rp_buf[rp_n];

			*o++ = prim | rp_noz | (uint32_t)n;
			for( int k = 0; k < n; k++ )
			{
				const uint32_t *s = c[idx[k]];

				o[0] = s[0];
				o[1] = s[1];
				o[2] = s[3];
				o[3] = s[4];
				o[4] = s[5];
				o[5] = s[6];
				o += 6;
			}
			rp_n += need;
			RP_Whole();
			return;
		}
		RP_Put( prim | rp_noz | (uint32_t)n );
		for( int k = 0; k < n; k++ )
			for( int j = 0; j < RP_VERTEX_WORDS; j++ )
				if( j != 2 )
					RP_Put( c[idx[k]][j] );
		RP_Whole();
		return;
	}
	if( rp_n + need <= RP_BUF_WORDS )
	{
		rp_corner_t       *out = (rp_corner_t *)&rp_buf[rp_n + 1];
		const rp_corner_t *src = (const rp_corner_t *)c;

		rp_buf[rp_n] = prim | (uint32_t)n;
		for( int k = 0; k < n; k++ )
			out[k] = src[idx[k]];
		rp_n += need;
		RP_Whole();
		return;
	}
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

/* The corner where the edge a-b (a in front, b behind) meets the plane.
   Its colour from the two corners' colour words (kit 53: the same bytes
   the staged corners held, so the fused pass needs no staging). */
static void RP_NearCorner( const uint32_t ( *c )[RP_VERTEX_WORDS], const int32_t ( *cq )[3], const int32_t ( *cst )[2],
	int a, int b, uint32_t out[RP_VERTEX_WORDS], int32_t xy[2] )
{
	/* f = (z_a - 1) / (z_a - z_b) as 0.16: z_a >= 1 > z_b, so 0 <= f < 1 */
	int64_t  num = (int64_t)( cq[a][2] - rp_near16 ) << 16;
	int64_t  den = (int64_t)cq[a][2] - cq[b][2];
	int32_t  f = (int32_t)( num / den ), p[3];
	uint32_t rgb = rp_vert_alpha;

	p[0] = RP_Lerp16( cq[a][0], cq[b][0], f );
	p[1] = RP_Lerp16( cq[a][1], cq[b][1], f );
	p[2] = rp_near16;
	RP_ProjectS( p, out, xy );
	out[3] = out[2];
	for( int j = 0; j < 3; j++ )
	{
		int ca = (int)( c[a][4] >> ( 16 - 8 * j ) & 0xFFu ), cb = (int)( c[b][4] >> ( 16 - 8 * j ) & 0xFFu );

		rgb |= (uint32_t)( ca + ((( cb - ca ) * f ) >> 16 )) << ( 16 - 8 * j );
	}
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

/*
Kit 68 (run 65: ~2,150 of a frame's ~3,500 model triangles dropped, about
1,050 facing away or off the screen and 1,100 wholly behind the near
plane, yet RP_StudioCmdLit made every corner's texels and colour first):
the one-pass path makes a command's positions, the sort decides which
triangles are sent, and only their corners get the texels and the colour
(RP_LitFill, once a corner). The same words for every corner sent; a
corner no sent triangle uses is never sent. Iterate (rule 0): GL and
ref_soft light and set up every vertex before culling; the TA needs words
only for what it draws.
*/
static struct
{
	const short *cmd; /* the command's first corner (four shorts a corner) */
	const byte ( *ncol )[3];
	const int ( *chrome )[2];
	int32_t     u0, v0, sw, th, smax, tmax;
	uint32_t    alpha;
	int         on;   /* the command's texels and colours are still to fill */
	uint16_t    gen;  /* this command's mark in rp_lit_filled */
} rp_lz;
static uint16_t rp_lit_filled[RP_STUDIO_CMD];

/* Corner k's texels and colour, as RP_StudioCmdLit made them before. */
static void RP_LitFill( int k, uint32_t ( *c )[RP_VERTEX_WORDS], int32_t ( *cst )[2] )
{
	const short *cmd = rp_lz.cmd + 4 * k;
	unsigned     ni = (unsigned)(int)cmd[1];
	int32_t      s, tt;

	if( rp_lit_filled[k] == rp_lz.gen )
		return;
	rp_lit_filled[k] = rp_lz.gen;
	if( rp_lz.chrome )
	{
		s = rp_lz.chrome[ni][0];
		tt = rp_lz.chrome[ni][1];
	}
	else
	{
		s = cmd[2] << 5;
		tt = cmd[3] << 5;
	}
	/* RP_StudioCmd's texels: wrapped past the skin, kept inside it */
	if( s < 0 || s > rp_lz.sw || tt < 0 || tt > rp_lz.th )
	{
		rp_count.wraps++;
		s %= rp_lz.sw;
		tt %= rp_lz.th;
		if( s < 0 )
			s += rp_lz.sw;
		if( tt < 0 )
			tt += rp_lz.th;
	}
	if( s > rp_lz.smax )
		s = rp_lz.smax;
	if( tt > rp_lz.tmax )
		tt = rp_lz.tmax;
	/* kit 72: the normal lit when first read */
	const byte *nc = rp_lz.ncol == (const byte( * )[3] )rp_nm_base ? RP_StudioNCol( (int)ni ) : rp_lz.ncol[ni];

	c[k][4] = rp_lz.alpha | (uint32_t)nc[0] << 16 | (uint32_t)nc[1] << 8 | nc[2];
	c[k][5] = RP_FixBits( rp_lz.u0 + s, 5 + 9 );
	c[k][6] = RP_FixBits( rp_lz.v0 + tt, 5 + 9 );
	cst[k][0] = rp_lz.u0 + s;
	cst[k][1] = rp_lz.v0 + tt;
}

/* Every corner of the command filled (a path that sends them all). */
static void RP_LitFillAll( int n, uint32_t ( *c )[RP_VERTEX_WORDS], int32_t ( *cst )[2] )
{
	if( !rp_lz.on )
		return;
	for( int k = 0; k < n; k++ )
		RP_LitFill( k, c, cst );
	rp_lz.on = 0;
}

static void RP_StudioClipCmd( int fan, int n, uint32_t ( *c )[RP_VERTEX_WORDS],
	const int32_t ( *cq )[3], const int32_t ( *cst )[2], int32_t ( *cxy )[2] )
{
	static uint16_t idx[RP_STUDIO_CMD + 1];
	static uint8_t  state[RP_STUDIO_CMD];
	int run = -1, tris = n - 2, cull = tr.fFlipViewModel ? 0 : rp_cull_on;

	/* each triangle's state */
	for( int i = 0; i < tris; i++ )
	{
		int t0 = fan ? 0 : i, t1 = i + 1, t2 = i + 2;
		/* in front: 1/z's word made (0 behind the near plane, both paths) */
		int ins = ( c[t0][2] != 0 ) + ( c[t1][2] != 0 ) + ( c[t2][2] != 0 );

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
	/* kit 68: texels and colours only for the corners of what is sent */
	uint16_t lazy = rp_lz.on ? rp_lz.gen : 0;

	if( rp_lz.on )
	{
		for( int i = 0; i < tris; i++ )
			if( state[i] == RP_TRI_KEEP || state[i] == RP_TRI_CUT )
			{
				RP_LitFill( fan ? 0 : i, c, (int32_t( * )[2] )cst );
				RP_LitFill( i + 1, c, (int32_t( * )[2] )cst );
				RP_LitFill( i + 2, c, (int32_t( * )[2] )cst );
			}
		rp_lz.on = 0;
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
			if( lazy )
				for( int k = 0; k < m; k++ )
					rp_lit_unfilled += rp_lit_filled[idx[k]] != lazy;
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
				in[j] = c[t[j]][2] != 0;
			for( int j = 0; j < 3; j++ )
			{
				int a = t[j], b = t[( j + 1 ) % 3], bin = in[( j + 1 ) % 3];

				if( in[j] )
					poly[pn++] = (uint16_t)a;
				if( in[j] != bin )
				{
					if( in[j] )
						RP_NearCorner( (const uint32_t( * )[RP_VERTEX_WORDS] )c, cq, cst, a, b, c[slot], cxy[slot] );
					else
						RP_NearCorner( (const uint32_t( * )[RP_VERTEX_WORDS] )c, cq, cst, b, a, c[slot], cxy[slot] );
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
			if( lazy )
				for( int k = 0; k < pn; k++ )
					rp_lit_unfilled += sidx[k] < n && rp_lit_filled[sidx[k]] != lazy;
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
/* A command's corners as the TA's words (`c`), with what the culling and
   the near plane read: the view-space points (`cq`, made only for a cut),
   the page texels as 27.5 (`cst`), the TA's x, y as 28.4 (`cxy`), and on
   the fixed path each corner's vertex in rp_mv (`cvi`, 0xFFFF: none). */
static uint32_t rp_sc[RP_STUDIO_CMD + 4][RP_VERTEX_WORDS];
static uint16_t rp_sidx[RP_STUDIO_CMD * 2];
static int32_t  rp_scq[RP_STUDIO_CMD][3], rp_scst[RP_STUDIO_CMD][2], rp_scxy[RP_STUDIO_CMD + 4][2];
static uint16_t rp_scvi[RP_STUDIO_CMD];

static void RP_StudioCmdSend( int fan, int n, int behind );

/* rp_lit 2's kept corners (set once, then read back) */
static const uint32_t( *RP_LitKeep( const uint32_t ( *set )[RP_VERTEX_WORDS + 5] ))[RP_VERTEX_WORDS + 5]
{
	static const uint32_t( *kept )[RP_VERTEX_WORDS + 5];

	if( set )
		kept = set;
	return kept;
}

/* What RP_StudioMesh asks of rp_lit: 0 staged, 1 one pass, 2 both. */
int RP_StudioLitMode( void )
{
	return rp_lit_on;
}

/* rp_lit 2: 1 keep the next command's one-pass corners, 2 compare the
   next staged command with them, 0 neither. */
void RP_StudioLitCheck( int step )
{
	rp_lit_cap = step == 2 && rp_lit_cap != 3 ? 0 : step;
}

void RP_StudioCmd( int fan, int n, const rp_svert_t *v )
{
	uint32_t( *c )[RP_VERTEX_WORDS] = rp_sc;
	int32_t( *cq )[3] = rp_scq, ( *cst )[2] = rp_scst, ( *cxy )[2] = rp_scxy;
	const rp_skin_t *e = rp_cur_skin;
	int32_t u0, v0, smax, tmax;
	int     behind = 0;

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
	{
		RP_ViewFixed();
		/* the first corners' records asked for at once (PLD never
		   faults; an index past the table is masked into it) */
		for( int k = 0; k < RP_MV_AHEAD && k < n; k++ )
			__builtin_prefetch( &rp_mv[v[k].idx & ( RP_STUDIO_VERTS - 1 )] );
	}

	for( int k = 0; k < n; k++ )
	{
		int32_t s = v[k].s, tt = v[k].t;

		if( rp_fixed_on && (unsigned)v[k].idx < RP_STUDIO_VERTS )
		{
			/* each vertex of the model's part worked once (step 4b) */
			rp_mvert_t *mv = &rp_mv[v[k].idx];

			rp_scvi[k] = (uint16_t)v[k].idx;
			if( k + RP_MV_AHEAD < n )
				__builtin_prefetch( &rp_mv[v[k + RP_MV_AHEAD].idx & ( RP_STUDIO_VERTS - 1 )] );
			if( mv->serial != rp_studio_serial )
			{
				int32_t p[3], q[3];

				for( int j = 0; j < 3; j++ )
					p[j] = RP_Fix16( v[k].xyz[j] );
				RP_ViewPoint( (const int32_t( * )[3] )rp_view.m, rp_view.o, p, q );
				RP_StudioVertSet( v[k].idx, mv, q );
			}
			else
				rp_count.mreuse++;
			cxy[k][0] = mv->xy[0];
			cxy[k][1] = mv->xy[1];
			if( !mv->zi )
				behind++;
			c[k][0] = mv->f[0];
			c[k][1] = mv->f[1];
			c[k][2] = mv->zi;
			c[k][3] = mv->zi;
		}
		else
		{
			vec3_t local, t;
			float  zi, sx, sy;

			rp_scvi[k] = 0xFFFFu;
			VectorSubtract( v[k].xyz, RI.rvp.vieworigin, local );
			TransformVector( local, t );
			for( int j = 0; j < 3; j++ )
				cq[k][j] = RP_Fix16( t[j] );
			if( cq[k][2] < rp_near16 )
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
	RP_StudioCmdSend( fan, n, behind );
}

/* The command's corners built (rp_sc and beside it): culled, cut at the
   near plane, sent as strips and fans. */
static void RP_StudioCmdSend( int fan, int n, int behind )
{
	uint32_t( *c )[RP_VERTEX_WORDS] = rp_sc;
	int32_t( *cq )[3] = rp_scq, ( *cxy )[2] = rp_scxy;
	const int32_t( *cst )[2] = (const int32_t( * )[2] )rp_scst;
	uint16_t *idx = rp_sidx;
	int       m;
	/* kit 116: the TA asked to cull (the probe), so the CPU's sort only
	   for a command with a corner behind the near plane */
	uint32_t  hw = rp_hwcull && rp_cull_on == 1 && !behind && !tr.fFlipViewModel ? rp_hwcull : 0;
	int       sort = behind || ( rp_cull_on && !hw );

	/* kit 68: a path that sends every corner (or rp_lit 2's check) fills
	   them all now; the sort below fills only the corners it sends */
	if( rp_lit_cap || !sort )
		RP_LitFillAll( n, c, rp_scst );

	if( rp_lit_cap == 1 )
	{
		/* rp_lit 2: the one-pass corners kept, not sent */
		static uint32_t keep[RP_STUDIO_CMD][RP_VERTEX_WORDS + 5];

		for( int k = 0; k < n; k++ )
		{
			memcpy( keep[k], c[k], sizeof( c[k] ));
			keep[k][7] = (uint32_t)cst[k][0];
			keep[k][8] = (uint32_t)cst[k][1];
			keep[k][9] = (uint32_t)cxy[k][0];
			keep[k][10] = (uint32_t)cxy[k][1];
			keep[k][11] = rp_scvi[k];
		}
		rp_lit_capn = n + ( behind << 16 );
		rp_lit_cap = 3;
		RP_LitKeep( keep );
		return;
	}
	if( rp_lit_cap == 2 )
	{
		/* rp_lit 2: the staged corners against the kept ones */
		const uint32_t( *keep )[RP_VERTEX_WORDS + 5] = RP_LitKeep( NULL );

		for( int k = 0; k < n; k++ )
		{
			int d = (( rp_lit_capn & 0xFFFF ) != n ) || ( rp_lit_capn >> 16 ) != behind
				|| memcmp( keep[k], c[k], sizeof( c[k] )) || keep[k][7] != (uint32_t)cst[k][0]
				|| keep[k][8] != (uint32_t)cst[k][1] || keep[k][9] != (uint32_t)cxy[k][0]
				|| keep[k][10] != (uint32_t)cxy[k][1] || keep[k][11] != rp_scvi[k];

			rp_lit_differ += (uint32_t)d;
		}
		rp_lit_corners += (uint32_t)n;
		rp_lit_cap = 0;
	}

	if( hw )
		rp_count.hwcull++;
	if( sort )
	{
		/* each triangle sorted (R299's near plane, the culling): the ones
		   kept in runs, a triangle across the near plane cut */
		if( behind )
		{
			/* the cut reads the view-space points: the fixed path's
			   from rp_mq, only now (the float path made its own) */
			rp_count.cmds_near++;
			/* kit 68: wholly behind, nothing is cut: the points unread */
			if( rp_fixed_on && behind < n )
				for( int k = 0; k < n; k++ )
				{
					unsigned i = rp_scvi[k];

					/* kit 53: rp_mq was written, not read, so its lines
					   are not in the cache (read-allocate): the ones a
					   few corners on asked for now */
					if( k + RP_MV_AHEAD < n )
						__builtin_prefetch( rp_mq[rp_scvi[k + RP_MV_AHEAD] & ( RP_STUDIO_VERTS - 1 )] );
					if( i < RP_STUDIO_VERTS )
					{
						cq[k][0] = rp_mq[i][0];
						cq[k][1] = rp_mq[i][1];
						cq[k][2] = rp_mq[i][2];
					}
				}
		}
		RP_StudioClipCmd( fan, n, c, (const int32_t( * )[3] )cq, cst, cxy );
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
			RP_StudioWords( RP_GFX_STRIP( 0 ) | hw, (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx, m );
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
			RP_StudioWords( RP_GFX_FAN( 0 ) | hw, (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx, m + 1 );
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
			RP_StudioWords( RP_GFX_STRIP( 0 ) | hw, (const uint32_t( * )[RP_VERTEX_WORDS] )c, idx + at, p );
		}
		rp_count.fans++;
	}
	rp_count.tris += n - 2;
	rp_count.verts += n;
}

/*
Kit 53: a lit part's command straight from the model's stored command
(R_StudioDrawPoints lit its normals in integers: rp_lit_part). Iterate,
don't imitate (rule 0): ref_soft and ref_gl hand each corner to their
triangle interface (a call, a colour and a texel conversion a corner);
kit 49 to 52 staged each corner in a table (RP_StudioMesh: the colour
bytes, the vertex, the texels) that RP_StudioCmd then read back, a write
and a read of ~24 bytes a corner, the PXA270's cache allocating on reads
only (run 51: RP_StudioMesh 3.0% of the play). Here one pass reads the
command's four shorts (vertex, normal, s, t), the vertex's record and the
normal's colour, and writes the TA's words: the same words in the same
order (the staging held exactly these values). `chrome` the chrome
mesh's texels a normal (27.5), NULL for a skin's own; any corner whose
vertex or normal is past the tables, or whose vertex was not worked this
part, sends the command the staged way.
*/
int RP_StudioCmdLit( int fan, int n, const short *cmd, const byte ( *ncol )[3], const int ( *chrome )[2] )
{
	uint32_t( *c )[RP_VERTEX_WORDS] = rp_sc;
	int32_t( *cxy )[2] = rp_scxy;
	const rp_skin_t *e = rp_cur_skin;
	const int        serial = rp_studio_serial;
	const short     *cmd0 = cmd;
	int              behind = 0;

	if( !e || n < 3 || n > RP_STUDIO_CMD || !rp_fixed_on )
		return 0;
	RP_ViewFixed();
	for( int k = 0; k < RP_MV_AHEAD_LIT && k < n; k++ )
		__builtin_prefetch( &rp_mv[(uint16_t)cmd[4 * k] & ( RP_STUDIO_VERTS - 1 )] );

	/* kit 68: the positions here; the texels and colours (RP_LitFill) only
	   for the corners of the triangles RP_StudioCmdSend sends */
	for( int k = 0; k < n; k++, cmd += 4 )
	{
		unsigned          vi = (unsigned)(int)cmd[0], ni = (unsigned)(int)cmd[1];
		const rp_mvert_t *mv;

		/* the command stream: a 32-byte line every four corners, read in
		   order, ahead of use (as RP_StudioMesh asked) */
		if( !( k & 3 ))
			__builtin_prefetch( cmd + 64 );
		if( k + RP_MV_AHEAD_LIT < n )
			__builtin_prefetch( &rp_mv[(uint16_t)cmd[4 * RP_MV_AHEAD_LIT] & ( RP_STUDIO_VERTS - 1 )] );
		if( vi >= RP_STUDIO_VERTS || ni >= RP_STUDIO_VERTS )
			return 0;
		mv = &rp_mv[vi];
		if( mv->serial != serial )
			return 0;
		rp_scvi[k] = (uint16_t)vi;
		cxy[k][0] = mv->xy[0];
		cxy[k][1] = mv->xy[1];
		if( !mv->zi )
			behind++;
		c[k][0] = mv->f[0];
		c[k][1] = mv->f[1];
		c[k][2] = mv->zi;
		c[k][3] = mv->zi;
	}
	rp_lz.cmd = cmd0;
	rp_lz.ncol = ncol;
	rp_lz.chrome = chrome;
	rp_lz.u0 = e->qx << 8;
	rp_lz.v0 = e->qy << 8;
	rp_lz.sw = e->w << 5;
	rp_lz.th = e->h << 5;
	rp_lz.smax = rp_lz.sw - 1;
	rp_lz.tmax = rp_lz.th - 1;
	rp_lz.alpha = rp_vert_alpha;
	if( ++rp_lz.gen == 0 )
	{
		/* the marks wrapped: none may look filled */
		memset( rp_lit_filled, 0, sizeof( rp_lit_filled ));
		rp_lz.gen = 1;
	}
	rp_lz.on = 1;
	rp_count.mreuse += n;
	RP_StudioCmdSend( fan, n, behind );
	rp_lz.on = 0;
	return 1;
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
	RP_HintCount( );
	if( rp_emit_on != 2 )
		return;
	RP_LmFlush();
	RP_StageFlush( ); /* R352: the rewrites in place, the render before ended */
	{
		/* the sky's six sides drawn first (below everything), then the
		   parts in their order: the words where they lie */
		struct rp_gfx_part list[RP_GFX_PARTS];
		uint32_t sky = rp_n, end;
		int      k = 0;

		if( rp_sky_seen )
			RP_SkyBox();
		end = sky <= rp_good ? sky : rp_good;
		if( rp_good > sky )
		{
			list[k].words = &rp_buf[sky];
			list[k++].count = rp_good - sky;
		}
		for( int i = 0; i < rp_nparts; i++ )
			list[k++] = rp_parts[i];
		if( end > rp_seg_from )
		{
			list[k].words = &rp_buf[rp_seg_from];
			list[k++].count = end - rp_seg_from;
		}
		if( rp_gfx_frame_parts( list, (size_t)k ) != 0 )
		{
			if( errno == ESTALE )
				RP_ChipLost( );
			else
				rp_count.refused++;
		}
	}
	if( rp_shot_now )
	{
		char path[40];

		snprintf( path, sizeof( path ), "/rp-xash/shot-%d.565", tr.framecount );
		RP_ChipShotTo( path );
		rp_shot_now = 0;
	}
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
/*
Kit 69 (run 66: 271 of RP_ChipFaceFixed's samples on its first load of a
face's corner table, a miss each face): the world walk asks for the next
face's corner line while this face is drawn (a hint only).
*/
void RP_FaceAhead( const msurface_t *fa )
{
	if( rp_wc )
		__builtin_prefetch( &rp_wc[fa->firstedge] );
}

void RP_EmitFace( msurface_t *fa )
{
	if( rp_emit_on == 2 )
	{
		if( rp_borrowed )
			RP_BorrowReturn( ); /* the world again: its textures back first */
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
static intptr_t rp_dc_gen; /* R352's decal arena (defined with RP_DecalCorners) */
static int      rp_dc_used;

void RP_EmitNewMap( void )
{
	rp_dc_gen++; /* every decal's corners built again on the new map */
	rp_dc_used = 0;
	uint64_t t0 = rp_time_us( );

	rp_wmodel = NULL;
	rp_skin_world = NULL;
	rp_state = NULL;
	RP_TurbFree(); /* R309: the water faces' shapes are the old map's */
	/* R352: the old map's last frames may still be drawing from the pages
	   (kit 113's overlap): waited for, as every other page drop does
	   (RP_IN_FLIGHT), before the new map's faces are written over them */
	rp_gfx_wait( );
	rp_stage_n = 0; /* (no rewrite of the old map's held) */
	for( int pg = 0; pg < rp_pages; pg++ )
		RP_PageDrop( &rp_page[pg] );
	if( rp_emit_on == 2 && rp_fixed_on )
		RP_WorldTables( );
	rp_map_frames = 0;
	fprintf( stderr, "rp-emit: new map: per-map state reset, tables in %u us\n", (uint32_t)( rp_time_us( ) - t0 ));
	{
		/* R310 (a map change out of memory after three): the C heap at each
		   new map, used and free in holes, to tell a leak (used grows) from
		   holes too small for a map's large blocks (free grows) */
		struct mallinfo mi = mallinfo( );

		fprintf( stderr, "rp-emit: the C heap at the new map: %u KiB used, %u KiB free in %u holes, %u KiB from the system\n",
			(unsigned)( mi.uordblks >> 10 ), (unsigned)( mi.fordblks >> 10 ), (unsigned)mi.ordblks, (unsigned)( mi.arena >> 10 ));
	}
}

/*
==============================================================================
The 2D layer (the owner, run 46: "no hud"; "switching between levels should
say loading"; the weapon list and the menu the buttons open): the HUD, the
console's text, the menu, the loading plaque and the touch keys, every
picture the engine hands R_DrawStretchPic and every fill (CL_FillRGBA,
Draw_Fill), drawn by the 2700G over the 3D frame.

Rule 0. `ref_soft` copies each picture's pixels on the CPU into its buffer
every frame, blending through tables at 8 levels of alpha (the HUD alone a
few ms on the PXA270); `ref_gl` uploads each picture once as a GL texture
and draws a quad. Ours: each picture (or each 256 x 256 tile of a larger
one: the menu's 800 x 600 background) uploaded once into the atlas, RGB565
or ARGB4444 when it has alpha (its 3-bit alpha widened, its colour from
the renderer's table), a quad of four corners a draw: the CPU writes 37
words where ref_soft wrote every pixel, and the 2700G blends at 8 bits
(alpha, additive, the screen fade's multiply: the models' proven words),
depth "always", not written, after the 3D. A fill samples one white
square, so fills use the same textured words (no untextured form new to
ref_rp).
==============================================================================
*/
static uint32_t rp_2d_colour = 0xFFFFFFFFu; /* the colour the engine set last, ARGB */
static struct
{
	int pg, qx, qy, gen;
} rp_white = { -1, 0, 0, 0 };

void RP_Emit2DColour( float r, float g, float b, float a )
{
	int32_t c[4] = { rp_fixq( a, 8 ), rp_fixq( r, 8 ), rp_fixq( g, 8 ), rp_fixq( b, 8 ) };
	uint32_t w = 0;

	for( int i = 0; i < 4; i++ )
		w = w << 8 | (uint32_t)( c[i] < 0 ? 0 : c[i] > 255 ? 255 : c[i] );
	rp_2d_colour = w;
}

/* A picture's texel: RGB565 through the renderer's table, or ARGB4444 from
   `alpha_pixels` (ref_soft's: alpha in the top 3 bits, the pixel << 3). */
static inline uint32_t RP_PicTexel( pixel_t p, int fmt )
{
	uint32_t a, c;

	if( !fmt )
		return vid.screen[p];
	a = p >> 13;
	if( !a )
		return 0;
	c = vid.screen[(pixel_t)( p << 3 )];
	return ( a * 15 + 3 ) / 7 << 12 | ( c >> 12 & 0xFu ) << 8 | ( c >> 7 & 0xFu ) << 4 | ( c >> 1 & 0xFu );
}

/* `w` x `h` texels from `src` (rows `stride` apart) into page `pg` at
   square (`qx`, `qy`), past the edges the last row and column again. */
static uint32_t RP_PicTexelAt( const pixel_t *src, int stride, int x, int y, int fmt, const byte *idx, const uint32_t *pal, const pixel_t *tab )
{
	const pixel_t p = tab ? tab[idx[y * stride + x]] : src[y * stride + x];

	if( idx && pal && fmt )
	{
		/* R352: the opacity from the 8-bit palette's alpha (Half-Life's
		   decals: the index itself, 256 levels) to the chip's 16, not
		   ref_soft's 8; the colour as before (the renderer's table) */
		uint32_t a = pal[idx[y * stride + x]] >> 28, cc = vid.screen[(pixel_t)( p << 3 )];

		return a ? a << 12 | ( cc >> 12 & 0xFu ) << 8 | ( cc >> 7 & 0xFu ) << 4 | ( cc >> 1 & 0xFu ) : 0;
	}
	return RP_PicTexel( p, fmt );
}

/* the picture at half size (rp_pic_half: a texel the mean of its 2 x 2, each
   channel; `w` x `h` the half size, the source's last row and column again
   past its edge) */
static int rp_pic_half, rp_pic_srcw, rp_pic_srch;

static uint32_t RP_PicTexelHalf( const pixel_t *src, int stride, int x, int y, int fmt, const byte *idx, const uint32_t *pal, const pixel_t *tab )
{
	uint32_t sum[4] = { 0, 0, 0, 0 }, t;

	for( int k = 0; k < 4; k++ )
	{
		int sx = 2 * x + ( k & 1 ), sy = 2 * y + ( k >> 1 );

		sx = sx < rp_pic_srcw ? sx : rp_pic_srcw - 1;
		sy = sy < rp_pic_srch ? sy : rp_pic_srch - 1;
		t = RP_PicTexelAt( src, stride, sx, sy, fmt, idx, pal, tab );
		if( fmt )
			for( int c = 0; c < 4; c++ )
				sum[c] += t >> ( 4 * c ) & 0xFu;
		else
		{
			sum[0] += t & 0x1Fu;
			sum[1] += t >> 5 & 0x3Fu;
			sum[2] += t >> 11 & 0x1Fu;
		}
	}
	if( fmt )
		return ( sum[0] + 2 ) / 4 | ( sum[1] + 2 ) / 4 << 4 | ( sum[2] + 2 ) / 4 << 8 | ( sum[3] + 2 ) / 4 << 12;
	return ( sum[0] + 2 ) / 4 | ( sum[1] + 2 ) / 4 << 5 | ( sum[2] + 2 ) / 4 << 11;
}

static void RP_PicUpload( const pixel_t *src, int stride, int w, int h, int pg, int qx, int qy, int fmt, const byte *idx, const uint32_t *pal, const pixel_t *tab )
{
	uint32_t *page = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES );
	int       w8 = ( w + 7 ) >> 3, h8 = ( h + 7 ) >> 3;

	if( !RP_ChipWrite( ))
		return;

	for( int sy = 0; sy < h8; sy++ )
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
					t[j] = rp_pic_half ? RP_PicTexelHalf( src, stride, x, y, fmt, idx, pal, tab )
						: RP_PicTexelAt( src, stride, x, y, fmt, idx, pal, tab );
				}
				dst[k] = t[0] | t[1] << 16;
			}
		}
}

/* A picture tile's place in the atlas (the skins' table, keyed by its
   image's upload and its first texel `off` there), uploaded when new or its
   page was dropped; NULL when there is no room (the tile not drawn this
   frame, counted). */
static const rp_skin_t *RP_PicPlace( image_t *img, uint32_t off, int stride, int w, int h, int fmt )
{
	uint32_t   key = RP_SkinKey( img, 1, fmt );
	uint32_t   koff = rp_pic_half ? off | 0x80000000u : off; /* the half-size copy its own place */
	uint32_t   slot = RP_SkinSlot( key, koff );
	rp_skin_t *e;

	if( rp_skin_world != WORLDMODEL )
	{
		memset( rp_skin, 0, sizeof( rp_skin ));
		rp_skin_world = WORLDMODEL;
	}
	for( int i = 0;; i++ )
	{
		e = &rp_skin[( slot + i ) % RP_SKINS];
		if( !e->key || ( e->key == key && e->off == koff && e->w == w && e->h == h ))
			break;
		if( i == RP_SKINS - 1 )
			return NULL;
	}
	if( !e->key || e->gen != rp_page[e->pg].gen )
	{
		int      x, y, pg;
		uint64_t t0 = rp_time_us();
		/* run 318: a compact picture read through its 256-entry tables (no
		   decoded copy: the menu banner's 534 KiB blocks); else decoded */
		static pixel_t c16[256], a16[256];
		const byte    *idx = NULL;
		const int      k = R_TexCompactAlpha( img, c16, a16, &idx );
		const pixel_t *tab = k && ( !fmt || k == 2 ) ? ( fmt ? a16 : c16 ) : NULL;
		const pixel_t *src = tab ? NULL : fmt ? R_TexAlpha( img ) : R_TexPix( img, 0 ); /* R335: only now */

		if( !tab && !src )
			return NULL;
		if( !tab )
			idx = NULL;
		pg = RP_AtlasAlloc(( w + 7 ) >> 3, ( h + 7 ) >> 3, &x, &y );
		if( pg < 0 )
			return NULL;
		RP_PicUpload( src ? src + off : NULL, stride, w, h, pg, x, y, fmt, idx ? idx + off : fmt && img->rp_idx && img->rp_pal ? img->rp_idx + off : NULL, img->rp_pal, tab );
		e->key = key;
		e->off = koff;
		e->w = (uint16_t)w;
		e->h = (uint16_t)h;
		e->pg = (uint8_t)pg;
		e->qx = (uint8_t)x;
		e->qy = (uint8_t)y;
		e->masked = (uint8_t)( 2 + fmt );
		e->gen = rp_page[pg].gen;
		rp_count.upload_kib += ( w * h * 2 ) >> 10;
		rp_count.upload_us += rp_time_us() - t0;
	}
	rp_page[e->pg].used = tr.framecount;
	rp_page[e->pg].live += (uint32_t)e->w * e->h * 2;
	return e;
}

/* The state block for a 2D draw from page `pg` (`fmt` 1: ARGB4444) in the
   engine's render mode. */
static int rp_2d_linear; /* the half-size pictures: filtered (TEX1 bit 31) */

static void RP_2DState( int pg, int fmt )
{
	int         mode = vid.rendermode, blend;
	uint32_t    tsp = ( RP_TSP_WORLD & 0x03FFF07Fu ) | 7u << 9 | 1u << 7; /* depth always, not written */
	const void *what;

	if( mode == kRenderTransAdd || mode == kRenderGlow )
		blend = 2;
	else if( mode == kRenderScreenFadeModulate )
		blend = 3;
	else if( fmt || ( rp_2d_colour >> 24 ) != 0xFF || mode != kRenderNormal )
		blend = 1;
	else
		blend = 0;
	what = (const uint8_t *)&rp_page[pg] + 32 + fmt * 4 + blend + ( rp_2d_linear ? 8 : 0 ); /* 32-47: below the effects' 48 */
	if( what == rp_state )
		return;
	RP_LmFlush(); /* the lit walls before the 2D layer */
	switch( blend )
	{
	case 1: tsp |= 4u << 29 | 5u << 26 | 1u << 13; break; /* source alpha, one minus it */
	case 2: tsp |= 1u << 29 | 1u << 26 | 1u << 13; break; /* one, one */
	case 3: tsp |= 2u << 29 | 0u << 26 | 1u << 13; break; /* the destination's colour, zero */
	default: tsp |= 1u << 29; break;                       /* one, zero */
	}
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_GFX_STATE );
	RP_Put( tsp );
	RP_Put( RP_ISP_ALWAYS );
	RP_Put( fmt ? ( RP_TEX0_PAGE & ~( 7u << 13 )) | 4u << 13 : RP_TEX0_PAGE );
	RP_Put( rp_2d_linear ? RP_TEX1_LINEAR( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ) : RP_TEX1( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ));
	RP_Put( fmt && !rp_tex2_plain ? 0x86084610u : RP_TEX2 ); /* 4444: the texel's alpha (rp-glesfeat L) */
	RP_Put( RP_CLIP );
	RP_Put( 0 );
	RP_Whole();
}

/* A quad in the game's 640 x 480 picture, page texels (u0, v0)-(u1, v1). */
static void RP_2DQuad( float x0, float y0, float x1, float y1, float u0, float v0, float u1, float v1 )
{
	const float gx[4] = { x0, x1, x0, x1 }, gy[4] = { y0, y0, y1, y1 };
	const float gu[4] = { u0, u1, u0, u1 }, gv[4] = { v0, v0, v1, v1 };

	RP_Put( RP_GFX_STRIP( 4 ) | rp_noz);
	for( int k = 0; k < 4; k++ )
	{
		RP_Put( RP_Bits( gy[k] ));                   /* the upright picture: x = the game's y */
		RP_Put( RP_Bits( (float)RP_GAME_W - gx[k] )); /* y = 640 - the game's x */
		if( !rp_noz ) RP_Put( 0x3F800000u );
		RP_Put( 0x3F800000u );
		RP_Put( rp_2d_colour );
		RP_Put( RP_Bits( gu[k] * ( 1.0f / RP_PAGE_SIDE )));
		RP_Put( RP_Bits( gv[k] * ( 1.0f / RP_PAGE_SIDE )));
	}
	RP_Whole();
	rp_count.tris += 2;
	rp_count.verts += 4;
	rp_count.pics++;
}

/* R_DrawStretchPic: the picture's (s1, t1)-(s2, t2) (0 to 1) at (x, y), w x h. */
/*
The menu's background picture in borrowed texture memory (the owner, run
317, 2026-11-21: "make the picture only show up whenever the menu is called
... and then clear the image when the menu is closed, it shouldnt need to be
using memory the entire time. Rule 0 that"; and he keeps the look: the
agreed picture behind every menu, in a game too).

The 2700G has 7 MiB of texture memory and the map nearly fills it. Run 316
drew the 640 x 480 picture as upstream's renderers would, as textures of
its own: six tiles in the atlas, which pushed the map's pictures out while
the menu was up; they came back one by one after it (half of that run's
1,202 texture rebuilds, its frames over 50 ms from 170 to 275). Run 317
showed the paused game instead and the owner refused the look.

Rule 0: while a menu covers the screen no face of the world is drawn, so
the map's own textures lie idle in the chip. Ours borrows the first two
pages of their room: the bytes there are copied to the program's memory,
the picture is written in their place (512 x 480 in the first page, the
rest in the second) and drawn from there; the first face of the world
drawn again copies the bytes back, and every texture of the map is where it
was: nothing pushed out, nothing rebuilt, and the picture holds video memory
only while a menu is up. The cost is two copies of 1 MiB, at the moments
the menu opens and closes. If the copy cannot be had (no memory, the chip
lost) the picture is drawn the old way, in tiles.
*/
static struct
{
	int      on;     /* the picture has the two pages */
	int      pg0;    /* the first of them (the room's last two) */
	uint8_t *save;   /* the map's bytes that lay there, if any did */
	uint32_t saved;  /* how many */
	uint32_t serial; /* the picture there */
	int      gen;    /* the textures' room as it was borrowed (RP_TexReset moves it on) */
	uint32_t borrows, returns, copied_us, refused;
} rp_borrow;

/* The map's textures back in their place, before anything of the world is
   drawn or placed. */
static void RP_BorrowReturn( void )
{
	uint64_t t0;

	if( !rp_borrow.on )
		return;
	t0 = rp_time_us( );
	/* no frame in flight still draws the picture; and a room emptied since
	   (a new map, the chip rebuilt) wants nothing back */
	rp_gfx_wait( );
	if( rp_borrow.save && rp_borrow.gen == rp_tex_gen && RP_ChipWrite( ))
		memcpy((uint8_t *)rp_g.tex + (size_t)rp_borrow.pg0 * RP_PAGE_BYTES, rp_borrow.save, rp_borrow.saved );
	free( rp_borrow.save );
	rp_borrow.save = NULL;
	rp_borrow.on = 0;
	rp_borrowed = 0;
	rp_borrow.serial = 0;
	rp_borrow.returns++;
	rp_borrow.copied_us += (uint32_t)( rp_time_us( ) - t0 );
	fprintf( stderr, "rp-emit: the menu's picture: its two pages given back, %u KiB of the map's textures copied back into them (%u borrowed, %u returned, %u ms of copies so far; %u times no memory for it)\n",
		rp_borrow.saved >> 10, rp_borrow.borrows, rp_borrow.returns, rp_borrow.copied_us / 1000u, rp_borrow.refused );
	rp_borrow.saved = 0;
}

/* The picture `pic` (`pw` x `ph`, no alpha) in the two borrowed pages: 1,
   or 0 if it cannot be (drawn the old way then). */
static int RP_BorrowPicture( const image_t *pic, int pw, int ph )
{
	const pixel_t *src = NULL;
	const byte    *idx = NULL;
	pixel_t        c16[256];
	int            compact;
	uint64_t       t0;

	if( rp_tex_pages < 2 || pw > 2 * RP_PAGE_SIDE || ph > RP_PAGE_SIDE || gEngfuncs.Sys_CheckParm( "-rpnoborrow" ))
		return 0;
	if( rp_borrow.on && rp_borrow.serial == pic->rp_serial )
		return 1;
	t0 = rp_time_us( );
	/* the picture as it is kept (8 bits a texel and its colours), read a
	   band at a time below: decoded whole it is 0.9 MiB more of the heap at
	   the moment the saved pages take 1 MiB (the emulator's play run with
	   both: out of memory at the menu) */
	compact = R_TexCompact( (image_t *)pic, c16, &idx );
	if( !compact )
	{
		src = R_TexPix( (image_t *)pic, 0 );
		if( !src )
			return 0;
	}
	if( !rp_borrow.on )
	{
		/* The room's LAST two pages (the owner, 2026-11-22: "close all opened
		   things": the heap's thin margin with the menu up, 36 MiB of 37.6
		   when the first two pages' megabyte was saved). The room is filled
		   from its start and run 317's map used 2,157 of its 3,072 KiB: the
		   end of it is empty, or nearly. Only the bytes the map has really
		   put in the two pages are saved (none, or a few tens of KiB),
		   so the picture costs the heap next to nothing. */
		const int      pg0 = rp_pages + rp_tex_pages - 2;
		const uint32_t off0 = (uint32_t)( rp_tex_pages - 2 ) * RP_PAGE_BYTES;
		uint32_t       used = rp_tex_used > off0 ? rp_tex_used - off0 : 0;
		uint8_t       *save = NULL;

		if( used > 2 * RP_PAGE_BYTES )
			used = 2 * RP_PAGE_BYTES;
		/* nothing to be had for what must be saved: no picture this frame,
		   asked again at the next. Never the old tiles from here: they decode
		   the picture whole, 0.9 MiB the heap would refuse too, and the
		   engine stops at a refused block. */
		if( used && !( save = malloc( used )))
		{
			rp_borrow.refused++;
			return -1;
		}
		/* no frame in flight still reads the map's textures */
		rp_gfx_wait( );
		if( !RP_ChipWrite( ))
		{
			free( save );
			return 0;
		}
		if( used )
			memcpy( save, (uint8_t *)rp_g.tex + (size_t)pg0 * RP_PAGE_BYTES, used );
		rp_borrow.save = save;
		rp_borrow.saved = used;
		rp_borrow.pg0 = pg0;
		rp_borrow.on = 1;
		rp_borrowed = 1;
		rp_borrow.gen = rp_tex_gen;
		rp_borrow.borrows++;
	}
	if( compact )
	{
		static pixel_t band[8 * 2 * RP_PAGE_SIDE];

		for( int y = 0; y < ph; y += 8 )
		{
			const int rows = ph - y < 8 ? ph - y : 8;

			for( int r = 0; r < rows; r++ )
				for( int x = 0; x < pw; x++ )
					band[r * pw + x] = c16[idx[( y + r ) * pw + x]];
			RP_PicUpload( band, pw, pw < RP_PAGE_SIDE ? pw : RP_PAGE_SIDE, rows, rp_borrow.pg0, 0, y >> 3, 0, NULL, NULL, NULL );
			if( pw > RP_PAGE_SIDE )
				RP_PicUpload( band + RP_PAGE_SIDE, pw, pw - RP_PAGE_SIDE, rows, rp_borrow.pg0 + 1, 0, y >> 3, 0, NULL, NULL, NULL );
		}
	}
	else
	{
		RP_PicUpload( src, pw, pw < RP_PAGE_SIDE ? pw : RP_PAGE_SIDE, ph, rp_borrow.pg0, 0, 0, 0, NULL, NULL, NULL );
		if( pw > RP_PAGE_SIDE )
			RP_PicUpload( src + RP_PAGE_SIDE, pw, pw - RP_PAGE_SIDE, ph, rp_borrow.pg0 + 1, 0, 0, 0, NULL, NULL, NULL );
	}
	rp_borrow.serial = pic->rp_serial;
	rp_borrow.copied_us += (uint32_t)( rp_time_us( ) - t0 );
	return 1;
}

void RP_EmitPic( float x, float y, float w, float h, float s1, float t1, float s2, float t2, const image_t *pic )
{
	int            fmt, pw, ph, tile;
	float          S1, T1, S2, T2;

	if( rp_emit_on != 2 || !pic || !R_TexLoaded( pic ) || w <= 0.0f || h <= 0.0f || s1 >= s2 || t1 >= t2 )
		return;
	fmt = R_TexHasAlpha( pic );
	pw = pic->width;
	ph = pic->height;
	if( pw < 1 || ph < 1 )
		return;
	/* a picture larger than a page in tiles of 256 (the menu's background) */
	tile = pw <= RP_PAGE_SIDE && ph <= RP_PAGE_SIDE ? 0 : 256;
	S1 = s1 * pw;
	S2 = s2 * pw;
	T1 = t1 * ph;
	T2 = t2 * ph;
	/* the menu's background: in the pages borrowed from the map's textures
	   (above), two quads, the first page's 512 columns and the rest */
	const int borrowed = tile && !fmt ? RP_BorrowPicture( pic, pw, ph ) : 0;

	if( borrowed < 0 )
		return; /* no memory for it now: the menu's words on a plain ground this frame */
	if( borrowed )
	{
		for( int k = 0; k < 2; k++ )
		{
			float c0 = k * (float)RP_PAGE_SIDE, c1 = k ? (float)pw : (float)( pw < RP_PAGE_SIDE ? pw : RP_PAGE_SIDE );
			float a0 = S1 > c0 ? S1 : c0, a1 = S2 < c1 ? S2 : c1;

			if( a0 >= a1 )
				continue;
			RP_2DState( rp_borrow.pg0 + k, 0 );
			/* (the half texels as below: R327, kit 98) */
			RP_2DQuad( x + ( a0 - S1 ) / ( S2 - S1 ) * w, y, x + ( a1 - S1 ) / ( S2 - S1 ) * w, y + h,
				a0 - c0 - 0.5f, T1 + 0.5f, a1 - c0 - 0.5f, T2 + 0.5f );
		}
		return;
	}
	for( int ty = tile ? (int)T1 / tile * tile : 0; ty < ( tile ? T2 : 1 ); ty += tile ? tile : 1 )
		for( int tx = tile ? (int)S1 / tile * tile : 0; tx < ( tile ? S2 : 1 ); tx += tile ? tile : 1 )
		{
			int   tw = tile ? ( pw - tx < tile ? pw - tx : tile ) : pw;
			int   th = tile ? ( ph - ty < tile ? ph - ty : tile ) : ph;
			float a0 = S1 > tx ? S1 : tx, a1 = S2 < tx + tw ? S2 : tx + tw;
			float b0 = T1 > ty ? T1 : ty, b1 = T2 < ty + th ? T2 : ty + th;
			const rp_skin_t *e;

			if( a0 >= a1 || b0 >= b1 )
				continue;
			/* run 318 (the owner's play: the menu 8-14 fps, "minor texture
			   glitching"; its log: 15 of the map's pages dropped in one menu
			   visit): the menu's animated banner, Half-Life's 4 blurred
			   logo pictures of 507 x 270 and their 4 blip copies, does not
			   fit beside the map in video memory, so each flash between the
			   two sets pushed pages out and the map's textures were placed
			   again after the menu. Blurred glows hold no detail finer than
			   a few texels: kept at half size each way (a quarter of the
			   room, each texel the mean of its 2 x 2) and drawn filtered,
			   both sets fit at once and nothing churns */
			const int half = fmt && !tile && strstr( pic->name, "_blurred" ) != NULL;

			rp_pic_half = half;
			rp_pic_srcw = tw;
			rp_pic_srch = th;
			e = RP_PicPlace( (image_t *)pic, (uint32_t)( ty * pw + tx ), pw, half ? ( tw + 1 ) / 2 : tw, half ? ( th + 1 ) / 2 : th, fmt );
			rp_pic_half = 0;
			if( !e )
			{
				rp_count.pics_skipped++;
				continue;
			}
			rp_2d_linear = half;
			RP_2DState( e->pg, fmt );
			rp_2d_linear = 0;
			if( half )
			{
				/* filtered: texel centres at k + 0.5 of the half picture */
				RP_2DQuad( x + ( a0 - S1 ) / ( S2 - S1 ) * w, y + ( b0 - T1 ) / ( T2 - T1 ) * h,
					x + ( a1 - S1 ) / ( S2 - S1 ) * w, y + ( b1 - T1 ) / ( T2 - T1 ) * h,
					e->qx * 8 + ( a0 - tx ) * 0.5f, e->qy * 8 + ( b0 - ty ) * 0.5f, e->qx * 8 + ( a1 - tx ) * 0.5f, e->qy * 8 + ( b1 - ty ) * 0.5f );
				continue;
			}
			{
				/* R327 (the owner, 2026-11-18: the flashlight's line, the
				   weapon HUD's digits with a sliver of the next glyph; run
				   130's picture: the flashlight icon's last columns are the
				   sprite's 142-144, so every picture showed texels S1+1 to
				   S2): the 2700G's point sampling takes the texel NEAREST
				   the coordinate (texel k from k - 0.5 to k + 0.5), where
				   GoldSrc's software renderer copies texel floor(u). Each
				   coordinate is moved back half a texel, so a pixel's
				   centre lands on its own texel's centre: the picture's
				   columns exactly, nothing of its neighbours. Kit 76's half
				   texel inside the tile's edges was this same shift seen
				   at a tile's edge, and goes. */
				/* kit 98 (the owner on kit 97: "lines ... above and
				   below"): the chip samples at each pixel's corner, not its
				   centre, and the picture is turned: across the game's
				   screen that corner is the pixel's right edge (half a
				   texel back puts it at the texel's centre), up and down it
				   is the pixel's top edge (half a texel forward does). Both
				   now land on texel centres, away from any boundary. */
				float ua = a0 - tx - 0.5f, ub = a1 - tx - 0.5f;
				float va = b0 - ty + 0.5f, vb = b1 - ty + 0.5f;

				RP_2DQuad( x + ( a0 - S1 ) / ( S2 - S1 ) * w, y + ( b0 - T1 ) / ( T2 - T1 ) * h,
					x + ( a1 - S1 ) / ( S2 - S1 ) * w, y + ( b1 - T1 ) / ( T2 - T1 ) * h,
					e->qx * 8 + ua, e->qy * 8 + va, e->qx * 8 + ub, e->qy * 8 + vb );
			}
		}
}

/*
Kit 74 (R320, the owner's play run on kit 72: c1a0e's electricity did not
show): the triangles of the engine's triangle interface (sprites, beams,
particles, the client's own drawing) on the 2700G. Each is what the
software renderer rasterises (R_DrawTriangle's three corners, already on
the screen, clipped to it and to the near plane): the bound frame placed
in the atlas as the HUD's pictures are, the TriAPI colour on each corner,
the render mode as the models' blends, depth compared with the world and
not written (a glow tested against nothing: GoldSrc hides glows by a
trace). A frame with no picture (or no room) is drawn plainly in its
colour, never dropped.
*/
static const image_t *rp_tri_image; /* GL_Bind's last */

void RP_EmitBind( const image_t *image )
{
	rp_tri_image = image;
}

/* The white texel the plain draws use (RP_EmitFill's): its page, or -1. */
static int RP_WhitePage( void )
{
	if( rp_white.pg < 0 || rp_white.gen != rp_page[rp_white.pg].gen )
	{
		int       pg = RP_AtlasAlloc( 1, 1, &rp_white.qx, &rp_white.qy );
		uint32_t *dst;

		rp_white.pg = pg;
		if( pg < 0 )
			return -1;
		dst = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES ) + RP_Interleave( rp_white.qx, rp_white.qy ) * 32;
		if( RP_ChipWrite( ))
			for( int k = 0; k < 32; k++ )
				dst[k] = 0xFFFFFFFFu;
		rp_white.gen = rp_page[pg].gen;
	}
	rp_page[rp_white.pg].used = tr.framecount;
	return rp_white.pg;
}

static void RP_FxState( int pg, int fmt, int mode )
{
	int         blend, glow = mode == kRenderGlow;
	uint32_t    tsp = RP_TSP_WORLD | RP_TSP_GOURAUD;
	const void *what;

	if( mode == kRenderTransAdd || glow )
		blend = 2;
	else if( fmt || ( rp_2d_colour >> 24 ) != 0xFF || mode != kRenderNormal )
		blend = 1;
	else
		blend = 0;
	what = (const uint8_t *)&rp_page[pg] + 48 + fmt * 4 + ( glow ? 3 : blend );
	if( what == rp_state )
		return;
	RP_LmFlush(); /* the lit walls before what blends over them */
	if( blend )
		/* the models' blends: source alpha over one minus it, or one plus
		   one; blending on (bit 13), depth not written (bit 7) */
		tsp = ( tsp & 0x03FFFFFFu ) | ( blend == 2 ? 1u << 29 | 1u << 26 : 4u << 29 | 5u << 26 ) | 1u << 13 | 1u << 7;
	if( glow )
		tsp = ( tsp & ~( 7u << 9 )) | 7u << 9; /* depth always */
	rp_state = what;
	rp_count.states++;
	RP_Put( RP_GFX_STATE );
	RP_Put( tsp );
	RP_Put( RP_ISP_ALWAYS );
	RP_Put( fmt ? ( RP_TEX0_PAGE & ~( 7u << 13 )) | 4u << 13 : RP_TEX0_PAGE );
	RP_Put( RP_TEX1( rp_g.tex_at + (uint32_t)pg * RP_PAGE_BYTES ));
	RP_Put( fmt && !rp_tex2_plain ? 0x86084610u : RP_TEX2 ); /* 4444: the texel's alpha */
	RP_Put( RP_CLIP );
	RP_Put( 0 );
	RP_Whole();
}

/* The bound frame's place and the state for an effect triangle (kit 74);
   0: not drawn (no room at all). */
typedef struct
{
	int32_t u0, v0, smin, tmin, smax, tmax;
} rp_fxctx_t;

static int RP_FxSetup( rp_fxctx_t *c )
{
	const image_t   *img = rp_tri_image;
	const int        mode = vid.rendermode;
	const rp_skin_t *e = NULL;
	int              fmt = 0, pg;

	if( img && R_TexLoaded( img ) && img->width >= 1 && img->height >= 1
		&& img->width <= RP_PAGE_SIDE && img->height <= RP_PAGE_SIDE )
	{
		fmt = mode != kRenderTransAdd && mode != kRenderGlow && R_TexHasAlpha( img );
		e = RP_PicPlace( (image_t *)img, 0, img->width, img->width, img->height, fmt );
	}
	if( e )
	{
		/* half a texel inside the frame's edges (kit 76: the filter read
		   the atlas texels beside a tile, the HUD's lines) */
		pg = e->pg;
		c->u0 = e->qx * 8 << 16;
		c->v0 = e->qy * 8 << 16;
		c->smin = 1 << 15;
		c->tmin = 1 << 15;
		c->smax = ( e->w << 16 ) - ( 1 << 15 );
		c->tmax = ( e->h << 16 ) - ( 1 << 15 );
	}
	else
	{
		/* no picture (or no room): drawn plainly in its colour */
		pg = RP_WhitePage( );
		if( pg < 0 )
		{
			rp_count.fx_skipped++;
			return 0;
		}
		fmt = 0;
		c->u0 = ( rp_white.qx * 8 + 4 ) << 16;
		c->v0 = ( rp_white.qy * 8 + 4 ) << 16;
		c->smin = c->tmin = c->smax = c->tmax = 0;
		rp_count.fx_plain++;
	}
	RP_FxState( pg, fmt, mode );
	return 1;
}

/* One corner's texel words. */
static inline void RP_FxTexel( const rp_fxctx_t *c, int32_t s, int32_t t )
{
	s = s < c->smin ? c->smin : s > c->smax ? c->smax : s;
	t = t < c->tmin ? c->tmin : t > c->tmax ? c->tmax : t;
	RP_Put( RP_FixBits( c->u0 + s, 16 + 9 )); /* page texels / 512 */
	RP_Put( RP_FixBits( c->v0 + t, 16 + 9 ));
}

static void RP_FxTriangle( void )
{
	const finalvert_t *v[3] = { aliastriangleparms.a, aliastriangleparms.b, aliastriangleparms.c };
	rp_fxctx_t         c;

	if( !RP_FxSetup( &c ))
		return;
	RP_Put( RP_GFX_STRIP( 3 ) | rp_noz);
	for( int i = 0; i < 3; i++ )
	{
		/* the upright picture: x = the game's y, y = 640 - the game's x */
		RP_Put( RP_FixBits( v[i]->v, 0 ));
		RP_Put( RP_FixBits( RP_GAME_W - v[i]->u, 0 ));
		RP_Put( RP_FixBits( v[i]->zi, 31 )); /* 1/z, the world's own units */
		if( !rp_noz ) RP_Put( RP_FixBits( v[i]->zi, 31 ));
		RP_Put( rp_2d_colour );
		RP_FxTexel( &c, v[i]->s, v[i]->t );
	}
	RP_Whole();
	rp_count.fx_tris++;
	rp_count.tris++;
	rp_count.verts += 3;
}

/*
Kit 77 (R324, run play75: c1a0e's start spent ~12% of its frame in the
triangle interface's per-corner floats and doubles: R_SetupFinalVert's
transform and projection, TriTexCoord2f's fmodf and double compares,
_TriColor4f's packing for the software spans). Iterate (rule 0):
ref_soft makes every corner a finalvert in floats for its rasteriser;
ref_gl hands GL immediate-mode floats; ours, with the 2700G drawing: each
corner into view space and onto the screen in fixed point as the world's
corners are (RP_ViewPoint, RP_ProjectS: the effects meet the walls in the
same numbers), the texel from the coordinate in 16.16, no colour packing
the 2700G never reads. A triangle with a corner behind the near plane
goes the software renderer's way (it clips). The same triangles, their
corners within a sub-pixel of the old ones (visually identical).
*/
typedef struct
{
	int32_t  p[3]; /* 16.16 world units (kit 78: the beams hand these over) */
	int      s, t;
	uint32_t w[3];
	int32_t  st[2]; /* the TA's x, y in 28.4: the off-screen cull */
	int      front;
} rp_tv_t;

static rp_tv_t rp_tv[3];
static int     rp_tv_mode, rp_tv_n, rp_tv_count;

void RP_TriBegin( int mode )
{
	rp_tv_mode = mode;
	rp_tv_n = rp_tv_count = 0;
	RP_ViewFixed();
}

/* TriTexCoord2f's texel, as its float code makes it (within a texel). */
int RP_TriCoord( float u, int size )
{
	uint32_t bits;

	__builtin_memcpy( &bits, &u, sizeof( bits ));
	if(( bits & 0x7FFFFFFFu ) >= 0x46000000u ) /* |u| >= 8192, or not finite: the float way */
	{
		double u1 = 0;

		u = fmodf( u, 10 );
		if( u < 1000 && u > -1000 )
			u1 = u;
		while( u1 < 0 )
			u1 = u1 + 1;
		while( u1 > 1 )
			u1 = u1 - 1;
		return size * bound( 0.01, u1, 0.99 );
	}
	/* a whole number: fmodf( u, 10 ) then the loops make 1 above 0 (0 for
	   a multiple of 10), else 0; then bound( 0.01, u, 0.99 ) */
	return RP_TriCoordFix( RP_Fix16( u ), size );
}

/* As RP_TriCoord, the coordinate already 16.16 (|q| below 8192). */
int RP_TriCoordFix( int32_t q, int size )
{
	uint32_t f = (uint32_t)q & 0xFFFFu;

	if( !f )
	{
		int32_t whole = q >> 16;

		f = whole > 0 && whole % 10 ? 0x10000u : 0;
	}
	f = f < 655u ? 655u : f > 64880u ? 64880u : f;
	return (int)(( (uint32_t)size * f ) >> 16 );
}

static void RP_TriEmit( const rp_tv_t *a, const rp_tv_t *b, const rp_tv_t *c3 )
{
	const rp_tv_t *v[3] = { a, b, c3 };
	rp_fxctx_t     c;

	if( !a->front && !b->front && !c3->front )
	{
		/* kit 114: wholly behind the near plane, as R_RenderTriangle's
		   "completely clipped" (every corner ALIAS_Z_CLIP), with no float
		   set-up first (run 235: 100-168 such triangles a fight frame,
		   R_SetupFinalVert's floats ~2% of the play) */
		rp_count.fx_behind++;
		return;
	}
	if( !a->front || !b->front || !c3->front )
	{
		/* across the near plane: the software renderer's set-up and clip */
		finalvert_t fv[3];

		for( int i = 0; i < 3; i++ )
			R_SetupFinalVert( &fv[i], v[i]->p[0] * ( 1.0f / 65536.0f ), v[i]->p[1] * ( 1.0f / 65536.0f ),
				v[i]->p[2] * ( 1.0f / 65536.0f ), 0, v[i]->s, v[i]->t );
		R_RenderTriangle( &fv[0], &fv[1], &fv[2] );
		rp_count.fx_cut++;
		return;
	}
	if( RP_OffScreen( a->st, b->st, c3->st ))
	{
		rp_count.fx_off++; /* as R_RenderTriangle's "completely clipped" */
		return;
	}
	if( !RP_FxSetup( &c ))
		return;
	RP_Put( RP_GFX_STRIP( 3 ) | rp_noz);
	for( int i = 0; i < 3; i++ )
	{
		RP_Put( v[i]->w[0] );
		RP_Put( v[i]->w[1] );
		if( !rp_noz ) RP_Put( v[i]->w[2] );
		RP_Put( v[i]->w[2] );
		RP_Put( rp_2d_colour );
		RP_FxTexel( &c, v[i]->s << 16, v[i]->t << 16 );
	}
	RP_Whole();
	rp_count.fx_tris++;
	rp_count.fx_fixed++;
	rp_count.tris++;
	rp_count.verts += 3;
}

static void RP_TriCorner( rp_tv_t *v, const int32_t p[3], int s, int t )
{
	int32_t q[3];

	v->p[0] = p[0];
	v->p[1] = p[1];
	v->p[2] = p[2];
	v->s = s;
	v->t = t;
	RP_ViewPoint( (const int32_t( * )[3] )rp_view.m, rp_view.o, p, q );
	v->front = q[2] >= ALIAS_Z_CLIP_PLANE * RP_ONE16; /* the software's near plane */
	if( v->front )
		RP_ProjectS( q, v->w, v->st );
}

/*
R352 (the owner, runs 293-298: wall patches flashing bright and back,
worst in fire fights; run 297's log: 35 faces changed drawing way between
two frames in a row, decal faces drawn the other way in 13 of 20 reports).
A face with a decal (a bullet hole, blood, a scorch) used to leave the
texture-and-lightmap path for the lit surface, the only place a decal
could be baked in, and the two paths do not look alike: the face's whole
rectangle changed look the frame a decal landed on it and back when the
decal was retired.

Iterate, don't imitate (rule 0). ref_soft bakes each decal into the
face's cached surface (the surface built again whenever its light
changes); ref_gl draws the decal as its own blended polygon over the
face. Ours: the face always by the one path, and each decal as a
see-through fan over it by the effects' own words (kit 74: the decal's
picture once in the atlas as ARGB4444, source alpha over one minus it,
depth compared and not written), its corners the engine's own clipped
ones (clipped to its picture here: RP_DecalPoly) through the face's view in fixed point. No face
changes way, no surface is rebuilt for a decal, and a decal costs its few
corners a frame. The decal's depth is nudged toward the eye (its 1/z up
by 2^-9 of itself: a polygon offset), since its corners are not the
wall's and equal depths would speckle. The colour is the picture's own,
unlit, as ref_soft's baked decals are.
*/
#define RP_DECAL_BIAS 0x4000u /* float bits: 2^14 of the mantissa's 2^23 steps */

/*
A decal's polygon on its face: the face's own corners in the decal's
texture space (0 to 1 across the picture), clipped to the picture's
square. The engine's R_DecalSetupVerts cannot serve here: it clips the
GL renderer's per-face polygon (`surf->polys`), which the software
renderer never builds, and with none it reads a count nobody set (the
emulator's walk hung in its lighting loop). Ours takes the corners from
the map's edges, as every world face here does, with the same basis and
scale (R_DecalComputeBasis; the decal's scale over the picture's size),
and clips by Sutherland and Hodgman's four passes in place. `out`: x, y,
z, s, t a corner; the count (0: nothing of the decal on the face).
*/
#define RP_DECAL_CORNERS 72 /* a face's 64 at most, four clips each adding one */

static int RP_DecalClipEdge( const float ( *in )[5], int n, float ( *out )[5], int axis, float sign, float limit )
{
	int k = 0;

	for( int i = 0; i < n && k < RP_DECAL_CORNERS - 1; i++ )
	{
		const float *a = in[i], *b = in[( i + 1 ) % n];
		float        da = ( a[axis] - limit ) * sign, db = ( b[axis] - limit ) * sign;

		if( da >= 0.0f )
			memcpy( out[k++], a, sizeof( out[0] ));
		if(( da >= 0.0f ) != ( db >= 0.0f ))
		{
			float f = da / ( da - db );

			for( int j = 0; j < 5; j++ )
				out[k][j] = a[j] + ( b[j] - a[j] ) * f;
			k++;
		}
	}
	return k;
}

static int RP_DecalPoly( decal_t *d, const msurface_t *fa, const image_t *img, float ( *out )[5] )
{
	static float tmp[RP_DECAL_CORNERS][5];
	const model_t *mod = RI.currentmodel ? RI.currentmodel : WORLDMODEL;
	vec3_t         basis[3];
	float          sx = d->scale / (float)img->width, sy = d->scale / (float)img->height, dx, dy;
	int            n = fa->numedges;

	if( n < 3 || n > 64 || fa->firstedge < 0 || fa->firstedge + n > mod->numsurfedges )
		return 0;
	R_DecalComputeBasis( (msurface_t *)fa, d->flags, basis );
	VectorScale( basis[0], sx, basis[0] );
	VectorScale( basis[1], sy, basis[1] );
	dx = DotProduct( d->position, basis[0] );
	dy = DotProduct( d->position, basis[1] );
	for( int i = 0; i < n; i++ )
	{
		int            lindex = mod->surfedges[fa->firstedge + i];
		const medge16_t *e = &mod->edges16[lindex > 0 ? lindex : -lindex];
		const float     *p = mod->vertexes[lindex > 0 ? e->v[0] : e->v[1]].position;

		VectorCopy( p, out[i] );
		out[i][3] = DotProduct( p, basis[0] ) - dx + 0.5f;
		out[i][4] = DotProduct( p, basis[1] ) - dy + 0.5f;
	}
	n = RP_DecalClipEdge( (const float( * )[5])out, n, tmp, 3, 1.0f, 0.0f );
	n = RP_DecalClipEdge( (const float( * )[5])tmp, n, out, 3, -1.0f, 1.0f );
	n = RP_DecalClipEdge( (const float( * )[5])out, n, tmp, 4, 1.0f, 0.0f );
	n = RP_DecalClipEdge( (const float( * )[5])tmp, n, out, 4, -1.0f, 1.0f );
	return n >= 3 ? n : 0;
}

/*
R352 (the owner, 2026-11-22: "looks at how the Half Life files are telling
you its supposed to work, and rule 0 it"). Half-Life's decals.wad: all 222
decals are type 0x43, one colour (the palette's last entry: blood 22,0,0,
alien blood 204,184,97, a bullet hole black) with the pixel's index its
opacity, 256 levels; GoldSrc's GL renderer draws them over the wall's
texture and then multiplies the wall's lightmap over both, so a decal is
lit as its wall (a dark corridor's blood is dark, the flashlight shows it).
ref_soft pastes them into the surface after the light (unlit, 8 levels of
opacity): a shortcut, not Half-Life's look. Ours: the wall is one pass,
texture x lightmap on the 2700G's two units (kit 112); the decal the same
way, its picture (ARGB4444, the opacity from the index: 16 levels) times
the SAME lightmap texels on unit 1, blended over the wall. The light is
the wall's to the texel (styles, flicker, dynamic lights) with no second
pass and no CPU work a pixel. A corner's lightmap place comes from its
point as the face's own (luxels from lightmapmins, 24.8).
*/
static rp_tex_t rp_decal_lit_mark;

static void RP_DecalLitStrip( const rp_skin_t *e, int lpg, int lqx, int lqy, int n,
	const int32_t ( *cq )[3], const int ( *cs )[2], const int32_t ( *cl )[2] )
{
	static int last_pg = -1, last_lpg = -1;
	int        lo = 1, hi = n - 1;
	int32_t    smax = ( e->w << 16 ) - ( 1 << 15 ), tmax = ( e->h << 16 ) - ( 1 << 15 );

	if( rp_state != &rp_decal_lit_mark || last_pg != e->pg || last_lpg != lpg )
	{
		RP_LmFlush( ); /* the second passes queued so far before what blends */
		rp_state = &rp_decal_lit_mark;
		last_pg = e->pg;
		last_lpg = lpg;
		rp_count.states++;
		RP_Put( RP_GFX_STATE2 );
		/* kit 112's two-unit word, blended source alpha over one minus it
		   (bit 13), depth not written (bit 7), as the effects' blends */
		RP_Put(((( RP_TSP_WORLD & ~0x000F0000u ) | 1u << 16 | 2u << 17 ) & 0x03FFFFFFu ) | 4u << 29 | 5u << 26 | 1u << 13 | 1u << 7 );
		RP_Put( RP_ISP_ALWAYS );
		RP_Put(( RP_TEX0_PAGE & ~( 7u << 13 )) | 4u << 13 ); /* ARGB4444 */
		RP_Put( RP_TEX1( rp_g.tex_at + (uint32_t)e->pg * RP_PAGE_BYTES ));
		RP_Put( rp_tex2_plain ? RP_TEX2 : 0x86084610u ); /* the texel's alpha */
		RP_Put( RP_TEX0_PAGE );
		RP_Put( RP_TEX1_LINEAR( rp_g.tex_at + (uint32_t)lpg * RP_PAGE_BYTES ));
		RP_Put( RP_TEX2_UNIT1 );
		RP_Put( RP_CLIP );
		RP_Put( 0 );
		RP_Whole();
	}
	RP_Put( RP_GFX_STRIP( n ) | rp_noz );
	for( int k = 0; k < n; k++ )
	{
		int      c = k == 0 ? 0 : ( k & 1 ) ? lo++ : hi--;
		uint32_t w[3];
		int32_t  st[2], s = cs[c][0] << 16, t = cs[c][1] << 16;

		RP_ProjectS( cq[c], w, st );
		w[2] += RP_DECAL_BIAS;
		s = s < ( 1 << 15 ) ? 1 << 15 : s > smax ? smax : s;
		t = t < ( 1 << 15 ) ? 1 << 15 : t > tmax ? tmax : t;
		RP_Put( w[0] );
		RP_Put( w[1] );
		if( !rp_noz ) RP_Put( w[2] );
		RP_Put( w[2] );
		RP_Put( 0xFFFFFFFFu );
		RP_Put( RP_FixBits(( e->qx * 8 << 16 ) + s, 16 + 9 ));
		RP_Put( RP_FixBits(( e->qy * 8 << 16 ) + t, 16 + 9 ));
		RP_Put( RP_FixBits(( lqx << 11 ) + ( cl[c][0] >> 4 ) + 128, 8 + 9 ));
		RP_Put( RP_FixBits(( lqy << 11 ) + ( cl[c][1] >> 4 ) + 128, 8 + 9 ));
	}
	RP_Whole();
	rp_count.tris += n - 2;
	rp_count.verts += n;
	rp_count.decals_lit++;
}

/*
R352's cost (the owner, 2026-11-22: "can we rule 0 the inpact of that hit
away?"). Run 328's samples: ~4% of the play in the decals' floats (the
face's corners clipped to the picture in soft float, RP_DecalClipEdge and
the dot products, every decal every frame). Half-Life's GL renderer builds
a decal's polygon once (R_DecalCreatePoly) and keeps it. Ours keeps it
too, in the numbers the frame uses: each corner's point in 16.16, its
texel and its lightmap place, once per decal in a 36 KiB arena; a frame
does only the integer view transform the walls do. The decal_t's own
spare words hold its place (reserved[0]), the arena's generation
(reserved[1]) and a check of what it was built from (reserved[2]), so a
decal reused for another hole is built again. A full arena starts over
(every decal built again as it is next drawn).
*/
typedef struct
{
	int32_t p[3]; /* the corner, 16.16 (the face's model space) */
	int16_t s, t; /* the picture's texel */
	int32_t l[2]; /* the face's lightmap, luxels 24.8 */
} rp_dcorner_t;

#define RP_DECAL_ARENA 1536 /* corners (24 bytes each) */
static rp_dcorner_t rp_dc[RP_DECAL_ARENA];
static int          rp_dc_used;
static intptr_t     rp_dc_gen = 1;

static intptr_t RP_DecalCookie( const decal_t *d, const msurface_t *fa )
{
	uint32_t h = (uint32_t)(uintptr_t)fa * 2654435761u;
	uint32_t b;

	__builtin_memcpy( &b, &d->dx, 4 );
	h = ( h ^ b ) * 2246822519u;
	__builtin_memcpy( &b, &d->dy, 4 );
	h = ( h ^ b ) * 3266489917u;
	__builtin_memcpy( &b, &d->scale, 4 );
	h = ( h ^ b ^ (uint32_t)d->texture << 16 ^ (uint16_t)d->flags ) * 668265263u;
	return (intptr_t)( h | 1u );
}

/* The decal's corners, built once (NULL: nothing of it on the face). */
static const rp_dcorner_t *RP_DecalCorners( decal_t *d, const msurface_t *fa, const image_t *img, int *count )
{
	static float        poly[RP_DECAL_CORNERS][5];
	const intptr_t      cookie = RP_DecalCookie( d, fa );
	const mextrasurf_t *info = fa->info;
	int                 n;

	if( d->reserved[1] == rp_dc_gen && d->reserved[2] == cookie )
	{
		*count = (int)( d->reserved[0] >> 16 );
		return *count ? &rp_dc[d->reserved[0] & 0xFFFF] : NULL;
	}
	n = RP_DecalPoly( d, fa, img, poly );
	if( rp_dc_used + n > RP_DECAL_ARENA )
	{
		rp_dc_used = 0;
		rp_dc_gen++;
		rp_count.decal_arena_resets++;
	}
	for( int i = 0; i < n; i++ )
	{
		const float  *v = poly[i];
		rp_dcorner_t *c = &rp_dc[rp_dc_used + i];

		c->p[0] = RP_Fix16( v[0] );
		c->p[1] = RP_Fix16( v[1] );
		c->p[2] = RP_Fix16( v[2] );
		c->s = (int16_t)RP_TriCoord( v[3], img->width );
		c->t = (int16_t)RP_TriCoord( v[4], img->height );
		c->l[0] = (int32_t)(( DotProduct( v, info->lmvecs[0] ) + info->lmvecs[0][3] - info->lightmapmins[0] ) * 256.0f );
		c->l[1] = (int32_t)(( DotProduct( v, info->lmvecs[1] ) + info->lmvecs[1][3] - info->lightmapmins[1] ) * 256.0f );
	}
	d->reserved[0] = (intptr_t)( rp_dc_used | n << 16 );
	d->reserved[1] = rp_dc_gen;
	d->reserved[2] = cookie;
	rp_count.decals_built++;
	*count = n;
	if( !n )
		return NULL;
	rp_dc_used += n;
	return &rp_dc[rp_dc_used - n];
}

static void RP_FaceDecals( msurface_t *fa, const int32_t m[3][3], const int32_t o[3] )
{
	const int      mode_was = vid.rendermode;
	const uint32_t colour_was = rp_2d_colour;
	const image_t *image_was = rp_tri_image;
	const int      world = m == (const int32_t( * )[3])rp_view.m;

	vid.rendermode = kRenderTransTexture;
	rp_2d_colour = 0xFFFFFFFFu;
	for( decal_t *d = fa->pdecals; d; d = d->pnext )
	{
		const image_t *img = R_GetTexture( d->texture );
		int            count = 0;
		const rp_dcorner_t *dc;
		rp_tv_t        c[3];

		if( !img || !R_TexLoaded( img ) || img->width < 1 || img->height < 1 )
			continue;
		dc = RP_DecalCorners( d, fa, img, &count );
		if( !dc )
			continue;
		rp_tri_image = img;
		rp_count.decals++;
		{
			/* each picture once (the first 16 pictures), so a log names every
			   kind of decal a run drew: run 320's red patch was one of the
			   fight's, not the map's own, which had used every line */
			static const image_t *seen[16];
			static int           nseen;
			int                  k = 0;

			while( k < nseen && seen[k] != img )
				k++;
			if( k == nseen && nseen < 16 )
			{
				seen[nseen++] = img;
				fprintf( stderr, "rp-emit: R352: a decal picture drawn as a polygon (%d): %s, %d x %d%s (bits 0x%x, flags 0x%x), scale %d/100, %d corners, frame %d\n",
					nseen, img->name, img->width, img->height, R_TexHasAlpha( img ) ? " with alpha" : " (no alpha)", (unsigned)img->rp_bits, (unsigned)img->flags,
					(int)( d->scale * 100.0f ), count, tr.framecount );
			}
		}
		int32_t bx0 = 0x7FFFFFFF, by0 = 0x7FFFFFFF, bx1 = -0x7FFFFFFF, by1 = -0x7FFFFFFF;
		int     smin = 1 << 30, smax = -1, tmin = 1 << 30, tmax = -1, behind = 0, n2 = 0;
		/* the corners in view space with their texels, then clipped to the
		   faces' own near plane here, in fixed point (as RP_ChipFace clips
		   its face). Iterate (rule 0): the first build sent a triangle across
		   the near plane to ref_soft's float clip (R_SetupFinalVert), which
		   projects by `aliastransform`: the world's matrix as the last
		   entities of the frame BEFORE left it (the world is drawn before
		   R_SetUpWorldTransform runs), a brush entity's never; and the
		   effects' plane (4 units) cut decals where their face (1 unit) still
		   showed. Now a decal's corners are its face's numbers through one
		   path, every triangle sent from in front of the eye, a brush
		   entity's decal across the plane drawn too. */
		static int32_t qv[RP_DECAL_CORNERS][3], cq[RP_DECAL_CORNERS + 1][3];
		static int     sv[RP_DECAL_CORNERS][2], cs[RP_DECAL_CORNERS + 1][2];
		static int32_t lv[RP_DECAL_CORNERS][2], cl[RP_DECAL_CORNERS + 1][2]; /* the lightmap's luxels, 24.8 */
		for( int i = 0; i < count; i++ )
		{
			sv[i][0] = dc[i].s;
			sv[i][1] = dc[i].t;
			lv[i][0] = dc[i].l[0];
			lv[i][1] = dc[i].l[1];
			RP_ViewPoint( m, o, dc[i].p, qv[i] );
			behind += qv[i][2] < rp_near16;
		}
		for( int i = 0; i < count && n2 < RP_DECAL_CORNERS; i++ )
		{
			const int j = i + 1 < count ? i + 1 : 0;
			const int ina = qv[i][2] >= rp_near16, inb = qv[j][2] >= rp_near16;

			if( ina )
			{
				memcpy( cq[n2], qv[i], sizeof( cq[0] ));
				cs[n2][0] = sv[i][0];
				cs[n2][1] = sv[i][1];
				cl[n2][0] = lv[i][0];
				cl[n2][1] = lv[i][1];
				n2++;
			}
			if( ina != inb && n2 < RP_DECAL_CORNERS )
			{
				/* f = (near - z_i) / (z_j - z_i), 0.16 */
				int64_t den = (int64_t)qv[j][2] - qv[i][2];
				int32_t f = (int32_t)(( (int64_t)( rp_near16 - qv[i][2] ) << 16 ) / den );

				for( int k = 0; k < 2; k++ )
				{
					cq[n2][k] = qv[i][k] + (int32_t)(( (int64_t)( qv[j][k] - qv[i][k] ) * f ) >> 16 );
					cs[n2][k] = sv[i][k] + (int)(( (int64_t)( sv[j][k] - sv[i][k] ) * f ) >> 16 );
					cl[n2][k] = lv[i][k] + (int32_t)(( (int64_t)( lv[j][k] - lv[i][k] ) * f ) >> 16 );
				}
				cq[n2][2] = rp_near16;
				n2++;
			}
		}
		if( behind )
			rp_count.decal_cut++;
		if( n2 >= 3 && rp_decal_lit && RP_Lm1Pass( ) && R_TexHasAlpha( img ) && img->width <= RP_PAGE_SIDE && img->height <= RP_PAGE_SIDE )
		{
			/* lit by its wall's lightmap (above): the face's place, as
			   RP_LmPlace keeps it, still valid this frame */
			uint32_t         loc = (uint32_t)RP_LOC( fa );
			int              lpg = loc >> 16 & 0xFF;
			const rp_skin_t *e;

			if(( loc & RP_VALID ) && ( loc & RP_LMAP ) && lpg < rp_pages && (int)( RP_GEN( fa ) >> 1 ) == rp_page[lpg].gen
				&& ( e = RP_PicPlace( (image_t *)img, 0, img->width, img->width, img->height, 1 )) != NULL
				&& (int)( RP_GEN( fa ) >> 1 ) == rp_page[lpg].gen ) /* the picture's place may have dropped a page */
			{
				rp_page[lpg].used = tr.framecount;
				RP_DecalLitStrip( e, lpg, loc & 0xFF, loc >> 8 & 0xFF, n2, (const int32_t( * )[3])cq, (const int( * )[2])cs, (const int32_t( * )[2])cl );
				n2 = 0; /* drawn: the unlit fan below has nothing to do */
			}
		}
		for( int i = 0; i < n2; i++ )
		{
			rp_tv_t *t = &c[i < 2 ? i : 2];

			t->s = cs[i][0];
			t->t = cs[i][1];
			t->front = 1;
			RP_ProjectS( cq[i], t->w, t->st );
			t->w[2] += RP_DECAL_BIAS;
			bx0 = t->st[0] < bx0 ? t->st[0] : bx0;
			bx1 = t->st[0] > bx1 ? t->st[0] : bx1;
			by0 = t->st[1] < by0 ? t->st[1] : by0;
			by1 = t->st[1] > by1 ? t->st[1] : by1;
			smin = t->s < smin ? t->s : smin;
			smax = t->s > smax ? t->s : smax;
			tmin = t->t < tmin ? t->t : tmin;
			tmax = t->t > tmax ? t->t : tmax;
			if( i < 2 )
				continue;
			RP_TriEmit( &c[0], &c[1], &c[2] );
			c[1] = c[2];
		}
		if( rp_shot_now && bx1 >= bx0 )
			fprintf( stderr, "rp-emit: R352 shot %d: decal %s on face %d%s, TA %d-%d x %d-%d, %d corners (%d behind), texels s %d-%d t %d-%d\n",
				tr.framecount, img->name, (int)( fa - ( RI.currentmodel ? RI.currentmodel : WORLDMODEL )->surfaces ), world ? "" : " (brush entity)",
				bx0 >> 4, bx1 >> 4, by0 >> 4, by1 >> 4, n2, behind, smin, smax, tmin, tmax );
		/* run 320's red patch (the chip's last frame: a solid red trapezoid
		   some 40 x 120 pixels on a wall, with the decals as polygons): a
		   decal polygon that covers much of the screen is said, with what it
		   is, where (the TA's x and y in pixels: the picture turned), its
		   texels' range and the face, so one run names it */
		{
			/* each face and picture said when first seen and again every
			   100 frames (runs 321-323: one off-screen floor decal took all
			   60 lines in frames 1-60, the patch's own polygon never named) */
			static struct { const msurface_t *fa; const image_t *img; int frame; } big[32];
			static int said_big;
			const int  w = ( bx1 - bx0 ) >> 4, h = ( by1 - by0 ) >> 4;
			int        say = 0;

			if( bx1 >= bx0 && w * h >= 2500 && said_big < 400 )
			{
				int k = 0;

				while( k < 32 && big[k].fa && ( big[k].fa != fa || big[k].img != img ))
					k++;
				if( k < 32 && ( !big[k].fa || tr.framecount - big[k].frame >= 100 ))
				{
					big[k].fa = fa;
					big[k].img = img;
					big[k].frame = tr.framecount;
					say = 1;
				}
			}
			if( say )
			{
				said_big++;
				fprintf( stderr, "rp-emit: R352: a large decal polygon: %s %d x %d, scale %d/100, flags 0x%x, on screen %d x %d at TA %d,%d (%d corners, %d behind the eye), texels s %d-%d t %d-%d, face %d%s, frame %d\n",
					img->name, img->width, img->height, (int)( d->scale * 100.0f ), (unsigned)d->flags, w, h, bx0 >> 4, by0 >> 4, count, behind,
					smin, smax, tmin, tmax, (int)( fa - ( RI.currentmodel ? RI.currentmodel : WORLDMODEL )->surfaces ), world ? "" : " of a brush entity", tr.framecount );
			}
		}
	}
	vid.rendermode = mode_was;
	rp_2d_colour = colour_was;
	rp_tri_image = image_was;
}

/* Kit 114: R_DrawSpriteQuad's four corners (down-left, up-left, up-right,
   down-right) in 16.16 from the sprite's origin, its up and right vectors
   and its frame's extents times its scale: integers where the float way
   made each corner with two VectorMAs (run 235: R_DrawSpriteModel's floats
   ~2.7% of a fight frame). Within ~1/16384 of a unit of the float corners. */
void RP_SpriteCorners( const float org[3], const float vu[3], const float vr[3],
	const float ext[4], float scale, int32_t c[4][3] )
{
	int32_t s16 = RP_Fix16( scale ), e[4], o[3], u[3], r[3];

	for( int i = 0; i < 4; i++ ) /* down, up, left, right */
		e[i] = (int32_t)(( (int64_t)RP_Fix16( ext[i] ) * s16 ) >> 16 );
	for( int j = 0; j < 3; j++ )
	{
		o[j] = RP_Fix16( org[j] );
		u[j] = RP_Fix30( vu[j] );
		r[j] = RP_Fix30( vr[j] );
	}
	for( int k = 0; k < 4; k++ )
	{
		int32_t ud = k == 0 || k == 3 ? e[0] : e[1], lr = k < 2 ? e[2] : e[3];

		for( int j = 0; j < 3; j++ )
			c[k][j] = o[j] + (int32_t)(( (int64_t)ud * u[j] ) >> 30 ) + (int32_t)(( (int64_t)lr * r[j] ) >> 30 );
	}
}

/* TriVertex3f on the 2700G path: the same triangles of the same modes. */
void RP_TriVertex( float x, float y, float z, int s, int t )
{
	int32_t p[3] = { RP_Fix16( x ), RP_Fix16( y ), RP_Fix16( z ) };

	RP_TriVertexFix( p, s, t );
}

/* Kit 78: a beam walked in 16.16 (0), out of the visible set (1) or off the screen (2), counted. */
void RP_BeamNote( int culled )
{
	if( culled == 2 )
		rp_count.beams_off++;
	else if( culled )
		rp_count.beams_culled++;
	else
		rp_count.beams_fixed++;
}

/* Kit 78: the colour word as RP_Emit2DColour makes it, from integers. */
void RP_Set2DColour( uint32_t argb )
{
	rp_2d_colour = argb;
}

/* A corner already in 16.16 (kit 78: the beams' fixed-point walk). */
void RP_TriVertexFix( const int32_t p[3], int s, int t )
{
	if( rp_tv_mode == TRI_TRIANGLES )
	{
		RP_TriCorner( &rp_tv[rp_tv_count], p, s, t );
		if( ++rp_tv_count == 3 )
		{
			RP_TriEmit( &rp_tv[0], &rp_tv[1], &rp_tv[2] );
			rp_tv_count = 0;
		}
	}
	else if( rp_tv_mode == TRI_TRIANGLE_FAN )
	{
		RP_TriCorner( &rp_tv[rp_tv_count], p, s, t );
		if( ++rp_tv_count >= 3 )
		{
			RP_TriEmit( &rp_tv[0], &rp_tv[1], &rp_tv[2] );
			rp_tv[1] = rp_tv[2];
			rp_tv_count = 2;
		}
	}
	else if( rp_tv_mode == TRI_TRIANGLE_STRIP )
	{
		RP_TriCorner( &rp_tv[rp_tv_n], p, s, t );
		if( ++rp_tv_n == 3 )
			rp_tv_n = 0;
		if( ++rp_tv_count >= 3 )
		{
			if( rp_tv_count & 1 )
				RP_TriEmit( &rp_tv[0], &rp_tv[1], &rp_tv[2] );
			else
				RP_TriEmit( &rp_tv[2], &rp_tv[1], &rp_tv[0] );
		}
	}
}

/* Draw_Fill: a rectangle in the colour the engine set (CL_FillRGBA's). */
void RP_EmitFill( int x, int y, int w, int h )
{
	float u, v;

	if( rp_emit_on != 2 || w <= 0 || h <= 0 )
		return;
	if( rp_white.pg < 0 || rp_white.gen != rp_page[rp_white.pg].gen )
	{
		int       pg = RP_AtlasAlloc( 1, 1, &rp_white.qx, &rp_white.qy );
		uint32_t *dst;

		rp_white.pg = pg;
		if( pg < 0 )
		{
			rp_count.pics_skipped++;
			return;
		}
		dst = (uint32_t *)((uint8_t *)rp_g.tex + (size_t)pg * RP_PAGE_BYTES ) + RP_Interleave( rp_white.qx, rp_white.qy ) * 32;
		if( RP_ChipWrite( ))
			for( int k = 0; k < 32; k++ )
				dst[k] = 0xFFFFFFFFu;
		rp_white.gen = rp_page[pg].gen;
	}
	rp_page[rp_white.pg].used = tr.framecount;
	u = rp_white.qx * 8 + 4.0f;
	v = rp_white.qy * 8 + 4.0f;
	RP_2DState( rp_white.pg, 0 );
	RP_2DQuad( (float)x, (float)y, (float)( x + w ), (float)( y + h ), u, v, u, v );
}
