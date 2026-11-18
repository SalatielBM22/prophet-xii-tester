// SPDX-License-Identifier: GPL-3.0-only
// Real Linux VST3 C COM ABI & X11 IPlugView smoke harness.
// Build: gcc -O2 -Wall -Wextra crates/prophet-plugin/tests/vst3_smoke.c -lX11 -ldl -o /tmp/vst3_smoke
// Run:   xvfb-run -a -s "-screen 0 1600x900x24" /tmp/vst3_smoke ./target/release/libprophet_plugin.so
#define _POSIX_C_SOURCE 200809L
#include <X11/Xlib.h>
#include <assert.h>
#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

typedef int32_t tresult;
typedef uint8_t TUID[16];
typedef struct {
    char vendor[64];
    char url[256];
    char email[128];
    int32_t flags;
} PFactoryInfo;

typedef struct {
    TUID cid;
    int32_t cardinality;
    char category[32];
    char name[64];
} PClassInfo;

typedef struct {
    int32_t left, top, right, bottom;
} ViewRect;

typedef struct IPluginFactoryVtbl {
    tresult (*queryInterface)(void *self, const TUID *iid, void **obj);
    uint32_t (*addRef)(void *self);
    uint32_t (*release)(void *self);
    tresult (*getFactoryInfo)(void *self, PFactoryInfo *info);
    int32_t (*countClasses)(void *self);
    tresult (*getClassInfo)(void *self, int32_t index, PClassInfo *info);
    tresult (*createInstance)(void *self, const TUID *cid, const TUID *iid, void **obj);
} IPluginFactoryVtbl;

typedef struct {
    IPluginFactoryVtbl *lpVtbl;
} IPluginFactory;

typedef struct IComponentVtbl {
    tresult (*queryInterface)(void *self, const TUID *iid, void **obj);
    uint32_t (*addRef)(void *self);
    uint32_t (*release)(void *self);
    tresult (*initialize)(void *self, void *context);
    tresult (*terminate)(void *self);
} IComponentVtbl;

typedef struct {
    IComponentVtbl *lpVtbl;
} IComponent;

typedef struct IEditControllerVtbl {
    tresult (*queryInterface)(void *self, const TUID *iid, void **obj);
    uint32_t (*addRef)(void *self);
    uint32_t (*release)(void *self);
    tresult (*initialize)(void *self, void *context);
    tresult (*terminate)(void *self);
    void *setComponentState;
    void *setState;
    void *getState;
    int32_t (*getParameterCount)(void *self);
    void *getParameterInfo;
    void *getParamStringByValue;
    void *getParamValueByString;
    void *normalizedParamToPlain;
    void *plainParamToNormalized;
    void *getParamNormalized;
    void *setParamNormalized;
    void *setComponentHandler;
    void *(*createView)(void *self, const char *name);
} IEditControllerVtbl;

typedef struct {
    IEditControllerVtbl *lpVtbl;
} IEditController;

typedef struct IPlugViewVtbl {
    tresult (*queryInterface)(void *self, const TUID *iid, void **obj);
    uint32_t (*addRef)(void *self);
    uint32_t (*release)(void *self);
    tresult (*isPlatformTypeSupported)(void *self, const char *type);
    tresult (*attached)(void *self, void *parent, const char *type);
    tresult (*removed)(void *self);
    void *onWheel;
    void *onKeyDown;
    void *onKeyUp;
    tresult (*getSize)(void *self, ViewRect *size);
} IPlugViewVtbl;

typedef struct {
    IPlugViewVtbl *lpVtbl;
} IPlugView;

static const TUID IID_IComponent = {
    0xE8, 0x31, 0xFF, 0x31, 0xF2, 0xD5, 0x43, 0x01,
    0x92, 0x8E, 0xBB, 0xEE, 0x25, 0x69, 0x78, 0x02};
static const TUID IID_IEditController = {
    0xDC, 0xD7, 0xBB, 0xE3, 0x77, 0x42, 0x44, 0x8D,
    0xA8, 0x74, 0xAA, 0xCC, 0x97, 0x9C, 0x75, 0x9E};

