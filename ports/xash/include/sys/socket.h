/*
 * BSD sockets as a C port on RetroPane sees them today: the types and the
 * calls, every call answering "no network" (ports/xash/absent.c: -1 with
 * errno ENETDOWN). The C library RetroPane builds (picolibc) has no
 * sockets; the OS's own network stack is not open to C programs yet.
 *
 * Xash3D needs these to compile its network layer, which single-player
 * Half-Life uses only through its internal loopback (no socket is ever
 * opened). When the port layer gets sockets these headers become its
 * face and the answers become real.
 *
 * Written for RetroPane from the POSIX specification's names; no other
 * system's header was copied.
 */
#ifndef RP_SYS_SOCKET_H
#define RP_SYS_SOCKET_H

#include <stddef.h>
#include <stdint.h>
#include <sys/types.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint32_t socklen_t;
typedef uint16_t sa_family_t;

#define AF_UNSPEC 0
#define AF_INET 2
#define AF_INET6 10
#define PF_UNSPEC AF_UNSPEC
#define PF_INET AF_INET
#define PF_INET6 AF_INET6

#define SOCK_STREAM 1
#define SOCK_DGRAM 2

#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SO_ERROR 4
#define SO_BROADCAST 6
#define SO_SNDBUF 7
#define SO_RCVBUF 8
#define SO_KEEPALIVE 9
#define SO_RCVTIMEO 20
#define SO_SNDTIMEO 21

#define MSG_PEEK 2
#define MSG_DONTWAIT 0x40
#define MSG_NOSIGNAL 0x4000

#define SHUT_RD 0
#define SHUT_WR 1
#define SHUT_RDWR 2

/* Sets or clears non-blocking mode (ioctl). */
#define FIONBIO 0x5421

struct sockaddr {
    sa_family_t sa_family;
    char sa_data[14];
};

/* Room for any address, aligned for any. */
struct sockaddr_storage {
    sa_family_t ss_family;
    char ss_pad[126 - sizeof(sa_family_t)];
    uint64_t ss_align;
};

int socket(int domain, int type, int protocol);
int bind(int fd, const struct sockaddr *addr, socklen_t len);
int connect(int fd, const struct sockaddr *addr, socklen_t len);
int listen(int fd, int backlog);
int accept(int fd, struct sockaddr *addr, socklen_t *len);
int getsockname(int fd, struct sockaddr *addr, socklen_t *len);
int getsockopt(int fd, int level, int name, void *value, socklen_t *len);
int setsockopt(int fd, int level, int name, const void *value, socklen_t len);
int shutdown(int fd, int how);
ssize_t send(int fd, const void *buf, size_t n, int flags);
ssize_t recv(int fd, void *buf, size_t n, int flags);
ssize_t sendto(int fd, const void *buf, size_t n, int flags, const struct sockaddr *to, socklen_t len);
ssize_t recvfrom(int fd, void *buf, size_t n, int flags, struct sockaddr *from, socklen_t *len);

#ifdef __cplusplus
}
#endif

#endif /* RP_SYS_SOCKET_H */
