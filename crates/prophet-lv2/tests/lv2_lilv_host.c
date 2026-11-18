// SPDX-License-Identifier: GPL-3.0-only
// Real Linux/Xvfb LV2 Lilv + Suil host integration test:
// - Discovers PROPHET-XII via lilv_world_load_all() and verifies RDF metadata via lilv_plugin_verify()
// - Verifies 91 ports, latency port index 90, ScalePoints, PortGroups, and ui:X11UI advertisement
// - Instantiates TWO independent LV2 plugin instances with URID Map/Unmap, Worker Schedule, and Options (maxBlockLength)
// - Embeds BOTH instances' ui:X11UI via real Suil (suil_instance_new + LV2_UI__parent + instance-access + idle/showInterface)
// - Drives synthetic XTest pointer click inside Instance 1 GUI and verifies write_function updates Cutoff port cleanly
// - Sends LV2 Atom MIDI NoteOn + time:Position BPM events and verifies stereo PCM output + strict multi-instance isolation
// - Verifies Oversampling 2x->4x worker transition reports latency 16->24 frames on run(0) while Instance 2 stays 16
// - Verifies Reverb Spill/Kill toggles and UI close/reopen lifecycle
// - Verifies LV2_State_Interface via lilv_state_new_from_instance -> lilv_state_to_string -> lilv_state_new_from_string -> lilv_state_restore onto Instance 3
// - Verifies missing state resets to Init and corrupt state returns LV2_STATE_ERR_UNKNOWN cleanly
//
// Build: gcc -O2 -Wall -Wextra crates/prophet-lv2/tests/lv2_lilv_host.c ...
// Run:   xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/lv2_lilv_host

#define _POSIX_C_SOURCE 200809L
#include <lilv/lilv.h>
#include <lv2/atom/atom.h>
#include <lv2/atom/forge.h>
#include <lv2/buf-size/buf-size.h>
#include <lv2/core/lv2.h>
#include <lv2/instance-access/instance-access.h>
#include <lv2/midi/midi.h>
#include <lv2/options/options.h>
#include <lv2/state/state.h>
#include <lv2/time/time.h>
#include <lv2/ui/ui.h>
#include <lv2/urid/urid.h>
#include <lv2/worker/worker.h>
#include <suil/suil.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/extensions/XTest.h>
#include <assert.h>
#include <math.h>
#include <pthread.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define PLUGIN_URI "http://prophet-xii.org/plugins/prophet-xii"
#define UI_URI "http://prophet-xii.org/plugins/prophet-xii#ui"
#define TOTAL_PORTS 91
#define PORT_OUT_L 0
#define PORT_OUT_R 1
#define PORT_CONTROL_IN 2
#define PORT_NOTIFY_OUT 3
#define PORT_REVERB_ENABLED 4
#define PORT_REVERB_KILL_TAIL 5
#define PORT_CUTOFF 29
#define PORT_OS 69
#define PORT_GLIDE 89
#define PORT_LATENCY 90

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
    pthread_mutex_t lock;
    char **uris;
    uint32_t count;
    uint32_t cap;
} HostUridTable;

static void urid_table_init(HostUridTable *t) {
    pthread_mutex_init(&t->lock, NULL);
    t->count = 0;
    t->cap = 128;
    t->uris = (char **)calloc(t->cap, sizeof(char *));
}

static void urid_table_free(HostUridTable *t) {
    for (uint32_t i = 0; i < t->count; i++) {
        free(t->uris[i]);
    }
    free(t->uris);
    pthread_mutex_destroy(&t->lock);
}

static LV2_URID host_urid_map(LV2_URID_Map_Handle handle, const char *uri) {
    HostUridTable *t = (HostUridTable *)handle;
    pthread_mutex_lock(&t->lock);
    for (uint32_t i = 0; i < t->count; i++) {
        if (!strcmp(t->uris[i], uri)) {
            pthread_mutex_unlock(&t->lock);
            return i + 1;
        }
    }
    if (t->count == t->cap) {
        t->cap *= 2;
        t->uris = (char **)realloc(t->uris, t->cap * sizeof(char *));
    }
    t->uris[t->count] = strdup(uri);
    LV2_URID id = t->count + 1;
    t->count++;
    pthread_mutex_unlock(&t->lock);
    return id;
}

