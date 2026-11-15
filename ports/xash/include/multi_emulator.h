/*
 * FWGS's MultiEmulator (tickets that stand in for a Steam ID when joining
 * a server) is not in RetroPane's build of Xash3D: the port has no
 * network. This is the one name engine/client/cl_main.c calls, answered
 * in ports/xash/absent.c with an empty ticket. To have the real thing,
 * fetch the 3rdparty/MultiEmulator submodule and drop this header.
 */
#ifndef RP_XASH_MULTI_EMULATOR_H
#define RP_XASH_MULTI_EMULATOR_H

#ifdef __cplusplus
extern "C" {
#endif

int GenerateRevEmu2013(void *dest, const char *key, unsigned int hash);

#ifdef __cplusplus
}
#endif

#endif /* RP_XASH_MULTI_EMULATOR_H */
