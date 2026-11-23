/*
rp_sort.h - RetroPane OS (Dell Axim X51v): ref_rp's sort of a model
command's triangles (docs/design/ref-rp.md; R360 step 3,
docs/research/sheets/r360-sound-chain-and-corners.md).

Pure (stdint only; the includer defines RP_VERTEX_WORDS and
RP_STUDIO_CMD), so the PC's check (check/sort_check.c: the new way against
the old over strips, fans, every cull and random, edge and extreme
corners) compiles this same file the engine does: the sort checked is the
sort shipped.
*/
#ifndef RP_SORT_H
#define RP_SORT_H

#include <stdint.h>

#define RP_TRI_KEEP 0
#define RP_TRI_BACK 1
#define RP_TRI_CUT  2
#define RP_TRI_GONE 3

static inline int RP_Facing( const int32_t a[2], const int32_t b[2], const int32_t c[2] )
{
	int64_t x = (int64_t)( b[0] - a[0] ) * ( c[1] - a[1] ) - (int64_t)( b[1] - a[1] ) * ( c[0] - a[0] );

	return x > 0 ? 1 : x < 0 ? -1 : 0;
}

static inline int RP_OffScreen( const int32_t a[2], const int32_t b[2], const int32_t c[2] )
{
	const int32_t w = 480 << 4, h = 640 << 4;

	return ( a[0] < 0 && b[0] < 0 && c[0] < 0 ) || ( a[0] > w && b[0] > w && c[0] > w )
		|| ( a[1] < 0 && b[1] < 0 && c[1] < 0 ) || ( a[1] > h && b[1] > h && c[1] > h );
}

/* The sort's states the old way (kit 68 to R360): each triangle's three
   corners tested one by one (1/z, the 64-bit cross product, the screen's
   four edges). -rpsort0, and -rpsort2's reference. */
static void RP_SortStatesOld( int fan, int n, const uint32_t ( *c )[RP_VERTEX_WORDS], const int32_t ( *cxy )[2], int cull, uint8_t *state )
{
	for( int i = 0; i < n - 2; i++ )
	{
		int t0 = fan ? 0 : i, t1 = i + 1, t2 = i + 2;
		/* in front: 1/z's word made (0 behind the near plane, both paths) */
		int ins = ( c[t0][2] != 0 ) + ( c[t1][2] != 0 ) + ( c[t2][2] != 0 );

		if( ins == 0 )
			state[i] = RP_TRI_GONE;
		else if( ins < 3 )
			state[i] = RP_TRI_CUT;
		else if( !cull )
			state[i] = RP_TRI_KEEP;
		else
		{
			/* GL's strip order: an odd triangle's first two corners swap */
			int f = RP_Facing( cxy[t0], cxy[t1], cxy[t2] );

			if( !fan && ( i & 1 ))
				f = -f;
			state[i] = f * cull > 0 && !RP_OffScreen( cxy[t0], cxy[t1], cxy[t2] ) ? RP_TRI_KEEP : RP_TRI_BACK;
		}
	}
}

/*
R360 step 3 (runs 342a-d: RP_StudioCmdSend 5.1% of the fight's cycles,
~14% of them waiting on data, ~5% on the I-cache: the sort's own
instructions, ~80 a triangle). Iterate (rule 0): GL and ref_soft clip
with a code a vertex (Cohen-Sutherland's outcodes) but cull a triangle at
a time in floats; kit 68's sort tested each triangle's three corners one
by one, reading each corner's 1/z and x, y three times and taking the
cross product in 64 bits. Here each vertex's code is made once where it
is projected (RP_CornerCode: behind the near plane, past each of the
screen's four edges, and whether its x and y are within 2^14: 28.4, the
screen 7,680 x 10,240), a corner copies its vertex's byte, and a triangle
is the AND and OR of three bytes; its cross product in 32 bits when all
three corners are within 2^14 (each difference under 2^15, each product
under 2^30, their difference under 2^31: exact), in 64 bits otherwise.
Runs 343a-d's first build made the codes in a pass over the corners and
rolled them through registers: exact, but slower than the old loop (the
pass waited on 1/z's freshly written lines, the loop spilled); hence the
codes made with the vertex and a loop each for strips, fans and no cull.
The same state for every triangle (-rpsort2 counts any that differ).
*/
#define RP_CC_OFF    15u /* x < 0, x > w, y < 0, y > h */
#define RP_CC_BEHIND 16u
#define RP_CC_FAR    32u

