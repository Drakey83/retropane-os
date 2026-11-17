/*
rp_fdot.h - RetroPane OS (Dell Axim X51v): a float dot product of three,
bit for bit C's ((a0 * b0 + a1 * b1) + a2 * b2), in one call (R316, kit 64).

Iterate, don't imitate (rule 0). The PXA270 has no FPU: C's dot product is
five software-float calls (three multiplies, two adds; rp_fmul_asm and
rp_fadd_asm, IEEE single, round to nearest even), each unpacking its two
operands, rounding and packing its result again (run 60: the floats 8.9% of
Half-Life's play, the dot products a large part: the traces' planes, the
flashlight's marks, the lighting). Quake and Xash write the dot as the C
expression; a fused multiply-add would round once and give different bits.
Here each operand is unpacked once and every product and sum is rounded
exactly as C rounds it, in integers kept unpacked between the steps: the
same float. Any operand or result off the common path (a NaN, an infinity,
a subnormal, an overflow or an underflow) takes C's own expression, so the
answer is always C's. Checked on the PC against IEEE hardware (x86 SSE, no
contraction) over billions of random and edge vectors (tools: the kit 64
note in docs/design/ref-rp.md).
*/
#ifndef RP_FDOT_H
#define RP_FDOT_H

#include <stdint.h>

/* The same in ARM assembly (kit 65, hal/mem/src/float_arm.rs), linked into
every module with the float routines: what DotProduct calls. The C below is
its reference (its own proof on the PC); the runner's -rpfloat check
compares the assembly against the compiler's generic routines. */
float rp_fdot3_asm( const float *a, const float *b );

/* A float[3] . float[3] by the assembly; anything else C's expression. */
#define RP_DOT_C( x, y ) ((x)[0]*(y)[0]+(x)[1]*(y)[1]+(x)[2]*(y)[2])
#define RP_DOT( x, y ) _Generic( +(x)[0], 	float: _Generic( +(y)[0], 		float: rp_fdot3_asm( (const float *)(x), (const float *)(y) ), 		default: RP_DOT_C( x, y )), 	default: RP_DOT_C( x, y ))

/* An unpacked float: zero (e = 0), or m (bit 23 set) x 2^(e - 150). */
typedef struct
{
	uint32_t s, e, m;
} rp_fx_t;

/* The product a b rounded to a float (1: done; 0: off the common path). */
static inline int rp_fx_mul( uint32_t a, uint32_t b, rp_fx_t *r )
{
	uint32_t ea = a >> 23 & 0xFF, eb = b >> 23 & 0xFF;
	uint64_t p;
	int32_t  e;
	uint32_t sh, m, rem, half;

	r->s = ( a ^ b ) >> 31;
	if( ea == 0 || eb == 0 )
	{
		/* a zero times a finite number: a signed zero (a subnormal: off) */
		if((( a << 1 ) == 0 && eb != 0xFF && ( eb != 0 || ( b << 1 ) == 0 ))
			|| (( b << 1 ) == 0 && ea != 0xFF && ( ea != 0 || ( a << 1 ) == 0 )))
		{
			r->e = 0;
			r->m = 0;
			return 1;
		}
		return 0;
	}
	if( ea == 0xFF || eb == 0xFF )
		return 0;
	p = (uint64_t)(( a & 0x7FFFFFu ) | 0x800000u ) * (( b & 0x7FFFFFu ) | 0x800000u );
	/* p in [2^46, 2^48): 24 bits kept, the rest rounded */
	sh = ( p >> 47 ) ? 24 : 23;
	e = (int32_t)ea + (int32_t)eb - 127 + (int32_t)( sh - 23 );
	if( e < 1 )
		return 0;
	m = (uint32_t)( p >> sh );
	rem = (uint32_t)( p & (( 1u << sh ) - 1 ));
	half = 1u << ( sh - 1 );
	if( rem > half || ( rem == half && ( m & 1 )))
		m++;
	if( m == 0x1000000u )
	{
		m >>= 1;
		e++;
	}
	if( e > 254 )
		return 0;
	r->e = (uint32_t)e;
	r->m = m;
	return 1;
}

/* x + y rounded to a float, both from rp_fx_mul or rp_fx_add (1: done). */
static inline int rp_fx_add( rp_fx_t x, rp_fx_t y, rp_fx_t *r )
{
	uint32_t d, xs, ys, sum, mant, rem;
	int32_t  e;

	if( y.e == 0 )
	{
		/* x + 0: x, or two zeros: -0 only if both are */
		if( x.e == 0 )
			x.s &= y.s;
		*r = x;
		return 1;
	}
	if( x.e == 0 )
	{
		*r = y;
		return 1;
	}
	if( y.e > x.e || ( y.e == x.e && y.m > x.m ))
	{
		rp_fx_t t = x;

		x = y;
		y = t;
	}
	/* the significands at bits 30..7, seven bits below for the rounding */
	d = x.e - y.e;
	xs = x.m << 7;
	if( d >= 32 )
		ys = 1;
	else
	{
		ys = ( y.m << 7 ) >> d;
		if(( ys << d ) != ( y.m << 7 ))
			ys |= 1;
	}
	e = (int32_t)x.e;
	if( x.s == y.s )
	{
		sum = xs + ys;
		if( sum >> 31 )
		{
			sum = ( sum >> 1 ) | ( sum & 1 );
			e++;
		}
	}
	else
	{
		uint32_t lz;

		sum = xs - ys;
		if( sum == 0 )
		{
			/* x - x: +0 */
			r->s = 0;
			r->e = 0;
			r->m = 0;
			return 1;
		}
		lz = (uint32_t)__builtin_clz( sum ) - 1;
		sum <<= lz;
		e -= (int32_t)lz;
		if( e < 1 )
			return 0;
	}
	mant = sum >> 7;
	rem = sum & 0x7F;
	if( rem > 0x40 || ( rem == 0x40 && ( mant & 1 )))
		mant++;
	if( mant == 0x1000000u )
	{
		mant >>= 1;
		e++;
	}
	if( e > 254 )
		return 0;
	r->s = x.s;
	r->e = (uint32_t)e;
	r->m = mant;
	return 1;
}

/* C's ((a[0] * b[0] + a[1] * b[1]) + a[2] * b[2]), bit for bit. */
static float __attribute__(( noinline, unused )) rp_fdot3( const float *a, const float *b )
{
	uint32_t ua[3], ub[3], u;
	rp_fx_t  p0, p1, p2, s, r;
	float    f;

	__builtin_memcpy( ua, a, sizeof( ua ));
	__builtin_memcpy( ub, b, sizeof( ub ));
	if( !rp_fx_mul( ua[0], ub[0], &p0 ) || !rp_fx_mul( ua[1], ub[1], &p1 ) || !rp_fx_add( p0, p1, &s )
		|| !rp_fx_mul( ua[2], ub[2], &p2 ) || !rp_fx_add( s, p2, &r ))
		return ( a[0] * b[0] + a[1] * b[1] ) + a[2] * b[2];
	u = r.e ? ( r.s << 31 | r.e << 23 | ( r.m & 0x7FFFFFu )) : r.s << 31;
	__builtin_memcpy( &f, &u, sizeof( f ));
	return f;
}

#endif
