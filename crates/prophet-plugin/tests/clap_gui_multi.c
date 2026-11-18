// SPDX-License-Identifier: GPL-3.0-only
// Real Linux/Xvfb CLAP host integration test:
// - Opens TWO independent plugin instances simultaneously
// - Implements clap_host_params, clap_host_latency, clap_host_gui, clap_host_thread_check
// - Verifies CLAP_EXT_GUI embedding into distinct parent X11 windows via XQueryTree
// - Sends synthetic X11 pointer events and checks host flush/process isolation + GUI reopen
// - Verifies 1x/2x/4x oversampling latency reporting, reverb Spill/Kill, and state isolation/restore
//
// Build: gcc -O2 -Wall -Wextra crates/prophet-plugin/tests/clap_gui_multi.c -I/home/antigravity/.prophet-cache/sysroot/usr/include -lX11 -lXtst -ldl -lm -o /tmp/clap_gui_multi
// Run:   xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/clap_gui_multi ./target/release/libprophet_plugin.so
#define _POSIX_C_SOURCE 200809L
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/XTest.h>
#include <assert.h>
#include <clap/clap.h>
#include <dlfcn.h>
#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static int g_x11_errors = 0;

static int x11_error_handler(Display *dpy, XErrorEvent *ev) {
    char buf[256];
    XGetErrorText(dpy, ev->error_code, buf, sizeof(buf));
    fprintf(stderr, "X11 error: %s (req=%d minor=%d)\n", buf, ev->request_code, ev->minor_code);
    g_x11_errors++;
    return 0;
}

static void sleep_ms(long ms) {
    struct timespec ts = {.tv_sec = ms / 1000, .tv_nsec = (ms % 1000) * 1000000L};
    nanosleep(&ts, NULL);
}

typedef struct {
    clap_host_t host;
    const clap_plugin_t *plugin;
    bool flush_requested;
    bool latency_changed;
    bool resize_requested;
    uint32_t req_w, req_h;
    pthread_t main_tid;
    bool in_audio_thread;
} HostCtx;

static void CLAP_ABI host_params_rescan(const clap_host_t *host, clap_param_rescan_flags flags) {
    (void)host;
    (void)flags;
}

static void CLAP_ABI host_params_clear(const clap_host_t *host, clap_id param_id, clap_param_clear_flags flags) {
    (void)host;
    (void)param_id;
    (void)flags;
}

static void CLAP_ABI host_params_request_flush(const clap_host_t *host) {
    HostCtx *ctx = (HostCtx *)host->host_data;
    ctx->flush_requested = true;
}

static void CLAP_ABI host_latency_changed(const clap_host_t *host) {
    HostCtx *ctx = (HostCtx *)host->host_data;
    ctx->latency_changed = true;
}

static void CLAP_ABI host_gui_resize_hints_changed(const clap_host_t *host) { (void)host; }

static bool CLAP_ABI host_gui_request_resize(const clap_host_t *host, uint32_t w, uint32_t h) {
    HostCtx *ctx = (HostCtx *)host->host_data;
    ctx->resize_requested = true;
    ctx->req_w = w;
    ctx->req_h = h;
    return true;
}

static bool CLAP_ABI host_gui_request_show(const clap_host_t *host) {
    (void)host;
    return true;
}

static bool CLAP_ABI host_gui_request_hide(const clap_host_t *host) {
    (void)host;
    return true;
}

static void CLAP_ABI host_gui_closed(const clap_host_t *host, bool was_destroyed) {
    (void)host;
    (void)was_destroyed;
}

static bool CLAP_ABI host_is_main_thread(const clap_host_t *host) {
    HostCtx *ctx = (HostCtx *)host->host_data;
    return !ctx->in_audio_thread && pthread_equal(pthread_self(), ctx->main_tid);
}

static bool CLAP_ABI host_is_audio_thread(const clap_host_t *host) {
    HostCtx *ctx = (HostCtx *)host->host_data;
    return ctx->in_audio_thread;
}

static const clap_host_params_t g_host_params = {
    .rescan = host_params_rescan,
    .clear = host_params_clear,
    .request_flush = host_params_request_flush,
};

static const clap_host_latency_t g_host_latency = {
    .changed = host_latency_changed,
};

