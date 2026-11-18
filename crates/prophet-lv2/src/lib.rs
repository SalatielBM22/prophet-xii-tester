// SPDX-License-Identifier: GPL-3.0-only
//! PROPHET-XII LV2 Plugin + `state:interface` + `work:interface` + `opts:interface` + `ui:X11UI`
//! backed by `prophet-host` (`RtHostSlot` + `HostControlPlane` + `prophet_host::embed`).
//!
//! Real-time audio callbacks (`run()`) are strictly lock-free, allocation-free, and never drop
//! `Box<PreparedEngine>` / `Box<PatternBank>` instances.

pub mod lv2_ffi;
pub mod ttl;
pub mod ui;

use core::ffi::{c_char, c_double, c_void, CStr};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use lv2_ffi::{
    atom_pad_size, LV2_Atom, LV2_Atom_Event, LV2_Atom_Object_Body, LV2_Atom_Property_Body,
    LV2_Atom_Sequence, LV2_Atom_Sequence_Body, LV2_Descriptor, LV2_Feature, LV2_Handle,
    LV2_Options_Interface, LV2_Options_Option, LV2_State_Handle, LV2_State_Interface,
    LV2_State_Retrieve_Function, LV2_State_Status, LV2_State_Store_Function, LV2_URID,
    LV2_URID_Map, LV2_Worker_Interface, LV2_Worker_Respond_Function, LV2_Worker_Respond_Handle,
    LV2_Worker_Schedule, LV2_Worker_Status, LV2_ATOM_BLANK_URI, LV2_ATOM_DOUBLE_URI,
    LV2_ATOM_FLOAT_URI, LV2_ATOM_INT_URI, LV2_ATOM_OBJECT_URI, LV2_ATOM_SEQUENCE_URI,
    LV2_ATOM_STRING_URI, LV2_BUF_SIZE_MAX_BLOCK_LENGTH_URI, LV2_MIDI_EVENT_URI,
    LV2_OPTIONS_ERR_BAD_KEY, LV2_OPTIONS_ERR_BAD_VALUE, LV2_OPTIONS_INTERFACE_URI,
    LV2_OPTIONS_OPTIONS_URI, LV2_OPTIONS_SUCCESS, LV2_STATE_ERR_UNKNOWN, LV2_STATE_INTERFACE_URI,
    LV2_STATE_IS_POD, LV2_STATE_IS_PORTABLE, LV2_STATE_SUCCESS, LV2_TIME_BEATS_PER_MINUTE_URI,
    LV2_TIME_POSITION_URI, LV2_URID_MAP_URI, LV2_WORKER_INTERFACE_URI, LV2_WORKER_SCHEDULE_URI,
    LV2_WORKER_SUCCESS, PLUGIN_URI, STATE_PRESET_V2_URI,
};
use prophet_control::{
    music::{Order, PatternBank},
    port::{Action, Command, Prepare, GLIDE_ENABLED},
    state::SoundState,
};
use prophet_dsp::{
    fx::{reverb::Algorithm, solaris_reverb::ReverbTailPolicy},
    vector::Patch,
};
use prophet_host::{
    create_host_session,
    preset::{decode_and_prepare, Preset},
    HostControlPlane, RtHostSlot, SerialIssuer, UiCommand, WireTailPolicy, ORIGIN_DAW_HOST,
};
use prophet_params::{AtomicParamBank, ParamId, NUM_PARAMS, PARAM_SPECS};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use ttl::{
    PORT_CONTROL_IN, PORT_GLIDE_ENABLED, PORT_LATENCY, PORT_NOTIFY_OUT, PORT_OUT_L, PORT_OUT_R,
    PORT_PARAMS_START, PORT_REVERB_ENABLED, PORT_REVERB_KILL_TAIL, TOTAL_PORTS,
};

pub(crate) const EDITOR_LOGICAL_WIDTH: u32 = 1180;
pub(crate) const EDITOR_LOGICAL_HEIGHT: u32 = 720;
const MAX_RT_CMDS: usize = 256;
const DEFAULT_MAX_FRAMES: u16 = 1024;

#[inline]
pub(crate) fn snap_oversampling(raw: f32) -> u8 {
    let r = raw.round() as i32;
    if r <= 1 {
        1
    } else if r == 2 {
        2
    } else {
        4
    }
}

#[inline]
pub(crate) fn sanitize_param_value(idx: usize, val: f32) -> f32 {
    let spec = &PARAM_SPECS[idx];
    if spec.id == ParamId::OversamplingMode {
        return f32::from(snap_oversampling(val));
    }
    let finite = if val.is_finite() { val } else { spec.min };
    let clamped = finite.clamp(spec.min, spec.max);
    if spec.integer {
        clamped.round()
    } else {
        clamped
    }
}

pub(crate) fn default_raw_values() -> [f32; NUM_PARAMS] {
    let bank = AtomicParamBank::new();
    core::array::from_fn(|i| bank.get(ParamId::from_index(i).unwrap()))
}

fn is_default_pattern_bank(bank: &PatternBank) -> bool {
    let default_pat = prophet_control::music::Pattern::default();
    bank.0.iter().all(|p| *p == default_pat)
}

#[derive(Clone, Copy, Default)]
struct Lv2Urids {
    atom_sequence: LV2_URID,
    atom_object: LV2_URID,
    atom_blank: LV2_URID,
    atom_float: LV2_URID,
    atom_double: LV2_URID,
    atom_int: LV2_URID,
    atom_string: LV2_URID,
    midi_event: LV2_URID,
    time_position: LV2_URID,
    time_beats_per_minute: LV2_URID,
    bufsz_max_block_length: LV2_URID,
    state_preset_v2: LV2_URID,
}

impl Lv2Urids {
    unsafe fn from_map(map: &LV2_URID_Map) -> Option<Self> {
        let map_fn = map.map?;
        let m = |uri: &core::ffi::CStr| -> LV2_URID { map_fn(map.handle, uri.as_ptr()) };
        let urids = Self {
            atom_sequence: m(LV2_ATOM_SEQUENCE_URI),
            atom_object: m(LV2_ATOM_OBJECT_URI),
            atom_blank: m(LV2_ATOM_BLANK_URI),
            atom_float: m(LV2_ATOM_FLOAT_URI),
            atom_double: m(LV2_ATOM_DOUBLE_URI),
            atom_int: m(LV2_ATOM_INT_URI),
            atom_string: m(LV2_ATOM_STRING_URI),
            midi_event: m(LV2_MIDI_EVENT_URI),
            time_position: m(LV2_TIME_POSITION_URI),
            time_beats_per_minute: m(LV2_TIME_BEATS_PER_MINUTE_URI),
            bufsz_max_block_length: m(LV2_BUF_SIZE_MAX_BLOCK_LENGTH_URI),
            state_preset_v2: m(STATE_PRESET_V2_URI),
        };
        if urids.atom_sequence == 0 || urids.state_preset_v2 == 0 {
            return None;
        }
        Some(urids)
    }
}

