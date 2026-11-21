/*
in_rp.c - RetroPane OS (Dell Axim X51v): the buttons and the stylus
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

#include "common.h"
#include "client.h"
#include "cl_tent.h"
#include "input.h"
#include "platform/platform.h"
#include <retropane.h>
#include "rp_hl_keys.h"
#include <stdio.h>

/* Kit 118: a switch on the command line, looked up once (the test scenes'
   driver below asks every frame; Sys_CheckParm compares every word of the
   line, ~0.5% of a fight frame in run 247). The line never changes while
   the program runs, so the answer kept is the answer; keyed on the string
   literal's address (each call site passes its own). */
static int RP_Parm( const char *word )
{
	static struct
	{
		const char *word;
		int         on;
	} memo[24];
	static int n;

	for( int i = 0; i < n; i++ )
		if( memo[i].word == word )
			return memo[i].on;
	if( n == (int)( sizeof( memo ) / sizeof( memo[0] )))
		return Sys_CheckParm( word );
	memo[n].word = word;
	memo[n].on = Sys_CheckParm( word );
	return memo[n++].on;
}

/*
The OS hands over what the X51v has, already turned for landscape and the
stylus already filtered and in screen pixels (rp_input_poll). Here each
button becomes an engine key, so the game's own bindings decide what it
does (every key can be bound again in the game's menu), and the stylus
becomes the engine's mouse: a drag turns the view, as the owner's DoomGLES
does it.

The owner's pick (2026-11-15, the mock-up hl-controls-doomgles): the
D-pad walks and turns, its centre fires and so does a touch of the
stylus, Calendar jumps, Contacts is use, Mail crouches, Home reloads,
Record is the next weapon, the wireless button the menu. Run 46 (the
owner: "like doom gles, but make it make sense for hl", then "the
flashlight key and the secondary fire keys to be on the screen like i
originally read, home to bring up the menu, and wifi/record to switch
through weapons"): Home the menu, Record the next weapon, the wireless
button (K_AUX6) the one before, the flashlight and the second fire two
touch keys (rp_hl_keys.h). That layout is
a set of default bindings over these engine keys (not written yet: the
engine reads them from its config), not code here.
*/

static const byte rp_keys[] =
{
	[RP_KEY_UP]       = K_UPARROW,
	[RP_KEY_DOWN]     = K_DOWNARROW,
	[RP_KEY_LEFT]     = K_LEFTARROW,
	[RP_KEY_RIGHT]    = K_RIGHTARROW,
	[RP_KEY_ENTER]    = K_ENTER,
	[RP_KEY_CALENDAR] = K_AUX1,
	[RP_KEY_CONTACTS] = K_AUX2,
	[RP_KEY_MAIL]     = K_AUX3,
	[RP_KEY_HOME]     = K_AUX4,
	[RP_KEY_RECORD]   = K_AUX5,
	[RP_KEY_WIFI]     = K_AUX6, // R1_BUTTON: the second fire (run 46)
};

static struct
{
	qboolean open;
	qboolean pen_down;
	int x, y;       // where the stylus last was
	int dx, dy;     // how far it was dragged since the engine last asked
	int touch_key;  // the engine key a touch key holds down, or 0
	int keys_on;    // the touch keys shown (in a game, not a menu or the console)
} in_rp;

/*
The layout's defaults, remappable (the owner, run 46: "make sure you make
the key remappable"). Rule 0: Xash (as Quake) execs a default config only
when no config.cfg exists, and a port that execs its own binds at every
start (ours until run 46) undoes the player's remaps each time. Ours: the
saved config carries the layout's number (`rp_layout`, archived);
`rp_defaults N` on the kit's command line execs rp_binds.cfg only when
the saved layout is older than N, so the player's own bindings stay and a
new layout from the kit still reaches him once.
*/
/* How opaque the touch keys are (the owner, run 47: "make the flashlight
   secondary fire buttons like 50% transparent"); saved, so it can be set
   from the console and stays. */
static CVAR_DEFINE_AUTO( rp_tk_alpha, "0.25", FCVAR_ARCHIVE, "RetroPane: how opaque the touch keys are, 0 to 1" );

static CVAR_DEFINE_AUTO( rp_layout, "0", FCVAR_ARCHIVE, "RetroPane: the button layout the saved bindings follow (rp_defaults)" );

static void RP_Defaults_f( void )
{
	int want = Cmd_Argc() > 1 ? Q_atoi( Cmd_Argv( 1 )) : 1;

	if( (int)rp_layout.value >= want )
	{
		Con_Printf( "rp_defaults: the saved bindings kept (layout %d)\n", (int)rp_layout.value );
		return;
	}
	Con_Printf( "rp_defaults: layout %d over the saved %d: rp_binds.cfg\n", want, (int)rp_layout.value );
	Cbuf_InsertText( "exec rp_binds.cfg\n" );
	Cvar_SetValue( "rp_layout", (float)want );
}

void RP_InputInit( void )
{
	Cvar_RegisterVariable( &rp_layout );
	Cvar_RegisterVariable( &rp_tk_alpha );
	Cvar_Get( "rp_touchkeys", "0", 0, "RetroPane: 1 while the touch keys are shown (in_rp.c sets it)" );
	Cmd_AddCommand( "rp_defaults", RP_Defaults_f, "RetroPane: rp_binds.cfg when the saved layout is older than the argument" );
}

