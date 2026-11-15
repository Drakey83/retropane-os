/*
absent.c - RetroPane OS (Dell Axim X51v): the answers of what is left out of the build
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
#include "soundlib.h"
#include "voice.h"
#include <bzlib.h>
#include <multi_emulator.h>
#include <netdb.h>
#include <netinet/in.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/socket.h>

/*
Left out of RetroPane's build of the engine, each because single-player
Half-Life from the user's own valve folder never needs it and every
kilobyte of the base image counts (docs/research/half-life-evaluation.md
has the list and what adding each back takes):

- Ogg Vorbis and Opus decoders (snd_ogg_*.c): the game's sounds are WAV
  and the Steam copy's music is MP3, which is in the build (mpg123's
  fixed-point decoder: xtask/src/xash.rs).
- Voice chat (voice.c) and its Opus codec: multiplayer.
- bzip2: packs file fragments between a server and its clients.
- MultiEmulator: Steam ID tickets for joining servers.
- The network: no socket can be opened (ports/xash/include/sys/socket.h).

Every name the engine still calls is answered here with "not there".
*/

// ---- compressed sound formats ----------------------------------------

qboolean Sound_LoadOggVorbis( const char *name, const byte *buffer, fs_offset_t filesize )
{
	return false;
}

qboolean Sound_LoadOggOpus( const char *name, const byte *buffer, fs_offset_t filesize )
{
	return false;
}

#define ABSENT_STREAM( format ) \
	stream_t *Stream_Open##format( const char *filename ) { return NULL; } \
	int Stream_Read##format( stream_t *stream, int bytes, void *buffer ) { return 0; } \
	int Stream_SetPos##format( stream_t *stream, int newpos ) { return false; } \
	int Stream_GetPos##format( stream_t *stream ) { return -1; } \
	void Stream_Free##format( stream_t *stream ) { }

ABSENT_STREAM( OggVorbis )
ABSENT_STREAM( OggOpus )

// ---- voice chat ---------------------------------------------------------

CVAR_DEFINE_AUTO( voice_loopback, "0", FCVAR_PRIVILEGED, "loopback voice back to the speaker (not in this build)" );

void CL_AddVoiceToDatagram( void )
{
}

void Voice_RegisterCvars( void )
{
	Cvar_RegisterVariable( &voice_loopback );
}

qboolean Voice_Init( const char *pszCodecName, int quality, qboolean preinit )
{
	return false;
}

void Voice_Idle( double frametime )
{
}

qboolean Voice_IsRecording( void )
{
	return false;
}

void Voice_RecordStop( void )
{
}

void Voice_RecordStart( void )
{
}

void Voice_Disconnect( void )
{
}

void Voice_AddIncomingData( int ent, const byte *data, uint size, uint frames )
{
}

void Voice_StopChannel( int entnum )
{
}

void Voice_LoopbackAck( void )
{
}

voice_audio_info_t Voice_GetAudioInfo( void )
{
	voice_audio_info_t info = { 0 };

	return info;
}

// ---- bzip2 --------------------------------------------------------------

const char *BZ2_bzlibVersion( void )
{
	return "not in this build";
}

int BZ2_bzBuffToBuffCompress( char *dest, unsigned int *destLen, char *source, unsigned int sourceLen,
	int blockSize100k, int verbosity, int workFactor )
{
	return BZ_CONFIG_ERROR; // the engine then sends the fragment as it is
}

int BZ2_bzBuffToBuffDecompress( char *dest, unsigned int *destLen, char *source, unsigned int sourceLen,
	int small, int verbosity )
{
	return BZ_DATA_ERROR;
}

// ---- Steam ID tickets ---------------------------------------------------

int GenerateRevEmu2013( void *dest, const char *key, unsigned int hash )
{
	return 0; // an empty ticket: there is no server to send it to
}

// ---- the network --------------------------------------------------------

const struct in6_addr in6addr_any = IN6ADDR_ANY_INIT;
const struct in6_addr in6addr_loopback = IN6ADDR_LOOPBACK_INIT;

static int no_network( void )
{
	errno = ENETDOWN;
	return -1;
}

int socket( int domain, int type, int protocol ) { return no_network(); }
int bind( int fd, const struct sockaddr *addr, socklen_t len ) { return no_network(); }
int connect( int fd, const struct sockaddr *addr, socklen_t len ) { return no_network(); }
int listen( int fd, int backlog ) { return no_network(); }
int accept( int fd, struct sockaddr *addr, socklen_t *len ) { return no_network(); }
int getsockname( int fd, struct sockaddr *addr, socklen_t *len ) { return no_network(); }
int getsockopt( int fd, int level, int name, void *value, socklen_t *len ) { return no_network(); }
int setsockopt( int fd, int level, int name, const void *value, socklen_t len ) { return no_network(); }
int shutdown( int fd, int how ) { return no_network(); }
ssize_t send( int fd, const void *buf, size_t n, int flags ) { return no_network(); }
ssize_t recv( int fd, void *buf, size_t n, int flags ) { return no_network(); }
ssize_t sendto( int fd, const void *buf, size_t n, int flags, const struct sockaddr *to, socklen_t len ) { return no_network(); }
ssize_t recvfrom( int fd, void *buf, size_t n, int flags, struct sockaddr *from, socklen_t *len ) { return no_network(); }
int ioctl( int fd, unsigned long op, void *param ) { return no_network(); }
int select( int n, fd_set *readfds, fd_set *writefds, fd_set *exceptfds, struct timeval *timeout ) { return no_network(); }

in_addr_t inet_addr( const char *text ) { return INADDR_NONE; }
const char *inet_ntop( int family, const void *addr, char *text, socklen_t size ) { errno = EAFNOSUPPORT; return NULL; }
int inet_pton( int family, const char *text, void *addr ) { return 0; }

struct hostent *gethostbyname( const char *name ) { return NULL; }
int gethostname( char *name, size_t size ) { return no_network(); }
int getaddrinfo( const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **out ) { return EAI_FAIL; }
void freeaddrinfo( struct addrinfo *list ) { }
const char *gai_strerror( int code ) { return "no network"; }
int getnameinfo( const struct sockaddr *addr, socklen_t len, char *host, socklen_t hostlen, char *serv,
	socklen_t servlen, int flags ) { return EAI_FAIL; }