pub(crate) struct Lv2SharedState {
    pub(crate) ctrl: Arc<Mutex<HostControlPlane>>,
    pub(crate) mirror_params: [AtomicU32; NUM_PARAMS],
    pub(crate) mirror_glide: AtomicBool,
    pub(crate) mirror_rev_enabled: AtomicBool,
    pub(crate) mirror_rev_kill: AtomicBool,
    pub(crate) rt_override_params: [AtomicBool; NUM_PARAMS],
    pub(crate) rt_override_glide: AtomicBool,
    pub(crate) rt_override_rev_enabled: AtomicBool,
    pub(crate) rt_override_rev_kill: AtomicBool,
    pub(crate) desired_os: AtomicU8,
    pub(crate) desired_rev_enabled: AtomicBool,
    pub(crate) desired_rev_kill: AtomicBool,
    pub(crate) pending_bg_reconfig: AtomicBool,
    pub(crate) work_scheduled: AtomicBool,
    pub(crate) zoom_percent: AtomicU32,
    pub(crate) preset_name: Mutex<String>,
    pub(crate) preset_category: Mutex<String>,
}

impl Lv2SharedState {
    fn from_ctrl(ctrl: Arc<Mutex<HostControlPlane>>, sound: &SoundState) -> Arc<Self> {
        let mirror_params = core::array::from_fn(|i| AtomicU32::new(sound.values[i].to_bits()));
        let rt_override_params = core::array::from_fn(|_| AtomicBool::new(false));
        let os = snap_oversampling(sound.values[ParamId::OversamplingMode as usize]);
        Arc::new(Self {
            ctrl,
            mirror_params,
            mirror_glide: AtomicBool::new(sound.glide),
            mirror_rev_enabled: AtomicBool::new(sound.reverb_enabled),
            mirror_rev_kill: AtomicBool::new(sound.kill_tail),
            rt_override_params,
            rt_override_glide: AtomicBool::new(false),
            rt_override_rev_enabled: AtomicBool::new(false),
            rt_override_rev_kill: AtomicBool::new(false),
            desired_os: AtomicU8::new(os),
            desired_rev_enabled: AtomicBool::new(sound.reverb_enabled),
            desired_rev_kill: AtomicBool::new(sound.kill_tail),
            pending_bg_reconfig: AtomicBool::new(false),
            work_scheduled: AtomicBool::new(false),
            zoom_percent: AtomicU32::new(100),
            preset_name: Mutex::new("Init".to_owned()),
            preset_category: Mutex::new("Init".to_owned()),
        })
    }

    pub(crate) fn new_standalone_ui_fallback() -> Arc<Self> {
        let (_, ctrl) = create_host_session(
            "lv2-ui-fallback",
            vec![
                "enginePortV1",
                "glideEnabled",
                "reverbState",
                "selectIdleReverb",
                "oscillatorPatch",
                "noteScheduler",
                "vectorXY",
            ],
            Prepare {
                sample_rate: 48_000,
                max_frames: DEFAULT_MAX_FRAMES,
                oversampling: 2,
            },
            true,
        )
        .expect("default PROPHET-XII UI fallback session must be valid");
        let sound = ctrl.sound.clone();
        Self::from_ctrl(Arc::new(Mutex::new(ctrl)), &sound)
    }

    fn sync_mirrors_from_sound(&self, sound: &SoundState) {
        for (i, &val) in sound.values.iter().enumerate() {
            let clean = sanitize_param_value(i, val);
            self.mirror_params[i].store(clean.to_bits(), Ordering::Release);
            self.rt_override_params[i].store(false, Ordering::Release);
        }
        self.mirror_glide.store(sound.glide, Ordering::Release);
        self.rt_override_glide.store(false, Ordering::Release);
        self.mirror_rev_enabled
            .store(sound.reverb_enabled, Ordering::Release);
        self.rt_override_rev_enabled
            .store(false, Ordering::Release);
        self.mirror_rev_kill
            .store(sound.kill_tail, Ordering::Release);
        self.rt_override_rev_kill.store(false, Ordering::Release);
        let os = snap_oversampling(sound.values[ParamId::OversamplingMode as usize]);
        self.desired_os.store(os, Ordering::Release);
        self.desired_rev_enabled
            .store(sound.reverb_enabled, Ordering::Release);
        self.desired_rev_kill
            .store(sound.kill_tail, Ordering::Release);
    }

    fn run_worker_job(&self) {
        let target_os = snap_oversampling(f32::from(self.desired_os.load(Ordering::Acquire)));
        let rev_enabled = self.desired_rev_enabled.load(Ordering::Acquire);
        let rev_kill = self.desired_rev_kill.load(Ordering::Acquire);

        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.poll();

        let current_os = snap_oversampling(ctrl.sound.values[ParamId::OversamplingMode as usize]);
        let sr = ctrl.sample_rate();
        let mf = ctrl.max_frames();

        if current_os != target_os {
            let mut next = ctrl.sound.clone();
            next.values[ParamId::OversamplingMode as usize] = f32::from(target_os);
            next.reverb_enabled = rev_enabled;
            next.kill_tail = rev_kill;
            if ctrl.worker.prepare_reconfig(&next, sr, mf).is_ok() {
                ctrl.sound = next;
                ctrl.sync_pod_from_sound();
            }
        } else if ctrl.sound.reverb_enabled != rev_enabled || ctrl.sound.kill_tail != rev_kill {
            let _ = ctrl.dispatch(UiCommand::SetReverb {
                algorithm: None,
                enabled: Some(rev_enabled),
                tail: Some(if rev_kill {
                    WireTailPolicy::Kill
                } else {
                    WireTailPolicy::Spill
                }),
            });
        }

        ctrl.poll();
        let still_pending = !ctrl.is_voices_idle()
            && (ctrl.sound.reverb_enabled != rev_enabled || ctrl.sound.kill_tail != rev_kill);
        self.pending_bg_reconfig
            .store(still_pending, Ordering::Release);
        self.work_scheduled.store(false, Ordering::Release);
    }
}

struct FallbackWorker {
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for FallbackWorker {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(h) = self.handle.take() {
            h.thread().unpark();
            let _ = h.join();
        }
    }
}

#[derive(Clone, Copy)]
struct StagedMidiCmd {
    abs_frame: usize,
    cmd: Command,
}

impl Default for StagedMidiCmd {
    fn default() -> Self {
        Self {
            abs_frame: 0,
            cmd: Command {
                offset: 0,
                origin: ORIGIN_DAW_HOST,
                serial: 0,
                action: Action::PanicOrigin,
            },
        }
    }
}

pub struct Lv2Plugin {
    sample_rate: u32,
    max_frames: u16,
    urids: Lv2Urids,
    ports: [*mut c_void; TOTAL_PORTS],
    rt_slot: RtHostSlot,
    shared: Arc<Lv2SharedState>,
    worker_schedule: Option<LV2_Worker_Schedule>,
    fallback_worker: Option<FallbackWorker>,
    daw_serials: SerialIssuer,
    last_host: [f32; NUM_PARAMS],
    last_glide: bool,
    last_host_rev_enabled: bool,
    last_host_rev_kill: bool,
    requested_os: u8,
    requested_rev_enabled: bool,
    requested_rev_kill: bool,
    last_swap_id: u64,
    last_latency: u32,
    just_restored: bool,
    frames_since_bg_poll: u64,
}

impl Lv2Plugin {
    pub(crate) fn shared_state(&self) -> Arc<Lv2SharedState> {
        self.shared.clone()
    }

    fn schedule_background_work(&self) {
        if self
            .shared
            .work_scheduled
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        if let Some(sched) = self.worker_schedule {
            if let Some(f) = sched.schedule_work {
                let req: u8 = 1;
                let st = unsafe {
                    f(
                        sched.handle,
                        1,
                        (&req as *const u8).cast::<c_void>(),
                    )
                };
                if st == LV2_WORKER_SUCCESS {
                    return;
                }
            }
        }
        if let Some(ref fw) = self.fallback_worker {
            if let Some(ref h) = fw.handle {
                h.thread().unpark();
                return;
            }
        }
        self.shared.work_scheduled.store(false, Ordering::Release);
    }