/*
The touch keys drawn (the engine's Platform_DrawCursor, after its HUD and
console), as the owner's mock-up hl-controls-doomgles shows them: silver
glass pills labelled LIGHT and FIRE 2, the label dark on the glass; held,
the blue glow (the OS's selection, Aero not Metro). Drawn with the engine's
own fill and text, so ref_rp's 2D layer draws them like the HUD: a few
fills of graded alpha make the glass's light from top to bottom, one pixel
cut from each corner rounds it.
*/
static void RP_GlassKey( int x, int y, int w, int h, const char *label, qboolean held )
{
	float       k = bound( 0.0f, rp_tk_alpha.value, 1.0f );
	byte        rim = (byte)( 210 * k ), body = (byte)( 200 * k ), shine = (byte)( 60 * k );

	static const byte top[3] = { 238, 242, 246 }, bottom[3] = { 168, 178, 190 };
	static const byte glow_top[3] = { 150, 200, 245 }, glow_bottom[3] = { 40, 110, 200 };
	const byte *a = held ? glow_top : top, *b = held ? glow_bottom : bottom;
	const int   bands = 8;
	rgba_t      ink = { 34, 40, 48, (byte)( 255 * ( 0.4f + 0.6f * k )) }; /* the label stays readable */
	int         tw = 0, th = 0;

	/* the rim: the rectangle less its four corner pixels */
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 1, y, w - 2, h, 70, 82, 96, rim );
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x, y + 1, 1, h - 2, 70, 82, 96, rim );
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + w - 1, y + 1, 1, h - 2, 70, 82, 96, rim );
	/* the body, lit from the top */
	for( int i = 0; i < bands; i++ )
	{
		int y0 = y + 1 + ( h - 2 ) * i / bands, y1 = y + 1 + ( h - 2 ) * ( i + 1 ) / bands;

		ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 1, y0, w - 2, y1 - y0,
			a[0] + ( b[0] - a[0] ) * i / ( bands - 1 ), a[1] + ( b[1] - a[1] ) * i / ( bands - 1 ),
			a[2] + ( b[2] - a[2] ) * i / ( bands - 1 ), body );
	}
	/* the glass's shine on its upper half */
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 2, y + 2, w - 4, ( h - 4 ) / 2, 255, 255, 255, shine );
	if( held )
		ink[0] = ink[1] = ink[2] = 255;
	Con_DrawStringLen( label, &tw, &th );
	Con_DrawString( x + ( w - tw ) / 2, y + ( h - th ) / 2, label, ink );
}

void Platform_DrawCursor( void )
{
	if( !in_rp.keys_on )
		return;
	RP_GlassKey( RP_TK_X( 0 ), RP_TK_Y( 0 ), RP_TK_W, RP_TK_H, "LIGHT", in_rp.touch_key == K_C_BUTTON );
	RP_GlassKey( RP_TK_X( 1 ), RP_TK_Y( 1 ), RP_TK_W, RP_TK_H, "FIRE 2", in_rp.touch_key == K_Z_BUTTON );
}

/* The touch key at the game's (x, y), or -1. */
static int RP_TouchKeyAt( int x, int y )
{
	for( int k = 0; k < RP_TK_COUNT; k++ )
		if( x >= RP_TK_X( k ) && x < RP_TK_X( k ) + RP_TK_W && y >= RP_TK_Y( k ) && y < RP_TK_Y( k ) + RP_TK_H )
			return k;
	return -1;
}

/*
A test image's memory log: with -rpmemlist on the command line the
engine's own list of its memory pools (its `memlist` command, with every
block of 64 KiB or more and the source line that asked for it) is printed
every five seconds. Where a game's memory goes is read from the run's log.
*/
static void RP_MemoryLog( void )
{
	static int      asked = -1;
	static uint64_t next;
	uint64_t        now;

	if( asked < 0 )
		asked = RP_Parm( "-rpmemlist" ) ? 1 : 0;
	if( !asked )
		return;

	now = rp_time_us();
	if( now < next )
		return;
	next = now + 5000000u;
	Cbuf_AddText( "memlist 256\nrp_memsum\n" ); /* R335: the pools by line too */
}

/*
A number of seconds after a word of the command line; 0 without it.
*/
static uint RP_Seconds( const char *parm )
{
	char value[16];
	int  n;

	if( !Sys_GetParmFromCmdLine( parm, value ))
		return 0;
	n = Q_atoi( value );
	return n > 0 ? (uint)n : 0;
}

/*
A test run that ends by itself, and its last two steps for the runner's
screen (rp_port::progress; steps 1 to 3 are the runner's and vid_rp.c's).

The engine never stops once a map plays, and a run on the X51v has to
come back to the loader with its log. Two words of the command line:

-rpend <seconds>    the run ends that long after the map began to play
                    (the client is active): the time that is measured
-rplimit <seconds>  the run ends when the port layer's clock (which
                    starts with the runner, as the clock on its screen)
                    passes this, whatever the engine is doing: a map
                    that never loads, a run left in the menu

Ending is the engine's own `quit`, queued as if typed: it disconnects,
shuts its parts down in its own order and returns from its main, so the
runner brings the sound down and hands the log back. A hang (no frame
at all) is not this code's to end: the loader's run limit does that.
With -rpendshot the last frame is written to the card first (vid_rp.c),
once, where it cannot disturb a frame being timed.
*/
void RP_LastShot( void );
/* ref_rp (the renderer's module, ref/soft r_emit.c), through two of its
   settings: `rp_gpu` is 1 while the 2700G draws the frames; the end shot
   is then one frame drawn both ways, the 2700G's picture and the software
   renderer's of the same view, both written by the renderer (`rp_both`:
   1 asked, 2 written). */