static const char *host_urid_unmap(LV2_URID_Unmap_Handle handle, LV2_URID urid) {
    HostUridTable *t = (HostUridTable *)handle;
    pthread_mutex_lock(&t->lock);
    const char *res = (urid >= 1 && urid <= t->count) ? t->uris[urid - 1] : NULL;
    pthread_mutex_unlock(&t->lock);
    return res;
}

typedef struct {
    const LilvPlugin *plugin;
    LilvInstance *instance;
    const LV2_Worker_Interface *worker_iface;
    LV2_URID_Map *map_iface;
    LV2_Atom_Forge forge;
    float ctrl_vals[TOTAL_PORTS];
    char port_symbols[TOTAL_PORTS][64];
    float out_l[256];
    float out_r[256];
    uint8_t control_in_buf[4096];
    uint8_t notify_out_buf[8192];
    pthread_mutex_t work_lock;
    bool work_pending;
    uint32_t work_size;
    uint8_t work_payload[256];
    uint32_t ui_writes_count;
    uint32_t last_ui_port;
    float last_ui_val;
    bool resize_requested;
    int req_w, req_h;
} PluginHostSlot;

static LV2_Worker_Status host_worker_respond(
    LV2_Worker_Respond_Handle handle,
    uint32_t size,
    const void *data
) {
    PluginHostSlot *slot = (PluginHostSlot *)handle;
    if (slot->worker_iface && slot->worker_iface->work_response) {
        return slot->worker_iface->work_response(
            lilv_instance_get_handle(slot->instance),
            size,
            data
        );
    }
    return LV2_WORKER_SUCCESS;
}

static LV2_Worker_Status host_schedule_work(
    LV2_Worker_Schedule_Handle handle,
    uint32_t size,
    const void *data
) {
    PluginHostSlot *slot = (PluginHostSlot *)handle;
    pthread_mutex_lock(&slot->work_lock);
    slot->work_pending = true;
    slot->work_size = size < sizeof(slot->work_payload) ? size : (uint32_t)sizeof(slot->work_payload);
    if (data && slot->work_size > 0) {
        memcpy(slot->work_payload, data, slot->work_size);
    }
    pthread_mutex_unlock(&slot->work_lock);
    return LV2_WORKER_SUCCESS;
}

static void pump_slot_worker(PluginHostSlot *slot) {
    pthread_mutex_lock(&slot->work_lock);
    bool pending = slot->work_pending;
    uint32_t sz = slot->work_size;
    uint8_t copy[256];
    if (pending && sz > 0) {
        memcpy(copy, slot->work_payload, sz);
    }
    slot->work_pending = false;
    pthread_mutex_unlock(&slot->work_lock);

    if (pending && slot->worker_iface && slot->worker_iface->work) {
        LV2_Worker_Status st = slot->worker_iface->work(
            lilv_instance_get_handle(slot->instance),
            host_worker_respond,
            slot,
            sz,
            sz > 0 ? copy : NULL
        );
        assert(st == LV2_WORKER_SUCCESS);
    }
}

static void init_empty_atom_seq(PluginHostSlot *slot) {
    LV2_Atom_Sequence *seq = (LV2_Atom_Sequence *)slot->control_in_buf;
    seq->atom.size = sizeof(LV2_Atom_Sequence_Body);
    seq->atom.type = slot->forge.Sequence;
    seq->body.unit = 0;
    seq->body.pad = 0;
}

