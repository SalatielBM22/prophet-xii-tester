// SPDX-License-Identifier: GPL-3.0-only
//! LV2 X11/GTK3 Embedded UI (`ui:X11UI`) backed by `prophet_host::embed` and `HostControlPlane`.

use crate::lv2_ffi::{
    LV2UI_Controller, LV2UI_Descriptor, LV2UI_Feature_Handle, LV2UI_Handle, LV2UI_Idle_Interface,
    LV2UI_Resize, LV2UI_Show_Interface, LV2UI_Widget, LV2UI_Write_Function, LV2_Feature,
    LV2_INSTANCE_ACCESS_URI, LV2_UI_IDLE_INTERFACE_URI, LV2_UI_PARENT_URI, LV2_UI_RESIZE_URI,
    LV2_UI_SHOW_INTERFACE_URI, UI_URI,
};
use crate::ttl::{
    PORT_GLIDE_ENABLED, PORT_PARAMS_START, PORT_REVERB_ENABLED, PORT_REVERB_KILL_TAIL,
};
use crate::{
    sanitize_param_value, snap_oversampling, Lv2Plugin, Lv2SharedState, EDITOR_LOGICAL_HEIGHT,
    EDITOR_LOGICAL_WIDTH,
};
use core::ffi::{c_char, c_int, c_void, CStr};
use prophet_control::port::GLIDE_ENABLED;
use prophet_host::embed::{
    spawn_embedded_editor, EditorViewConfig, EmbeddedEditorHandle, HostBridgeSink, ParentWindow,
};
use prophet_host::preset::{factory_preset, Preset};
use prophet_host::{ui_snapshot_to_json, UiCommand, UiCommandReply, UiSnapshot, WireTailPolicy};
use prophet_params::{ParamId, NUM_PARAMS};
use std::sync::{atomic::Ordering, Arc, Mutex};

#[derive(Clone, Copy)]
struct SendController(LV2UI_Controller);

unsafe impl Send for SendController {}
unsafe impl Sync for SendController {}

struct Lv2HostBridge {
    shared: Arc<Lv2SharedState>,
    write_function: LV2UI_Write_Function,
    controller: SendController,
    write_guard: Mutex<()>,
    host_resize: Option<LV2UI_Resize>,
}

impl Lv2HostBridge {
    fn write_port_float(&self, port_index: u32, val: f32) {
        if let Some(wf) = self.write_function {
            let _g = self.write_guard.lock().unwrap_or_else(|e| e.into_inner());
            unsafe {
                wf(
                    self.controller.0,
                    port_index,
                    core::mem::size_of::<f32>() as u32,
                    0,
                    (&val as *const f32).cast::<c_void>(),
                );
            }
        }
    }

    fn sync_sound_to_host_ports(&self, sound: &prophet_control::state::SoundState) {
        self.shared
            .mirror_rev_enabled
            .store(sound.reverb_enabled, Ordering::Release);
        self.shared
            .rt_override_rev_enabled
            .store(false, Ordering::Release);
        self.write_port_float(PORT_REVERB_ENABLED, if sound.reverb_enabled { 1.0 } else { 0.0 });

        self.shared
            .mirror_rev_kill
            .store(sound.kill_tail, Ordering::Release);
        self.shared
            .rt_override_rev_kill
            .store(false, Ordering::Release);
        self.write_port_float(PORT_REVERB_KILL_TAIL, if sound.kill_tail { 1.0 } else { 0.0 });

        for (idx, &raw) in sound.values.iter().enumerate() {
            let clean = sanitize_param_value(idx, raw);
            self.shared.mirror_params[idx].store(clean.to_bits(), Ordering::Release);
            self.shared.rt_override_params[idx].store(false, Ordering::Release);
            self.write_port_float(PORT_PARAMS_START + idx as u32, clean);
        }

        self.shared
            .mirror_glide
            .store(sound.glide, Ordering::Release);
        self.shared.rt_override_glide.store(false, Ordering::Release);
        self.write_port_float(PORT_GLIDE_ENABLED, if sound.glide { 1.0 } else { 0.0 });
    }

    fn apply_zoom(&self, zoom_pct: u32) {
        let clamped = zoom_pct.clamp(75, 200);
        self.shared.zoom_percent.store(clamped, Ordering::Release);
        if let Some(ref hr) = self.host_resize {
            if let Some(resize_fn) = hr.ui_resize {
                let scale = clamped as f32 / 100.0;
                let w = ((EDITOR_LOGICAL_WIDTH as f32) * scale).round() as c_int;
                let h = ((EDITOR_LOGICAL_HEIGHT as f32) * scale).round() as c_int;
                unsafe {
                    let _ = resize_fn(hr.handle, w, h);
                }
            }
        }
    }
}

