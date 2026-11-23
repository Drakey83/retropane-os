/*
sort_check.c - RetroPane OS: the PC's check of ref_rp's sort (rp_sort.h,
R360 step 3): RP_SortStates on RP_CornerCode's codes against
RP_SortStatesOld, and RP_SortNeed
against the old fill set (each sent triangle's three corners), over
strips and fans of 3 to 1,024 corners, cull -1, 0 and 1, corners behind
the near plane, and random, edge (the screen's sides, 2^14, 2^15) and
extreme (INT32_MIN, INT32_MAX) x, y. -fwrapv: the old cross product's
32-bit differences wrap as the ARM's do (both ways call it for far
corners). Run from this folder (WSL or any gcc / clang):

    gcc -O2 -std=gnu11 -Wall -Wextra -fwrapv sort_check.c -o /tmp/sort_check && /tmp/sort_check

2026-11-23: 12,108,717 triangles (all four states), 0 differ (both builds).
*/
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define RP_VERTEX_WORDS 7
#define RP_STUDIO_CMD   1024
#include "../rp_sort.h"

static uint64_t rng = 0x9E3779B97F4A7C15ull;
static uint32_t rnd( void )
{
	rng ^= rng << 13;
	rng ^= rng >> 7;
	rng ^= rng << 17;
	return (uint32_t)rng;
}

static int32_t coord( int kind, int32_t lim )
{
	static const int32_t edge[] = { 0, -1, 1, 7680, 7679, 7681, 10240, 10239, 10241, 16383, 16384, -16384, -16385,
		32767, -32768, INT32_MAX, INT32_MIN, INT32_MAX - 1, INT32_MIN + 1, 1 << 30, -( 1 << 30 ) };
	switch( kind )
	{
	case 0: return (int32_t)( rnd( ) % (uint32_t)( 2 * lim + 1 )) - lim;
	case 1: return edge[rnd( ) % ( sizeof( edge ) / sizeof( edge[0] ))];
	case 2: return (int32_t)rnd( );
	default: return (int32_t)( rnd( ) % 20000 ) - 5000;
	}
}

int main( void )
{
	static uint32_t c[RP_STUDIO_CMD][RP_VERTEX_WORDS];
	static int32_t  cxy[RP_STUDIO_CMD][2];
	static uint8_t  code[RP_STUDIO_CMD], s1[RP_STUDIO_CMD], s2[RP_STUDIO_CMD], need[RP_STUDIO_CMD], old[RP_STUDIO_CMD];
	uint64_t        tris = 0, bad = 0, badfill = 0, kinds[4] = { 0 };

	for( int cmd = 0; cmd < 400000; cmd++ )
	{
		int n = 3 + (int)( rnd( ) % ( cmd % 50 == 0 ? RP_STUDIO_CMD - 3 : 40 ));
		int fan = rnd( ) & 1, cull = (int)( rnd( ) % 3 ) - 1, mix = rnd( ) % 4;
		int32_t lim = ( rnd( ) & 1 ) ? 12000 : 40000;
		int behind_odds = rnd( ) % 4; /* 0: none behind */

		for( int k = 0; k < n; k++ )
		{
			int kind = mix == 3 ? (int)( rnd( ) % 4 ) : mix == 2 ? 1 : 0;

			cxy[k][0] = coord( kind, lim );
			cxy[k][1] = coord( kind, lim );
			c[k][2] = behind_odds && rnd( ) % ( 2 + 4 * behind_odds ) == 0 ? 0 : 1 + rnd( );
			/* as the engine: 1/z's word 0 only behind the near plane */
			code[k] = RP_CornerCode( cxy[k][0], cxy[k][1], c[k][2] != 0 );
		}
		RP_SortStates( fan, n, code, (const int32_t( * )[2] )cxy, cull, s1 );
		RP_SortStatesOld( fan, n, (const uint32_t( * )[RP_VERTEX_WORDS] )c, (const int32_t( * )[2] )cxy, cull, s2 );
		for( int i = 0; i < n - 2; i++ )
		{
			bad += s1[i] != s2[i];
			kinds[s2[i]]++;
		}
		tris += (uint64_t)( n - 2 );
		RP_SortNeed( fan, n, s2, need );
		memset( old, 0, (size_t)n );
		for( int i = 0; i < n - 2; i++ )
			if( s2[i] == RP_TRI_KEEP || s2[i] == RP_TRI_CUT )
				old[fan ? 0 : i] = old[i + 1] = old[i + 2] = 1;
		badfill += memcmp( old, need, (size_t)n ) != 0;
	}
	printf( "%llu triangles (keep %llu back %llu cut %llu gone %llu): %llu states differ, %llu fill sets differ\n",
		(unsigned long long)tris, (unsigned long long)kinds[0], (unsigned long long)kinds[1], (unsigned long long)kinds[2],
		(unsigned long long)kinds[3], (unsigned long long)bad, (unsigned long long)badfill );
	return bad || badfill;
}