    #[inline]
    unsafe fn read_control_port(&self, port_idx: u32) -> Option<f32> {
        let ptr = *self.ports.get(port_idx as usize)?;
        if ptr.is_null() {
            None
        } else {
            let v = *(ptr as *const f32);
            v.is_finite().then_some(v)
        }
    }

    #[inline]
    fn stage_cmd(&mut self, buf: &mut [StagedMidiCmd; MAX_RT_CMDS], len: &mut usize, abs_frame: usize, action: Action) {
        if *len < MAX_RT_CMDS {
            buf[*len] = StagedMidiCmd {
                abs_frame,
                cmd: Command {
                    offset: 0,
                    origin: ORIGIN_DAW_HOST,
                    serial: self.daw_serials.next_serial(ORIGIN_DAW_HOST),
                    action,
                },
            };
            *len += 1;
        }
    }

    fn snapshot_ports_into_baseline(&mut self) {
        for idx in 0..NUM_PARAMS {
            let port_idx = PORT_PARAMS_START + idx as u32;
            if let Some(raw) = unsafe { self.read_control_port(port_idx) } {
                self.last_host[idx] = sanitize_param_value(idx, raw);
            }
        }
        if let Some(raw) = unsafe { self.read_control_port(PORT_GLIDE_ENABLED) } {
            self.last_glide = raw >= 0.5;
        }
        if let Some(raw) = unsafe { self.read_control_port(PORT_REVERB_ENABLED) } {
            self.last_host_rev_enabled = raw >= 0.5;
        }
        if let Some(raw) = unsafe { self.read_control_port(PORT_REVERB_KILL_TAIL) } {
            self.last_host_rev_kill = raw >= 0.5;
        }
    }

    fn parse_control_in_atoms(
        &mut self,
        num_frames: usize,
        staged: &mut [StagedMidiCmd; MAX_RT_CMDS],
        staged_len: &mut usize,
    ) {
        let seq_ptr = self.ports[PORT_CONTROL_IN as usize] as *const LV2_Atom_Sequence;
        if seq_ptr.is_null() || self.urids.atom_sequence == 0 {
            return;
        }
        let seq = unsafe { ptr::read_unaligned(seq_ptr) };
        if seq.atom.atom_type != self.urids.atom_sequence {
            return;
        }
        let seq_body_len = (seq.atom.size as usize)
            .saturating_sub(core::mem::size_of::<LV2_Atom_Sequence_Body>());
        if seq_body_len < core::mem::size_of::<LV2_Atom_Event>() {
            return;
        }
        let base = unsafe {
            (seq_ptr as *const u8).add(core::mem::size_of::<LV2_Atom_Sequence>())
        };
        let mut off = 0usize;
        while off + core::mem::size_of::<LV2_Atom_Event>() <= seq_body_len {
            let ev = unsafe { ptr::read_unaligned(base.add(off) as *const LV2_Atom_Event) };
            let body_size = ev.body.size as usize;
            let payload_off = off + core::mem::size_of::<LV2_Atom_Event>();
            if payload_off + body_size > seq_body_len {
                break;
            }
            let payload = unsafe { base.add(payload_off) };
            let frame = if unsafe { ev.time.frames } <= 0 {
                0usize
            } else {
                (unsafe { ev.time.frames } as usize).min(num_frames)
            };

            if ev.body.atom_type == self.urids.midi_event && body_size > 0 {
                let msg = unsafe { core::slice::from_raw_parts(payload, body_size) };
                let status = msg[0];
                if status == 0xff {
                    self.stage_cmd(staged, staged_len, frame, Action::PanicOrigin);
                } else if (0x80..=0xef).contains(&status) {
                    let kind = status & 0xf0;
                    let ch = status & 0x0f;
                    let d1 = *msg.get(1).unwrap_or(&0) & 0x7f;
                    let d2 = *msg.get(2).unwrap_or(&0) & 0x7f;
                    match kind {
                        0x80 | 0x90 => {
                            self.stage_cmd(
                                staged,
                                staged_len,
                                frame,
                                Action::Midi {
                                    status,
                                    data1: d1,
                                    data2: d2,
                                    note_id: u32::from(d1),
                                },
                            );
                        }
                        0xa0 => {
                            // Map poly key pressure onto channel pressure (CCR-04 EnginePort v1 compatibility).
                            self.stage_cmd(
                                staged,
                                staged_len,
                                frame,
                                Action::Midi {
                                    status: 0xd0 | ch,
                                    data1: d2,
                                    data2: 0,
                                    note_id: u32::from(d1),
                                },
                            );
                        }
                        0xb0 | 0xd0 | 0xe0 => {
                            self.stage_cmd(
                                staged,
                                staged_len,
                                frame,
                                Action::Midi {
                                    status,
                                    data1: d1,
                                    data2: d2,
                                    note_id: 0,
                                },
                            );
                        }
                        _ => {}
                    }
                }
            } else if (ev.body.atom_type == self.urids.atom_object
                || ev.body.atom_type == self.urids.atom_blank)
                && body_size >= core::mem::size_of::<LV2_Atom_Object_Body>()
            {
                let obj_body =
                    unsafe { ptr::read_unaligned(payload as *const LV2_Atom_Object_Body) };
                if obj_body.otype == self.urids.time_position && self.urids.time_position != 0 {
                    let mut prop_off = core::mem::size_of::<LV2_Atom_Object_Body>();
                    while prop_off + core::mem::size_of::<LV2_Atom_Property_Body>() <= body_size {
                        let prop = unsafe {
                            ptr::read_unaligned(
                                payload.add(prop_off) as *const LV2_Atom_Property_Body
                            )
                        };
                        let val_size = prop.value.size as usize;
                        let val_off = prop_off + core::mem::size_of::<LV2_Atom_Property_Body>();
                        if val_off + val_size > body_size {
                            break;
                        }
                        if prop.key == self.urids.time_beats_per_minute
                            && self.urids.time_beats_per_minute != 0
                        {
                            let bpm_opt = if prop.value.atom_type == self.urids.atom_float
                                && val_size >= 4
                            {
                                Some(unsafe {
                                    f64::from(ptr::read_unaligned(payload.add(val_off) as *const f32))
                                })
                            } else if prop.value.atom_type == self.urids.atom_double && val_size >= 8
                            {
                                Some(unsafe {
                                    ptr::read_unaligned(payload.add(val_off) as *const f64)
                                })
                            } else {
                                None
                            };
                            if let Some(bpm) = bpm_opt {
                                if bpm.is_finite() && bpm > 0.0 {
                                    let tempo_milli =
                                        (bpm.clamp(20.0, 400.0) * 1000.0).round() as u32;
                                    if let Some(m) = self.rt_slot.slot.music.as_mut() {
                                        let _ = m.set_tempo(tempo_milli);
                                    }
                                }
                            }
                        }
                        prop_off = val_off + atom_pad_size(prop.value.size);
                    }
                }
            }

            off = payload_off + atom_pad_size(ev.body.size);
        }
    }