/* A corner's code: `front` 0 behind the near plane (then x, y unused) */
static inline uint8_t RP_CornerCode( int32_t x, int32_t y, int front )
{
	const int32_t w = 480 << 4, h = 640 << 4;
	uint32_t      cc;

	if( !front )
		return RP_CC_BEHIND;
	/* RP_OffScreen's four tests, a bit each (comparisons: exact for any
	   x, y) */
	cc = (uint32_t)( x < 0 ) | (uint32_t)( x > w ) << 1 | (uint32_t)( y < 0 ) << 2 | (uint32_t)( y > h ) << 3;
	if( (uint32_t)x + ( 1u << 14 ) >= ( 1u << 15 ) || (uint32_t)y + ( 1u << 14 ) >= ( 1u << 15 ))
		cc |= RP_CC_FAR;
	return (uint8_t)cc;
}

/* A front triangle's state from its codes' AND and OR and its corners:
   `cs` the cull sign for this triangle (cull, negated for a strip's odd
   triangles: GL's strip order) */
static inline uint8_t RP_SortCull( uint32_t a, uint32_t o, const int32_t *p0, const int32_t *p1, const int32_t *p2, int cs )
{
	int32_t x;

	if( a & RP_CC_OFF )
		return RP_TRI_BACK;
	if( o & RP_CC_FAR )
		return RP_Facing( p0, p1, p2 ) * cs > 0 ? RP_TRI_KEEP : RP_TRI_BACK;
	x = ( p1[0] - p0[0] ) * ( p2[1] - p0[1] ) - ( p1[1] - p0[1] ) * ( p2[0] - p0[0] );
	return ( cs > 0 ? x > 0 : x < 0 ) ? RP_TRI_KEEP : RP_TRI_BACK;
}

static void RP_SortStates( int fan, int n, const uint8_t *code, const int32_t ( *cxy )[2], int cull, uint8_t *state )
{
	int tris = n - 2;

	if( !cull )
	{
		for( int i = 0; i < tris; i++ )
		{
			uint32_t k0 = code[fan ? 0 : i], k1 = code[i + 1], k2 = code[i + 2];

			state[i] = ( k0 & k1 & k2 & RP_CC_BEHIND ) ? RP_TRI_GONE : (( k0 | k1 | k2 ) & RP_CC_BEHIND ) ? RP_TRI_CUT : RP_TRI_KEEP;
		}
		return;
	}
	if( fan )
	{
		const uint32_t k0 = code[0];

		for( int i = 0; i < tris; i++ )
		{
			uint32_t k1 = code[i + 1], k2 = code[i + 2], a = k0 & k1 & k2, o = k0 | k1 | k2;

			state[i] = ( a & RP_CC_BEHIND ) ? RP_TRI_GONE : ( o & RP_CC_BEHIND ) ? RP_TRI_CUT
				: RP_SortCull( a, o, cxy[0], cxy[i + 1], cxy[i + 2], cull );
		}
		return;
	}
	for( int i = 0, cs = cull; i < tris; i++, cs = -cs )
	{
		uint32_t k0 = code[i], k1 = code[i + 1], k2 = code[i + 2], a = k0 & k1 & k2, o = k0 | k1 | k2;

		state[i] = ( a & RP_CC_BEHIND ) ? RP_TRI_GONE : ( o & RP_CC_BEHIND ) ? RP_TRI_CUT
			: RP_SortCull( a, o, cxy[i], cxy[i + 1], cxy[i + 2], cs );
	}
}

/* The corners a sent (kept or cut) triangle uses: a strip's corner k by
   triangles k - 2 to k, a fan's by k - 2 and k - 1, and its first by any. */
static void RP_SortNeed( int fan, int n, const uint8_t *state, uint8_t *need )
{
	int      tris = n - 2;
	uint32_t s2 = 0, s1 = 0, any = 0;

	/* sent: RP_TRI_KEEP (0) or RP_TRI_CUT (2); bit 0 set for the others */
	for( int k = 0; k < n; k++ )
	{
		uint32_t s0 = k < tris ? !( state[k] & 1 ) : 0;

		any |= s0;
		need[k] = (uint8_t)( fan ? s1 | s2 : s0 | s1 | s2 );
		s2 = s1;
		s1 = s0;
	}
	if( fan )
		need[0] = (uint8_t)any;
}

#endif