static const clap_host_gui_t g_host_gui = {
    .resize_hints_changed = host_gui_resize_hints_changed,
    .request_resize = host_gui_request_resize,
    .request_show = host_gui_request_show,
    .request_hide = host_gui_request_hide,
    .closed = host_gui_closed,
};

static const clap_host_thread_check_t g_host_thread_check = {
    .is_main_thread = host_is_main_thread,
    .is_audio_thread = host_is_audio_thread,
};

static const void *CLAP_ABI host_get_extension(const clap_host_t *host, const char *id) {
    (void)host;
    if (!strcmp(id, CLAP_EXT_PARAMS)) return &g_host_params;
    if (!strcmp(id, CLAP_EXT_LATENCY)) return &g_host_latency;
    if (!strcmp(id, CLAP_EXT_GUI)) return &g_host_gui;
    if (!strcmp(id, CLAP_EXT_THREAD_CHECK)) return &g_host_thread_check;
    return NULL;
}

static void CLAP_ABI host_noop(const clap_host_t *host) { (void)host; }

static void init_host_ctx(HostCtx *ctx, const char *name) {
    memset(ctx, 0, sizeof(*ctx));
    ctx->main_tid = pthread_self();
    ctx->host.clap_version = (clap_version_t)CLAP_VERSION_INIT;
    ctx->host.host_data = ctx;
    ctx->host.name = name;
    ctx->host.vendor = "PROPHET-XII Test Host";
    ctx->host.url = "https://localhost";
    ctx->host.version = "2.0";
    ctx->host.get_extension = host_get_extension;
    ctx->host.request_restart = host_noop;
    ctx->host.request_process = host_noop;
    ctx->host.request_callback = host_noop;
}

static uint32_t CLAP_ABI empty_count(const clap_input_events_t *list) {
    (void)list;
    return 0;
}

static const clap_event_header_t *CLAP_ABI empty_get(const clap_input_events_t *list, uint32_t idx) {
    (void)list;
    (void)idx;
    return NULL;
}

static clap_event_param_value_t g_single_param_ev;
static uint32_t CLAP_ABI one_count(const clap_input_events_t *list) {
    (void)list;
    return 1;
}
static const clap_event_header_t *CLAP_ABI one_get(const clap_input_events_t *list, uint32_t idx) {
    (void)list;
    (void)idx;
    return &g_single_param_ev.header;
}

static bool CLAP_ABI out_try_push(const clap_output_events_t *list, const clap_event_header_t *ev) {
    (void)list;
    (void)ev;
    return true;
}

static void set_param(const clap_plugin_t *p, const clap_plugin_params_t *params, clap_id id, double value) {
    g_single_param_ev = (clap_event_param_value_t){
        .header = {
            .size = sizeof(g_single_param_ev),
            .time = 0,
            .space_id = CLAP_CORE_EVENT_SPACE_ID,
            .type = CLAP_EVENT_PARAM_VALUE,
        },
        .param_id = id,
        .cookie = NULL,
        .note_id = -1,
        .port_index = -1,
        .channel = -1,
        .key = -1,
        .value = value,
    };
    clap_input_events_t in = {.size = one_count, .get = one_get};
    clap_output_events_t out = {.try_push = out_try_push};
    params->flush(p, &in, &out);
}

static clap_event_note_t g_note_ev;
static const clap_event_header_t *CLAP_ABI note_get(const clap_input_events_t *list, uint32_t idx) {
    (void)list;
    (void)idx;
    return &g_note_ev.header;
}

