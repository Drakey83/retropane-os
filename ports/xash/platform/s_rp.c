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

static struct
{
	int64_t sent; // frames handed to the OS
	qboolean open;
} s_rp;

qboolean SNDDMA_Init( void )
{
	if( rp_snd_open( SOUND_DMA_SPEED, 2 ) != 0 )
	{
		Con_Printf( S_ERROR "%s: the OS gave no sound stream\n", __func__ );
		return false;
	}

	snd.format.speed = SOUND_DMA_SPEED;
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

	played = rp_snd_played();
	if( played >= 0 )
		snd.samplepos = (int)(( played * 2 ) % snd.samples );
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

	while( s_rp.sent < end )
	{
		int at = (int)( s_rp.sent % RING_FRAMES );
		int frames = (int)( end - s_rp.sent );
		int took;

		if( frames > RING_FRAMES - at )
			frames = RING_FRAMES - at;

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
