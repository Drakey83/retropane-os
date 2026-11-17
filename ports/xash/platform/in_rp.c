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
#include "input.h"
#include "platform/platform.h"
#include <retropane.h>
#include "rp_hl_keys.h"
#include <stdio.h>

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
	static const byte top[3] = { 238, 242, 246 }, bottom[3] = { 168, 178, 190 };
	static const byte glow_top[3] = { 150, 200, 245 }, glow_bottom[3] = { 40, 110, 200 };
	const byte *a = held ? glow_top : top, *b = held ? glow_bottom : bottom;
	const int   bands = 8;
	rgba_t      ink = { 34, 40, 48, 255 };
	int         tw = 0, th = 0;

	/* the rim: the rectangle less its four corner pixels */
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 1, y, w - 2, h, 70, 82, 96, 210 );
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x, y + 1, 1, h - 2, 70, 82, 96, 210 );
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + w - 1, y + 1, 1, h - 2, 70, 82, 96, 210 );
	/* the body, lit from the top */
	for( int i = 0; i < bands; i++ )
	{
		int y0 = y + 1 + ( h - 2 ) * i / bands, y1 = y + 1 + ( h - 2 ) * ( i + 1 ) / bands;

		ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 1, y0, w - 2, y1 - y0,
			a[0] + ( b[0] - a[0] ) * i / ( bands - 1 ), a[1] + ( b[1] - a[1] ) * i / ( bands - 1 ),
			a[2] + ( b[2] - a[2] ) * i / ( bands - 1 ), 200 );
	}
	/* the glass's shine on its upper half */
	ref.dllFuncs.FillRGBA( kRenderTransTexture, x + 2, y + 2, w - 4, ( h - 4 ) / 2, 255, 255, 255, 60 );
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
		asked = Sys_CheckParm( "-rpmemlist" ) ? 1 : 0;
	if( !asked )
		return;

	now = rp_time_us();
	if( now < next )
		return;
	next = now + 5000000u;
	Cbuf_AddText( "memlist 64\n" );
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

static void RP_Walk( uint64_t played_us )
{
	static int      started, turn; // turn: 0 none, 1 left, -1 right
	static uint64_t next, stuck_check, turn_until, used;
	static vec3_t   last;
	uint64_t        ms = played_us / 1000;
	int             want = turn;

	if( !started )
	{
		started = 1;
		Cbuf_AddText( "sv_cheats 1\ngod\n+forward\n" );
		VectorCopy( cl.simorg, last );
		stuck_check = ms + 1000;
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

	if( state == 1 && Sys_CheckParm( "-rpwalk" ))
		RP_Walk( now - played_from );
	if( state == 1 && end && now - played_from >= (uint64_t)end * 1000000u )
		why = "its time of play|is over";
	else if( limit && now >= (uint64_t)limit * 1000000u )
		why = "the run's time|limit is reached";
	if( !why )
		return;

	state = 2;
	fprintf( stderr, "rp-step: 5/5 Ending by itself:|%s.|The engine shuts|down, then back to|the loader.\n", why );
	if( Sys_CheckParm( "-rpendshot" ) && Cvar_VariableValue( "rp_gpu" ) == 1.0f )
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

	/* the touch keys while a game is played (not the menu or console):
	   ref_rp draws them by this cvar, the touch tests them by it */
	{
		int on = cls.state == ca_active && cls.key_dest == key_game;

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
