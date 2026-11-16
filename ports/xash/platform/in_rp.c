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
Record is the next weapon, the wireless button the menu. That layout is
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
	[RP_KEY_WIFI]     = K_ESCAPE,
};

static struct
{
	qboolean open;
	qboolean pen_down;
	int x, y;       // where the stylus last was
	int dx, dy;     // how far it was dragged since the engine last asked
} in_rp;

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
			if( ev.code < ARRAYSIZE( rp_keys ) && rp_keys[ev.code] )
				Key_Event( rp_keys[ev.code], ev.down );
		}
		else if( ev.type == RP_EV_PEN )
		{
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