    fn init_empty_notify_out(&self) {
        let notify_ptr = self.ports[PORT_NOTIFY_OUT as usize] as *mut LV2_Atom_Sequence;
        if notify_ptr.is_null() || self.urids.atom_sequence == 0 {
            return;
        }
        let empty = LV2_Atom_Sequence {
            atom: LV2_Atom {
                size: core::mem::size_of::<LV2_Atom_Sequence_Body>() as u32,
                atom_type: self.urids.atom_sequence,
            },
            body: LV2_Atom_Sequence_Body { unit: 0, pad: 0 },
        };
        unsafe {
            ptr::write_unaligned(notify_ptr, empty);
        }
    }
}

unsafe extern "C" fn instantiate(
    _descriptor: *const LV2_Descriptor,
    sample_rate: c_double,
    _bundle_path: *const c_char,
    features: *const *const LV2_Feature,
) -> LV2_Handle {
    let mut urid_map: Option<&LV2_URID_Map> = None;
    let mut worker_schedule: Option<LV2_Worker_Schedule> = None;
    let mut options_ptr: *const LV2_Options_Option = core::ptr::null();

    if !features.is_null() {
        let mut cur = features;
        while !(*cur).is_null() {
            let feat = &*(*cur);
            if !feat.URI.is_null() {
                let uri = CStr::from_ptr(feat.URI);
                if uri == LV2_URID_MAP_URI && !feat.data.is_null() {
                    urid_map = Some(&*(feat.data as *const LV2_URID_Map));
                } else if uri == LV2_WORKER_SCHEDULE_URI && !feat.data.is_null() {
                    worker_schedule = Some(*(feat.data as *const LV2_Worker_Schedule));
                } else if uri == LV2_OPTIONS_OPTIONS_URI && !feat.data.is_null() {
                    options_ptr = feat.data as *const LV2_Options_Option;
                }
            }
            cur = cur.add(1);
        }
    }

    let Some(map_iface) = urid_map else {
        return core::ptr::null_mut();
    };
    let Some(urids) = Lv2Urids::from_map(map_iface) else {
        return core::ptr::null_mut();
    };

    let sr = (sample_rate.round() as u32).clamp(8_000, 192_000);
    let mut max_frames = DEFAULT_MAX_FRAMES;
    if !options_ptr.is_null() && urids.bufsz_max_block_length != 0 {
        let mut cur = options_ptr;
        loop {
            let opt = *cur;
            if opt.key == 0 && opt.size == 0 && opt.value.is_null() {
                break;
            }
            if opt.key == urids.bufsz_max_block_length
                && opt.option_type == urids.atom_int
                && opt.size >= core::mem::size_of::<i32>() as u32
                && !opt.value.is_null()
            {
                let v = ptr::read_unaligned(opt.value as *const i32);
                if v > 0 {
                    max_frames = (v as u16).clamp(1, DEFAULT_MAX_FRAMES);
                }
            }
            cur = cur.add(1);
        }
    }

    let raw_defaults = default_raw_values();
    let initial_os = snap_oversampling(raw_defaults[ParamId::OversamplingMode as usize]);
    let Ok((rt_slot, mut ctrl)) = create_host_session(
        "lv2",
        vec![
            "enginePortV1",
            "glideEnabled",
            "reverbState",
            "selectIdleReverb",
            "oscillatorPatch",
            "noteScheduler",
            "vectorXY",
        ],
        Prepare {
            sample_rate: sr,
            max_frames,
            oversampling: initial_os,
        },
        true,
    ) else {
        return core::ptr::null_mut();
    };

    ctrl.sync_pod_from_sound();
    let initial_sound = ctrl.sound.clone();
    let last_latency = rt_slot.slot.engine.latency_samples();
    let last_swap_id = rt_slot.slot.active_swap_id();
    let shared = Lv2SharedState::from_ctrl(Arc::new(Mutex::new(ctrl)), &initial_sound);

    let fallback_worker = if worker_schedule.is_none() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let sd_clone = shutdown.clone();
        let sh_clone = shared.clone();
        let handle = std::thread::Builder::new()
            .name("prophet-lv2-worker".to_owned())
            .spawn(move || {
                while !sd_clone.load(Ordering::Acquire) {
                    if sh_clone.work_scheduled.load(Ordering::Acquire) {
                        sh_clone.run_worker_job();
                    } else {
                        std::thread::park();
                    }
                }
            })
            .ok();
        Some(FallbackWorker { shutdown, handle })
    } else {
        None
    };

    let plugin = Box::new(Lv2Plugin {
        sample_rate: sr,
        max_frames,
        urids,
        ports: [core::ptr::null_mut(); TOTAL_PORTS],
        rt_slot,
        shared,
        worker_schedule,
        fallback_worker,
        daw_serials: SerialIssuer::new(),
        last_host: initial_sound.values,
        last_glide: initial_sound.glide,
        last_host_rev_enabled: initial_sound.reverb_enabled,
        last_host_rev_kill: initial_sound.kill_tail,
        requested_os: initial_os,
        requested_rev_enabled: initial_sound.reverb_enabled,
        requested_rev_kill: initial_sound.kill_tail,
        last_swap_id,
        last_latency,
        just_restored: false,
        frames_since_bg_poll: 0,
    });
    Box::into_raw(plugin).cast::<c_void>()
}

unsafe extern "C" fn connect_port(
    instance: LV2_Handle,
    port: u32,
    data_location: *mut c_void,
) {
    if instance.is_null() {
        return;
    }
    let plugin = &mut *(instance as *mut Lv2Plugin);
    if let Some(slot) = plugin.ports.get_mut(port as usize) {
        *slot = data_location;
    }
}

unsafe extern "C" fn activate(instance: LV2_Handle) {
    if instance.is_null() {
        return;
    }
    let plugin = &mut *(instance as *mut Lv2Plugin);
    if let Ok(mut ctrl) = plugin.shared.ctrl.lock() {
        ctrl.set_audio_running(true, Some(plugin.sample_rate), Some(plugin.max_frames));
        ctrl.worker.collect_retired();
    }
}

