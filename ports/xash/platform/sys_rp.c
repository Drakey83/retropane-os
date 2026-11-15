/*
sys_rp.c - RetroPane OS (Dell Axim X51v): the clock, sleeping, where the engine is
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

#include "platform/platform.h"
#include <retropane.h>

/*
The engine's clock is a double of seconds. The port layer's is whole
microseconds (exact, from the PXA270's counter): one conversion and one
multiply in software floating point a call, where the POSIX backend pays
a kernel call and two of each. Reading it also wakes the sound's mixer
and reads the buttons if their time has come.
*/
double Platform_DoubleTime( void )
{
	return (double)rp_time_us() * 0.000001;
}

/*
The port layer has no call to sleep yet (a program is the only thing
running; idling the CPU until an interrupt is the OS's to offer): the
time is waited out on the clock, which keeps the sound fed meanwhile.
*/
void Platform_Sleep( int msec )
{
	uint64_t until = rp_time_us() + (uint64_t)( msec > 0 ? msec : 0 ) * 1000u;

	while( rp_time_us() < until )
		;
}

void Platform_ShellExecute( const char *path, const char *parms )
{
	// no other program to hand a path to
}

platform_orientation_t Platform_GetDisplayOrientation( void )
{
	// the game owns the whole screen turned on its side (see vid_rp.c)
	return ORIENTATION_LANDSCAPE;
}

qboolean Platform_CheckLibraryDirectDependency( const byte *data, size_t size, const char *depname )
{
	return false;
}

/*
whereami.c (the path of the running program) is left out of the build: a
RetroPane program has no path of its own, the game's folder is found by
the port layer (the internal flash, then the SD card, then the CF card).
*/
int wai_getExecutablePath( char *out, int capacity, int *dirname_length )
{
	return -1;
}

int wai_getModulePath( char *out, int capacity, int *dirname_length )
{
	return -1;
}