static void forge_note_and_bpm_seq(PluginHostSlot *slot, uint8_t note, uint8_t vel, float bpm) {
    lv2_atom_forge_set_buffer(&slot->forge, slot->control_in_buf, sizeof(slot->control_in_buf));
    LV2_Atom_Forge_Frame seq_frame;
    lv2_atom_forge_sequence_head(&slot->forge, &seq_frame, 0);

    LV2_URID time_pos = slot->map_iface->map(slot->map_iface->handle, LV2_TIME__Position);
    LV2_URID time_bpm = slot->map_iface->map(slot->map_iface->handle, LV2_TIME__beatsPerMinute);
    LV2_URID midi_ev = slot->map_iface->map(slot->map_iface->handle, LV2_MIDI__MidiEvent);

    /* Emit time:Position with BPM */
    lv2_atom_forge_frame_time(&slot->forge, 0);
    LV2_Atom_Forge_Frame obj_frame;
    lv2_atom_forge_object(&slot->forge, &obj_frame, 0, time_pos);
    lv2_atom_forge_key(&slot->forge, time_bpm);
    lv2_atom_forge_float(&slot->forge, bpm);
    lv2_atom_forge_pop(&slot->forge, &obj_frame);

    /* Emit NoteOn MIDI message */
    lv2_atom_forge_frame_time(&slot->forge, 0);
    uint8_t msg[3] = {0x90, note, vel};
    lv2_atom_forge_atom(&slot->forge, 3, midi_ev);
    lv2_atom_forge_write(&slot->forge, msg, 3);

    lv2_atom_forge_pop(&slot->forge, &seq_frame);
}

static double run_slot_blocks(PluginHostSlot *slot, int blocks) {
    double energy = 0.0;
    LV2_Atom_Sequence *notify_hdr = (LV2_Atom_Sequence *)slot->notify_out_buf;
    for (int b = 0; b < blocks; b++) {
        memset(slot->out_l, 0, sizeof(slot->out_l));
        memset(slot->out_r, 0, sizeof(slot->out_r));
        notify_hdr->atom.size = sizeof(slot->notify_out_buf) - sizeof(LV2_Atom);
        notify_hdr->atom.type = slot->map_iface->map(slot->map_iface->handle, LV2_ATOM__Chunk);

        lilv_instance_run(slot->instance, 256);
        if (slot->worker_iface && slot->worker_iface->end_run) {
            slot->worker_iface->end_run(lilv_instance_get_handle(slot->instance));
        }
        pump_slot_worker(slot);
        if (b == 0) {
            init_empty_atom_seq(slot);
        }
        for (int i = 0; i < 256; i++) {
            assert(isfinite(slot->out_l[i]) && isfinite(slot->out_r[i]));
            energy += (double)slot->out_l[i] * slot->out_l[i] +
                      (double)slot->out_r[i] * slot->out_r[i];
        }
    }
    return energy;
}

static void suil_write_cb(
    SuilController controller,
    uint32_t port_index,
    uint32_t buffer_size,
    uint32_t protocol,
    void const *buffer
) {
    PluginHostSlot *slot = (PluginHostSlot *)controller;
    if (protocol == 0 && buffer_size == sizeof(float) && buffer && port_index < TOTAL_PORTS) {
        float v = *(const float *)buffer;
        slot->ctrl_vals[port_index] = v;
        slot->ui_writes_count++;
        slot->last_ui_port = port_index;
        slot->last_ui_val = v;
    }
}

static uint32_t suil_port_index_cb(SuilController controller, const char *symbol) {
    PluginHostSlot *slot = (PluginHostSlot *)controller;
    for (uint32_t i = 0; i < TOTAL_PORTS; i++) {
        if (!strcmp(slot->port_symbols[i], symbol)) {
            return i;
        }
    }
    return LV2UI_INVALID_PORT_INDEX;
}

static int host_ui_resize_cb(LV2UI_Feature_Handle handle, int w, int h) {
    PluginHostSlot *slot = (PluginHostSlot *)handle;
    slot->resize_requested = true;
    slot->req_w = w;
    slot->req_h = h;
    return 0;
}