unsafe extern "C" fn run(instance: LV2_Handle, sample_count: u32) {
    if instance.is_null() {
        return;
    }
    let plugin = &mut *(instance as *mut Lv2Plugin);
    let num_frames = sample_count as usize;
    plugin.init_empty_notify_out();

    // Poll off-thread engine/preset swaps cleanly before inspecting host control ports.
    let _ = plugin.rt_slot.slot.poll_swap(None);
    let pre_swap_id = plugin.rt_slot.slot.active_swap_id();
    if pre_swap_id != plugin.last_swap_id {
        plugin.last_swap_id = pre_swap_id;
        plugin.requested_os = plugin.rt_slot.slot.engine.configuration().oversampling;
        let rev = plugin.rt_slot.slot.engine.reverb_state();
        plugin.requested_rev_enabled = rev.enabled;
        plugin.requested_rev_kill = rev.policy == ReverbTailPolicy::Kill;
        plugin.last_latency = plugin.rt_slot.slot.engine.latency_samples();
    }

    let mut staged = [StagedMidiCmd::default(); MAX_RT_CMDS];
    let mut staged_len = 0usize;

    if plugin.just_restored {
        if num_frames > 0 {
            plugin.snapshot_ports_into_baseline();
            plugin.just_restored = false;
        }
    } else {
        for idx in 0..NUM_PARAMS {
            if idx == ParamId::OversamplingMode as usize {
                continue;
            }
            if let Some(raw) = plugin.read_control_port(PORT_PARAMS_START + idx as u32) {
                let clean = sanitize_param_value(idx, raw);
                if clean.to_bits() != plugin.last_host[idx].to_bits() {
                    plugin.last_host[idx] = clean;
                    plugin.shared.mirror_params[idx].store(clean.to_bits(), Ordering::Release);
                    plugin
                        .shared
                        .rt_override_params[idx]
                        .store(true, Ordering::Release);
                    plugin.stage_cmd(
                        &mut staged,
                        &mut staged_len,
                        0,
                        Action::Parameter {
                            id: idx as u16,
                            value: clean,
                        },
                    );
                }
            }
        }

        if let Some(raw_glide) = plugin.read_control_port(PORT_GLIDE_ENABLED) {
            let glide = raw_glide >= 0.5;
            if glide != plugin.last_glide {
                plugin.last_glide = glide;
                plugin.shared.mirror_glide.store(glide, Ordering::Release);
                plugin
                    .shared
                    .rt_override_glide
                    .store(true, Ordering::Release);
                plugin.stage_cmd(
                    &mut staged,
                    &mut staged_len,
                    0,
                    Action::Parameter {
                        id: GLIDE_ENABLED,
                        value: if glide { 1.0 } else { 0.0 },
                    },
                );
            }
        }

        let mut host_reconfig = false;
        let os_idx = ParamId::OversamplingMode as usize;
        if let Some(raw_os) = plugin.read_control_port(PORT_PARAMS_START + os_idx as u32) {
            let snapped = snap_oversampling(raw_os);
            let f = f32::from(snapped);
            if f.to_bits() != plugin.last_host[os_idx].to_bits() {
                plugin.last_host[os_idx] = f;
                plugin.shared.mirror_params[os_idx].store(f.to_bits(), Ordering::Release);
                plugin
                    .shared
                    .rt_override_params[os_idx]
                    .store(true, Ordering::Release);
                if snapped != plugin.requested_os {
                    plugin.requested_os = snapped;
                    plugin.shared.desired_os.store(snapped, Ordering::Release);
                    host_reconfig = true;
                }
            }
        }

        if let Some(raw_rev_en) = plugin.read_control_port(PORT_REVERB_ENABLED) {
            let en = raw_rev_en >= 0.5;
            if en != plugin.last_host_rev_enabled {
                plugin.last_host_rev_enabled = en;
                plugin
                    .shared
                    .mirror_rev_enabled
                    .store(en, Ordering::Release);
                plugin
                    .shared
                    .rt_override_rev_enabled
                    .store(true, Ordering::Release);
                if en != plugin.requested_rev_enabled {
                    let wet = if en {
                        plugin
                            .read_control_port(PORT_PARAMS_START + ParamId::ReverbWet as u32)
                            .map(|v| sanitize_param_value(ParamId::ReverbWet as usize, v))
                            .unwrap_or(plugin.last_host[ParamId::ReverbWet as usize])
                    } else {
                        0.0
                    };
                    plugin.stage_cmd(
                        &mut staged,
                        &mut staged_len,
                        0,
                        Action::Parameter {
                            id: ParamId::ReverbWet as u16,
                            value: wet,
                        },
                    );
                    plugin.requested_rev_enabled = en;
                    plugin
                        .shared
                        .desired_rev_enabled
                        .store(en, Ordering::Release);
                    host_reconfig = true;
                }
            }
        }

        if let Some(raw_rev_kill) = plugin.read_control_port(PORT_REVERB_KILL_TAIL) {
            let kill = raw_rev_kill >= 0.5;
            if kill != plugin.last_host_rev_kill {
                plugin.last_host_rev_kill = kill;
                plugin
                    .shared
                    .mirror_rev_kill
                    .store(kill, Ordering::Release);
                plugin
                    .shared
                    .rt_override_rev_kill
                    .store(true, Ordering::Release);
                if kill != plugin.requested_rev_kill {
                    plugin.requested_rev_kill = kill;
                    plugin
                        .shared
                        .desired_rev_kill
                        .store(kill, Ordering::Release);
                    host_reconfig = true;
                }
            }
        }

        if host_reconfig {
            plugin
                .shared
                .pending_bg_reconfig
                .store(true, Ordering::Release);
            plugin.schedule_background_work();
        }
    }

    plugin.parse_control_in_atoms(num_frames, &mut staged, &mut staged_len);

    let out_l_ptr = plugin.ports[PORT_OUT_L as usize] as *mut f32;
    let out_r_ptr = plugin.ports[PORT_OUT_R as usize] as *mut f32;

    if num_frames == 0 || out_l_ptr.is_null() || out_r_ptr.is_null() {
        let mut rt_cmds = [Command {
            offset: 0,
            origin: ORIGIN_DAW_HOST,
            serial: 0,
            action: Action::PanicOrigin,
        }; MAX_RT_CMDS];
        for i in 0..staged_len {
            let mut c = staged[i].cmd;
            c.offset = 0;
            rt_cmds[i] = c;
        }
        let mut el = [];
        let mut er = [];
        let _ = plugin.rt_slot.process_planar_f32(
            &mut el,
            &mut er,
            &rt_cmds[..staged_len],
            None,
            None,
            None,
        );
    } else if num_frames <= plugin.max_frames as usize {
        let mut rt_cmds = [Command {
            offset: 0,
            origin: ORIGIN_DAW_HOST,
            serial: 0,
            action: Action::PanicOrigin,
        }; MAX_RT_CMDS];
        for i in 0..staged_len {
            let mut c = staged[i].cmd;
            c.offset = staged[i].abs_frame.min(num_frames) as u16;
            rt_cmds[i] = c;
        }
        let out_l = core::slice::from_raw_parts_mut(out_l_ptr, num_frames);
        let out_r = core::slice::from_raw_parts_mut(out_r_ptr, num_frames);
        let _ = plugin.rt_slot.process_planar_f32(
            out_l,
            out_r,
            &rt_cmds[..staged_len],
            None,
            None,
            None,
        );
    } else {
        let out_l = core::slice::from_raw_parts_mut(out_l_ptr, num_frames);
        let out_r = core::slice::from_raw_parts_mut(out_r_ptr, num_frames);
        let max_chunk = (plugin.max_frames as usize).max(1);
        let mut pos = 0usize;
        while pos < num_frames {
            let end = (pos + max_chunk).min(num_frames);
            let mut chunk_cmds = [Command {
                offset: 0,
                origin: ORIGIN_DAW_HOST,
                serial: 0,
                action: Action::PanicOrigin,
            }; MAX_RT_CMDS];
            let mut chunk_len = 0usize;
            for item in &staged[..staged_len] {
                let in_chunk = if end == num_frames {
                    item.abs_frame >= pos && item.abs_frame <= end
                } else {
                    item.abs_frame >= pos && item.abs_frame < end
                };
                if in_chunk && chunk_len < MAX_RT_CMDS {
                    let mut c = item.cmd;
                    c.offset = (item.abs_frame - pos) as u16;
                    chunk_cmds[chunk_len] = c;
                    chunk_len += 1;
                }
            }
            let _ = plugin.rt_slot.process_planar_f32(
                &mut out_l[pos..end],
                &mut out_r[pos..end],
                &chunk_cmds[..chunk_len],
                None,
                None,
                None,
            );
            pos = end;
        }
    }

    let post_swap_id = plugin.rt_slot.slot.active_swap_id();
    if post_swap_id != plugin.last_swap_id {
        plugin.last_swap_id = post_swap_id;
        plugin.requested_os = plugin.rt_slot.slot.engine.configuration().oversampling;
        let rev = plugin.rt_slot.slot.engine.reverb_state();
        plugin.requested_rev_enabled = rev.enabled;
        plugin.requested_rev_kill = rev.policy == ReverbTailPolicy::Kill;
        plugin.schedule_background_work();
    } else if plugin.shared.pending_bg_reconfig.load(Ordering::Relaxed) {
        plugin.frames_since_bg_poll = plugin
            .frames_since_bg_poll
            .saturating_add(num_frames as u64);
        let interval = u64::from(plugin.sample_rate / 10).max(512);
        if plugin.frames_since_bg_poll >= interval {
            plugin.frames_since_bg_poll = 0;
            plugin.schedule_background_work();
        }
    }

    plugin.last_latency = plugin.rt_slot.slot.engine.latency_samples();
    let lat_ptr = plugin.ports[PORT_LATENCY as usize] as *mut f32;
    if !lat_ptr.is_null() {
        *lat_ptr = plugin.last_latency as f32;
    }
}

