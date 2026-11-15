/*
 * bzip2 is not in RetroPane's build of Xash3D: the engine uses it only to
 * pack and unpack fragments of files sent between a server and its
 * clients, and the port has no network (single-player Half-Life never
 * reaches this code). These are the four names engine/common/net_chan.c
 * uses, answered in ports/xash/absent.c: packing says it failed (the
 * engine then sends the fragment unpacked), unpacking says the data is
 * bad.
 *
 * The names and numbers are libbzip2's (its bzlib.h, BSD-style licence);
 * nothing else of it is here. To have the real thing, fetch the
 * 3rdparty/bzip2 submodule and drop this header.
 */
#ifndef RP_XASH_BZLIB_H
#define RP_XASH_BZLIB_H

#define BZ_OK 0
#define BZ_DATA_ERROR (-4)
#define BZ_CONFIG_ERROR (-9)

const char *BZ2_bzlibVersion(void);
int BZ2_bzBuffToBuffCompress(char *dest, unsigned int *destLen, char *source, unsigned int sourceLen,
                             int blockSize100k, int verbosity, int workFactor);
int BZ2_bzBuffToBuffDecompress(char *dest, unsigned int *destLen, char *source, unsigned int sourceLen,
                               int small, int verbosity);

#endif /* RP_XASH_BZLIB_H */