impl HostBridgeSink for Lv2HostBridge {
    fn snapshot_json(&self) -> String {
        let mut ctrl = self.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        let snap = ctrl.snapshot();
        ui_snapshot_to_json(&snap)
    }

    fn capture_ui_state(&self) -> UiSnapshot {
        let mut ctrl = self.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.snapshot()
    }

    fn dispatch_command(&self, cmd: UiCommand) -> UiCommandReply {
        if let UiCommand::HostControl {
            op,
            buffer_frames,
            ..
        } = &cmd
        {
            if op == "setUiZoom" {
                if let Some(zp) = buffer_frames {
                    self.apply_zoom(*zp);
                }
                return UiCommandReply::ok("UI zoom updated");
            }
        }

        let (reply, sound_snapshot) = {
            let mut ctrl = self.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
            let r = ctrl.dispatch(cmd.clone());
            let sound = ctrl.sound.clone();
            (r, sound)
        };

        if reply.ok {
            match cmd {
                UiCommand::Param { id, value } => {
                    if id == GLIDE_ENABLED {
                        let b = value >= 0.5;
                        self.shared.mirror_glide.store(b, Ordering::Release);
                        self.shared.rt_override_glide.store(false, Ordering::Release);
                        self.write_port_float(PORT_GLIDE_ENABLED, if b { 1.0 } else { 0.0 });
                    } else if usize::from(id) < NUM_PARAMS {
                        let idx = usize::from(id);
                        let clean = sanitize_param_value(idx, value);
                        self.shared.mirror_params[idx].store(clean.to_bits(), Ordering::Release);
                        self.shared.rt_override_params[idx].store(false, Ordering::Release);
                        self.write_port_float(PORT_PARAMS_START + idx as u32, clean);
                    }
                }
                UiCommand::SetOversampling { factor } => {
                    let snapped_u8 = snap_oversampling(f32::from(factor));
                    let snapped = f32::from(snapped_u8);
                    let os_idx = ParamId::OversamplingMode as usize;
                    self.shared.desired_os.store(snapped_u8, Ordering::Release);
                    self.shared.mirror_params[os_idx].store(snapped.to_bits(), Ordering::Release);
                    self.shared.rt_override_params[os_idx].store(false, Ordering::Release);
                    self.write_port_float(PORT_PARAMS_START + os_idx as u32, snapped);
                    self.shared
                        .pending_bg_reconfig
                        .store(true, Ordering::Release);
                }
                UiCommand::SetReverb { enabled, tail, .. } => {
                    if let Some(en) = enabled {
                        self.shared.desired_rev_enabled.store(en, Ordering::Release);
                        self.shared.mirror_rev_enabled.store(en, Ordering::Release);
                        self.shared
                            .rt_override_rev_enabled
                            .store(false, Ordering::Release);
                        self.write_port_float(PORT_REVERB_ENABLED, if en { 1.0 } else { 0.0 });
                    }
                    if let Some(tp) = tail {
                        let kill = tp == WireTailPolicy::Kill;
                        self.shared.desired_rev_kill.store(kill, Ordering::Release);
                        self.shared.mirror_rev_kill.store(kill, Ordering::Release);
                        self.shared
                            .rt_override_rev_kill
                            .store(false, Ordering::Release);
                        self.write_port_float(PORT_REVERB_KILL_TAIL, if kill { 1.0 } else { 0.0 });
                    }
                    self.shared
                        .pending_bg_reconfig
                        .store(true, Ordering::Release);
                }
                UiCommand::FactoryPreset { index } => {
                    if let Some(fp) = factory_preset(index) {
                        *self
                            .shared
                            .preset_name
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = fp.name;
                        *self
                            .shared
                            .preset_category
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = fp.category;
                    }
                    self.shared.desired_os.store(
                        snap_oversampling(sound_snapshot.values[ParamId::OversamplingMode as usize]),
                        Ordering::Release,
                    );
                    self.shared
                        .desired_rev_enabled
                        .store(sound_snapshot.reverb_enabled, Ordering::Release);
                    self.shared
                        .desired_rev_kill
                        .store(sound_snapshot.kill_tail, Ordering::Release);
                    self.sync_sound_to_host_ports(&sound_snapshot);
                    self.shared
                        .pending_bg_reconfig
                        .store(true, Ordering::Release);
                }
                UiCommand::PresetLoadJson { json } => {
                    if let Ok(decoded) = Preset::decode(json.as_bytes()) {
                        *self
                            .shared
                            .preset_name
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = decoded.name;
                        *self
                            .shared
                            .preset_category
                            .lock()
                            .unwrap_or_else(|e| e.into_inner()) = decoded.category;
                    }
                    self.shared.desired_os.store(
                        snap_oversampling(sound_snapshot.values[ParamId::OversamplingMode as usize]),
                        Ordering::Release,
                    );
                    self.shared
                        .desired_rev_enabled
                        .store(sound_snapshot.reverb_enabled, Ordering::Release);
                    self.shared
                        .desired_rev_kill
                        .store(sound_snapshot.kill_tail, Ordering::Release);
                    self.sync_sound_to_host_ports(&sound_snapshot);
                    self.shared
                        .pending_bg_reconfig
                        .store(true, Ordering::Release);
                }
                _ => {}
            }
        }
        reply
    }

