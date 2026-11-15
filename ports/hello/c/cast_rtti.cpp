// The C++ run-time cast (dynamic_cast, ports/libc/cxxrt.cpp), checked by
// the program itself. This one file is built with run-time type
// information (its name's `_rtti`: xtask/src/cc.rs); the rest of the
// program is not, as the Half-Life SDK is not.

#include <stdio.h>

extern "C" int cast_checks(void);

namespace {

int check(const char *what, bool ok)
{
    if (!ok)
        printf("FAILED: %s\n", what);
    return ok;
}

struct Panel {
    virtual ~Panel() {}
    int id = 1;
};
struct Frame : Panel {
    int frame = 2;
};
struct Dialog : Frame {
    int dialog = 3;
};
struct Signal {
    virtual ~Signal() {}
    int signal = 4;
};
// Two bases: the second lies at an offset inside the object.
struct Button : Panel, Signal {
    int button = 5;
};
// The same base twice (once through each side), and a virtual base.
struct Left : Panel {};
struct Right : Panel {};
struct Both : Left, Right {};
struct Shared {
    virtual ~Shared() {}
    int shared = 6;
};
struct A : virtual Shared {
    int a = 7;
};
struct B : virtual Shared {
    int b = 8;
};
struct Diamond : A, B {
    int d = 9;
};
// A private base may not be cast through.
struct Hidden : private Signal {
    virtual ~Hidden() {}
    Signal *as_signal() { return this; }
};

// The compiler answers a cast it can see through by itself; these keep
// it from seeing.
template <typename T> T *hide(T *p)
{
    T *volatile q = p;
    return q;
}

} // namespace

int cast_checks(void)
{
    int ok = 1;
    Dialog dialog;
    Frame frame;
    Button button;
    Panel *p = hide<Panel>(&dialog);
    // Down a chain: to the class itself, to one in between, not past it.
    ok &= check("cast: down to the object's own class", dynamic_cast<Dialog *>(p) == &dialog);
    ok &= check("cast: down to a class in between", dynamic_cast<Frame *>(p) == &dialog);
    p = hide<Panel>(&frame);
    ok &= check("cast: not to a class the object is not", dynamic_cast<Dialog *>(p) == nullptr);
    ok &= check("cast: not to an unrelated class", dynamic_cast<Signal *>(p) == nullptr);
    // Two bases: from the second (at an offset) down, and across.
    Signal *s = hide<Signal>(&button);
    Button *back = dynamic_cast<Button *>(s);
    ok &= check("cast: down from a second base", back == &button && back->button == 5);
    Panel *across = dynamic_cast<Panel *>(s);
    ok &= check("cast: across to the other base", across == static_cast<Panel *>(&button) && across->id == 1);
    ok &= check("cast: and back", dynamic_cast<Signal *>(hide(across)) == s);
    // The same base twice: down from either is clear.
    Both both;
    Panel *left = hide<Panel>(static_cast<Left *>(&both));
    Panel *right = hide<Panel>(static_cast<Right *>(&both));
    ok &= check("cast: down from a twice-held base, each side",
                dynamic_cast<Both *>(left) == &both && dynamic_cast<Both *>(right) == &both &&
                    dynamic_cast<Left *>(left) == static_cast<Left *>(&both) &&
                    dynamic_cast<Right *>(right) == static_cast<Right *>(&both));
    ok &= check("cast: across, to the other side's own class",
                dynamic_cast<Right *>(left) == static_cast<Right *>(&both));
    // Across to "the" Panel from elsewhere is not clear (two of them).
    Left *l = hide<Left>(&both);
    ok &= check("cast: one's own base is found, of two", dynamic_cast<Panel *>(static_cast<Panel *>(l)) == left);
    // A virtual base: its place is read from the object.
    Diamond diamond;
    Shared *shared = hide<Shared>(&diamond);
    A *a = dynamic_cast<A *>(shared);
    B *b = dynamic_cast<B *>(shared);
    ok &= check("cast: down from a virtual base",
                a == static_cast<A *>(&diamond) && b == static_cast<B *>(&diamond) && a && b && a->a == 7 &&
                    b->b == 8 && dynamic_cast<Diamond *>(shared) == &diamond);
    ok &= check("cast: across a diamond", dynamic_cast<B *>(hide(a)) == b);
    ok &= check("cast: across to a shared virtual base's sibling and back",
                dynamic_cast<A *>(hide(b)) == a);
    // A private base: no way through.
    Hidden hidden;
    ok &= check("cast: not through a private base", dynamic_cast<Hidden *>(hide(hidden.as_signal())) == nullptr);
    return ok;
}