/*
-rpwalk: Gordon walks the map on his own while a run plays (the owner,
2026-11-16: a varied test through the corridors, the view sweeping the
map's brushes and its people). Forward all the time, steered: five times
a second three probe lines (ahead, 40 degrees left and right, 128 units,
at waist height) through the engine's own client trace, and a turn
toward the more open side when ahead is blocked; standing still for a
second (a wall, a corner, a person) a turn of about 180 degrees, a jump
and the use key; the use key also when something is close ahead, so
doors open (the owner: "the doors wont open"). God mode so nothing ends the walk; the cheats are the
single-player engine's own (sv_cheats). The first walk, on a timer,
ran into walls (the owner).
*/
static float RP_Probe( float yaw_deg )
{
	vec3_t     start, end;
	pmtrace_t  tr;
	float      a = DEG2RAD( yaw_deg );

	VectorCopy( cl.simorg, start );
	start[2] += 8.0f;
	end[0] = start[0] + 128.0f * cosf( a );
	end[1] = start[1] + 128.0f * sinf( a );
	end[2] = start[2];
	tr = CL_TraceLine( start, end, PM_NORMAL );
	return tr.fraction;
}

/*
The people to walk among (the owner: "i want him walking around npcs"):
the nearest monster the client sees this frame (a stepping studio model,
not a player), 80 to 2000 units away and not one of the last four
reached; its index, or 0.
*/
static int rp_reached[4], rp_reached_at;

static int RP_Person( vec3_t to )
{
	int   best = 0;
	float best_d = 2000.0f * 2000.0f;

	for( int i = cl.maxclients + 1; i < clgame.maxEntities; i++ )
	{
		cl_entity_t *e = CL_GetEntityByIndex( i );
		vec3_t       d;
		float        dd;
		int          seen = 0;

		if( !e || !e->model || e->model->type != mod_studio || e->curstate.messagenum != cl.parsecount
			|| e->curstate.movetype != MOVETYPE_STEP )
			continue;
		for( int k = 0; k < 4; k++ )
			seen |= rp_reached[k] == i;
		if( seen )
			continue;
		VectorSubtract( e->origin, cl.simorg, d );
		d[2] = 0.0f;
		dd = DotProduct( d, d );
		if( dd < best_d && dd > 80.0f * 80.0f )
		{
			best_d = dd;
			best = i;
			VectorCopy( d, to );
		}
	}
	return best;
}

/*
-rptour <map>,<map>: the walk goes on to these maps in turn (kit 51, R309:
the sky, water and conveyors are few in c1a0; c2a5 opens on 44 sky and 4
water faces, c2a4b on 35 conveyor faces, counted from each map's start in
its own visibility data). The run's play is shared out evenly: the first
map its share (the same walk as run 50 for the frame rate), then each
listed one, started by `map` (a new game there: god mode and the walk's
settings given again once it plays).
*/
static int RP_Tour( uint64_t ms, int *started )
{
	static char     list[64];
	static int      have = -1, at; // the maps gone to
	static uint64_t share;
	char            name[32];
	const char      *p;
	int             k = 0;

	if( have < 0 )
	{
		have = Sys_GetParmFromCmdLine( "-rptour", list ) ? 1 : 0;
		if( have )
		{
			int n = 1;

			for( p = list; *p; p++ )
				n += *p == ',';
			share = (uint64_t)RP_Seconds( "-rpend" ) * 1000u / (uint64_t)( n + 1 );
		}
	}
	if( !have || !share || ms < share * (uint64_t)( at + 1 ))
		return 0;
	// the next map's name
	p = list;
	for( int i = 0; i < at && p; i++ )
	{
		p = Q_strchr( p, ',' );
		if( p )
			p++;
	}
	if( !p || !*p )
		return 0;
	while( p[k] && p[k] != ',' && k < (int)sizeof( name ) - 1 )
	{
		name[k] = p[k];
		k++;
	}
	name[k] = 0;
	at++;
	fprintf( stderr, "rp-walk: on to %s (%u s into the play)\n", name, (uint)( ms / 1000 ));
	Cbuf_AddText( va( "-forward\n-left\n-right\nmap %s\n", name ));
	*started = 0;
	return 1;
}

/*
The walk at a fixed step (the owner, 2026-11-17: "fixed-step walk"; run 54:
two runs of one kit differed by up to 10%, the walk's views following its
own timing). The game's time goes on 1/30 s a frame whatever the frame
took (`host_framerate`, the engine's own lock), the walk's timers read the
game's clock, and the random generator starts from one seed, so every run
plays the same frames in the same order: server, physics, sound, HUD and
all, the whole game (a `timedemo` replays the client only). The score is
the real time those frames took, logged every RP_WALK_WINDOW frames from
the walk's start: two kits compare window by window.
*/
#define RP_WALK_STEP   "0.0333333"
#define RP_WALK_WINDOW 250