    fn on_zoom_changed(&self, zoom: f32) {
        let zp = (zoom * 100.0).round().clamp(75.0, 200.0) as u32;
        self.apply_zoom(zp);
    }
}

pub struct Lv2UiInstance {
    shared: Arc<Lv2SharedState>,
    handle: Option<EmbeddedEditorHandle>,
    visible: bool,
}

unsafe extern "C" fn ui_instantiate(
    _descriptor: *const LV2UI_Descriptor,
    _plugin_uri: *const c_char,
    _bundle_path: *const c_char,
    write_function: LV2UI_Write_Function,
    controller: LV2UI_Controller,
    widget: *mut LV2UI_Widget,
    features: *const *const LV2_Feature,
) -> LV2UI_Handle {
    let mut parent_xid: u32 = 0;
    let mut shared_opt: Option<Arc<Lv2SharedState>> = None;
    let mut host_resize: Option<LV2UI_Resize> = None;

    if !features.is_null() {
        let mut cur = features;
        while !(*cur).is_null() {
            let feat = &*(*cur);
            if !feat.URI.is_null() {
                let uri = CStr::from_ptr(feat.URI);
                if uri == LV2_UI_PARENT_URI {
                    parent_xid = feat.data as usize as u32;
                } else if uri == LV2_INSTANCE_ACCESS_URI && !feat.data.is_null() {
                    let plugin = &*(feat.data as *const Lv2Plugin);
                    shared_opt = Some(plugin.shared_state());
                } else if uri == LV2_UI_RESIZE_URI && !feat.data.is_null() {
                    host_resize = Some(*(feat.data as *const LV2UI_Resize));
                }
            }
            cur = cur.add(1);
        }
    }

    let shared = shared_opt.unwrap_or_else(Lv2SharedState::new_standalone_ui_fallback);
    let zp = shared.zoom_percent.load(Ordering::Relaxed).clamp(75, 200) as f32 / 100.0;
    let width = ((EDITOR_LOGICAL_WIDTH as f32) * zp).round() as u32;
    let height = ((EDITOR_LOGICAL_HEIGHT as f32) * zp).round() as u32;

    let sink: Arc<dyn HostBridgeSink> = Arc::new(Lv2HostBridge {
        shared: shared.clone(),
        write_function,
        controller: SendController(controller),
        write_guard: Mutex::new(()),
        host_resize,
    });

    let cfg = EditorViewConfig {
        title: "PROPHET-XII Virtual Analog Synthesizer".to_owned(),
        backend_tag: "lv2".to_owned(),
        logical_width: width,
        logical_height: height,
        scale_factor: 1.0,
        resizable: true,
    };

    let Ok(editor) = spawn_embedded_editor(ParentWindow::X11(parent_xid), cfg, sink) else {
        return core::ptr::null_mut();
    };

    if !widget.is_null() {
        let child_xid = editor.child_x11_window().unwrap_or(0);
        *widget = (child_xid as usize) as *mut c_void;
    }

    if let Some(ref hr) = host_resize {
        if let Some(resize_fn) = hr.ui_resize {
            let _ = resize_fn(hr.handle, width as c_int, height as c_int);
        }
    }

    let ui = Box::new(Lv2UiInstance {
        shared,
        handle: Some(editor),
        visible: true,
    });
    Box::into_raw(ui).cast::<c_void>()
}

unsafe extern "C" fn ui_cleanup(ui: LV2UI_Handle) {
    if !ui.is_null() {
        drop(Box::from_raw(ui.cast::<Lv2UiInstance>()));
    }
}