unsafe extern "C" fn deactivate(instance: LV2_Handle) {
    if instance.is_null() {
        return;
    }
    let plugin = &mut *(instance as *mut Lv2Plugin);
    let _ = plugin.rt_slot.flush_stop_before_suspend();
    if let Ok(mut ctrl) = plugin.shared.ctrl.lock() {
        ctrl.set_audio_running(false, None, None);
        ctrl.worker.collect_retired();
    }
}

unsafe extern "C" fn cleanup(instance: LV2_Handle) {
    if !instance.is_null() {
        drop(Box::from_raw(instance.cast::<Lv2Plugin>()));
    }
}

unsafe extern "C" fn worker_work(
    instance: LV2_Handle,
    _respond: LV2_Worker_Respond_Function,
    _handle: LV2_Worker_Respond_Handle,
    _size: u32,
    _data: *const c_void,
) -> LV2_Worker_Status {
    if instance.is_null() {
        return lv2_ffi::LV2_WORKER_ERR_UNKNOWN;
    }
    let plugin = &*(instance as *const Lv2Plugin);
    plugin.shared.run_worker_job();
    LV2_WORKER_SUCCESS
}

unsafe extern "C" fn worker_response(
    _instance: LV2_Handle,
    _size: u32,
    _body: *const c_void,
) -> LV2_Worker_Status {
    LV2_WORKER_SUCCESS
}

unsafe extern "C" fn worker_end_run(_instance: LV2_Handle) -> LV2_Worker_Status {
    LV2_WORKER_SUCCESS
}

unsafe extern "C" fn state_save(
    instance: LV2_Handle,
    store: LV2_State_Store_Function,
    handle: LV2_State_Handle,
    _flags: u32,
    _features: *const *const LV2_Feature,
) -> LV2_State_Status {
    if instance.is_null() {
        return LV2_STATE_ERR_UNKNOWN;
    }
    let Some(store_fn) = store else {
        return LV2_STATE_ERR_UNKNOWN;
    };
    let plugin = &*(instance as *const Lv2Plugin);

    let (mut sound, bank, performance, name, category) = {
        let mut ctrl = plugin.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.poll();
        let sound = ctrl.sound.clone();
        let bank = ctrl.music_bank.clone();
        let perf = ctrl.current_performance();
        let name = plugin
            .shared
            .preset_name
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let cat = plugin
            .shared
            .preset_category
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        (sound, bank, perf, name, cat)
    };

    // Overlay lock-free atomic mirrors ONLY for parameters driven directly via real-time
    // host control ports so recent host automation not yet flushed through RT telemetry is
    // preserved without clobbering UI/control-plane edits (such as Oversampling 4x).
    for idx in 0..NUM_PARAMS {
        if plugin.shared.rt_override_params[idx].load(Ordering::Acquire) {
            let v = f32::from_bits(plugin.shared.mirror_params[idx].load(Ordering::Acquire));
            sound.values[idx] = sanitize_param_value(idx, v);
        }
    }
    if plugin.shared.rt_override_glide.load(Ordering::Acquire) {
        sound.glide = plugin.shared.mirror_glide.load(Ordering::Acquire);
    }
    if plugin.shared.rt_override_rev_enabled.load(Ordering::Acquire) {
        sound.reverb_enabled = plugin.shared.mirror_rev_enabled.load(Ordering::Acquire);
    }
    if plugin.shared.rt_override_rev_kill.load(Ordering::Acquire) {
        sound.kill_tail = plugin.shared.mirror_rev_kill.load(Ordering::Acquire);
    }

    let preset = Preset {
        name,
        category,
        sound,
        bank: (!is_default_pattern_bank(&bank)).then_some(bank),
        performance: Some(performance),
    };
    let Ok(mut bytes) = preset.encode() else {
        return LV2_STATE_ERR_UNKNOWN;
    };
    bytes.push(0); // LV2 atom:String expects NUL-terminated UTF-8 payload.

    store_fn(
        handle,
        plugin.urids.state_preset_v2,
        bytes.as_ptr().cast::<c_void>(),
        bytes.len(),
        plugin.urids.atom_string,
        LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE,
    )
}

