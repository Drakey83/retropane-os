// The C++ a game port leans on (the Half-Life SDK is C++ with C's headers:
// classes, virtual calls, new and delete, global objects), checked by the
// same program as the C library (hello.c calls cxx_checks).

#include <string.h>

extern "C" int check(const char *what, int ok);
extern "C" int cxx_checks(void);

namespace {

struct Counter {
    int n;
    Counter() : n(41) { n++; }
};

// Made before main, by the constructor table.
Counter made_before_main;

struct Shape {
    virtual ~Shape() {}
    virtual int sides() const = 0;
    virtual const char *name() const { return "shape"; }
};

struct Triangle : Shape {
    int sides() const override { return 3; }
};

struct Square : Shape {
    int *corners;
    Square() : corners(new int[4])
    {
        for (int i = 0; i < 4; i++)
            corners[i] = i;
    }
    ~Square() override { delete[] corners; }
    int sides() const override { return corners[3] + 1; }
    const char *name() const override { return "square"; }
};

int destroyed;

struct Tracked {
    ~Tracked() { destroyed++; }
};

int once()
{
    static Counter c;
    return ++c.n;
}

template <typename T> T larger(T a, T b) { return a > b ? a : b; }

} // namespace

int cxx_checks(void)
{
    int ok = 1;
    ok &= check("C++: a global's constructor ran before main", made_before_main.n == 42);
    Shape *a = new Triangle;
    Shape *b = new Square;
    ok &= check("C++: new and virtual calls", a->sides() == 3 && b->sides() == 4);
    ok &= check("C++: a base's own function and an override",
                strcmp(a->name(), "shape") == 0 && strcmp(b->name(), "square") == 0);
    delete a;
    delete b;
    {
        Tracked *t = new Tracked[3];
        delete[] t;
    }
    ok &= check("C++: delete runs the destructors", destroyed == 3);
    ok &= check("C++: a function's static is made once", once() == 43 && once() == 44);
    ok &= check("C++: templates", larger(2, 7) == 7 && larger(2.5f, 1.5f) == 2.5f);
    return ok;
}