static void RP_ScoreAs( const char *what );

static void RP_WalkScore( void )
{
	RP_ScoreAs( "rp-walk" );
}

/* R335 (the owner, 2026-11-19: "are there hitches in the fight scene?"):
   each frame's own time beside the window's: the slowest frame and the
   frames over 50 and 100 ms (ref_rp's R335 line says which of its own slow
   frames rebuilt a texture: that counter is in another module). */
/* The walk's score under another yardstick's name (kit 78: the cascade). */
static void RP_ScoreAs( const char *what )
{
	static uint32_t frames, worst_us, worst_at, over50, over100;
	static uint64_t t0, from, last;
	uint64_t        now = rp_time_us();

	if( !frames )
		t0 = from = last = now;
	else
	{
		uint32_t dt = (uint32_t)( now - last );

		if( dt > worst_us )
		{
			worst_us = dt;
			worst_at = frames + 1;
		}
		/* each hitch on its own line, in the log's order with the
		   runner's card lines (-rpplayopens): what fell inside it */
		if( dt > 50000u )
			fprintf( stderr, "%s: frame %u took %u ms (%u to %u ms on rp_time_us)\n", what, frames + 1, dt / 1000u,
				(uint32_t)( last / 1000u ), (uint32_t)( now / 1000u ));
		over50 += dt > 50000u;
		over100 += dt > 100000u;
	}
	last = now;
	frames++;
	if( frames % RP_WALK_WINDOW == 0 )
	{
		fprintf( stderr, "%s: frames %u-%u one by one: the slowest %u.%u ms (frame %u), %u over 50 ms, %u over 100 ms; the window ends at %u ms on rp_time_us (the samples' step mark)\n",
			what, frames - RP_WALK_WINDOW + 1, frames, worst_us / 1000u, worst_us / 100u % 10u, worst_at, over50, over100,
			(uint32_t)( now / 1000u ));
		worst_us = worst_at = over50 = over100 = 0;
	}
	if( frames % RP_WALK_WINDOW == 0 )
	{
		uint32_t ms = (uint32_t)(( now - from ) / 1000u ), all = (uint32_t)(( now - t0 ) / 1000u );

		/* where the player is: the same in every run when the frames are */
		fprintf( stderr, "%s: frames %u-%u took %u ms (%u.%u fps); frames 1-%u %u ms (%u.%u fps); at %d %d %d\n",
			what, frames - RP_WALK_WINDOW + 1, frames, ms, RP_WALK_WINDOW * 10000u / ( ms ? ms : 1 ) / 10u,
			RP_WALK_WINDOW * 10000u / ( ms ? ms : 1 ) % 10u, frames, all,
			(uint32_t)( (uint64_t)frames * 10000u / ( all ? all : 1 ) / 10u ),
			(uint32_t)( (uint64_t)frames * 10000u / ( all ? all : 1 ) % 10u ),
			(int)cl.simorg[0], (int)cl.simorg[1], (int)cl.simorg[2] );
		from = now;
	}
}