static Window wait_child(Display *dpy, Window parent) {
    struct timespec ts = {.tv_sec = 0, .tv_nsec = 40 * 1000 * 1000L};
    for (int i = 0; i < 40; i++) {
        Window r = None, p = None, *c = NULL;
        unsigned int nc = 0;
        if (XQueryTree(dpy, parent, &r, &p, &c, &nc) && c) {
            Window w = nc > 0 ? c[0] : None;
            XFree(c);
            if (w != None) return w;
        }
        XFlush(dpy);
        nanosleep(&ts, NULL);
    }
    return None;
}

int main(int argc, char **argv) {
    assert(argc == 2);
    Display *dpy = XOpenDisplay(NULL);
    assert(dpy != NULL);

    void *lib = dlopen(argv[1], RTLD_NOW);
    assert(lib != NULL);

    typedef bool (*FnModuleEntry)(void *);
    typedef bool (*FnModuleExit)(void);
    typedef IPluginFactory *(*FnGetPluginFactory)(void);

    FnModuleEntry mod_entry = (FnModuleEntry)dlsym(lib, "ModuleEntry");
    FnModuleExit mod_exit = (FnModuleExit)dlsym(lib, "ModuleExit");
    FnGetPluginFactory get_factory = (FnGetPluginFactory)dlsym(lib, "GetPluginFactory");
    assert(mod_entry && mod_exit && get_factory);
    assert(mod_entry(lib));

    IPluginFactory *factory = get_factory();
    assert(factory != NULL);

    PFactoryInfo finfo = {0};
    assert(factory->lpVtbl->getFactoryInfo(factory, &finfo) == 0);
    assert(factory->lpVtbl->countClasses(factory) == 1);

    PClassInfo cinfo = {0};
    assert(factory->lpVtbl->getClassInfo(factory, 0, &cinfo) == 0);
    assert(!strcmp(cinfo.name, "PROPHET-XII"));
    assert(!strcmp(cinfo.category, "Audio Module Class"));

    void *comp_raw = NULL;
    assert(factory->lpVtbl->createInstance(factory, &cinfo.cid, &IID_IComponent, &comp_raw) == 0);
    IComponent *comp = (IComponent *)comp_raw;
    assert(comp != NULL);
    assert(comp->lpVtbl->initialize(comp, NULL) == 0);

    void *ctrl_raw = NULL;
    assert(comp->lpVtbl->queryInterface(comp, &IID_IEditController, &ctrl_raw) == 0);
    IEditController *ctrl = (IEditController *)ctrl_raw;
    assert(ctrl != NULL);
    int32_t param_count = ctrl->lpVtbl->getParameterCount(ctrl);
    assert(param_count >= 86);

    IPlugView *view = (IPlugView *)ctrl->lpVtbl->createView(ctrl, "editor");
    assert(view != NULL);
    assert(view->lpVtbl->isPlatformTypeSupported(view, "X11EmbedWindowID") == 0);

    ViewRect rect = {0};
    assert(view->lpVtbl->getSize(view, &rect) == 0);
    assert((rect.right - rect.left) == 1180 && (rect.bottom - rect.top) == 720);

    Window parent = XCreateSimpleWindow(dpy, RootWindow(dpy, DefaultScreen(dpy)), 0, 0, 1180, 720, 0, 0, 0);
    XMapWindow(dpy, parent);
    XSync(dpy, False);

    assert(view->lpVtbl->attached(view, (void *)(uintptr_t)parent, "X11EmbedWindowID") == 0);
    Window child = wait_child(dpy, parent);
    assert(child != None);

    assert(view->lpVtbl->removed(view) == 0);
    view->lpVtbl->release(view);
    comp->lpVtbl->terminate(comp);
    ctrl->lpVtbl->release(ctrl);
    comp->lpVtbl->release(comp);
    factory->lpVtbl->release(factory);
    assert(mod_exit());
    dlclose(lib);

    XDestroyWindow(dpy, parent);
    XSync(dpy, False);
    XCloseDisplay(dpy);

    printf("PASS: VST3 COM factory (%s), %d params, IPlugView 1180x720 X11 embed child 0x%lx & clean tear-down\n",
        cinfo.name, param_count, (unsigned long)child);
    return 0;
}
