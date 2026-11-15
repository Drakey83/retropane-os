// The C++ a game port needs under it (docs/design/half-life-readiness.md
// line 3), whole.
//
// Iterate, don't imitate (rule 0). A C++ program on Windows Mobile links
// the compiler's runtime in coredll; on Linux libstdc++ or libc++ with
// libc++abi and an unwinder, a megabyte before the program starts. The
// Half-Life SDK uses none of that: no exceptions, no run-time type
// information, no standard C++ library (its own headers and C's). What it
// does use is here: new and delete over the C heap, and the two calls the
// compiler makes for a pure virtual function and for a global object's
// destructor. Built with -fno-exceptions -fno-rtti -fno-threadsafe-statics
// (one thread: a function's static object needs no lock), so nothing else
// is asked for.
//
// A global object's destructor is never run: a game ends by going back to
// the loader, which takes all its memory at once. Its constructor is run
// before main (boot/link.x's .init_array, rp-hal-cabi).

#include <stddef.h>
#include <stdlib.h>

// No memory is a stop, not a null pointer handed to code that never
// checks (C++ code does not: new cannot fail without exceptions).
static void *take(size_t n)
{
    void *p = malloc(n ? n : 1);
    if (!p)
        abort();
    return p;
}

void *operator new(size_t n) { return take(n); }
void *operator new[](size_t n) { return take(n); }
void operator delete(void *p) noexcept { free(p); }
void operator delete[](void *p) noexcept { free(p); }
void operator delete(void *p, size_t) noexcept { free(p); }
void operator delete[](void *p, size_t) noexcept { free(p); }

extern "C" {

// A call of a pure virtual function: an object used while it is being
// made or after it was destroyed.
void __cxa_pure_virtual(void) { abort(); }

// The ARM ABI's registration of a global object's destructor: taken and
// dropped (see the top).
int __aeabi_atexit(void *, void (*)(void *), void *) { return 0; }

void *__dso_handle = 0;

}
