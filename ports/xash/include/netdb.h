/*
 * Name look-ups for a C port on RetroPane: see <sys/socket.h> here (every
 * look-up answers that no name can be found: there is no network).
 */
#ifndef RP_NETDB_H
#define RP_NETDB_H

#include <netinet/in.h>
#include <sys/socket.h>

#ifdef __cplusplus
extern "C" {
#endif

struct hostent {
    char *h_name;
    char **h_aliases;
    int h_addrtype;
    int h_length;
    char **h_addr_list;
};
#define h_addr h_addr_list[0]

struct addrinfo {
    int ai_flags;
    int ai_family;
    int ai_socktype;
    int ai_protocol;
    socklen_t ai_addrlen;
    struct sockaddr *ai_addr;
    char *ai_canonname;
    struct addrinfo *ai_next;
};

#define AI_PASSIVE 1
#define AI_NUMERICHOST 4

#define EAI_AGAIN (-3)
#define EAI_FAIL (-4)
#define EAI_NONAME (-2)

#define NI_MAXHOST 1025
#define NI_NUMERICHOST 1

struct hostent *gethostbyname(const char *name);
int gethostname(char *name, size_t size);
int getaddrinfo(const char *node, const char *service, const struct addrinfo *hints, struct addrinfo **out);
void freeaddrinfo(struct addrinfo *list);
const char *gai_strerror(int code);
int getnameinfo(const struct sockaddr *addr, socklen_t len, char *host, socklen_t hostlen, char *serv,
                socklen_t servlen, int flags);

#ifdef __cplusplus
}
#endif

#endif /* RP_NETDB_H */