static void RP_Walk( uint64_t played_us )
{
	static int      started, turn; // turn: 0 none, 1 left, -1 right
	static uint64_t next, stuck_check, turn_until, used, active_at, fx_next;
	static int      toured, count; // a map of the tour started: wait for it (its server count)
	static vec3_t   last;
	static double   game0 = -1.0;
	uint64_t        ms;
	int             want = turn;

	/* the game's clock, not the wall's: the same frames every run */
	(void)played_us;
	if( game0 < 0.0 && CL_Active( ))
		game0 = cl.time;
	ms = game0 < 0.0 ? 0 : (uint64_t)(( cl.time - game0 ) * 1000.0 );

	if( RP_Tour( ms, &started ))
	{
		turn = 0;
		active_at = 0;
		toured = 1;
		count = cl.servercount;
		return;
	}
	// a tour's map coming up: the walk waits until it has played 3 s (the
	// first map's walk starts at once, as run 50's)
	if( toured && ( !CL_Active( ) || cl.servercount == count ))
	{
		active_at = 0;
		return;
	}
	if( !active_at )
		active_at = ms;
	if( toured && !started && ms - active_at < 3000 )
		return;
	if( !started )
	{
		started = 1;
		/* the suit first (the owner, 2026-11-17: tests measure with the
		   HUD, and the touch keys show only with the suit; without it the
		   game ignored impulse 100, so the runs to 53 had no flashlight),
		   the flashlight once it is picked up */
		Cbuf_AddText( "sv_cheats 1\ngod\ngive item_suit\nwait\nwait\nwait\nimpulse 100\nvolume 0.05\nMP3Volume 0\n+forward\n" ); // quiet: nobody is listening (the owner, 2026-11-16: "my daughter is trying to sleep") // the flashlight on (R308: its dynamic light every frame)
		/* kit 74's check (-rpfx): the MP5 held firing, so every frame has
		   the effects R320 draws (muzzle flashes, tracers, impact sparks:
		   sprites, beams, particles); not for the timed walk */
		if( RP_Parm( "-rpfx" ))
			Cbuf_AddText( "give weapon_egon\ngive ammo_gaussclip\ngive ammo_gaussclip\nwait\nweapon_egon\nwait\nwait\n+attack\n" );
		VectorCopy( cl.simorg, last );
		stuck_check = ms + 1000;
		turn_until = next = used = 0;
	}
	/* -rpfx: the gluon gun's ammunition topped up every 4 s, so it never
	   stops firing (its beam and sprites in every frame, the end shot too) */
	/* R340 (the owner's lead, run 297: the stalls, the hitches and the
	   flashing come most with the flashlight on): the walk keeps it lit.
	   The game's battery lasts 120 s (1.2 s a unit) and fills in 20 s,
	   and the light goes out by itself at empty, so one more impulse 100
	   every 145 s of the game's clock lights it again: on for 120 s of
	   every 145, the same frames every run. */
	{
		static uint64_t flash_next = 145000;

		if( ms >= flash_next )
		{
			flash_next += 145000;
			Cbuf_AddText( "impulse 100\n" );
		}
	}
	if( ms >= fx_next && RP_Parm( "-rpfx" ))
	{
		fx_next = ms + 4000;
		Cbuf_AddText( "give ammo_gaussclip\n" );
	}
	if( ms >= stuck_check )
	{
		vec3_t d;

		VectorSubtract( cl.simorg, last, d );
		VectorCopy( cl.simorg, last );
		stuck_check = ms + 1000;
		if( DotProduct( d, d ) < 16.0f * 16.0f )
		{
			// a wall or a corner: about face, and a jump for a step
			want = 1;
			turn_until = ms + 850;
			Cbuf_AddText( "+jump\nwait\n-jump\n+use\nwait\n-use\n" );
		}
	}
	if( ms >= turn_until && ms >= next )
	{
		float yaw = cl.viewangles[YAW];
		float ahead = RP_Probe( yaw );

		next = ms + 200;
		// something close ahead: a door opens to the use key (once a second)
		if( ahead < 0.5f && ms >= used )
		{
			used = ms + 1000;
			Cbuf_AddText( "+use\nwait\n-use\n" );
		}
		vec3_t to;
		int    who = RP_Person( to );

		if( ahead > 0.5f && who )
		{
			// toward the person, while the way ahead is open
			float diff = RAD2DEG( atan2f( to[1], to[0] )) - yaw;

			while( diff > 180.0f )
				diff -= 360.0f;
			while( diff < -180.0f )
				diff += 360.0f;
			want = diff > 15.0f ? 1 : diff < -15.0f ? -1 : 0;
			if( DotProduct( to, to ) < 120.0f * 120.0f )
			{
				// reached: the next person next time
				rp_reached[rp_reached_at++ & 3] = who;
			}
		}
		else if( ahead > 0.9f )
			want = 0;
		else
			want = RP_Probe( yaw + 40.0f ) >= RP_Probe( yaw - 40.0f ) ? 1 : -1;
	}
	if( want != turn )
	{
		if( turn == 1 )
			Cbuf_AddText( "-left\n" );
		else if( turn == -1 )
			Cbuf_AddText( "-right\n" );
		if( want == 1 )
			Cbuf_AddText( "+left\n" );
		else if( want == -1 )
			Cbuf_AddText( "+right\n" );
		turn = want;
	}
}

