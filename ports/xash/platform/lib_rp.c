/*
lib_rp.c - RetroPane OS (Dell Axim X51v): loading the engine's libraries
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
#include "library.h"
#include "platform/platform.h"

/*
The engine loads its parts by name: the file system, a renderer, the menu,
and the game's own logic (the client and server libraries of the
Half-Life SDK). The game's logic must stay a library of its own, loaded at
run time and never linked into the engine (Valve's SDK terms and the GPL
do not combine: docs/research/half-life-licences.md).

How RetroPane runs a program this big and loads its libraries is not
decided yet (docs/design/half-life-readiness.md line 6: the OS's own
loader for portable executables is the plan). Until it is, every load is
refused here, plainly, so the engine says which library it wanted instead
of crashing: this is the one piece between a built engine and a running
one.
*/

void *COM_LoadLibrary( const char *dllname, int build_ordinals_table, qboolean directpath )
{
	Con_Printf( S_ERROR "%s: %s not loaded: RetroPane's library loader is not built yet\n", __func__, dllname );
	return NULL;
}

void COM_FreeLibrary( void *hInstance )
{
}

void *COM_GetProcAddress( void *hInstance, const char *name )
{
	return NULL;
}

void *COM_FunctionFromName( void *hInstance, const char *pName )
{
	return NULL;
}

const char *COM_NameForFunction( void *hInstance, void *function )
{
	return NULL;
}
