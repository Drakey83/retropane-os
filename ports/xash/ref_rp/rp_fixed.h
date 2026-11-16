/*
rp_fixed.h - RetroPane OS (Dell Axim X51v): ref_rp's integer maths for the
models' bones and lighting (docs/design/ref-rp.md "Step 4", 4d's rest).

Pure (stdint only), so the PC's check (gcc, against the float
path over random bones, points and normals) compiles this same file the
engine does: the maths checked is the maths shipped.

Iterate, don't imitate (rule 0). `ref_soft` and `ref_gl` move each model
vertex by its bone's float 3 x 4 matrix into world space
(`Matrix3x4_VectorTransform`: 9 multiplies and 9 adds), and step 4b then
turned that float into 16.16 and moved it again by the view; the PXA270
has no FPU, so each of those is a software-float call (run 13: the bones
and lighting about 13% of the frame). Ours: each bone's matrix is joined
with the view's once (4.28 rotation, 16.16 offset in view space), so a
vertex goes from the model's file straight to view space in one integer
3 x 4 (nine `smull`), no world space and no float. The lighting
(`R_StudioLighting`'s hemispherical Lambert, its divide and the gamma
table, then `R_LightLambert`'s scale to bytes) becomes one dot product of
2.30 and 4.28, two multiplies by constants and the engine's own gamma
table, straight to the vertex colour's bytes.
*/
#ifndef RP_FIXED_H
#define RP_FIXED_H

#include <stdint.h>

/* f times 2^q as an integer, by the float's bits (no helper call):
   toward zero, as a C cast; saturated past 31 bits. */
static inline int32_t rp_fixq( float f, int q )
{
	uint32_t u, m;
	int      sh;
	int32_t  v;

	/* inline even where the build is -ffreestanding (no builtins) */
	__builtin_memcpy( &u, &f, sizeof( u ));
	if(( u & 0x7FFFFFFFu ) == 0 )
		return 0;
	/* f = 1.m 2^e, the value m 2^(e - 23 + q) */
	sh = (int)( u >> 23 & 0xFF ) - 127 - 23 + q;
	if( sh < -24 )
		return 0;
	if( sh > 7 )
		return u >> 31 ? -0x7FFFFFFF : 0x7FFFFFFF;
	m = ( u & 0x007FFFFFu ) | 0x00800000u;
	v = (int32_t)( sh >= 0 ? m << sh : m >> -sh );
	return u >> 31 ? -v : v;
}

/* A bone joined with the view: model space to view space. */
typedef struct
{
	int32_t m[3][3]; /* rotation (with the model's scale) as 4.28 */
	int32_t t[3];    /* the bone's origin in view space, 16.16 */
} rp_fbone_t;

/* `v` the view's rows (right, up, forward) as 2.30, `vo` the eye 16.16,
   `b` the bone's float 3 x 4 (rows: rotation, then the origin in column
   3, Matrix3x4_VectorTransform's layout). */
static inline void rp_bone_join( const int32_t v[3][3], const int32_t vo[3], const float b[3][4], rp_fbone_t *o )
{
	int32_t r[3][3], d[3];

	for( int i = 0; i < 3; i++ )
	{
		for( int j = 0; j < 3; j++ )
			r[i][j] = rp_fixq( b[i][j], 28 );
		d[i] = rp_fixq( b[i][3], 16 ) - vo[i];
	}
	for( int i = 0; i < 3; i++ )
	{
		for( int j = 0; j < 3; j++ )
			o->m[i][j] = (int32_t)(( (int64_t)v[i][0] * r[0][j] + (int64_t)v[i][1] * r[1][j]
				+ (int64_t)v[i][2] * r[2][j] ) >> 30 );
		o->t[i] = (int32_t)(( (int64_t)v[i][0] * d[0] + (int64_t)v[i][1] * d[1]
			+ (int64_t)v[i][2] * d[2] ) >> 30 );
	}
}

/* A model-space point (16.16) into view space (16.16). */
static inline void rp_bone_point( const rp_fbone_t *o, const int32_t p[3], int32_t q[3] )
{
	for( int i = 0; i < 3; i++ )
		q[i] = o->t[i] + (int32_t)(( (int64_t)o->m[i][0] * p[0] + (int64_t)o->m[i][1] * p[1]
			+ (int64_t)o->m[i][2] * p[2] ) >> 28 );
}

/*
R_StudioLighting's light (no local light): `cos30` the normal's dot with
the bone's light vector (2.30; -1 lit straight on, 1 from behind),
`a16`, `s16` the ambient and shade light (16.16). The hemispherical
Lambert (SHADE_LAMBERT 1.4953241: illum = a + s - s max(0, (cos + 0.4953241)
/ 1.4953241)), clamped to 0..255, as the gamma table's index (illum x 4,
toward zero as the float's cast).
*/
#define RP_LAMBERT_ADD ( (int32_t)( 0.4953241 * 1073741824.0 ))
#define RP_LAMBERT_MUL ( (int32_t)( 1073741824.0 / 1.4953241 ))

static inline int rp_illum_index( int32_t illum16 )
{
	if( illum16 < 0 )
		illum16 = 0;
	if( illum16 > ( 255 << 16 ))
		illum16 = 255 << 16;
	return illum16 >> 14;
}

static inline int rp_light_index( int32_t cos30, int32_t a16, int32_t s16 )
{
	int32_t illum16 = a16 + s16, l;

	if( cos30 > ( 1 << 30 ))
		cos30 = 1 << 30;
	l = (int32_t)(( (int64_t)( cos30 + RP_LAMBERT_ADD ) * RP_LAMBERT_MUL ) >> 30 );
	if( l > 0 )
		illum16 -= (int32_t)(( (int64_t)s16 * l ) >> 30 );
	return rp_illum_index( illum16 );
}

/* A normal (2.30) with a light vector (4.28): the dot as 2.30. */
static inline int32_t rp_dot30( const int32_t n[3], const int32_t l[3] )
{
	return (int32_t)(( (int64_t)n[0] * l[0] + (int64_t)n[1] * l[1] + (int64_t)n[2] * l[2] ) >> 28 );
}

/* A colour byte: `g16` the light's colour x 255 / 1023 (16.16), `gamma`
   the gamma table's value (0..1023), toward zero, at most 255. */
static inline uint8_t rp_light_byte( int32_t g16, uint32_t gamma )
{
	int64_t c = ( (int64_t)g16 * gamma ) >> 16;

	return c < 0 ? 0 : c > 255 ? 255 : (uint8_t)c;
}

#endif