unsafe extern "C" fn state_restore(
    instance: LV2_Handle,
    retrieve: LV2_State_Retrieve_Function,
    handle: LV2_State_Handle,
    _flags: u32,
    _features: *const *const LV2_Feature,
) -> LV2_State_Status {
    if instance.is_null() {
        return LV2_STATE_ERR_UNKNOWN;
    }
    let Some(retrieve_fn) = retrieve else {
        return LV2_STATE_ERR_UNKNOWN;
    };
    let plugin = &mut *(instance as *mut Lv2Plugin);

    let mut size: usize = 0;
    let mut val_type: u32 = 0;
    let mut val_flags: u32 = 0;
    let raw_ptr = retrieve_fn(
        handle,
        plugin.urids.state_preset_v2,
        &mut size,
        &mut val_type,
        &mut val_flags,
    );

    if raw_ptr.is_null() {
        // Property absent in state chunk: reset cleanly to Init per LV2 state specification.
        let mut raw_values = default_raw_values();
        for (i, v) in raw_values.iter_mut().enumerate() {
            *v = sanitize_param_value(i, *v);
        }
        let init_sound = SoundState {
            values: raw_values,
            glide: false,
            patch: Patch::default(),
            algorithm: Algorithm::LegacyRC1,
            reverb_enabled: true,
            kill_tail: false,
            drive_makeup: false,
        };
        let Ok(engine) = init_sound.prepare(plugin.sample_rate, plugin.max_frames) else {
            return LV2_STATE_ERR_UNKNOWN;
        };
        while plugin.rt_slot.slot.poll_swap(None) {}
        plugin.rt_slot.slot.engine = Box::new(engine);
        if let Some(m) = plugin.rt_slot.slot.music.as_mut() {
            *m.bank = PatternBank::default();
            m.active_pattern = 0;
            m.enabled = false;
            m.stop_pending_flush = false;
            m.current_frame = 0;
            m.step_index = 0;
        }
        {
            let mut ctrl = plugin.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
            ctrl.worker.collect_retired();
            ctrl.sound = init_sound.clone();
            *ctrl.music_bank = PatternBank::default();
            ctrl.music_active_pattern = 0;
            ctrl.sync_pod_from_sound();
        }
        *plugin
            .shared
            .preset_name
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = "Init".to_owned();
        *plugin
            .shared
            .preset_category
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = "Init".to_owned();
        plugin.shared.sync_mirrors_from_sound(&init_sound);
        plugin.requested_os = snap_oversampling(init_sound.values[ParamId::OversamplingMode as usize]);
        plugin.requested_rev_enabled = init_sound.reverb_enabled;
        plugin.requested_rev_kill = init_sound.kill_tail;
        plugin.last_swap_id = plugin.rt_slot.slot.active_swap_id();
        plugin.last_latency = plugin.rt_slot.slot.engine.latency_samples();
        plugin.just_restored = true;
        return LV2_STATE_SUCCESS;
    }

    let mut raw_bytes = core::slice::from_raw_parts(raw_ptr as *const u8, size);
    while raw_bytes.last() == Some(&0) {
        raw_bytes = &raw_bytes[..raw_bytes.len() - 1];
    }
    if raw_bytes.is_empty() {
        return LV2_STATE_ERR_UNKNOWN;
    }

    let Ok(bundle) = decode_and_prepare(raw_bytes, plugin.sample_rate, plugin.max_frames) else {
        return LV2_STATE_ERR_UNKNOWN;
    };

    while plugin.rt_slot.slot.poll_swap(None) {}
    plugin.rt_slot.slot.engine = Box::new(bundle.engine);
    let target_bank = bundle.bank.unwrap_or_else(|| Box::<PatternBank>::default());
    if let Some(m) = plugin.rt_slot.slot.music.as_mut() {
        m.bank.clone_from(&target_bank);
        if let Some((sched, arp)) = bundle.performance {
            m.scheduler = sched;
            m.arp = arp;
            if let Some(meta) = bundle.preset.performance {
                m.active_pattern = usize::from(meta.pattern.min(63));
                m.tempo_milli = meta.tempo_milli;
                m.swing = meta.swing;
                m.arp_octaves = meta.octaves;
                m.arp_latch = meta.latch;
                m.arp.order = match meta.mode {
                    1 => Order::Down,
                    2 => Order::UpDown,
                    3 => Order::AsPlayed,
                    4 => Order::Random,
                    5 => Order::Chord,
                    _ => Order::Up,
                };
            }
        }
        m.sample_rate = plugin.sample_rate;
        m.enabled = false;
        m.stop_pending_flush = false;
        m.current_frame = 0;
        m.step_index = 0;
    }

    let restored_sound = bundle.preset.sound.clone();
    {
        let mut ctrl = plugin.shared.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.worker.collect_retired();
        ctrl.sound = restored_sound.clone();
        ctrl.music_bank = target_bank;
        if let Some(meta) = bundle.preset.performance {
            ctrl.music_active_pattern = meta.pattern;
            ctrl.music_tempo_milli = meta.tempo_milli;
            ctrl.music_swing = meta.swing;
            ctrl.music_arp_octaves = meta.octaves;
            ctrl.music_arp_latch = meta.latch;
            ctrl.music_arp_seed = meta.seed;
        }
        ctrl.sync_pod_from_sound();
    }

    *plugin
        .shared
        .preset_name
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = bundle.preset.name;
    *plugin
        .shared
        .preset_category
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = bundle.preset.category;
    plugin.shared.sync_mirrors_from_sound(&restored_sound);
    plugin.requested_os = snap_oversampling(restored_sound.values[ParamId::OversamplingMode as usize]);
    plugin.requested_rev_enabled = restored_sound.reverb_enabled;
    plugin.requested_rev_kill = restored_sound.kill_tail;
    plugin.last_swap_id = plugin.rt_slot.slot.active_swap_id();
    plugin.last_latency = plugin.rt_slot.slot.engine.latency_samples();
    plugin.just_restored = true;
    LV2_STATE_SUCCESS
}

unsafe extern "C" fn options_get(_instance: LV2_Handle, _options: *mut LV2_Options_Option) -> u32 {
    LV2_OPTIONS_ERR_BAD_KEY
}

unsafe extern "C" fn options_set(instance: LV2_Handle, options: *const LV2_Options_Option) -> u32 {
    if instance.is_null() || options.is_null() {
        return LV2_OPTIONS_ERR_BAD_VALUE;
    }
    let plugin = &mut *(instance as *mut Lv2Plugin);
    let mut cur = options;
    let mut status = LV2_OPTIONS_SUCCESS;
    loop {
        let opt = *cur;
        if opt.key == 0 && opt.size == 0 && opt.value.is_null() {
            break;
        }
        if opt.key == plugin.urids.bufsz_max_block_length
            && plugin.urids.bufsz_max_block_length != 0
        {
            if opt.option_type != plugin.urids.atom_int
                || opt.size < core::mem::size_of::<i32>() as u32
                || opt.value.is_null()
            {
                status |= LV2_OPTIONS_ERR_BAD_VALUE;
            } else {
                let v = ptr::read_unaligned(opt.value as *const i32);
                if v <= 0 {
                    status |= LV2_OPTIONS_ERR_BAD_VALUE;
                } else {
                    plugin.max_frames = (v as u16).clamp(1, DEFAULT_MAX_FRAMES);
                }
            }
        } else {
            status |= LV2_OPTIONS_ERR_BAD_KEY;
        }
        cur = cur.add(1);
    }
    status
}

static WORKER_INTERFACE: LV2_Worker_Interface = LV2_Worker_Interface {
    work: Some(worker_work),
    work_response: Some(worker_response),
    end_run: Some(worker_end_run),
};

static STATE_INTERFACE: LV2_State_Interface = LV2_State_Interface {
    save: Some(state_save),
    restore: Some(state_restore),
};

static OPTIONS_INTERFACE: LV2_Options_Interface = LV2_Options_Interface {
    get: Some(options_get),
    set: Some(options_set),
};

unsafe extern "C" fn extension_data(uri: *const c_char) -> *const c_void {
    if uri.is_null() {
        return core::ptr::null();
    }
    let s = CStr::from_ptr(uri);
    if s == LV2_WORKER_INTERFACE_URI {
        (&WORKER_INTERFACE as *const LV2_Worker_Interface).cast::<c_void>()
    } else if s == LV2_STATE_INTERFACE_URI {
        (&STATE_INTERFACE as *const LV2_State_Interface).cast::<c_void>()
    } else if s == LV2_OPTIONS_INTERFACE_URI {
        (&OPTIONS_INTERFACE as *const LV2_Options_Interface).cast::<c_void>()
    } else {
        core::ptr::null()
    }
}

static PLUGIN_DESCRIPTOR: LV2_Descriptor = LV2_Descriptor {
    URI: PLUGIN_URI.as_ptr(),
    instantiate: Some(instantiate),
    connect_port: Some(connect_port),
    activate: Some(activate),
    run: Some(run),
    deactivate: Some(deactivate),
    cleanup: Some(cleanup),
    extension_data: Some(extension_data),
};