static void RP_End( void )
{
	static int      state = -1; // 0: before the map plays, 1: it plays, 2: ending, 3: waiting for the frame drawn both ways
	static uint64_t asked;
	static uint     end, limit;
	static uint64_t played_from;
	uint64_t        now;
	const char      *why = NULL;

	if( state < 0 )
	{
		end = RP_Seconds( "-rpend" );
		limit = RP_Seconds( "-rplimit" );
		state = 0;
		/* the fixed step and the seed from the first frame, before the
		   map loads: its scientists and doors move from its first
		   frame, in game time */
		if( RP_Parm( "-rpwalk" ) || RP_Parm( "-rpcascade" ) || RP_Parm( "-rpfight" ))
		{
			COM_SetRandomSeed( 0x5EED );
			Cvar_Set( "host_framerate", RP_WALK_STEP );
		}
	}
	if( state == 2 )
		return;

	now = rp_time_us();
	if( state == 3 )
	{
		// the frame drawn both ways is written (or none came in 5 s)
		if( Cvar_VariableValue( "rp_both" ) != 2.0f && now - asked < 5000000u )
			return;
		state = 2;
		Cbuf_AddText( "quit\n" );
		return;
	}
	if( state == 0 && CL_Active( ))
	{
		state = 1;
		played_from = now;
		if( end )
			fprintf( stderr, "rp-step: 4/5 The map plays.|Its frame rate is|below, new every|5 seconds. The run|ends by itself|after %u s of play.\n", end );
		else if( limit )
			fprintf( stderr, "rp-step: 4/5 The map plays.|Its frame rate is|below, new every|5 seconds. The run|ends by itself with|the clock at %u:%02u.\n", limit / 60, limit % 60 );
		else
			fprintf( stderr, "rp-step: 4/5 The map plays.|Its frame rate is|below, new every|5 seconds. It does|not end by itself:|soft reset to stop.\n" );
	}

	if( state == 1 && RP_Parm( "-rpwalk" ))
	{
		RP_Walk( now - played_from );
		RP_WalkScore();
	}
	/*
	The cascade's yardstick (R324, the owner 2026-11-17: "I want the
	cascade to still run at 30fps"; kit 76's play: 12 fps, 4.6 at its
	worst). -rpcascade on c1a0e: the fixed step and seed of the walk, the
	suit and god mode at 2 s of game time, then at 4 s the map's own
	cascade fired (`fire probe_arm_mm`, the SDK's cheat: the probe arm's
	multi_manager, which runs the explosions, beams, shakes and debris as
	the map scripts them), the player standing where the map starts him.
	Every run plays the same frames; scored from the firing, window by
	window as the walk ("rp-cascade: frames ...").
	*/
	if( state == 1 && RP_Parm( "-rpcascade" ))
	{
		static double game0 = -1.0;
		static int    stage;
		double        ms;

		if( game0 < 0.0 && CL_Active( ))
			game0 = cl.time;
		ms = game0 < 0.0 ? 0.0 : ( cl.time - game0 ) * 1000.0;
		if( stage == 0 && ms > 2000.0 )
		{
			stage = 1;
			/* the player held in the chamber facing the zapper (noclip: no
			   floor needed), so the cascade is in view */
			Cbuf_AddText( "sv_cheats 1\ngod\ngive item_suit\nnoclip\nrp_setpos 1400 624 -260 -40 0\n" ); // the zapper at 1664 624 0: ahead, 40 degrees up
		}
		else if( stage == 1 && ms > 4000.0 )
		{
			stage = 2;
			fprintf( stderr, "rp-cascade: fired (fire probe_arm_mm) at %u ms of game time\n", (unsigned)ms );
			Cbuf_AddText( "fire probe_arm_mm\n" );
			/* -rpcalm: the sampled play is the calm frames 1-250 alone */
			if( RP_Parm( "-rpcalm" ))
				fprintf( stderr, "rp-step: 4/5 The cascade's|calm frames: 1|to 250.\n" );
		}
		else if( stage == 2 )
		{
			static uint32_t frames;

			RP_ScoreAs( "rp-cascade" );
			/*
			-rpstorm (the owner, 2026-11-18: "build the 500 debris stress
			test"): on top of the cascade's own debris, every frame a
			burst of 40 metal gib pieces (4 s of life) is thrown around the
			zapper, in view, by the engine's own R_BreakModel (the call
			TE_BREAKMODEL makes), so the temporary entities stay at the
			engine's limit (max_tents, 500) for the whole scored run: the
			worst a scene can throw at the debris path. The model is the
			first gib model the map has loaded (named once in the log).
			*/
			if( RP_Parm( "-rpstorm" ))
			{
				static int gib = -1;

				if( gib < 0 )
				{
					gib = 0;
					for( int m = 1; m < MAX_MODELS && !gib; m++ )
						if( cl.models[m] && cl.models[m]->type == mod_studio && Q_stristr( cl.models[m]->name, "gibs" ))
							gib = m;
					fprintf( stderr, "rp-storm: 40 pieces a frame of model %d (%s), 4 s each: the temporary entities held at the limit (%d)\n",
						gib, gib ? cl.models[gib]->name : "none loaded", GI->max_tents );
				}
				if( gib )
				{
					const vec3_t pos = { 1664.0f, 624.0f, -40.0f }, size = { 256.0f, 256.0f, 128.0f }, dir = { 0.0f, 0.0f, 60.0f };

					R_BreakModel( pos, size, dir, 200.0f, 4.0f, 40, gib, BREAK_METAL );
				}
			}
			/* step 4 said again at the heavy half's start: the sampled
			   run's "play" is then frames 251-500 alone (the runner marks
			   the samples at each step line) */
			if( frames + 1u == RP_WALK_WINDOW && !RP_Parm( "-rpcalm" ))
				fprintf( stderr, "rp-step: 4/5 The cascade's|heavy half: frames|251 to 500.\n" );
			/* ref_rp's counts reported at frames 250 and 480: the second
			   covers the heavy half alone (the run quits at 500, before a later one) */
			if( frames + 1u == RP_WALK_WINDOW || frames + 20u == 2u * RP_WALK_WINDOW )
			{
				extern unsigned rp_direct_count[4];

				Cvar_SetValue( "rp_report", 1.0f );
				/* ADR-0040 step 7a's counts since the last line */
				fprintf( stderr, "rp-direct: %u states passed on the block compare, %u delta-written; physics lists %u built, %u kept\n",
					rp_direct_count[0], rp_direct_count[1], rp_direct_count[2], rp_direct_count[3] );
				memset( rp_direct_count, 0, sizeof( rp_direct_count ));
			}
			/* the run ends at one game frame, so two kits' end pictures
			   (the 2700G's and the software's) show the same moment */
			if( ++frames >= 2u * RP_WALK_WINDOW )
				why = "the cascade's|500 frames are done";
			else if( frames >= RP_WALK_WINDOW && RP_Parm( "-rpcalm" ))
				why = "the cascade's|calm frames are done";
			/* -rpcascademid (the owner, 2026-11-18: "Mid-cascade shots"):
			   the run ends at frame 300, the debris falling, so the end
			   picture shows the debris's shading (options A and B judged
			   against the plain kit at the same game frame) */
			else if( frames >= RP_WALK_WINDOW + 50u && RP_Parm( "-rpcascademid" ))
				why = "the cascade's|frame 300: a|picture mid-cascade";
		}
	}
	/*
	R329 (the owner, 2026-11-18: "will fps dip significantly on areas where
	there is a lot of fighting too?"; "Fight test first"): -rpfight on
	c2a5f (Surface Tension's 19 soldiers, 4 alien slaves and an alien
	grunt, as the map places them), the cascade's way: the fixed step and
	seed, the player put among them (god, noclip, every weapon, the MP5 in
	hand) and firing from 4 s of game time, so the map's own soldiers and
	aliens fight; scored from the first shot, frames 1-250 and 251-500
	("rp-fight: frames ..."), ending at frame 500.
	*/
	if( state == 1 && RP_Parm( "-rpfight" ))
	{
		static double game0 = -1.0;
		static int    stage;
		double        ms;

		if( game0 < 0.0 && CL_Active( ))
			game0 = cl.time;
		ms = game0 < 0.0 ? 0.0 : ( cl.time - game0 ) * 1000.0;
		if( stage == 0 && ms > 2000.0 )
		{
			stage = 1;
			/* beside the soldiers at 402 339 -1805 and 368 305 -1806, above
			   the floor, facing the ones to the south-east (416 -156, 982
			   -574, 1024 -624) */
			Cbuf_AddText( "sv_cheats 1\ngod\nimpulse 101\nnoclip\nrp_setpos 450 400 -1730 10 -60\n" );
		}
		else if( stage == 1 && ms > 4000.0 )
		{
			stage = 2;
			fprintf( stderr, "rp-fight: firing from %u ms of game time\n", (unsigned)ms );
			/* the MP5 taken now: impulse 101 gave it two seconds ago (the
			   weapons arrive after the impulse, so a selection in the same
			   breath found none: run 130 held the hornet gun) */
			Cbuf_AddText( "weapon_9mmAR\n+attack\n" );
		}
		else if( stage == 2 )
		{
			static uint32_t frames;

			RP_ScoreAs( "rp-fight" );
			if( frames + 1u == RP_WALK_WINDOW )
				fprintf( stderr, "rp-step: 4/5 The fight's|second half: frames|251 to 500.\n" );
			if( frames + 1u == RP_WALK_WINDOW || frames + 20u == 2u * RP_WALK_WINDOW )
				Cvar_SetValue( "rp_report", 1.0f );
			/* the ammunition topped up every 100 frames (impulse 101), so
			   the MP5 never runs dry */
			if( frames % 100u == 99u )
				Cbuf_AddText( "impulse 101\n" );
			if( ++frames >= 2u * RP_WALK_WINDOW )
				why = "the fight's|500 frames are done";
		}
	}
	/* -rpplay (the owner, 2026-11-19: "put into a scene with some guns and
	   be able to play it"): the fight's map and place with every weapon
	   and god mode, then the player's own hands (no script, no frame
	   limit, real time: the scenes' fixed step is not set), the
	   ammunition topped up every 20 s; the run ends at -rpend or by the
	   menu's quit */
	if( state == 1 && RP_Parm( "-rpplay" ))
	{
		static double game0 = -1.0, topped;
		static int    stage;
		double        ms;

		if( game0 < 0.0 && CL_Active( ))
			game0 = cl.time;
		ms = game0 < 0.0 ? 0.0 : ( cl.time - game0 ) * 1000.0;
		if( stage == 0 && ms > 2000.0 )
		{
			stage = 1;
			Cbuf_AddText( "sv_cheats 1\ngod\nimpulse 101\nrp_setpos 450 400 -1730 10 -60\n" );
		}
		else if( stage == 1 && ms > 4000.0 )
		{
			stage = 2;
			topped = ms;
			/* -rpsnark (the owner, 2026-11-19: the snark in his hand looked
			   wrong in run 273): held from the start, for its end picture */
			Cbuf_AddText( RP_Parm( "-rpsnark" ) ? "weapon_snark\n" : "weapon_9mmAR\n" );
			fprintf( stderr, "rp-play: the player's own from %u ms of game time\n", (unsigned)ms );
		}
		else if( stage == 2 )
		{
			RP_ScoreAs( "rp-play" );
			if( ms - topped > 20000.0 )
			{
				topped = ms;
				Cbuf_AddText( "impulse 101\n" );
			}
		}
	}
	/* R321's check (-rpcam x,y,z,pitch,yaw): the player put at one spot
	   facing one way, held there (noclip), so the end frame shows it */
	{
		static int cam_done;
		char       cam[64];

		/* -rpsuit (the owner, kit 75's run: a map started on its own, such
		   as c1a0e, gives no HEV suit, so its scripted doors stay shut):
		   the suit given once the map plays */
		static int suit_done;

		if( state == 1 && !suit_done && now - played_from > 2000000u && RP_Parm( "-rpsuit" ))
		{
			suit_done = 1;
			Cbuf_AddText( "sv_cheats 1\ngive item_suit\n" );
		}
		if( state == 1 && !cam_done && now - played_from > 2000000u && Sys_GetParmFromCmdLine( "-rpcam", cam ))
		{
			char cmd[128];

			cam_done = 1;
			for( char *c = cam; *c; c++ )
				if( *c == ',' )
					*c = ' ';
			Q_snprintf( cmd, sizeof( cmd ), "sv_cheats 1\nnoclip\nrp_setpos %s\n", cam );
			Cbuf_AddText( cmd );
		}
	}
	if( why )
		; /* the cascade's frames are done */
	else if( state == 1 && end && now - played_from >= (uint64_t)end * 1000000u )
		why = "its time of play|is over";
	else if( limit && now >= (uint64_t)limit * 1000000u )
		why = "the run's time|limit is reached";
	if( !why )
		return;

	state = 2;
	fprintf( stderr, "rp-step: 5/5 Ending by itself:|%s.|The engine shuts|down, then back to|the loader.\n", why );
	if( RP_Parm( "-rpendshot" ) && Cvar_VariableValue( "rp_gpu" ) == 1.0f )
	{
		Cvar_SetValue( "rp_both", 1.0f );
		asked = now;
		state = 3;
		return;
	}
	RP_LastShot();
	Cbuf_AddText( "quit\n" );
}

