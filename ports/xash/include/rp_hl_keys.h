/*
rp_hl_keys.h - RetroPane OS (Dell Axim X51v): Half-Life's two touch keys
Copyright (C) 2026 the RetroPane OS authors

The owner, run 46: "like doom gles, but make it make sense for hl" and
"make sure you make the key remappable". The hardware keys carry jump,
use, crouch, the menu (Home) and the weapons (Record next, the wireless
button back); the flashlight and the second fire are two glass keys in
the picture's bottom corners (the owner's pick 2026-11-15, confirmed run
46: "the flashlight key and the secondary fire keys to be on the
screen"). The input layer (platform/in_rp.c) turns a touch on one into an
engine key, so the game's bindings decide what it does (Options, Controls
remaps it like any key); ref_rp (ref_rp/r_emit.c) draws them while the
game is played (the cvar `rp_touchkeys` 1). Both read these rectangles,
in the game's 640 x 480 picture, so what is drawn is what is touched.
*/
#ifndef RP_HL_KEYS_H
#define RP_HL_KEYS_H

#define RP_TK_COUNT  2
#define RP_TK_W      100 /* the mock-up's pill keys (hl-controls-doomgles.png), a stylus or fingertip */
#define RP_TK_H      40
#define RP_TK_MARGIN 8
/* the mock-up's bottom corners, raised over Half-Life's HUD numbers (health
   bottom left, ammo bottom right: their row is the picture's last ~48 px),
   so neither hides the other */
#define RP_TK_TOP    ( 480 - 48 - RP_TK_H )

/* key 0: the flashlight, LIGHT (bottom left); key 1: the second fire, FIRE 2 (bottom right) */
#define RP_TK_X( k ) ( ( k ) == 0 ? RP_TK_MARGIN : 640 - RP_TK_MARGIN - RP_TK_W )
#define RP_TK_Y( k ) ( RP_TK_TOP )

#endif