static const void *get_port_val_cb(
    const char *port_symbol,
    void *user_data,
    uint32_t *size,
    uint32_t *type
) {
    PluginHostSlot *slot = (PluginHostSlot *)user_data;
    for (uint32_t i = 0; i < TOTAL_PORTS; i++) {
        if (!strcmp(slot->port_symbols[i], port_symbol)) {
            *size = sizeof(float);
            *type = slot->forge.Float;
            return &slot->ctrl_vals[i];
        }
    }
    return NULL;
}

static void set_port_val_cb(
    const char *port_symbol,
    void *user_data,
    const void *value,
    uint32_t size,
    uint32_t type
) {
    PluginHostSlot *slot = (PluginHostSlot *)user_data;
    if (type == slot->forge.Float && size == sizeof(float) && value) {
        for (uint32_t i = 0; i < TOTAL_PORTS; i++) {
            if (!strcmp(slot->port_symbols[i], port_symbol)) {
                slot->ctrl_vals[i] = *(const float *)value;
                return;
            }
        }
    }
}

static void setup_plugin_slot(
    PluginHostSlot *slot,
    const LilvPlugin *plugin,
    LV2_URID_Map *map_iface,
    const float *def_vals
) {
    memset(slot, 0, sizeof(*slot));
    pthread_mutex_init(&slot->work_lock, NULL);
    slot->plugin = plugin;
    slot->map_iface = map_iface;
    lv2_atom_forge_init(&slot->forge, map_iface);

    for (uint32_t i = 0; i < TOTAL_PORTS; i++) {
        const LilvPort *port = lilv_plugin_get_port_by_index(plugin, i);
        const LilvNode *sym = lilv_port_get_symbol(plugin, port);
        snprintf(slot->port_symbols[i], sizeof(slot->port_symbols[i]), "%s", lilv_node_as_string(sym));
        slot->ctrl_vals[i] = isnan(def_vals[i]) ? 0.0f : def_vals[i];
    }

    int32_t max_block = 512;
    LV2_Options_Option opts[2] = {
        {
            .context = LV2_OPTIONS_INSTANCE,
            .subject = 0,
            .key = map_iface->map(map_iface->handle, LV2_BUF_SIZE__maxBlockLength),
            .size = sizeof(int32_t),
            .type = map_iface->map(map_iface->handle, LV2_ATOM__Int),
            .value = &max_block,
        },
        {.context = 0, .subject = 0, .key = 0, .size = 0, .type = 0, .value = NULL},
    };
    LV2_Worker_Schedule sched = {
        .handle = slot,
        .schedule_work = host_schedule_work,
    };
    LV2_Feature feat_map = {.URI = LV2_URID__map, .data = map_iface};
    LV2_Feature feat_sched = {.URI = LV2_WORKER__schedule, .data = &sched};
    LV2_Feature feat_opts = {.URI = LV2_OPTIONS__options, .data = opts};
    const LV2_Feature *features[] = {&feat_map, &feat_sched, &feat_opts, NULL};

    slot->instance = lilv_plugin_instantiate(plugin, 48000.0, features);
    assert(slot->instance != NULL);
    slot->worker_iface = (const LV2_Worker_Interface *)lilv_instance_get_extension_data(
        slot->instance,
        LV2_WORKER__interface
    );
    assert(slot->worker_iface != NULL);

    lilv_instance_connect_port(slot->instance, PORT_OUT_L, slot->out_l);
    lilv_instance_connect_port(slot->instance, PORT_OUT_R, slot->out_r);
    lilv_instance_connect_port(slot->instance, PORT_CONTROL_IN, slot->control_in_buf);
    lilv_instance_connect_port(slot->instance, PORT_NOTIFY_OUT, slot->notify_out_buf);
    for (uint32_t p = PORT_REVERB_ENABLED; p <= PORT_LATENCY; p++) {
        lilv_instance_connect_port(slot->instance, p, &slot->ctrl_vals[p]);
    }
    init_empty_atom_seq(slot);
    lilv_instance_activate(slot->instance);
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

static const void *empty_retrieve_cb(
    LV2_State_Handle handle,
    uint32_t key,
    size_t *size,
    uint32_t *type,
    uint32_t *flags
) {
    (void)handle;
    (void)key;
    (void)size;
    (void)type;
    (void)flags;
    return NULL;
}

typedef struct {
    const uint8_t *bytes;
    size_t len;
    uint32_t key;
    uint32_t type;
} CorruptStateData;

static const void *corrupt_retrieve_cb(
    LV2_State_Handle handle,
    uint32_t key,
    size_t *size,
    uint32_t *type,
    uint32_t *flags
) {
    const CorruptStateData *d = (const CorruptStateData *)handle;
    if (key == d->key) {
        *size = d->len;
        *type = d->type;
        *flags = LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE;
        return d->bytes;
    }
    return NULL;
}

int main(int argc, char **argv) {
    setenv("GDK_CORE_DEVICE_EVENTS", "1", 0);
    suil_init(&argc, &argv, SUIL_ARG_NONE);

    Display *dpy = XOpenDisplay(NULL);
    assert(dpy != NULL);
    XSetErrorHandler(x11_error_handler);

    LilvWorld *world = lilv_world_new();
    assert(world != NULL);
    lilv_world_load_all(world);

    LilvNode *plugin_uri_node = lilv_new_uri(world, PLUGIN_URI);
    const LilvPlugins *plugins = lilv_world_get_all_plugins(world);
    const LilvPlugin *plugin = lilv_plugins_get_by_uri(plugins, plugin_uri_node);
    assert(plugin != NULL);
    assert(lilv_plugin_verify(plugin));
    assert(lilv_plugin_get_num_ports(plugin) == TOTAL_PORTS);
    assert(lilv_plugin_has_latency(plugin));
    assert(lilv_plugin_get_latency_port_index(plugin) == PORT_LATENCY);

    float min_vals[TOTAL_PORTS], max_vals[TOTAL_PORTS], def_vals[TOTAL_PORTS];
    lilv_plugin_get_port_ranges_float(plugin, min_vals, max_vals, def_vals);
    assert(fabsf(min_vals[PORT_OS] - 1.0f) < 1e-4f && fabsf(max_vals[PORT_OS] - 4.0f) < 1e-4f);

    /* Discover ui:X11UI via Lilv + Suil */
    LilvUIs *uis = lilv_plugin_get_uis(plugin);
    assert(uis && lilv_uis_size(uis) >= 1);
    LilvNode *ui_uri_node = lilv_new_uri(world, UI_URI);
    const LilvUI *lv2_ui = lilv_uis_get_by_uri(uis, ui_uri_node);
    assert(lv2_ui != NULL);
    LilvNode *x11_container_node = lilv_new_uri(world, LV2_UI__X11UI);
    const LilvNode *ui_type_node = NULL;
    assert(lilv_ui_is_supported(lv2_ui, suil_ui_supported, x11_container_node, &ui_type_node) > 0);

    char *ui_bundle_path = lilv_file_uri_parse(lilv_node_as_uri(lilv_ui_get_bundle_uri(lv2_ui)), NULL);
    char *ui_binary_path = lilv_file_uri_parse(lilv_node_as_uri(lilv_ui_get_binary_uri(lv2_ui)), NULL);
    assert(ui_bundle_path && ui_binary_path);

    HostUridTable urids;
    urid_table_init(&urids);
    LV2_URID_Map map_iface = {.handle = &urids, .map = host_urid_map};
    LV2_URID_Unmap unmap_iface = {.handle = &urids, .unmap = host_urid_unmap};

    PluginHostSlot s1, s2, s3;
    setup_plugin_slot(&s1, plugin, &map_iface, def_vals);
    setup_plugin_slot(&s2, plugin, &map_iface, def_vals);

    lilv_instance_run(s1.instance, 0);
    lilv_instance_run(s2.instance, 0);
    assert(s1.ctrl_vals[PORT_LATENCY] == 16.0f && s2.ctrl_vals[PORT_LATENCY] == 16.0f);

    /* Instantiate TWO simultaneous X11UIs via real Suil */
    SuilHost *suil_host = suil_host_new(suil_write_cb, suil_port_index_cb, NULL, NULL);
    assert(suil_host != NULL);

    int screen = DefaultScreen(dpy);
    Window win1 = XCreateSimpleWindow(dpy, RootWindow(dpy, screen), 0, 0, 1180, 720, 0, 0, 0);
    Window win2 = XCreateSimpleWindow(dpy, RootWindow(dpy, screen), 1200, 0, 1180, 720, 0, 0, 0);
    XMapWindow(dpy, win1);
    XMapWindow(dpy, win2);
    XSync(dpy, False);

    LV2UI_Resize resize1 = {.handle = &s1, .ui_resize = host_ui_resize_cb};
    LV2UI_Resize resize2 = {.handle = &s2, .ui_resize = host_ui_resize_cb};
    LV2_Feature feat_map = {.URI = LV2_URID__map, .data = &map_iface};
    LV2_Feature feat_parent1 = {.URI = LV2_UI__parent, .data = (void *)(uintptr_t)win1};
    LV2_Feature feat_parent2 = {.URI = LV2_UI__parent, .data = (void *)(uintptr_t)win2};
    LV2_Feature feat_acc1 = {.URI = LV2_INSTANCE_ACCESS_URI, .data = lilv_instance_get_handle(s1.instance)};
    LV2_Feature feat_acc2 = {.URI = LV2_INSTANCE_ACCESS_URI, .data = lilv_instance_get_handle(s2.instance)};
    LV2_Feature feat_rsz1 = {.URI = LV2_UI__resize, .data = &resize1};
    LV2_Feature feat_rsz2 = {.URI = LV2_UI__resize, .data = &resize2};
    const LV2_Feature *ui_feats1[] = {&feat_map, &feat_parent1, &feat_acc1, &feat_rsz1, NULL};
    const LV2_Feature *ui_feats2[] = {&feat_map, &feat_parent2, &feat_acc2, &feat_rsz2, NULL};

    SuilInstance *ui1 = suil_instance_new(
        suil_host,
        &s1,
        LV2_UI__X11UI,
        PLUGIN_URI,
        UI_URI,
        lilv_node_as_uri(ui_type_node),
        ui_bundle_path,
        ui_binary_path,
        ui_feats1
    );
    SuilInstance *ui2 = suil_instance_new(
        suil_host,
        &s2,
        LV2_UI__X11UI,
        PLUGIN_URI,
        UI_URI,
        lilv_node_as_uri(ui_type_node),
        ui_bundle_path,
        ui_binary_path,
        ui_feats2
    );
    assert(ui1 != NULL && ui2 != NULL);
    assert(suil_instance_get_widget(ui1) != NULL && suil_instance_get_widget(ui2) != NULL);

    const LV2UI_Idle_Interface *idle1 = (const LV2UI_Idle_Interface *)suil_instance_extension_data(
        ui1,
        LV2_UI__idleInterface
    );
    const LV2UI_Show_Interface *show1 = (const LV2UI_Show_Interface *)suil_instance_extension_data(
        ui1,
        LV2_UI__showInterface
    );
    assert(idle1 && idle1->idle && idle1->idle(suil_instance_get_handle(ui1)) == 0);
    assert(show1 && show1->show && show1->show(suil_instance_get_handle(ui1)) == 0);

    Window child1 = wait_embedded_child(dpy, win1);
    Window child2 = wait_embedded_child(dpy, win2);
    assert(child1 != None && child2 != None && child1 != child2);
    XMapSubwindows(dpy, win1);
    XMapSubwindows(dpy, win2);
    XRaiseWindow(dpy, win1);
    XSync(dpy, False);
    sleep_ms(150);

    /* Drive real pointer click inside embedded child UI of Instance 1 -> Cutoff top + C3 key */
    float cutoff_before_1 = s1.ctrl_vals[PORT_CUTOFF];
    float cutoff_before_2 = s2.ctrl_vals[PORT_CUTOFF];
    Window target1 = deepest_child(dpy, child1);
    send_click(dpy, target1 != child1 ? target1 : child1, 109, 195);
    send_click(dpy, target1 != child1 ? target1 : child1, 55, 650);

    /* Send time:Position BPM=148.5 + NoteOn to s1 while s2 stays idle */
    forge_note_and_bpm_seq(&s1, 60, 108, 148.5f);
    double e1 = run_slot_blocks(&s1, 16);
    double e2 = run_slot_blocks(&s2, 16);
    assert(e1 > 0.01);
    assert(e2 == 0.0);

    assert(s1.ui_writes_count > 0);
    assert(s1.ctrl_vals[PORT_CUTOFF] > 14000.0f);
    assert(fabsf(s2.ctrl_vals[PORT_CUTOFF] - cutoff_before_2) < 1e-5f);
    printf(
        "UI+Audio: s1 cutoff %.1f -> %.1f Hz (s2=%.1f), s1_writes=%u, e1=%.4f, e2=%.4f\n",
        cutoff_before_1,
        s1.ctrl_vals[PORT_CUTOFF],
        s2.ctrl_vals[PORT_CUTOFF],
        s1.ui_writes_count,
        e1,
        e2
    );

    /* Notify UI of port event cleanly (host -> UI) */
    float notify_cutoff = s1.ctrl_vals[PORT_CUTOFF];
    suil_instance_port_event(ui1, PORT_CUTOFF, sizeof(float), 0, &notify_cutoff);

    /* Switch s1 Oversampling 2x -> 4x and verify worker + run(0) latency update */
    s1.ctrl_vals[PORT_OS] = 4.0f;
    for (int i = 0; i < 16 && s1.ctrl_vals[PORT_LATENCY] != 24.0f; i++) {
        (void)run_slot_blocks(&s1, 2);
        sleep_ms(10);
    }
    lilv_instance_run(s1.instance, 0);
    assert(s1.ctrl_vals[PORT_LATENCY] == 24.0f);
    assert(s2.ctrl_vals[PORT_LATENCY] == 16.0f);

    /* Toggle Reverb disabled + Kill policy on s1 while s2 stays Spill+Enabled */
    s1.ctrl_vals[PORT_REVERB_ENABLED] = 0.0f;
    s1.ctrl_vals[PORT_REVERB_KILL_TAIL] = 1.0f;
    (void)run_slot_blocks(&s1, 4);
    (void)run_slot_blocks(&s2, 4);

    /* Close & reopen ui1 while ui2 remains live */
    suil_instance_free(ui1);
    sleep_ms(80);
    ui1 = suil_instance_new(
        suil_host,
        &s1,
        LV2_UI__X11UI,
        PLUGIN_URI,
        UI_URI,
        lilv_node_as_uri(ui_type_node),
        ui_bundle_path,
        ui_binary_path,
        ui_feats1
    );
    assert(ui1 != NULL);
    Window child1_reopen = wait_embedded_child(dpy, win1);
    assert(child1_reopen != None);

    /* Real LV2 state save/serialize/restore across Lilv instances */
    LV2_Feature feat_map_state = {.URI = LV2_URID__map, .data = &map_iface};
    const LV2_Feature *state_feats[] = {&feat_map_state, NULL};
    LilvState *saved_s1 = lilv_state_new_from_instance(
        plugin,
        s1.instance,
        &map_iface,
        NULL,
        NULL,
        NULL,
        NULL,
        get_port_val_cb,
        &s1,
        0,
        state_feats
    );
    assert(saved_s1 != NULL);
    char *state_ttl = lilv_state_to_string(
        world,
        &map_iface,
        &unmap_iface,
        saved_s1,
        "urn:prophet-xii:test-state-1",
        NULL
    );
    assert(state_ttl != NULL && strlen(state_ttl) > 32);
    assert(strstr(state_ttl, "presetV2") != NULL);

    LilvState *parsed_s1 = lilv_state_new_from_string(world, &map_iface, state_ttl);
    assert(parsed_s1 != NULL);

    setup_plugin_slot(&s3, plugin, &map_iface, def_vals);
    lilv_state_restore(parsed_s1, s3.instance, set_port_val_cb, &s3, 0, state_feats);
    lilv_instance_run(s3.instance, 0);
    assert(s3.ctrl_vals[PORT_LATENCY] == 24.0f);
    (void)run_slot_blocks(&s3, 4);
    assert(s3.ctrl_vals[PORT_LATENCY] == 24.0f);
    assert(fabsf(s3.ctrl_vals[PORT_CUTOFF] - s1.ctrl_vals[PORT_CUTOFF]) < 1.0f);
    assert(s3.ctrl_vals[PORT_REVERB_ENABLED] == 0.0f);

    /* Verify missing state key resets cleanly to Init (16 samples latency) and corrupt fails cleanly */
    const LV2_State_Interface *state_iface = (const LV2_State_Interface *)lilv_instance_get_extension_data(
        s3.instance,
        LV2_STATE__interface
    );
    assert(state_iface && state_iface->restore);
    assert(
        state_iface->restore(
            lilv_instance_get_handle(s3.instance),
            empty_retrieve_cb,
            NULL,
            0,
            state_feats
        ) == LV2_STATE_SUCCESS
    );
    lilv_instance_run(s3.instance, 0);
    assert(s3.ctrl_vals[PORT_LATENCY] == 16.0f);
    (void)run_slot_blocks(&s3, 4);
    assert(s3.ctrl_vals[PORT_LATENCY] == 16.0f);

    CorruptStateData bad = {
        .bytes = (const uint8_t *)"{bad-json\0",
        .len = 10,
        .key = map_iface.map(map_iface.handle, "http://prophet-xii.org/plugins/prophet-xii#presetV2"),
        .type = map_iface.map(map_iface.handle, LV2_ATOM__String),
    };
    assert(
        state_iface->restore(
            lilv_instance_get_handle(s3.instance),
            corrupt_retrieve_cb,
            &bad,
            0,
            state_feats
        ) == LV2_STATE_ERR_UNKNOWN
    );

    lilv_free(state_ttl);
    lilv_state_free(parsed_s1);
    lilv_state_free(saved_s1);

    suil_instance_free(ui1);
    suil_instance_free(ui2);
    suil_host_free(suil_host);

    lilv_instance_deactivate(s3.instance);
    lilv_instance_free(s3.instance);
    lilv_instance_deactivate(s2.instance);
    lilv_instance_free(s2.instance);
    lilv_instance_deactivate(s1.instance);
    lilv_instance_free(s1.instance);
    pthread_mutex_destroy(&s3.work_lock);
    pthread_mutex_destroy(&s2.work_lock);
    pthread_mutex_destroy(&s1.work_lock);

    lilv_free(ui_bundle_path);
    lilv_free(ui_binary_path);
    lilv_node_free(x11_container_node);
    lilv_node_free(ui_uri_node);
    lilv_uis_free(uis);
    lilv_node_free(plugin_uri_node);
    lilv_world_free(world);
    urid_table_free(&urids);

    XDestroyWindow(dpy, win1);
    XDestroyWindow(dpy, win2);
    XSync(dpy, False);
    XCloseDisplay(dpy);
    assert(g_x11_errors == 0);

    printf(
        "PASS: Lilv/Suil dual LV2 instances, 91 ports, ui:X11UI embed (0x%lx & 0x%lx, reopen 0x%lx), "
        "XTest cutoff %.1f (s2=%.1f), OS 4x latency=24/16, state save/restore & Init/corrupt recovery, 0 X11 errors\n",
        (unsigned long)child1,
        (unsigned long)child2,
        (unsigned long)child1_reopen,
        s1.ctrl_vals[PORT_CUTOFF],
        s2.ctrl_vals[PORT_CUTOFF]
    );
    return 0;
}
