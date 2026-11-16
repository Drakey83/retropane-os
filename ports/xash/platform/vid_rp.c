/*
vid_rp.c - RetroPane OS (Dell Axim X51v): the screen for the software renderer
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
#include "vid_common.h"
#include "platform/platform.h"

/*
The X51v's panel is 480 x 640, upright; the game plays in landscape, so it
draws 640 x 480 and the picture is turned on its way out.

This first backend is for the evaluation with the engine's own software
renderer (ref_soft): one 16-bit buffer in RAM that the renderer fills.
The port layer has no call yet to put a buffer on the screen (the C face
of the graphics service is not built), so a finished frame is counted
and goes nowhere. The real renderer (ref_rp) draws through the 2700G's
display lists and never uses this buffer.
*/

#define RP_WIDTH  640
#define RP_HEIGHT 480

static struct
{
	word *buffer;
	uint frames;
} vid_rp;

static vidmode_t rp_mode = { "640 x 480 (landscape)", RP_WIDTH, RP_HEIGHT };

qboolean R_Init_Video( ref_graphic_apis_t type )
{
	if( type != REF_SOFTWARE )
		return false; // ref_rp brings its own way to the 2700G

	if( !VID_SetMode( ))
		return false;

	host.renderinfo_changed = false;
	return true;
}

void R_Free_Video( void )
{
	if( vid_rp.buffer )
	{
		Mem_Free( vid_rp.buffer );
		vid_rp.buffer = NULL;
	}

	ref.dllFuncs.GL_ClearExtensions();
}

qboolean VID_SetMode( void )
{
	return R_ChangeDisplaySettings( RP_WIDTH, RP_HEIGHT, WINDOW_MODE_FULLSCREEN ) == rserr_ok;
}

rserr_t R_ChangeDisplaySettings( int width, int height, window_mode_t window_mode )
{
	int render_w = RP_WIDTH, render_h = RP_HEIGHT;

	// one size, the whole screen: what was asked for is not looked at
	VID_SetDisplayTransform( &render_w, &render_h );
	R_SaveVideoMode( RP_WIDTH, RP_HEIGHT, render_w, render_h, false );

	return rserr_ok;
}

int R_MaxVideoModes( void )
{
	return 1;
}

vidmode_t *R_GetVideoMode( int num )
{
	return num == 0 ? &rp_mode : NULL;
}

ref_window_type_t R_GetWindowHandle( void **handle, ref_window_type_t type )
{
	return REF_WINDOW_TYPE_NULL;
}

void VID_Info_f( void )
{
	Con_Printf( "RetroPane: %d x %d, 16 bits, %u frames\n", RP_WIDTH, RP_HEIGHT, vid_rp.frames );
}

void Platform_Minimize_f( void )
{
}

/*
The software renderer's buffer: RGB 5-6-5, as the 2700G's planes take it.
*/
qboolean SW_CreateBuffer( int width, int height, uint *stride, uint *bpp, uint *r, uint *g, uint *b )
{
	if( width != RP_WIDTH || height != RP_HEIGHT )
		return false;

	if( !vid_rp.buffer )
		vid_rp.buffer = Z_Calloc( RP_WIDTH * RP_HEIGHT * sizeof( word ));

	*stride = RP_WIDTH;
	*bpp = 2;
	*r = 31 << 11;
	*g = 63 << 5;
	*b = 31;

	return true;
}

void *SW_LockBuffer( void )
{
	return vid_rp.buffer;
}

/*
A test image's eye: with -rpshot on the command line the 30th finished
frame is written to the card as it is in the buffer (640 x 480 words of
RGB 5-6-5, rows from the top), in the runner's folder. The PC turns it
into a picture (`cargo xtask emu-xash`): until the port layer has a screen
call, this is how a frame is looked at.
*/
#define RP_SHOT_FRAME 30
#define RP_SHOT_FILE  "/rp-xash/frame.565"

static void RP_Shot( void )
{
	FILE *f;

	if( !Sys_CheckParm( "-rpshot" ))
		return;

	f = fopen( RP_SHOT_FILE, "wb" );
	if( !f )
	{
		Con_Printf( S_ERROR "frame %u not written: %s could not be made\n", vid_rp.frames, RP_SHOT_FILE );
		return;
	}
	fwrite( vid_rp.buffer, sizeof( word ), RP_WIDTH * RP_HEIGHT, f );
	fclose( f );
	Con_Printf( "frame %u written to %s\n", vid_rp.frames, RP_SHOT_FILE );
}

void SW_UnlockBuffer( void )
{
	// the frame is whole: where the port layer's screen call will go
	vid_rp.frames++;
	if( vid_rp.frames == RP_SHOT_FRAME )
		RP_Shot();
}

/*
No OpenGL here: these are asked for by name whatever the renderer.
*/
void GL_SwapBuffers( void )
{
}

void GL_UpdateSwapInterval( void )
{
}

int GL_SetAttribute( int attr, int val )
{
	return 0;
}

int GL_GetAttribute( int attr, int *val )
{
	return 0;
}

void *GL_GetProcAddress( const char *name )
{
	return NULL;
}