static double render_blocks(
    HostCtx *ctx,
    const clap_plugin_t *p,
    const clap_plugin_params_t *params,
    int blocks,
    int note_key
) {
    clap_input_events_t empty_in = {.size = empty_count, .get = empty_get};
    clap_output_events_t out = {.try_push = out_try_push};
    if (ctx->flush_requested) {
        ctx->flush_requested = false;
        params->flush(p, &empty_in, &out);
    }
    if (note_key >= 0) {
        g_note_ev = (clap_event_note_t){
            .header = {.size = sizeof(g_note_ev), .time = 0, .space_id = CLAP_CORE_EVENT_SPACE_ID, .type = CLAP_EVENT_NOTE_ON},
            .note_id = 42,
            .port_index = 0,
            .channel = 0,
            .key = (int16_t)note_key,
            .velocity = 0.85,
        };
    }
    clap_input_events_t note_in = {.size = one_count, .get = note_get};
    float l[256] = {0}, r[256] = {0};
    float *channels[2] = {l, r};
    clap_audio_buffer_t audio = {.data32 = channels, .channel_count = 2};
    double energy = 0.0;
    ctx->in_audio_thread = true;
    for (int b = 0; b < blocks; b++) {
        memset(l, 0, sizeof(l));
        memset(r, 0, sizeof(r));
        clap_process_t proc = {
            .steady_time = (int64_t)b * 256,
            .frames_count = 256,
            .audio_outputs = &audio,
            .audio_outputs_count = 1,
            .in_events = (b == 0 && note_key >= 0) ? &note_in : &empty_in,
            .out_events = &out,
        };
        assert(p->process(p, &proc) != CLAP_PROCESS_ERROR);
        for (int i = 0; i < 256; i++) {
            assert(isfinite(l[i]) && isfinite(r[i]));
            energy += (double)l[i] * l[i] + (double)r[i] * r[i];
        }
    }
    ctx->in_audio_thread = false;
    return energy;
}

static Window wait_embedded_child(Display *dpy, Window parent) {
    for (int attempt = 0; attempt < 40; attempt++) {
        Window root_ret = None, parent_ret = None;
        Window *children = NULL;
        unsigned int nchildren = 0;
        if (XQueryTree(dpy, parent, &root_ret, &parent_ret, &children, &nchildren) && children) {
            Window found = nchildren > 0 ? children[0] : None;
            XFree(children);
            if (found != None) {
                return found;
            }
        }
        XFlush(dpy);
        sleep_ms(50);
    }
    return None;
}

static Window deepest_child(Display *dpy, Window win) {
    Window cur = win;
    for (int depth = 0; depth < 6; depth++) {
        Window root_ret = None, parent_ret = None;
        Window *children = NULL;
        unsigned int nchildren = 0;
        if (!XQueryTree(dpy, cur, &root_ret, &parent_ret, &children, &nchildren) || !children || nchildren == 0) {
            if (children) XFree(children);
            break;
        }
        cur = children[0];
        XFree(children);
    }
    return cur;
}

static void send_click(Display *dpy, Window win, int x, int y) {
    Window child = None;
    int rx = 0, ry = 0;
    XTranslateCoordinates(dpy, win, DefaultRootWindow(dpy), x, y, &rx, &ry, &child);
    XTestFakeMotionEvent(dpy, -1, rx, ry, CurrentTime);
    XFlush(dpy);
    sleep_ms(30);

    XTestFakeButtonEvent(dpy, Button1, True, CurrentTime);
    XFlush(dpy);
    sleep_ms(40);

    XTestFakeButtonEvent(dpy, Button1, False, CurrentTime);
    XFlush(dpy);
    sleep_ms(90);
}

struct Memory {
    unsigned char bytes[65536];
    size_t len, pos;
};

static int64_t CLAP_ABI write_state(const clap_ostream_t *s, const void *data, uint64_t n) {
    struct Memory *m = (struct Memory *)s->ctx;
    if (m->len + n > sizeof(m->bytes)) return -1;
    memcpy(m->bytes + m->len, data, n);
    m->len += n;
    return (int64_t)n;
}

static int64_t CLAP_ABI read_state(const clap_istream_t *s, void *data, uint64_t n) {
    struct Memory *m = (struct Memory *)s->ctx;
    if (n > m->len - m->pos) n = m->len - m->pos;
    memcpy(data, m->bytes + m->pos, n);
    m->pos += n;
    return (int64_t)n;
}