#[no_mangle]
pub unsafe extern "C" fn lv2_descriptor(index: u32) -> *const LV2_Descriptor {
    if index == 0 {
        &PLUGIN_DESCRIPTOR
    } else {
        core::ptr::null()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::CString;

    struct MockHostUrids {
        next: LV2_URID,
        by_uri: HashMap<String, LV2_URID>,
    }

    impl MockHostUrids {
        fn new() -> Self {
            Self {
                next: 1,
                by_uri: HashMap::new(),
            }
        }
        fn map_str(&mut self, s: &str) -> LV2_URID {
            if let Some(&id) = self.by_uri.get(s) {
                return id;
            }
            let id = self.next;
            self.next += 1;
            self.by_uri.insert(s.to_owned(), id);
            id
        }
    }

    unsafe extern "C" fn mock_map(handle: *mut c_void, uri: *const c_char) -> LV2_URID {
        let host = &mut *(handle as *mut MockHostUrids);
        let s = CStr::from_ptr(uri).to_string_lossy();
        host.map_str(&s)
    }

    struct MockStateBag {
        entries: HashMap<LV2_URID, (Vec<u8>, u32, u32)>,
    }

    unsafe extern "C" fn mock_store(
        handle: LV2_State_Handle,
        key: u32,
        value: *const c_void,
        size: usize,
        val_type: u32,
        flags: u32,
    ) -> LV2_State_Status {
        let bag = &mut *(handle as *mut MockStateBag);
        let data = core::slice::from_raw_parts(value as *const u8, size).to_vec();
        bag.entries.insert(key, (data, val_type, flags));
        LV2_STATE_SUCCESS
    }

    unsafe extern "C" fn mock_retrieve(
        handle: LV2_State_Handle,
        key: u32,
        size: *mut usize,
        val_type: *mut u32,
        flags: *mut u32,
    ) -> *const c_void {
        let bag = &*(handle as *const MockStateBag);
        let Some((data, vt, fl)) = bag.entries.get(&key) else {
            return core::ptr::null();
        };
        if !size.is_null() {
            *size = data.len();
        }
        if !val_type.is_null() {
            *val_type = *vt;
        }
        if !flags.is_null() {
            *flags = *fl;
        }
        data.as_ptr().cast::<c_void>()
    }

    #[test]
    fn lv2_descriptor_and_state_roundtrip_preserve_vector4_and_latency() {
        unsafe {
            let desc = lv2_descriptor(0);
            assert!(!desc.is_null());
            assert!(lv2_descriptor(1).is_null());
            assert_eq!(CStr::from_ptr((*desc).URI), PLUGIN_URI);

            let mut urids = MockHostUrids::new();
            let mut map_iface = LV2_URID_Map {
                handle: (&mut urids as *mut MockHostUrids).cast::<c_void>(),
                map: Some(mock_map),
            };
            let feat_map = LV2_Feature {
                URI: LV2_URID_MAP_URI.as_ptr(),
                data: (&mut map_iface as *mut LV2_URID_Map).cast::<c_void>(),
            };
            let features = [&feat_map as *const LV2_Feature, core::ptr::null()];

            let bundle = CString::new("/tmp/prophet-xii.lv2").unwrap();
            let h1 = ((*desc).instantiate.unwrap())(desc, 48_000.0, bundle.as_ptr(), features.as_ptr());
            assert!(!h1.is_null());

            let p1 = &mut *(h1 as *mut Lv2Plugin);
            {
                let mut ctrl = p1.shared.ctrl.lock().unwrap();
                let mut patch = Patch::default();
                patch.topology = prophet_dsp::vector::Topology::Vector4;
                ctrl.dispatch(UiCommand::SetPatch {
                    topology: patch.topology.into(),
                    routes: patch.routes.map(Into::into),
                    ratios: patch.ratios,
                    stages: patch.stages.map(Into::into),
                    modulation: patch.modulation.map(Into::into),
                    loop_held: patch.loop_held,
                });
                ctrl.dispatch(UiCommand::Param {
                    id: ParamId::FilterCutoff as u16,
                    value: 1450.0,
                });
                ctrl.dispatch(UiCommand::SetOversampling { factor: 4 });
            }

            let state_iface = &*(extension_data(LV2_STATE_INTERFACE_URI.as_ptr())
                as *const LV2_State_Interface);
            let mut bag = MockStateBag {
                entries: HashMap::new(),
            };
            assert_eq!(
                (state_iface.save.unwrap())(
                    h1,
                    Some(mock_store),
                    (&mut bag as *mut MockStateBag).cast::<c_void>(),
                    0,
                    features.as_ptr(),
                ),
                LV2_STATE_SUCCESS
            );
            assert!(!bag.entries.is_empty());

            // Restore onto fresh second instance and check ports + latency on run(0) + run(64)
            let h2 = ((*desc).instantiate.unwrap())(desc, 48_000.0, bundle.as_ptr(), features.as_ptr());
            assert!(!h2.is_null());
            assert_eq!(
                (state_iface.restore.unwrap())(
                    h2,
                    Some(mock_retrieve),
                    (&mut bag as *mut MockStateBag).cast::<c_void>(),
                    0,
                    features.as_ptr(),
                ),
                LV2_STATE_SUCCESS
            );

            let p2 = &mut *(h2 as *mut Lv2Plugin);
            assert_eq!(
                p2.rt_slot.slot.engine.patch().topology,
                prophet_dsp::vector::Topology::Vector4
            );
            assert_eq!(p2.rt_slot.slot.engine.latency_samples(), 24);

            let mut out_l = [0.0f32; 64];
            let mut out_r = [0.0f32; 64];
            let mut latency_out = -1.0f32;
            let mut ctrl_vals = [0.0f32; TOTAL_PORTS];
            ctrl_vals[PORT_REVERB_ENABLED as usize] = 1.0;
            let defaults = default_raw_values();
            for (idx, val) in defaults.iter().enumerate() {
                ctrl_vals[PORT_PARAMS_START as usize + idx] = *val;
            }
            let seq_in = LV2_Atom_Sequence {
                atom: LV2_Atom {
                    size: 8,
                    atom_type: urids.map_str(LV2_ATOM_SEQUENCE_URI.to_str().unwrap()),
                },
                body: LV2_Atom_Sequence_Body { unit: 0, pad: 0 },
            };
            let mut seq_out = [0u8; 256];

            let connect = (*desc).connect_port.unwrap();
            connect(h2, PORT_OUT_L, out_l.as_mut_ptr().cast::<c_void>());
            connect(h2, PORT_OUT_R, out_r.as_mut_ptr().cast::<c_void>());
            connect(h2, PORT_CONTROL_IN, &seq_in as *const LV2_Atom_Sequence as *mut c_void);
            connect(h2, PORT_NOTIFY_OUT, seq_out.as_mut_ptr().cast::<c_void>());
            connect(h2, PORT_LATENCY, (&mut latency_out as *mut f32).cast::<c_void>());
            for port_idx in PORT_REVERB_ENABLED..=PORT_GLIDE_ENABLED {
                connect(
                    h2,
                    port_idx,
                    (&mut ctrl_vals[port_idx as usize] as *mut f32).cast::<c_void>(),
                );
            }

            ((*desc).activate.unwrap())(h2);
            ((*desc).run.unwrap())(h2, 0);
            assert_eq!(latency_out, 24.0);
            ((*desc).run.unwrap())(h2, 64);
            // Restored Vector4 + cutoff must NOT be clobbered by un-updated host control port defaults!
            assert_eq!(
                p2.rt_slot.slot.engine.sound_state().values[ParamId::FilterCutoff as usize],
                1450.0
            );
            assert_eq!(p2.rt_slot.slot.engine.configuration().oversampling, 4);

            // Corrupt state must fail cleanly without panicking.
            let key = *bag.entries.keys().next().unwrap();
            bag.entries.insert(
                key,
                (b"{broken-json\0".to_vec(), urids.map_str(LV2_ATOM_STRING_URI.to_str().unwrap()), 0),
            );
            assert_eq!(
                (state_iface.restore.unwrap())(
                    h2,
                    Some(mock_retrieve),
                    (&mut bag as *mut MockStateBag).cast::<c_void>(),
                    0,
                    features.as_ptr(),
                ),
                LV2_STATE_ERR_UNKNOWN
            );

            ((*desc).deactivate.unwrap())(h2);
            ((*desc).cleanup.unwrap())(h2);
            ((*desc).cleanup.unwrap())(h1);
        }
    }
}
