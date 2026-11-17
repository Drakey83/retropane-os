/*
s_rp.c - RetroPane OS (Dell Axim X51v): sound through the OS's mixer
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
#include "sound.h"
#include "platform/platform.h"
#include <retropane.h>
#include <stdio.h>

/*
The engine paints its mix into a ring (snd.buffer) ahead of where the
device plays (snd.samplepos), as a sound card's DMA buffer worked. The
other backends copy out of that ring on an audio thread and guess the
play position from the last copy. Here there is no thread: each frame
the play position is the OS's own count of frames that left the device
(rp_snd_played), and what the engine painted since the last frame is
handed to the OS's mixer (rp_snd_write), which is one of its streams.
*/

#define RING_FRAMES 16384 // 0.37 s at 44.1 kHz: power of two, as the engine's mixer wants

/*
Kit 51 (R306): the OS's mixer is called once a paint, not once a frame.
The engine paints in pieces (S_TransferPaintBuffer: each piece its own
BeginPainting and Submit) and S_ExtraUpdate paints again from the client's
model drawing, so run 50 called the OS ~15 times a game frame (333 a
second), and its own sound lines show each call's cost is mostly the call
(the mixer took 2,562 ms in 16,839 calls one minute, 2,926 ms in 20,673
another: ~95 us an extra call, ~1 s a minute for the mixing itself; 4.8%
of the time in all). The SDL and ALSA backends never meet this: their
audio thread pulls once a period. Ours, with no thread: the play position
read again only when 8 ms have passed (the engine paints a little less
from a position at most 8 ms old: a mix ahead of 100 ms covers it), and
what was painted handed over once a game frame's worth is waiting or 20 ms
have passed (a piece stays in the engine's ring, which holds 370 ms).
*/
#define SUBMIT_FRAMES 1024  // 23 ms at 44.1 kHz
#define SUBMIT_US     20000
#define PLAYED_US     8000

static struct
{
	int64_t sent; // frames handed to the OS
	qboolean open;
	uint64_t played_at, sent_at; // when the OS was last asked, last given sound
	int samplepos;
	int calls, saved; // calls made and not made (the run log's line)
	uint64_t since;
} s_rp;

qboolean SNDDMA_Init( void )
{
	/* -rpmix22 (the owner's go, 2026-11-17, R306): the engine mixes at
	   22,050 Hz, half the work of its mixer and room effects, and the OS
	   mixer upsamples to the codec's 44.1 kHz (most of Half-Life's sounds
	   are 22 or 11 kHz; a 44.1 kHz sound loses its top octave). Measured
	   against 44.1 kHz on the device before it is a default. */
	const int rate = Sys_CheckParm( "-rpmix22" ) ? SOUND_22k : SOUND_DMA_SPEED;

	if( rp_snd_open( rate, 2 ) != 0 )
	{
		Con_Printf( S_ERROR "%s: the OS gave no sound stream\n", __func__ );
		return false;
	}

	snd.format.speed = rate;
	Con_Printf( "sound: the engine mixes at %d Hz\n", rate );
	snd.format.channels = 2;
	snd.format.width = 2;
	snd.samples = RING_FRAMES * 2;
	snd.samplepos = 0;
	snd.buffer = Z_Calloc( snd.samples * 2 );
	snd.backend_name = "RetroPane";
	snd.initialized = true;

	s_rp.sent = 0;
	s_rp.open = true;

	return true;
}

void SNDDMA_Shutdown( void )
{
	snd.initialized = false;

	if( s_rp.open )
	{
		rp_snd_close();
		s_rp.open = false;
	}

	if( snd.buffer )
	{
		Mem_Free( snd.buffer );
		snd.buffer = NULL;
	}
}

/*
Where the device plays, as the engine counts it: mono samples into the
ring. The engine's soundtime follows from this.
*/
void SNDDMA_BeginPainting( void )
{
	int64_t played;

	if( !s_rp.open )
		return;

	{
		uint64_t now = rp_time_us();

		if( s_rp.played_at && now - s_rp.played_at < PLAYED_US )
		{
			snd.samplepos = s_rp.samplepos;
			s_rp.saved++;
			return;
		}
		s_rp.played_at = now;
	}
	s_rp.calls++;
	played = rp_snd_played();
	if( played >= 0 )
		snd.samplepos = (int)(( played * 2 ) % snd.samples );
	s_rp.samplepos = snd.samplepos;
}

/*
What was painted since the last frame goes to the OS, in at most two
pieces (the ring's wrap). The OS takes what it has room for; the rest is
still in the ring next frame, unless the engine has painted over it (a
frame slower than the ring is long: that sound is lost, as on any device).
*/
void SNDDMA_Submit( void )
{
	int64_t end = snd.paintedtime;

	if( !s_rp.open || !snd.buffer )
		return;

	if( end - s_rp.sent > RING_FRAMES )
		s_rp.sent = end - RING_FRAMES;

	{
		uint64_t now = rp_time_us();

		if( end - s_rp.sent < SUBMIT_FRAMES && now - s_rp.sent_at < SUBMIT_US )
		{
			if( end > s_rp.sent )
				s_rp.saved++;
			return;
		}
		s_rp.sent_at = now;
		if( now - s_rp.since >= 60000000u )
		{
			fprintf( stderr, "s_rp: the OS's sound asked %d times in the last minute, %d calls saved (kit 51)\n", s_rp.calls, s_rp.saved );
			/* kit 63's guard on the network's bounded clear (net_chan.c) */
			{
				extern uint32_t rp_netsend_short;

				fprintf( stderr, "net: packets past the bounded clear so far: %u (kit 63; 0 expected)\n", (unsigned)rp_netsend_short );
			}
			s_rp.calls = s_rp.saved = 0;
			s_rp.since = now;
		}
	}

	while( s_rp.sent < end )
	{
		int at = (int)( s_rp.sent % RING_FRAMES );
		int frames = (int)( end - s_rp.sent );
		int took;

		if( frames > RING_FRAMES - at )
			frames = RING_FRAMES - at;

		{
			/* the sound "cant hardly be heard" (the owner, run 37): the
			   engine's own output peak every 5 s, against full scale
			   (32767), so the stage that loses level is measured */
			static int      peak;
			static uint64_t since;
			const int16_t  *p = (const int16_t *)snd.buffer + at * 2;
			uint64_t        now = rp_time_us();

			for( int i = 0; i < frames * 2; i++ )
			{
				int a = p[i] < 0 ? -p[i] : p[i];

				if( a > peak )
					peak = a;
			}
			if( now - since >= 60000000u ) /* a minute: each line is a card write (the hitches) */
			{
				fprintf( stderr, "s_rp: the engine's output peak %d of 32767 (%d%%) in the last minute, volume %s\n",
					peak, peak * 100 / 32767, Cvar_VariableString( "volume" ));
				peak = 0;
				since = now;
			}
		}
		s_rp.calls++;
		took = rp_snd_write((const int16_t *)snd.buffer + at * 2, (size_t)frames * 2 );
		if( took <= 0 )
			break;

		s_rp.sent += took;
		if( took < frames )
			break;
	}
}

void SNDDMA_Activate( qboolean active )
{
	// the OS pauses the stream itself when the headphones come out
}

/*
No microphone for a game yet (the OS asks the user first, ADR-0032 A5).
*/
qboolean VoiceCapture_Init( void )
{
	return false;
}

qboolean VoiceCapture_Activate( qboolean activate )
{
	return false;
}

qboolean VoiceCapture_Lock( qboolean lock )
{
	return false;
}

void VoiceCapture_Shutdown( void )
{
}
