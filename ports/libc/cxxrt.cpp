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

// ---- run-time casts (dynamic_cast) ---------------------------------------
//
// For the few libraries built with run-time type information (FreeVGUI
// asks which kind of panel it was handed; the Half-Life SDK itself is
// built without). Iterate, don't imitate (rule 0): libc++abi and libsupc++
// answer this with a general engine for every C++ feature (exceptions
// caught by base, private and ambiguous bases told apart, hints), a few
// thousand lines. A cast needs one walk: from the object's real class
// down its bases, with each base's address worked out on the way, looking
// for the wanted class with the pointer we started from inside it. That
// is all this is, over the type records the compiler already writes (the
// C++ ABI for ARM fixes their layout and their names, which is why the
// classes below must be named as they are).

#include <string.h>

namespace std {
class type_info {
public:
    virtual ~type_info();
    const char *name_;
};
type_info::~type_info() {}
} // namespace std

namespace __cxxabiv1 {

// A class with no bases. `base` gives base number `i` of the object of
// this class at `at` and where that base lies, or null past the last.
class __class_type_info : public std::type_info {
public:
    ~__class_type_info() override;
    virtual const __class_type_info *base(unsigned i, const char *at, const char **base_at) const;
};
__class_type_info::~__class_type_info() {}
const __class_type_info *__class_type_info::base(unsigned, const char *, const char **) const
{
    return nullptr;
}

// A class with one public base at its own address.
class __si_class_type_info : public __class_type_info {
public:
    ~__si_class_type_info() override;
    const __class_type_info *base(unsigned i, const char *at, const char **base_at) const override;
    const __class_type_info *base_;
};
__si_class_type_info::~__si_class_type_info() {}
const __class_type_info *__si_class_type_info::base(unsigned i, const char *at, const char **base_at) const
{
    if (i != 0)
        return nullptr;
    *base_at = at;
    return base_;
}

// Any other class: each base with its offset, or for a virtual base the
// place in the object's own table where its offset is kept.
class __vmi_class_type_info : public __class_type_info {
public:
    ~__vmi_class_type_info() override;
    const __class_type_info *base(unsigned i, const char *at, const char **base_at) const override;
    unsigned flags_;
    unsigned count_;
    struct {
        const __class_type_info *type;
        long offset_flags; // bit 0: virtual; bit 1: public; the rest: the offset
    } bases_[1];
};
__vmi_class_type_info::~__vmi_class_type_info() {}
const __class_type_info *__vmi_class_type_info::base(unsigned i, const char *at, const char **base_at) const
{
    // A base that is not public is skipped: a cast may not go through it.
    for (unsigned n = 0; n < count_; n++) {
        long of = bases_[n].offset_flags;
        if (!(of & 2))
            continue;
        if (i-- != 0)
            continue;
        long offset = of >> 8;
        if (of & 1) {
            const char *table = *reinterpret_cast<const char *const *>(at);
            offset = *reinterpret_cast<const long *>(table + offset);
        }
        *base_at = at + offset;
        return bases_[n].type;
    }
    return nullptr;
}

namespace {

bool same(const std::type_info *a, const std::type_info *b)
{
    return a == b || strcmp(a->name_, b->name_) == 0;
}

// A hierarchy deeper than this is refused (no cast), never followed.
const int DEEPEST = 32;

// Is the `src` object at `src_at` a base of (or the same as) the object
// of class `t` at `at`?
bool holds(const __class_type_info *t, const char *at, const std::type_info *src, const char *src_at, int depth)
{
    if (at == src_at && same(t, src))
        return true;
    if (depth >= DEEPEST)
        return false;
    const char *base_at = nullptr;
    for (unsigned i = 0; const __class_type_info *b = t->base(i, at, &base_at); i++)
        if (holds(b, base_at, src, src_at, depth + 1))
            return true;
    return false;
}

struct Search {
    const std::type_info *src;
    const char *src_at;
    const std::type_info *dst;
    const char *down; // a `dst` that holds the pointer we came from
    const char *any;  // a `dst` anywhere in the object
    int count;        // how many different `dst` there are
};

void find(const __class_type_info *t, const char *at, Search &s, int depth)
{
    if (same(t, s.dst)) {
        // A virtual base is reached by more than one road: once.
        if (s.any != at) {
            s.count++;
            s.any = at;
        }
        if (!s.down && holds(t, at, s.src, s.src_at, depth))
            s.down = at;
        return;
    }
    if (depth >= DEEPEST)
        return;
    const char *base_at = nullptr;
    for (unsigned i = 0; const __class_type_info *b = t->base(i, at, &base_at); i++)
        find(b, base_at, s, depth + 1);
}

} // namespace

extern "C" void *__dynamic_cast(const void *from, const __class_type_info *src, const __class_type_info *dst,
                                long /* the compiler's hint, not needed */)
{
    // The object's table: its real class one word before, and how far
    // `from` is from the whole object's start one word before that.
    const void *const *table = *static_cast<const void *const *const *>(from);
    const __class_type_info *real = static_cast<const __class_type_info *>(table[-1]);
    // Built without type information (the SDK's own classes): no cast.
    if (!real)
        return nullptr;
    const char *whole = static_cast<const char *>(from) + reinterpret_cast<long>(table[-2]);
    Search s = { src, static_cast<const char *>(from), dst, nullptr, nullptr, 0 };
    find(real, whole, s, 0);
    // Down to a class that holds us; else across, if there is exactly one
    // and the object shows the class we came from (a public base of it).
    const char *to = s.down;
    if (!to && s.count == 1 && holds(real, whole, src, s.src_at, 0))
        to = s.any;
    return const_cast<char *>(to);
}

} // namespace __cxxabiv1