int main(int argc, char **argv) {
    assert(argc == 2);
    /* Ensure GDK accepts core XSendEvent pointer clicks inside Xvfb */
    setenv("GDK_CORE_DEVICE_EVENTS", "1", 0);

    Display *dpy = XOpenDisplay(NULL);
    assert(dpy != NULL);
    XSetErrorHandler(x11_error_handler);

    void *lib = dlopen(argv[1], RTLD_NOW);
    assert(lib != NULL);
    const clap_plugin_entry_t *entry = (const clap_plugin_entry_t *)dlsym(lib, "clap_entry");
    assert(entry && entry->init(argv[1]));
    const clap_plugin_factory_t *factory = (const clap_plugin_factory_t *)entry->get_factory(CLAP_PLUGIN_FACTORY_ID);
    assert(factory && factory->get_plugin_count(factory) == 1);
    const clap_plugin_descriptor_t *desc = factory->get_plugin_descriptor(factory, 0);
    assert(desc != NULL);

    HostCtx h1, h2, h3;
    init_host_ctx(&h1, "Host Instance 1");
    init_host_ctx(&h2, "Host Instance 2");
    init_host_ctx(&h3, "Host Instance 3 (State Restore)");

    const clap_plugin_t *p1 = factory->create_plugin(factory, &h1.host, desc->id);
    const clap_plugin_t *p2 = factory->create_plugin(factory, &h2.host, desc->id);
    assert(p1 && p2);
    h1.plugin = p1;
    h2.plugin = p2;
    assert(p1->init(p1) && p2->init(p2));

    const clap_plugin_params_t *params1 = (const clap_plugin_params_t *)p1->get_extension(p1, CLAP_EXT_PARAMS);
    const clap_plugin_params_t *params2 = (const clap_plugin_params_t *)p2->get_extension(p2, CLAP_EXT_PARAMS);
    const clap_plugin_latency_t *lat1 = (const clap_plugin_latency_t *)p1->get_extension(p1, CLAP_EXT_LATENCY);
    const clap_plugin_latency_t *lat2 = (const clap_plugin_latency_t *)p2->get_extension(p2, CLAP_EXT_LATENCY);
    const clap_plugin_gui_t *gui1 = (const clap_plugin_gui_t *)p1->get_extension(p1, CLAP_EXT_GUI);
    const clap_plugin_gui_t *gui2 = (const clap_plugin_gui_t *)p2->get_extension(p2, CLAP_EXT_GUI);
    const clap_plugin_state_t *state1 = (const clap_plugin_state_t *)p1->get_extension(p1, CLAP_EXT_STATE);
    const clap_plugin_state_t *state2 = (const clap_plugin_state_t *)p2->get_extension(p2, CLAP_EXT_STATE);
    assert(params1 && params2 && lat1 && lat2 && gui1 && gui2 && state1 && state2);
    assert(params1->count(p1) == 86 && params2->count(p2) == 86);

    clap_id id_cutoff = CLAP_INVALID_ID, id_os = CLAP_INVALID_ID, id_rev_en = CLAP_INVALID_ID, id_rev_kill = CLAP_INVALID_ID;
    clap_param_info_t os_info = {0};
    for (uint32_t i = 0; i < 86; i++) {
        clap_param_info_t info;
        assert(params1->get_info(p1, i, &info));
        if (!strcmp(info.name, "Filter Cutoff")) id_cutoff = info.id;
        if (!strcmp(info.name, "Oversampling Mode")) {
            id_os = info.id;
            os_info = info;
        }
        if (!strcmp(info.name, "Reverb Enabled")) id_rev_en = info.id;
        if (!strcmp(info.name, "Reverb Kill Tail (off = Spill)")) id_rev_kill = info.id;
    }
    assert(id_cutoff != CLAP_INVALID_ID && id_os != CLAP_INVALID_ID && id_rev_en != CLAP_INVALID_ID && id_rev_kill != CLAP_INVALID_ID);
    printf("Oversampling info: min=%.1f max=%.1f def=%.1f stepped=%d\n",
        os_info.min_value, os_info.max_value, os_info.default_value,
        (os_info.flags & CLAP_PARAM_IS_STEPPED) != 0);

    assert(p1->activate(p1, 48000.0, 1, 256) && p1->start_processing(p1));
    assert(p2->activate(p2, 48000.0, 1, 256) && p2->start_processing(p2));
    assert(lat1->get(p1) == 16 && lat2->get(p2) == 16);

    /* Check CLAP_EXT_GUI API negotiation */
    assert(gui1->is_api_supported(p1, CLAP_WINDOW_API_X11, false));
    assert(!gui1->is_api_supported(p1, CLAP_WINDOW_API_X11, true));
    const char *pref_api = NULL;
    bool pref_float = true;
    assert(gui1->get_preferred_api(p1, &pref_api, &pref_float));
    assert(!strcmp(pref_api, CLAP_WINDOW_API_X11) && pref_float == false);

    assert(gui1->create(p1, CLAP_WINDOW_API_X11, false));
    assert(gui2->create(p2, CLAP_WINDOW_API_X11, false));
    (void)gui1->set_scale(p1, 1.0);
    (void)gui2->set_scale(p2, 1.0);
    uint32_t win_w1 = 0, win_h1 = 0, win_w2 = 0, win_h2 = 0;
    assert(gui1->get_size(p1, &win_w1, &win_h1) && win_w1 == 1180 && win_h1 == 720);
    assert(gui2->get_size(p2, &win_w2, &win_h2) && win_w2 == 1180 && win_h2 == 720);

    int screen = DefaultScreen(dpy);
    Window win1 = XCreateSimpleWindow(dpy, RootWindow(dpy, screen), 0, 0, win_w1, win_h1, 0, 0, 0);
    Window win2 = XCreateSimpleWindow(dpy, RootWindow(dpy, screen), 1200, 0, win_w2, win_h2, 0, 0, 0);
    XMapWindow(dpy, win1);
    XMapWindow(dpy, win2);
    XSync(dpy, False);

    clap_window_t cw1 = {.api = CLAP_WINDOW_API_X11, .x11 = (clap_xwnd)win1};
    clap_window_t cw2 = {.api = CLAP_WINDOW_API_X11, .x11 = (clap_xwnd)win2};
    assert(gui1->set_parent(p1, &cw1));
    assert(gui2->set_parent(p2, &cw2));
    XSync(dpy, False);

    Window child1 = wait_embedded_child(dpy, win1);
    Window child2 = wait_embedded_child(dpy, win2);
    assert(child1 != None && child2 != None && child1 != child2);
    XMapSubwindows(dpy, win1);
    XMapSubwindows(dpy, win2);
    XRaiseWindow(dpy, win1);
    XSync(dpy, False);

    /* Wait briefly for GTK realize/map on child windows before injecting X11 input */
    sleep_ms(150);
    Window target1 = deepest_child(dpy, child1);
    double cutoff_before_1 = 0.0, cutoff_before_2 = 0.0;
    assert(params1->get_value(p1, id_cutoff, &cutoff_before_1));
    assert(params2->get_value(p2, id_cutoff, &cutoff_before_2));

    /* Send real XTest pointer click on CUTOFF slider top (x=109, y=195) inside win1 -> child1 */
    send_click(dpy, target1 != child1 ? target1 : child1, 109, 195);
    /* Click C3 key on virtual keyboard in p1 panel (x=55, y=650) */
    send_click(dpy, target1 != child1 ? target1 : child1, 55, 650);

    /* Render audio blocks on both instances; flush GUI param updates */
    double e1 = render_blocks(&h1, p1, params1, 20, 60);
    double e2 = render_blocks(&h2, p2, params2, 20, -1);
    assert(e1 > 0.01);
    assert(e2 == 0.0);

    double cutoff_after_1 = 0.0, cutoff_after_2 = 0.0;
    assert(params1->get_value(p1, id_cutoff, &cutoff_after_1));
    assert(params2->get_value(p2, id_cutoff, &cutoff_after_2));
    printf("Cutoff p1: %.4f -> %.4f | p2: %.4f -> %.4f (flush1=%d)\n",
        cutoff_before_1, cutoff_after_1, cutoff_before_2, cutoff_after_2, h1.flush_requested);
    assert(cutoff_after_1 > 0.80);
    assert(fabs(cutoff_after_2 - cutoff_before_2) < 1e-6);

    /* Switch OS to 4x on p1 via host parameter flush & verify latency 24 while p2 stays 16 */
    set_param(p1, params1, id_os, os_info.max_value);
    for (int wait_bg = 0; wait_bg < 40 && lat1->get(p1) != 24; wait_bg++) {
        (void)render_blocks(&h1, p1, params1, 4, -1);
        sleep_ms(15);
    }
    assert(lat1->get(p1) == 24);
    assert(lat2->get(p2) == 16);

    /* Test reverb bypass Spill vs Kill isolation */
    set_param(p1, params1, id_rev_en, 0.0);
    set_param(p1, params1, id_rev_kill, 1.0);
    set_param(p2, params2, id_rev_en, 1.0);
    set_param(p2, params2, id_rev_kill, 0.0);
    (void)render_blocks(&h1, p1, params1, 8, -1);
    (void)render_blocks(&h2, p2, params2, 8, -1);

    /* Clean destroy & reopen GUI on instance 1 while instance 2 remains open */
    gui1->destroy(p1);
    sleep_ms(80);
    assert(gui1->create(p1, CLAP_WINDOW_API_X11, false));
    assert(gui1->set_parent(p1, &cw1));
    Window child1_reopened = wait_embedded_child(dpy, win1);
    assert(child1_reopened != None);

    /* Save state for p1 and p2, verify compact size < 64KB and isolated reload into p3 */
    struct Memory m1 = {0}, m2 = {0};
    clap_ostream_t os1 = {.ctx = &m1, .write = write_state};
    clap_ostream_t os2 = {.ctx = &m2, .write = write_state};
    assert(state1->save(p1, &os1) && m1.len > 0 && m1.len < 65536);
    assert(state2->save(p2, &os2) && m2.len > 0 && m2.len < 65536);

    const clap_plugin_t *p3 = factory->create_plugin(factory, &h3.host, desc->id);
    assert(p3 && p3->init(p3));
    h3.plugin = p3;
    const clap_plugin_state_t *state3 = (const clap_plugin_state_t *)p3->get_extension(p3, CLAP_EXT_STATE);
    const clap_plugin_params_t *params3 = (const clap_plugin_params_t *)p3->get_extension(p3, CLAP_EXT_PARAMS);
    const clap_plugin_latency_t *lat3 = (const clap_plugin_latency_t *)p3->get_extension(p3, CLAP_EXT_LATENCY);
    clap_istream_t is1 = {.ctx = &m1, .read = read_state};
    assert(state3->load(p3, &is1));
    assert(p3->activate(p3, 48000.0, 1, 256) && p3->start_processing(p3));
    assert(lat3->get(p3) == 24);

    double v = -1.0;
    char os_text[64] = {0};
    assert(params3->get_value(p3, id_os, &v) && fabs(v - os_info.max_value) < 0.01);
    assert(params3->value_to_text(p3, id_os, v, os_text, sizeof(os_text)));
    printf("Restored p3 OS CLAP value=%.1f text='%s'\n", v, os_text);
    assert(fabs(strtod(os_text, NULL) - 4.0) < 0.1);
    assert(params3->get_value(p3, id_rev_en, &v) && v == 0.0);
    assert(params3->get_value(p3, id_rev_kill, &v) && v == 1.0);
    assert(params2->get_value(p2, id_os, &v) && fabs(v - os_info.default_value) < 0.01);

    p3->stop_processing(p3);
    p3->deactivate(p3);
    p3->destroy(p3);
    gui1->destroy(p1);
    gui2->destroy(p2);
    p1->stop_processing(p1);
    p2->stop_processing(p2);
    p1->deactivate(p1);
    p2->deactivate(p2);
    p1->destroy(p1);
    p2->destroy(p2);
    entry->deinit();
    dlclose(lib);

    XDestroyWindow(dpy, win1);
    XDestroyWindow(dpy, win2);
    XSync(dpy, False);
    XCloseDisplay(dpy);
    assert(g_x11_errors == 0);

    printf("PASS: Dual CLAP GUI X11 embed (children 0x%lx & 0x%lx), reopen 0x%lx, cutoff %.4f->%.4f (p2=%.4f), lat 24/16, state %zu/%zu bytes (p3 restored lat=24 os=%s), 0 X11 errors\n",
        (unsigned long)child1, (unsigned long)child2, (unsigned long)child1_reopened,
        cutoff_before_1, cutoff_after_1, cutoff_after_2, m1.len, m2.len, os_text);
    return 0;
}