unsafe extern "C" fn ui_port_event(
    ui: LV2UI_Handle,
    port_index: u32,
    buffer_size: u32,
    format: u32,
    buffer: *const c_void,
) {
    if ui.is_null() || format != 0 || buffer_size < core::mem::size_of::<f32>() as u32 || buffer.is_null() {
        return;
    }
    let inst = &*(ui as *const Lv2UiInstance);
    let raw = *(buffer as *const f32);
    if !raw.is_finite() {
        return;
    }

    let mut ctrl = inst.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
    let mut changed = false;

    if port_index == PORT_REVERB_ENABLED {
        let en = raw >= 0.5;
        if ctrl.sound.reverb_enabled != en {
            ctrl.sound.reverb_enabled = en;
            changed = true;
        }
        inst.shared.mirror_rev_enabled.store(en, Ordering::Release);
    } else if port_index == PORT_REVERB_KILL_TAIL {
        let kill = raw >= 0.5;
        if ctrl.sound.kill_tail != kill {
            ctrl.sound.kill_tail = kill;
            changed = true;
        }
        inst.shared.mirror_rev_kill.store(kill, Ordering::Release);
    } else if port_index == PORT_GLIDE_ENABLED {
        let g = raw >= 0.5;
        if ctrl.sound.glide != g {
            ctrl.sound.glide = g;
            changed = true;
        }
        inst.shared.mirror_glide.store(g, Ordering::Release);
    } else if port_index >= PORT_PARAMS_START && port_index < PORT_GLIDE_ENABLED {
        let idx = (port_index - PORT_PARAMS_START) as usize;
        if idx < NUM_PARAMS {
            let clean = sanitize_param_value(idx, raw);
            if ctrl.sound.values[idx].to_bits() != clean.to_bits() {
                ctrl.sound.values[idx] = clean;
                changed = true;
            }
            inst.shared.mirror_params[idx].store(clean.to_bits(), Ordering::Release);
        }
    }

    if changed {
        ctrl.sync_pod_from_sound();
    }
}

unsafe extern "C" fn ui_idle(ui: LV2UI_Handle) -> c_int {
    if ui.is_null() {
        return 1;
    }
    let inst = &*(ui as *const Lv2UiInstance);
    match &inst.handle {
        Some(h) if h.is_open() => 0,
        _ => 1,
    }
}

unsafe extern "C" fn ui_show(ui: LV2UI_Handle) -> c_int {
    if ui.is_null() {
        return 1;
    }
    let inst = &mut *(ui as *mut Lv2UiInstance);
    inst.visible = true;
    match &inst.handle {
        Some(h) if h.is_open() => 0,
        _ => 1,
    }
}

unsafe extern "C" fn ui_hide(ui: LV2UI_Handle) -> c_int {
    if ui.is_null() {
        return 1;
    }
    let inst = &mut *(ui as *mut Lv2UiInstance);
    inst.visible = false;
    0
}

unsafe extern "C" fn ui_resize_cb(
    handle: LV2UI_Feature_Handle,
    width: c_int,
    height: c_int,
) -> c_int {
    if handle.is_null() || width <= 0 || height <= 0 {
        return 1;
    }
    let inst = &*(handle as *const Lv2UiInstance);
    if let Some(ref editor) = inst.handle {
        let zp = (((width as f32) / (EDITOR_LOGICAL_WIDTH as f32)) * 100.0)
            .round()
            .clamp(75.0, 200.0) as u32;
        inst.shared.zoom_percent.store(zp, Ordering::Release);
        editor.resize(width as u32, height as u32, 1.0);
        0
    } else {
        1
    }
}

static IDLE_INTERFACE: LV2UI_Idle_Interface = LV2UI_Idle_Interface {
    idle: Some(ui_idle),
};

static SHOW_INTERFACE: LV2UI_Show_Interface = LV2UI_Show_Interface {
    show: Some(ui_show),
    hide: Some(ui_hide),
};

static RESIZE_INTERFACE: LV2UI_Resize = LV2UI_Resize {
    handle: core::ptr::null_mut(),
    ui_resize: Some(ui_resize_cb),
};

unsafe extern "C" fn ui_extension_data(uri: *const c_char) -> *const c_void {
    if uri.is_null() {
        return core::ptr::null();
    }
    let s = CStr::from_ptr(uri);
    if s == LV2_UI_IDLE_INTERFACE_URI {
        (&IDLE_INTERFACE as *const LV2UI_Idle_Interface).cast::<c_void>()
    } else if s == LV2_UI_SHOW_INTERFACE_URI {
        (&SHOW_INTERFACE as *const LV2UI_Show_Interface).cast::<c_void>()
    } else if s == LV2_UI_RESIZE_URI {
        (&RESIZE_INTERFACE as *const LV2UI_Resize).cast::<c_void>()
    } else {
        core::ptr::null()
    }
}

static UI_DESCRIPTOR: LV2UI_Descriptor = LV2UI_Descriptor {
    URI: UI_URI.as_ptr(),
    instantiate: Some(ui_instantiate),
    cleanup: Some(ui_cleanup),
    port_event: Some(ui_port_event),
    extension_data: Some(ui_extension_data),
};

#[no_mangle]
pub unsafe extern "C" fn lv2ui_descriptor(index: u32) -> *const LV2UI_Descriptor {
    if index == 0 {
        &UI_DESCRIPTOR
    } else {
        core::ptr::null()
    }
}
