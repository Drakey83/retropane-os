/*
 * A C port on RetroPane is one thread: no thread can be made. This is the
 * little of POSIX threads Xash3D's network code names (a helper thread
 * for name look-ups, which the engine does without when the thread cannot
 * be made): a lock is nothing to take with one thread, and making a
 * thread says it cannot (EAGAIN).
 *
 * Written for RetroPane from the POSIX specification's names.
 */
#ifndef RP_PTHREAD_H
#define RP_PTHREAD_H

#include <errno.h>

typedef int pthread_t;
typedef int pthread_attr_t;
typedef int pthread_mutex_t;
typedef int pthread_mutexattr_t;

#define PTHREAD_MUTEX_INITIALIZER 0

static inline int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *a)
{
    (void)a;
    *m = 0;
    return 0;
}
static inline int pthread_mutex_destroy(pthread_mutex_t *m)
{
    (void)m;
    return 0;
}
static inline int pthread_mutex_lock(pthread_mutex_t *m)
{
    (void)m;
    return 0;
}
static inline int pthread_mutex_unlock(pthread_mutex_t *m)
{
    (void)m;
    return 0;
}
static inline int pthread_create(pthread_t *t, const pthread_attr_t *a, void *(*start)(void *), void *arg)
{
    (void)t;
    (void)a;
    (void)start;
    (void)arg;
    return EAGAIN;
}
static inline int pthread_detach(pthread_t t)
{
    (void)t;
    return 0;
}

#endif /* RP_PTHREAD_H */