void Platform_RunEvents( void )
{
	struct rp_event ev;

	RP_MemoryLog();
	RP_End();

	/* the touch keys while a game is played (not the menu or console)
	   and the player has the HEV suit (the owner, 2026-11-17: only once
	   the suit is on, as the game's own HUD: its WEAPON_SUIT bit, 31, in
	   the weapons the server sends; the flashlight is the suit's too):
	   ref_rp draws them by this cvar, the touch tests them by it */
	{
		int suit = ( (unsigned)cl.frames[cl.parsecountmod].clientdata.weapons >> 31 ) & 1;
		int on = cls.state == ca_active && cls.key_dest == key_game && suit;

		if( on != in_rp.keys_on )
		{
			in_rp.keys_on = on;
			Cvar_SetValue( "rp_touchkeys", (float)on );
			if( !on && in_rp.touch_key )
			{
				Key_Event( in_rp.touch_key, false );
				in_rp.touch_key = 0;
			}
		}
	}

	if( !in_rp.open )
	{
		// the owner's hold (2026-11-15): the D-pad under the left thumb, so
		// the device's top edge is at the player's right
		if( rp_input_open( RP_TURN_RIGHT ) != 0 )
			return;
		in_rp.open = true;
	}

	while( rp_input_poll( &ev ) == 1 )
	{
		if( ev.type == RP_EV_KEY )
		{
			static uint32_t seen; // each button's first press, logged once (runs 35-38)

			if( ev.down && ev.code < 32 && !( seen & ( 1u << ev.code )))
			{
				seen |= 1u << ev.code;
				fprintf( stderr, "in_rp: button %d pressed first, the engine's key %d (%s)\n", ev.code,
					ev.code < ARRAYSIZE( rp_keys ) ? rp_keys[ev.code] : 0,
					ev.code < ARRAYSIZE( rp_keys ) && rp_keys[ev.code] ? Key_KeynumToString( rp_keys[ev.code] ) : "none" );
			}
			if( ev.code < ARRAYSIZE( rp_keys ) && rp_keys[ev.code] )
				Key_Event( rp_keys[ev.code], ev.down );
		}
		else if( ev.type == RP_EV_PEN )
		{
			/* a touch key: its engine key down while the stylus stays,
			   no look and no MOUSE1 (the flashlight's C_BUTTON, the
			   second fire's Z_BUTTON: bound in rp_binds.cfg, remappable) */
			if( ev.code == RP_PEN_DOWN && in_rp.keys_on && !in_rp.touch_key )
			{
				int k = RP_TouchKeyAt( ev.x, ev.y );

				if( k >= 0 )
				{
					in_rp.touch_key = k == 0 ? K_C_BUTTON : K_Z_BUTTON;
					Key_Event( in_rp.touch_key, true );
					continue;
				}
			}
			if( in_rp.touch_key )
			{
				if( ev.code == RP_PEN_UP )
				{
					Key_Event( in_rp.touch_key, false );
					in_rp.touch_key = 0;
				}
				continue;
			}
			if( ev.code == RP_PEN_MOVE && in_rp.pen_down )
			{
				in_rp.dx += ev.x - in_rp.x;
				in_rp.dy += ev.y - in_rp.y;
			}

			in_rp.x = ev.x;
			in_rp.y = ev.y;

			if( ev.code == RP_PEN_DOWN )
			{
				in_rp.pen_down = true;
				Key_Event( K_MOUSE1, true );
			}
			else if( ev.code == RP_PEN_UP )
			{
				in_rp.pen_down = false;
				Key_Event( K_MOUSE1, false );
			}
		}
	}
}

/*
How far the stylus was dragged since the last call, as a mouse's movement.
*/
void Platform_MouseMove( float *x, float *y )
{
	if( x ) *x = (float)in_rp.dx;
	if( y ) *y = (float)in_rp.dy;

	in_rp.dx = in_rp.dy = 0;
}

void Platform_EnableTextInput( qboolean enable, int x, int y, int w, int h )
{
	// no keyboard: the OS's on-screen one is not open to a game yet
}
