// SPDX-License-Identifier: GPL-3.0-only
//! PROPHET-XII VST3 & CLAP Plugin Wrapper backed by `prophet-host` (`RtHostSlot` +
//! `HostControlPlane` + `prophet_host::embed`).
//!
//! Real-time audio callbacks (`process()` / `reset()`) are strictly lock-free, allocation-free,
//! and never drop `Box<PreparedEngine>` / `Box<PatternBank>` instances.

use nih_plug::{
    params::persist::PersistentField, prelude::*, wrapper::state::ParamValue,
};
use prophet_control::{
    music::PatternBank,
    port::{Action, Command, EnginePort, Prepare, GLIDE_ENABLED, ORIGINS},
    state::SoundState,
};
use prophet_dsp::{
    fx::{reverb::Algorithm, solaris_reverb::ReverbTailPolicy},
    oversample::OversampleFactor,
    vector::Patch,
};
use prophet_host::{
    create_host_session,
    embed::{
        spawn_embedded_editor, EditorViewConfig, EmbeddedEditorHandle, HostBridgeSink,
        ParentWindow,
    },
    preset::{Performance, Preset},
    ui_snapshot_to_json, HostControlPlane, RtHostSlot, SerialIssuer, UiCommand, UiCommandReply,
    UiSnapshot, ORIGIN_DAW_HOST,
};
use prophet_params::{AtomicParamBank, ParamId, NUM_PARAMS, PARAM_SPECS};
use std::any::Any;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex, Weak,
};

const EDITOR_LOGICAL_WIDTH: u32 = 1180;
const EDITOR_LOGICAL_HEIGHT: u32 = 720;
const MAX_HOST_CMDS_PER_BLOCK: usize = 256;

#[derive(Clone)]
struct PendingPresetData {
    name: String,
    category: String,
    sound: SoundState,
    bank: Box<PatternBank>,
    performance: Option<Performance>,
}

#[derive(Clone)]
enum PendingRestore {
    LegacyRc1,
    PresetV2(Box<PendingPresetData>),
}

#[derive(Clone)]
struct PersistSnapshot {
    name: String,
    category: String,
    sound: SoundState,
    bank: Box<PatternBank>,
    performance: Option<Performance>,
    params: Weak<SynthParams>,
    pending_restore: Option<PendingRestore>,
}

impl Default for PersistSnapshot {
    fn default() -> Self {
        let params = SynthParams::default_values_only();
        Self {
            name: "Init".to_owned(),
            category: "Init".to_owned(),
            sound: sound_from_raw_array(params, false, Patch::default(), Algorithm::LegacyRC1, true, false, false),
            bank: Box::<PatternBank>::default(),
            performance: None,
            params: Weak::new(),
            pending_restore: None,
        }
    }
}

struct PresetPersistField {
    state: Arc<Mutex<PersistSnapshot>>,
}

impl Default for PresetPersistField {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(PersistSnapshot::default())),
        }
    }
}

fn is_default_pattern_bank(bank: &PatternBank) -> bool {
    let default_pat = prophet_control::music::Pattern::default();
    bank.0.iter().all(|p| *p == default_pat)
}

fn snap_oversampling(raw: f32) -> u8 {
    let r = raw.round() as i32;
    if r <= 1 {
        1
    } else if r == 2 {
        2
    } else {
        4
    }
}

fn sanitize_param_value(idx: usize, val: f32) -> f32 {
    let spec = &PARAM_SPECS[idx];
    if spec.id == ParamId::OversamplingMode {
        return f32::from(snap_oversampling(val));
    }
    let finite = if val.is_finite() {
        val
    } else {
        spec.min
    };
    let clamped = finite.clamp(spec.min, spec.max);
    if spec.integer {
        clamped.round()
    } else {
        clamped
    }
}

fn sound_from_raw_array(
    mut values: [f32; NUM_PARAMS],
    glide: bool,
    patch: Patch,
    algorithm: Algorithm,
    reverb_enabled: bool,
    kill_tail: bool,
    drive_makeup: bool,
) -> SoundState {
    for (i, v) in values.iter_mut().enumerate() {
        *v = sanitize_param_value(i, *v);
    }
    SoundState {
        values,
        glide,
        patch,
        algorithm,
        reverb_enabled,
        kill_tail,
        drive_makeup,
    }
}

impl<'a> PersistentField<'a, String> for PresetPersistField {
    fn set(&self, new_value: String) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let trimmed = new_value.trim();
        if trimmed.is_empty() {
            guard.name = "Init".to_owned();
            guard.category = "Init".to_owned();
            guard.sound.patch = Patch::default();
            guard.sound.algorithm = Algorithm::LegacyRC1;
            guard.sound.drive_makeup = false;
            *guard.bank = PatternBank::default();
            guard.performance = None;
            guard.pending_restore = Some(PendingRestore::LegacyRc1);
            return;
        }
        if let Ok(mut decoded) = Preset::decode(trimmed.as_bytes()) {
            let bank = decoded
                .bank
                .take()
                .unwrap_or_else(|| Box::<PatternBank>::default());
            guard.name = decoded.name.clone();
            guard.category = decoded.category.clone();
            guard.sound = decoded.sound.clone();
            *guard.bank = *bank;
            guard.performance = decoded.performance;
            guard.pending_restore = Some(PendingRestore::PresetV2(Box::new(PendingPresetData {
                name: decoded.name,
                category: decoded.category,
                sound: decoded.sound,
                bank: guard.bank.clone(),
                performance: decoded.performance,
            })));
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&String) -> R,
    {
        let encoded = {
            let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut sound = guard.sound.clone();
            if let Some(params) = guard.params.upgrade() {
                sound.reverb_enabled = params.reverb_enabled.value();
                sound.kill_tail = params.reverb_kill_tail.value();
                sound.glide = params.p83.value();
                sound.values = params.collect_values();
            }
            for (i, v) in sound.values.iter_mut().enumerate() {
                *v = sanitize_param_value(i, *v);
            }
            let preset = Preset {
                name: guard.name.clone(),
                category: guard.category.clone(),
                sound,
                bank: (!is_default_pattern_bank(&guard.bank)).then(|| guard.bank.clone()),
                performance: guard.performance,
            };
            preset
                .encode()
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_default()
        };
        f(&encoded)
    }
}

#[derive(Params)]
struct SynthParams {
    #[id = "reverb_enabled"]
    reverb_enabled: BoolParam,
    #[id = "reverb_kill_tail"]
    reverb_kill_tail: BoolParam,
    #[id = "p0"]
    p0: FloatParam,
    #[id = "p1"]
    p1: FloatParam,
    #[id = "p2"]
    p2: FloatParam,
    #[id = "p3"]
    p3: FloatParam,
    #[id = "p4"]
    p4: FloatParam,
    #[id = "p5"]
    p5: FloatParam,
    #[id = "p6"]
    p6: FloatParam,
    #[id = "p7"]
    p7: FloatParam,
    #[id = "p8"]
    p8: FloatParam,
    #[id = "p9"]
    p9: FloatParam,
    #[id = "p10"]
    p10: FloatParam,
    #[id = "p11"]
    p11: FloatParam,
    #[id = "p12"]
    p12: FloatParam,
    #[id = "p13"]
    p13: FloatParam,
    #[id = "p14"]
    p14: FloatParam,
    #[id = "p15"]
    p15: FloatParam,
    #[id = "p16"]
    p16: FloatParam,
    #[id = "p17"]
    p17: FloatParam,
    #[id = "p18"]
    p18: FloatParam,
    #[id = "p19"]
    p19: FloatParam,
    #[id = "p20"]
    p20: FloatParam,
    #[id = "p21"]
    p21: FloatParam,
    #[id = "p22"]
    p22: FloatParam,
    #[id = "p23"]
    p23: FloatParam,
    #[id = "p24"]
    p24: FloatParam,
    #[id = "p25"]
    p25: FloatParam,
    #[id = "p26"]
    p26: FloatParam,
    #[id = "p27"]
    p27: FloatParam,
    #[id = "p28"]
    p28: FloatParam,
    #[id = "p29"]
    p29: FloatParam,
    #[id = "p30"]
    p30: FloatParam,
    #[id = "p31"]
    p31: FloatParam,
    #[id = "p32"]
    p32: FloatParam,
    #[id = "p33"]
    p33: FloatParam,
    #[id = "p34"]
    p34: FloatParam,
    #[id = "p35"]
    p35: FloatParam,
    #[id = "p36"]
    p36: FloatParam,
    #[id = "p37"]
    p37: FloatParam,
    #[id = "p38"]
    p38: FloatParam,
    #[id = "p39"]
    p39: FloatParam,
    #[id = "p40"]
    p40: FloatParam,
    #[id = "p41"]
    p41: FloatParam,
    #[id = "p42"]
    p42: FloatParam,
    #[id = "p43"]
    p43: FloatParam,
    #[id = "p44"]
    p44: FloatParam,
    #[id = "p45"]
    p45: FloatParam,
    #[id = "p46"]
    p46: FloatParam,
    #[id = "p47"]
    p47: FloatParam,
    #[id = "p48"]
    p48: FloatParam,
    #[id = "p49"]
    p49: FloatParam,
    #[id = "p50"]
    p50: FloatParam,
    #[id = "p51"]
    p51: FloatParam,
    #[id = "p52"]
    p52: FloatParam,
    #[id = "p53"]
    p53: FloatParam,
    #[id = "p54"]
    p54: FloatParam,
    #[id = "p55"]
    p55: FloatParam,
    #[id = "p56"]
    p56: FloatParam,
    #[id = "p57"]
    p57: FloatParam,
    #[id = "p58"]
    p58: FloatParam,
    #[id = "p59"]
    p59: FloatParam,
    #[id = "p60"]
    p60: FloatParam,
    #[id = "p61"]
    p61: FloatParam,
    #[id = "p62"]
    p62: FloatParam,
    #[id = "p63"]
    p63: FloatParam,
    #[id = "p64"]
    p64: FloatParam,
    #[id = "p65"]
    p65: FloatParam,
    #[id = "p66"]
    p66: FloatParam,
    #[id = "p67"]
    p67: FloatParam,
    #[id = "p68"]
    p68: FloatParam,
    #[id = "p69"]
    p69: FloatParam,
    #[id = "p70"]
    p70: FloatParam,
    #[id = "p71"]
    p71: FloatParam,
    #[id = "p72"]
    p72: FloatParam,
    #[id = "p73"]
    p73: FloatParam,
    #[id = "p74"]
    p74: FloatParam,
    #[id = "p75"]
    p75: FloatParam,
    #[id = "p76"]
    p76: FloatParam,
    #[id = "p77"]
    p77: FloatParam,
    #[id = "p78"]
    p78: FloatParam,
    #[id = "p79"]
    p79: FloatParam,
    #[id = "p80"]
    p80: FloatParam,
    #[id = "p81"]
    p81: FloatParam,
    #[id = "p82"]
    p82: FloatParam,
    #[id = "p83"]
    p83: BoolParam,
    #[persist = "prophet_preset_v2"]
    preset_v2: PresetPersistField,
}

impl SynthParams {
    fn default_values_only() -> [f32; NUM_PARAMS] {
        let bank = AtomicParamBank::new();
        core::array::from_fn(|i| bank.get(ParamId::from_index(i).unwrap()))
    }

    fn float_param(&self, idx: usize) -> &FloatParam {
        match idx {
            0 => &self.p0,
            1 => &self.p1,
            2 => &self.p2,
            3 => &self.p3,
            4 => &self.p4,
            5 => &self.p5,
            6 => &self.p6,
            7 => &self.p7,
            8 => &self.p8,
            9 => &self.p9,
            10 => &self.p10,
            11 => &self.p11,
            12 => &self.p12,
            13 => &self.p13,
            14 => &self.p14,
            15 => &self.p15,
            16 => &self.p16,
            17 => &self.p17,
            18 => &self.p18,
            19 => &self.p19,
            20 => &self.p20,
            21 => &self.p21,
            22 => &self.p22,
            23 => &self.p23,
            24 => &self.p24,
            25 => &self.p25,
            26 => &self.p26,
            27 => &self.p27,
            28 => &self.p28,
            29 => &self.p29,
            30 => &self.p30,
            31 => &self.p31,
            32 => &self.p32,
            33 => &self.p33,
            34 => &self.p34,
            35 => &self.p35,
            36 => &self.p36,
            37 => &self.p37,
            38 => &self.p38,
            39 => &self.p39,
            40 => &self.p40,
            41 => &self.p41,
            42 => &self.p42,
            43 => &self.p43,
            44 => &self.p44,
            45 => &self.p45,
            46 => &self.p46,
            47 => &self.p47,
            48 => &self.p48,
            49 => &self.p49,
            50 => &self.p50,
            51 => &self.p51,
            52 => &self.p52,
            53 => &self.p53,
            54 => &self.p54,
            55 => &self.p55,
            56 => &self.p56,
            57 => &self.p57,
            58 => &self.p58,
            59 => &self.p59,
            60 => &self.p60,
            61 => &self.p61,
            62 => &self.p62,
            63 => &self.p63,
            64 => &self.p64,
            65 => &self.p65,
            66 => &self.p66,
            67 => &self.p67,
            68 => &self.p68,
            69 => &self.p69,
            70 => &self.p70,
            71 => &self.p71,
            72 => &self.p72,
            73 => &self.p73,
            74 => &self.p74,
            75 => &self.p75,
            76 => &self.p76,
            77 => &self.p77,
            78 => &self.p78,
            79 => &self.p79,
            80 => &self.p80,
            81 => &self.p81,
            _ => &self.p82,
        }
    }

    fn collect_values(&self) -> [f32; NUM_PARAMS] {
        core::array::from_fn(|i| sanitize_param_value(i, self.float_param(i).value()))
    }
}

impl Default for SynthParams {
    fn default() -> Self {
        let bank = AtomicParamBank::new();
        let make = |idx: usize| -> FloatParam {
            let spec = &PARAM_SPECS[idx];
            let p = FloatParam::new(
                spec.name,
                bank.get(spec.id),
                FloatRange::Linear {
                    min: spec.min,
                    max: spec.max,
                },
            );
            if spec.integer {
                p.with_step_size(1.0)
            } else {
                p
            }
        };
        Self {
            reverb_enabled: BoolParam::new("Reverb Enabled", true),
            reverb_kill_tail: BoolParam::new("Reverb Kill Tail (off = Spill)", false),
            p0: make(0),
            p1: make(1),
            p2: make(2),
            p3: make(3),
            p4: make(4),
            p5: make(5),
            p6: make(6),
            p7: make(7),
            p8: make(8),
            p9: make(9),
            p10: make(10),
            p11: make(11),
            p12: make(12),
            p13: make(13),
            p14: make(14),
            p15: make(15),
            p16: make(16),
            p17: make(17),
            p18: make(18),
            p19: make(19),
            p20: make(20),
            p21: make(21),
            p22: make(22),
            p23: make(23),
            p24: make(24),
            p25: make(25),
            p26: make(26),
            p27: make(27),
            p28: make(28),
            p29: make(29),
            p30: make(30),
            p31: make(31),
            p32: make(32),
            p33: make(33),
            p34: make(34),
            p35: make(35),
            p36: make(36),
            p37: make(37),
            p38: make(38),
            p39: make(39),
            p40: make(40),
            p41: make(41),
            p42: make(42),
            p43: make(43),
            p44: make(44),
            p45: make(45),
            p46: make(46),
            p47: make(47),
            p48: make(48),
            p49: make(49),
            p50: make(50),
            p51: make(51),
            p52: make(52),
            p53: make(53),
            p54: make(54),
            p55: make(55),
            p56: make(56),
            p57: make(57),
            p58: make(58),
            p59: make(59),
            p60: make(60),
            p61: make(61),
            p62: make(62),
            p63: make(63),
            p64: make(64),
            p65: make(65),
            p66: make(66),
            p67: make(67),
            p68: make(68),
            p69: make(69),
            p70: make(70),
            p71: make(71),
            p72: make(72),
            p73: make(73),
            p74: make(74),
            p75: make(75),
            p76: make(76),
            p77: make(77),
            p78: make(78),
            p79: make(79),
            p80: make(80),
            p81: make(81),
            p82: make(82),
            p83: BoolParam::new("Glide Enabled", bank.glide_enabled()),
            preset_v2: PresetPersistField::default(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum BgTask {
    PollControlPlane,
    HostReconfig,
}

pub struct Prophet {
    params: Arc<SynthParams>,
    persist: Arc<Mutex<PersistSnapshot>>,
    rt_slot: RtHostSlot,
    ctrl: Arc<Mutex<HostControlPlane>>,
    daw_serials: Arc<SerialIssuer>,
    last_host: [f32; NUM_PARAMS],
    last_glide: bool,
    requested_os: u8,
    requested_rev_enabled: bool,
    requested_rev_kill: bool,
    last_swap_id: u64,
    last_latency: u32,
    pending_bg_reconfig: Arc<AtomicBool>,
    frames_since_bg_poll: u64,
    cached_sr: u32,
    zoom_percent: Arc<AtomicU32>,
}

impl Default for Prophet {
    fn default() -> Self {
        let params = Arc::new(SynthParams::default());
        let persist = params.preset_v2.state.clone();
        let raw_values = params.collect_values();
        let glide = params.p83.value();
        let rev_enabled = params.reverb_enabled.value();
        let rev_kill = params.reverb_kill_tail.value();
        let initial_sound = sound_from_raw_array(
            raw_values,
            glide,
            Patch::default(),
            Algorithm::LegacyRC1,
            rev_enabled,
            rev_kill,
            false,
        );
        {
            let mut pguard = persist.lock().unwrap_or_else(|e| e.into_inner());
            pguard.params = Arc::downgrade(&params);
            pguard.sound = initial_sound.clone();
        }

        let initial_os = snap_oversampling(raw_values[ParamId::OversamplingMode as usize]);
        let (mut rt_slot, mut ctrl) = create_host_session(
            "clap-vst3",
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
                max_frames: 1024,
                oversampling: initial_os,
            },
            true,
        )
        .expect("default PROPHET-XII session configuration must be valid");

        if let Ok(engine) = initial_sound.prepare(48_000, 1024) {
            rt_slot.slot.engine = Box::new(engine);
        }
        ctrl.sound = initial_sound;
        ctrl.sync_pod_from_sound();

        let last_latency = rt_slot.slot.engine.latency_samples();
        Self {
            params,
            persist,
            rt_slot,
            ctrl: Arc::new(Mutex::new(ctrl)),
            daw_serials: Arc::new(SerialIssuer::new()),
            last_host: raw_values,
            last_glide: glide,
            requested_os: initial_os,
            requested_rev_enabled: rev_enabled,
            requested_rev_kill: rev_kill,
            last_swap_id: 0,
            last_latency,
            pending_bg_reconfig: Arc::new(AtomicBool::new(false)),
            frames_since_bg_poll: 0,
            cached_sr: 48_000,
            zoom_percent: Arc::new(AtomicU32::new(100)),
        }
    }
}

impl Prophet {
    fn sync_persist_from_ctrl(ctrl: &HostControlPlane, persist: &Arc<Mutex<PersistSnapshot>>) {
        let mut pguard = persist.lock().unwrap_or_else(|e| e.into_inner());
        pguard.sound = ctrl.sound.clone();
        pguard.bank.clone_from(&ctrl.music_bank);
        pguard.performance = Some(ctrl.current_performance());
    }

    fn apply_host_reconfig(
        params: &Arc<SynthParams>,
        ctrl_arc: &Arc<Mutex<HostControlPlane>>,
        persist_arc: &Arc<Mutex<PersistSnapshot>>,
        pending_flag: &Arc<AtomicBool>,
    ) {
        let target_os = snap_oversampling(params.p63.value());
        let rev_enabled = params.reverb_enabled.value();
        let rev_kill = params.reverb_kill_tail.value();
        let mut ctrl = ctrl_arc.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.poll();

        let current_os = snap_oversampling(ctrl.sound.values[ParamId::OversamplingMode as usize]);
        let sr = ctrl.sample_rate();
        let mf = ctrl.max_frames();

        if current_os != target_os {
            let mut next = ctrl.sound.clone();
            next.values[ParamId::OversamplingMode as usize] = f32::from(target_os);
            next.reverb_enabled = rev_enabled;
            next.kill_tail = rev_kill;
            if let Ok(swap_id) = ctrl.worker.prepare_reconfig(&next, sr, mf) {
                ctrl.sound = next;
                let _ = swap_id;
                ctrl.sync_pod_from_sound();
            }
        } else if ctrl.sound.reverb_enabled != rev_enabled || ctrl.sound.kill_tail != rev_kill {
            let _ = ctrl.dispatch(UiCommand::SetReverb {
                algorithm: None,
                enabled: Some(rev_enabled),
                tail: Some(if rev_kill {
                    prophet_host::WireTailPolicy::Kill
                } else {
                    prophet_host::WireTailPolicy::Spill
                }),
            });
        }

        ctrl.poll();
        let still_pending = !ctrl.is_voices_idle() && (ctrl.sound.reverb_enabled != rev_enabled || ctrl.sound.kill_tail != rev_kill);
        pending_flag.store(still_pending, Ordering::Release);
        Self::sync_persist_from_ctrl(&ctrl, persist_arc);
    }

    fn stage_daw_cmd(
        cmds: &mut [Command; MAX_HOST_CMDS_PER_BLOCK],
        len: &mut usize,
        serials: &SerialIssuer,
        offset: u16,
        action: Action,
    ) {
        if *len < MAX_HOST_CMDS_PER_BLOCK {
            cmds[*len] = Command {
                offset,
                origin: ORIGIN_DAW_HOST,
                serial: serials.next_serial(ORIGIN_DAW_HOST),
                action,
            };
            *len += 1;
        }
    }

    fn update_compute_status(&self) -> ProcessStatus {
        let voices = self.rt_slot.slot.engine.voices();
        let voice_active = voices.iter().any(|v| v.active || v.gate);
        let music_active = self
            .rt_slot
            .slot
            .music
            .as_ref()
            .is_some_and(|m| m.should_render());
        if voice_active || music_active {
            return ProcessStatus::KeepAlive;
        }
        let rev = self.rt_slot.slot.engine.reverb_state();
        let killing = !rev.enabled && rev.policy == ReverbTailPolicy::Kill;
        if !rev.dormant && !killing {
            let decay = self.params.p53.value().clamp(0.2, 60.0);
            let sr = f32::from(self.cached_sr.min(u16::MAX as u32) as u16)
                .max(self.cached_sr as f32);
            let samples = (decay * sr).round().clamp(1.0, 60.0 * 192_000.0) as u32;
            return ProcessStatus::Tail(samples);
        }
        ProcessStatus::Normal
    }

    #[cfg(test)]
    fn engine_latency(&self) -> u32 {
        self.rt_slot.slot.engine.latency_samples()
    }
}

trait ProcessHostHook {
    fn next_event(&mut self) -> Option<PluginNoteEvent<Prophet>>;
    fn execute_background(&self, task: BgTask);
    fn set_latency_samples(&self, samples: u32);
}

struct LiveProcCtx<'a, C: ProcessContext<Prophet>>(&'a mut C);

impl<C: ProcessContext<Prophet>> ProcessHostHook for LiveProcCtx<'_, C> {
    #[inline]
    fn next_event(&mut self) -> Option<PluginNoteEvent<Prophet>> {
        self.0.next_event()
    }
    #[inline]
    fn execute_background(&self, task: BgTask) {
        self.0.execute_background(task);
    }
    #[inline]
    fn set_latency_samples(&self, samples: u32) {
        self.0.set_latency_samples(samples);
    }
}

impl Plugin for Prophet {
    const NAME: &'static str = "PROPHET-XII";
    const VENDOR: &'static str = "PROPHET-XII Contributors";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = BgTask;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let params = self.params.clone();
        let ctrl_arc = self.ctrl.clone();
        let persist_arc = self.persist.clone();
        let pending_flag = self.pending_bg_reconfig.clone();
        Box::new(move |task| match task {
            BgTask::PollControlPlane => {
                let mut ctrl = ctrl_arc.lock().unwrap_or_else(|e| e.into_inner());
                ctrl.poll();
                Self::sync_persist_from_ctrl(&ctrl, &persist_arc);
                if ctrl.is_voices_idle() {
                    pending_flag.store(false, Ordering::Release);
                }
            }
            BgTask::HostReconfig => {
                Self::apply_host_reconfig(&params, &ctrl_arc, &persist_arc, &pending_flag);
            }
        })
    }

    fn filter_state(state: &mut PluginState) {
        if !state.params.contains_key("p83") {
            let glide = match state.params.get("p42") {
                Some(ParamValue::F32(v)) => *v > 0.0005,
                Some(ParamValue::I32(v)) => *v != 0,
                Some(ParamValue::Bool(v)) => *v,
                _ => false,
            };
            state.params.insert("p83".to_owned(), ParamValue::Bool(glide));
        }
        if !state.fields.contains_key("prophet_preset_v2") {
            state
                .fields
                .insert("prophet_preset_v2".to_owned(), "\"\"".to_owned());
        }
    }

    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        cfg: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        let sr = (cfg.sample_rate.round() as u32).clamp(8_000, 192_000);
        let max_frames = (cfg.max_buffer_size as u16).clamp(1, 1024);
        self.cached_sr = sr;

        let raw_values = self.params.collect_values();
        let glide = self.params.p83.value();
        let rev_enabled = self.params.reverb_enabled.value();
        let rev_kill = self.params.reverb_kill_tail.value();

        let (name, category, mut target_sound, target_bank, target_perf) = {
            let mut pguard = self.persist.lock().unwrap_or_else(|e| e.into_inner());
            let pending = pguard.pending_restore.take();
            match pending {
                Some(PendingRestore::LegacyRc1) => (
                    "Init".to_owned(),
                    "Init".to_owned(),
                    sound_from_raw_array(
                        raw_values,
                        glide,
                        Patch::default(),
                        Algorithm::LegacyRC1,
                        rev_enabled,
                        rev_kill,
                        false,
                    ),
                    Box::<PatternBank>::default(),
                    None,
                ),
                Some(PendingRestore::PresetV2(preset)) => {
                    let bank = preset.bank;
                    let sound = sound_from_raw_array(
                        raw_values,
                        glide,
                        preset.sound.patch,
                        preset.sound.algorithm,
                        rev_enabled,
                        rev_kill,
                        preset.sound.drive_makeup,
                    );
                    (preset.name, preset.category, sound, bank, preset.performance)
                }
                None => {
                    let sound = sound_from_raw_array(
                        raw_values,
                        glide,
                        pguard.sound.patch,
                        pguard.sound.algorithm,
                        rev_enabled,
                        rev_kill,
                        pguard.sound.drive_makeup,
                    );
                    (
                        pguard.name.clone(),
                        pguard.category.clone(),
                        sound,
                        pguard.bank.clone(),
                        pguard.performance,
                    )
                }
            }
        };

        let os = snap_oversampling(raw_values[ParamId::OversamplingMode as usize]);
        target_sound.values[ParamId::OversamplingMode as usize] = f32::from(os);

        let Ok(engine) = target_sound.prepare(sr, max_frames) else {
            return false;
        };

        // Drain any stale queued swaps before replacing directly at initialize boundary.
        while self.rt_slot.slot.poll_swap(None) {}
        self.rt_slot.slot.engine = Box::new(engine);
        if let Some(m) = self.rt_slot.slot.music.as_mut() {
            m.bank.clone_from(&target_bank);
            if let Some(perf) = target_perf {
                if let Ok((sched, arp)) = perf.prepare(sr) {
                    m.scheduler = sched;
                    m.arp = arp;
                    m.active_pattern = usize::from(perf.pattern.min(63));
                    m.tempo_milli = perf.tempo_milli;
                    m.swing = perf.swing;
                    m.arp_octaves = perf.octaves;
                    m.arp_latch = perf.latch;
                }
            } else if m.sample_rate != sr {
                if let Ok(rebuilt) = prophet_control::music::NoteScheduler::new(sr, m.tempo_milli, m.swing) {
                    m.scheduler = rebuilt;
                }
            }
            m.sample_rate = sr;
            m.enabled = false;
            m.stop_pending_flush = false;
            m.current_frame = 0;
            m.step_index = 0;
        }

        {
            let mut pguard = self.persist.lock().unwrap_or_else(|e| e.into_inner());
            pguard.name = name;
            pguard.category = category;
            pguard.sound = target_sound.clone();
            pguard.bank.clone_from(&target_bank);
            pguard.performance = target_perf;
        }

        {
            let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
            ctrl.worker.collect_retired();
            ctrl.sound = target_sound;
            ctrl.music_bank = target_bank;
            if let Some(perf) = target_perf {
                ctrl.music_active_pattern = perf.pattern;
                ctrl.music_tempo_milli = perf.tempo_milli;
                ctrl.music_swing = perf.swing;
                ctrl.music_arp_octaves = perf.octaves;
                ctrl.music_arp_latch = perf.latch;
                ctrl.music_arp_seed = perf.seed;
            }
            ctrl.set_audio_running(true, Some(sr), Some(max_frames));
            ctrl.sync_pod_from_sound();
        }

        self.last_host = raw_values;
        self.last_glide = glide;
        self.requested_os = os;
        self.requested_rev_enabled = rev_enabled;
        self.requested_rev_kill = rev_kill;
        self.last_swap_id = self.rt_slot.slot.active_swap_id();
        self.last_latency = self.rt_slot.slot.engine.latency_samples();
        self.pending_bg_reconfig.store(false, Ordering::Release);
        self.frames_since_bg_poll = 0;
        context.set_latency_samples(self.last_latency);
        true
    }

    fn deactivate(&mut self) {
        if let Ok(mut ctrl) = self.ctrl.try_lock() {
            ctrl.set_audio_running(false, None, None);
            ctrl.worker.collect_retired();
        }
    }

    fn reset(&mut self) {
        let mut cmds = [Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        }; ORIGINS];
        for (i, slot) in cmds.iter_mut().enumerate() {
            let origin = i as u8;
            let serial = if origin == ORIGIN_DAW_HOST {
                self.daw_serials.next_serial(origin)
            } else {
                // Zero-lock serial epoch bump on active slot origins cleanly releases stuck notes.
                1
            };
            *slot = Command {
                offset: 0,
                origin,
                serial,
                action: Action::PanicOrigin,
            };
        }
        let mut empty_l = [];
        let mut empty_r = [];
        let _ = self.rt_slot.slot.engine.process(
            &mut empty_l,
            &mut empty_r,
            &cmds[..1],
            &mut [Default::default(); 1],
        );
        let _ = self.rt_slot.flush_stop_before_suspend();
    }

    fn editor(&mut self, _: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(ProphetEditor {
            params: self.params.clone(),
            ctrl: self.ctrl.clone(),
            persist: self.persist.clone(),
            pending_bg_reconfig: self.pending_bg_reconfig.clone(),
            zoom_percent: self.zoom_percent.clone(),
        }))
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        let mut hook = LiveProcCtx(context);
        self.process_with_hook(buffer, &mut hook)
    }
}

impl Prophet {
    fn process_with_hook<H: ProcessHostHook>(
        &mut self,
        buffer: &mut Buffer,
        context: &mut H,
    ) -> ProcessStatus {
        let num_frames = buffer.samples();
        let mut cmds = [Command {
            offset: 0,
            origin: ORIGIN_DAW_HOST,
            serial: 0,
            action: Action::PanicOrigin,
        }; MAX_HOST_CMDS_PER_BLOCK];
        let mut cmd_len = 0usize;

        for idx in 0..NUM_PARAMS {
            if idx == ParamId::OversamplingMode as usize {
                continue;
            }
            let cur = sanitize_param_value(idx, self.params.float_param(idx).value());
            if cur.to_bits() != self.last_host[idx].to_bits() {
                self.last_host[idx] = cur;
                Self::stage_daw_cmd(
                    &mut cmds,
                    &mut cmd_len,
                    &self.daw_serials,
                    0,
                    Action::Parameter {
                        id: idx as u16,
                        value: cur,
                    },
                );
            }
        }

        let glide = self.params.p83.value();
        if glide != self.last_glide {
            self.last_glide = glide;
            Self::stage_daw_cmd(
                &mut cmds,
                &mut cmd_len,
                &self.daw_serials,
                0,
                Action::Parameter {
                    id: GLIDE_ENABLED,
                    value: if glide { 1.0 } else { 0.0 },
                },
            );
        }

        let host_os = snap_oversampling(self.params.p63.value());
        let host_rev_en = self.params.reverb_enabled.value();
        let host_rev_kill = self.params.reverb_kill_tail.value();
        if host_os != self.requested_os
            || host_rev_en != self.requested_rev_enabled
            || host_rev_kill != self.requested_rev_kill
        {
            if host_rev_en != self.requested_rev_enabled {
                let wet = if host_rev_en {
                    sanitize_param_value(ParamId::ReverbWet as usize, self.params.p62.value())
                } else {
                    0.0
                };
                Self::stage_daw_cmd(
                    &mut cmds,
                    &mut cmd_len,
                    &self.daw_serials,
                    0,
                    Action::Parameter {
                        id: ParamId::ReverbWet as u16,
                        value: wet,
                    },
                );
            }
            self.requested_os = host_os;
            self.requested_rev_enabled = host_rev_en;
            self.requested_rev_kill = host_rev_kill;
            self.pending_bg_reconfig.store(true, Ordering::Release);
            context.execute_background(BgTask::HostReconfig);
        }

        while let Some(event) = context.next_event() {
            let offset = event.timing().min(num_frames as u32) as u16;
            match event {
                NoteEvent::NoteOn {
                    channel,
                    note,
                    velocity,
                    voice_id,
                    ..
                } => {
                    let key = note.min(127);
                    let nid = voice_id
                        .filter(|v| *v >= 0)
                        .map(|v| v as u32)
                        .unwrap_or_else(|| u32::from(key));
                    let ch = channel & 0x0f;
                    if velocity <= 0.0 {
                        Self::stage_daw_cmd(
                            &mut cmds,
                            &mut cmd_len,
                            &self.daw_serials,
                            offset,
                            Action::Midi {
                                status: 0x80 | ch,
                                data1: key,
                                data2: 0,
                                note_id: nid,
                            },
                        );
                    } else {
                        let vel = ((velocity.clamp(0.0, 1.0) * 127.0).round() as u8).clamp(1, 127);
                        Self::stage_daw_cmd(
                            &mut cmds,
                            &mut cmd_len,
                            &self.daw_serials,
                            offset,
                            Action::Midi {
                                status: 0x90 | ch,
                                data1: key,
                                data2: vel,
                                note_id: nid,
                            },
                        );
                    }
                }
                NoteEvent::NoteOff {
                    channel,
                    note,
                    voice_id,
                    ..
                }
                | NoteEvent::Choke {
                    channel,
                    note,
                    voice_id,
                    ..
                } => {
                    let key = note.min(127);
                    let nid = voice_id
                        .filter(|v| *v >= 0)
                        .map(|v| v as u32)
                        .unwrap_or_else(|| u32::from(key));
                    Self::stage_daw_cmd(
                        &mut cmds,
                        &mut cmd_len,
                        &self.daw_serials,
                        offset,
                        Action::Midi {
                            status: 0x80 | (channel & 0x0f),
                            data1: key,
                            data2: 0,
                            note_id: nid,
                        },
                    );
                }
                // Workaround per CCR-04: EnginePort v1 Ownership maps 0xD0 channel pressure cleanly
                // while rejecting 0xA0 poly key pressure as Unsupported.
                NoteEvent::PolyPressure {
                    channel,
                    pressure,
                    note,
                    voice_id,
                    ..
                } => {
                    let nid = voice_id
                        .filter(|v| *v >= 0)
                        .map(|v| v as u32)
                        .unwrap_or_else(|| u32::from(note.min(127)));
                    let p = ((pressure.clamp(0.0, 1.0) * 127.0).round() as u8).min(127);
                    Self::stage_daw_cmd(
                        &mut cmds,
                        &mut cmd_len,
                        &self.daw_serials,
                        offset,
                        Action::Midi {
                            status: 0xd0 | (channel & 0x0f),
                            data1: p,
                            data2: 0,
                            note_id: nid,
                        },
                    );
                }
                NoteEvent::MidiChannelPressure {
                    channel, pressure, ..
                } => {
                    let p = ((pressure.clamp(0.0, 1.0) * 127.0).round() as u8).min(127);
                    Self::stage_daw_cmd(
                        &mut cmds,
                        &mut cmd_len,
                        &self.daw_serials,
                        offset,
                        Action::Midi {
                            status: 0xd0 | (channel & 0x0f),
                            data1: p,
                            data2: 0,
                            note_id: 0,
                        },
                    );
                }
                NoteEvent::MidiPitchBend { channel, value, .. } => {
                    let raw14 = ((value.clamp(0.0, 1.0) * 16383.0).round() as u16).min(16383);
                    Self::stage_daw_cmd(
                        &mut cmds,
                        &mut cmd_len,
                        &self.daw_serials,
                        offset,
                        Action::Midi {
                            status: 0xe0 | (channel & 0x0f),
                            data1: (raw14 & 0x7f) as u8,
                            data2: ((raw14 >> 7) & 0x7f) as u8,
                            note_id: 0,
                        },
                    );
                }
                NoteEvent::MidiCC {
                    channel, cc, value, ..
                } => {
                    let v = ((value.clamp(0.0, 1.0) * 127.0).round() as u8).min(127);
                    Self::stage_daw_cmd(
                        &mut cmds,
                        &mut cmd_len,
                        &self.daw_serials,
                        offset,
                        Action::Midi {
                            status: 0xb0 | (channel & 0x0f),
                            data1: cc.min(127),
                            data2: v,
                            note_id: 0,
                        },
                    );
                }
                _ => {}
            }
        }

        let channels = buffer.as_slice();
        if num_frames == 0 || channels.len() < 2 {
            let mut empty_l = [];
            let mut empty_r = [];
            let _ = self.rt_slot.process_planar_f32(
                &mut empty_l,
                &mut empty_r,
                &cmds[..cmd_len],
                None,
                None,
                None,
            );
        } else {
            let (left_head, right_tail) = channels.split_at_mut(1);
            let left = &mut left_head[0][..num_frames];
            let right = &mut right_tail[0][..num_frames];
            let _ = self.rt_slot.process_planar_f32(
                left,
                right,
                &cmds[..cmd_len],
                None,
                None,
                None,
            );
        }

        let swap_id = self.rt_slot.slot.active_swap_id();
        if swap_id != self.last_swap_id {
            self.last_swap_id = swap_id;
            let cfg = self.rt_slot.slot.engine.configuration();
            self.requested_os = cfg.oversampling;
            let rev = self.rt_slot.slot.engine.reverb_state();
            self.requested_rev_enabled = rev.enabled;
            self.requested_rev_kill = rev.policy == ReverbTailPolicy::Kill;
            let lat = self.rt_slot.slot.engine.latency_samples();
            if lat != self.last_latency {
                self.last_latency = lat;
                context.set_latency_samples(lat);
            }
            context.execute_background(BgTask::PollControlPlane);
        } else if self.pending_bg_reconfig.load(Ordering::Relaxed) {
            self.frames_since_bg_poll = self.frames_since_bg_poll.saturating_add(num_frames as u64);
            let interval = u64::from(self.cached_sr / 10).max(512);
            if self.frames_since_bg_poll >= interval {
                self.frames_since_bg_poll = 0;
                context.execute_background(BgTask::PollControlPlane);
            }
        }

        self.update_compute_status()
    }
}

struct ProphetEditor {
    params: Arc<SynthParams>,
    ctrl: Arc<Mutex<HostControlPlane>>,
    persist: Arc<Mutex<PersistSnapshot>>,
    pending_bg_reconfig: Arc<AtomicBool>,
    zoom_percent: Arc<AtomicU32>,
}

struct SpawnedEditorGuard {
    _handle: Option<EmbeddedEditorHandle>,
}

impl Editor for ProphetEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        let parent_win = match parent {
            ParentWindowHandle::X11Window(xid) => ParentWindow::X11(xid),
            ParentWindowHandle::Win32Hwnd(hwnd) => ParentWindow::Win32(hwnd),
            ParentWindowHandle::AppKitNsView(ptr) => ParentWindow::Cocoa(ptr),
        };
        let sink = Arc::new(PluginHostBridge {
            params: self.params.clone(),
            ctrl: self.ctrl.clone(),
            persist: self.persist.clone(),
            gui_ctx: context,
            setter_lock: Mutex::new(()),
            pending_bg_reconfig: self.pending_bg_reconfig.clone(),
            zoom_percent: self.zoom_percent.clone(),
        });
        let (w, h) = self.size();
        let cfg = EditorViewConfig {
            title: "PROPHET-XII Virtual Analog Synthesizer".to_owned(),
            backend_tag: "clap-vst3".to_owned(),
            logical_width: w,
            logical_height: h,
            scale_factor: 1.0,
            resizable: false,
        };
        let handle = spawn_embedded_editor(parent_win, cfg, sink).ok();
        Box::new(SpawnedEditorGuard { _handle: handle })
    }

    fn size(&self) -> (u32, u32) {
        let zp = self.zoom_percent.load(Ordering::Relaxed).clamp(75, 200) as f32 / 100.0;
        (
            ((EDITOR_LOGICAL_WIDTH as f32) * zp).round() as u32,
            ((EDITOR_LOGICAL_HEIGHT as f32) * zp).round() as u32,
        )
    }

    fn set_scale_factor(&self, _factor: f32) -> bool {
        false
    }

    fn param_value_changed(&self, id: &str, _normalized_value: f32) {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        if id == "reverb_enabled" {
            ctrl.sound.reverb_enabled = self.params.reverb_enabled.value();
        } else if id == "reverb_kill_tail" {
            ctrl.sound.kill_tail = self.params.reverb_kill_tail.value();
        } else if id == "p83" {
            ctrl.sound.glide = self.params.p83.value();
        } else if let Some(rest) = id.strip_prefix('p') {
            if let Ok(idx) = rest.parse::<usize>() {
                if idx < NUM_PARAMS {
                    ctrl.sound.values[idx] =
                        sanitize_param_value(idx, self.params.float_param(idx).value());
                }
            }
        }
        if !ctrl.ready {
            ctrl.sync_pod_from_sound();
        }
        Prophet::sync_persist_from_ctrl(&ctrl, &self.persist);
    }

    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {}

    fn param_values_changed(&self) {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.sound.reverb_enabled = self.params.reverb_enabled.value();
        ctrl.sound.kill_tail = self.params.reverb_kill_tail.value();
        ctrl.sound.glide = self.params.p83.value();
        ctrl.sound.values = self.params.collect_values();
        if !ctrl.ready {
            ctrl.sync_pod_from_sound();
        }
        Prophet::sync_persist_from_ctrl(&ctrl, &self.persist);
    }
}

struct PluginHostBridge {
    params: Arc<SynthParams>,
    ctrl: Arc<Mutex<HostControlPlane>>,
    persist: Arc<Mutex<PersistSnapshot>>,
    gui_ctx: Arc<dyn GuiContext>,
    setter_lock: Mutex<()>,
    pending_bg_reconfig: Arc<AtomicBool>,
    zoom_percent: Arc<AtomicU32>,
}

impl PluginHostBridge {
    fn sync_param_float_if_changed(&self, idx: usize, new_val: f32) {
        if idx >= NUM_PARAMS {
            return;
        }
        let target = sanitize_param_value(idx, new_val);
        let p = self.params.float_param(idx);
        if p.value().to_bits() == target.to_bits() {
            return;
        }
        let _guard = self.setter_lock.lock().unwrap_or_else(|e| e.into_inner());
        let setter = ParamSetter::new(self.gui_ctx.as_ref());
        setter.begin_set_parameter(p);
        setter.set_parameter(p, target);
        setter.end_set_parameter(p);
    }

    fn sync_param_bool_if_changed(&self, p: &BoolParam, new_val: bool) {
        if p.value() == new_val {
            return;
        }
        let _guard = self.setter_lock.lock().unwrap_or_else(|e| e.into_inner());
        let setter = ParamSetter::new(self.gui_ctx.as_ref());
        setter.begin_set_parameter(p);
        setter.set_parameter(p, new_val);
        setter.end_set_parameter(p);
    }

    fn sync_sound_to_host_params(&self, sound: &SoundState) {
        self.sync_param_bool_if_changed(&self.params.reverb_enabled, sound.reverb_enabled);
        self.sync_param_bool_if_changed(&self.params.reverb_kill_tail, sound.kill_tail);
        for (i, &v) in sound.values.iter().enumerate() {
            self.sync_param_float_if_changed(i, v);
        }
        self.sync_param_bool_if_changed(&self.params.p83, sound.glide);
    }
}

impl HostBridgeSink for PluginHostBridge {
    fn snapshot_json(&self) -> String {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        let snap = ctrl.snapshot();
        Prophet::sync_persist_from_ctrl(&ctrl, &self.persist);
        ui_snapshot_to_json(&snap)
    }

    fn capture_ui_state(&self) -> UiSnapshot {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
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
                    self.zoom_percent.store((*zp).clamp(75, 200), Ordering::Release);
                    let _ = self.gui_ctx.request_resize();
                }
                return UiCommandReply::ok("UI zoom updated");
            }
        }

        let (reply, sound_snapshot) = {
            let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
            let r = ctrl.dispatch(cmd.clone());
            let sound = ctrl.sound.clone();
            Prophet::sync_persist_from_ctrl(&ctrl, &self.persist);
            (r, sound)
        };

        if reply.ok {
            match cmd {
                UiCommand::Param { id, value } => {
                    if id == GLIDE_ENABLED {
                        self.sync_param_bool_if_changed(&self.params.p83, value >= 0.5);
                    } else if usize::from(id) < NUM_PARAMS {
                        self.sync_param_float_if_changed(usize::from(id), value);
                    }
                }
                UiCommand::SetOversampling { factor } => {
                    self.sync_param_float_if_changed(
                        ParamId::OversamplingMode as usize,
                        f32::from(factor),
                    );
                    self.pending_bg_reconfig.store(true, Ordering::Release);
                }
                UiCommand::SetReverb { enabled, tail, .. } => {
                    if let Some(en) = enabled {
                        self.sync_param_bool_if_changed(&self.params.reverb_enabled, en);
                    }
                    if let Some(tp) = tail {
                        self.sync_param_bool_if_changed(
                            &self.params.reverb_kill_tail,
                            tp == prophet_host::WireTailPolicy::Kill,
                        );
                    }
                    self.pending_bg_reconfig.store(true, Ordering::Release);
                }
                UiCommand::FactoryPreset { .. } | UiCommand::PresetLoadJson { .. } => {
                    self.sync_sound_to_host_params(&sound_snapshot);
                    self.pending_bg_reconfig.store(true, Ordering::Release);
                }
                _ => {}
            }
        }
        reply
    }

    fn on_zoom_changed(&self, zoom: f32) {
        let zp = (zoom * 100.0).round().clamp(75.0, 200.0) as u32;
        self.zoom_percent.store(zp, Ordering::Release);
        let _ = self.gui_ctx.request_resize();
    }
}

impl ClapPlugin for Prophet {
    const CLAP_ID: &'static str = "org.prophet-xii.synth";
    const CLAP_DESCRIPTION: Option<&'static str> =
        Some("Independent 12-voice virtual analog instrument");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Prophet {
    const VST3_CLASS_ID: [u8; 16] = *b"ProphetXiiSynth1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nih_export_clap!(Prophet);
nih_export_vst3!(Prophet);

// Keep `OversampleFactor` explicitly referenced when only unit checks run under cfg(test).
const _: fn(u8) -> OversampleFactor = |f| match f {
    1 => OversampleFactor::X1,
    4 => OversampleFactor::X4,
    _ => OversampleFactor::X2,
};

#[cfg(test)]
mod tests {
    use super::*;
    use prophet_dsp::vector::{Point, Route, Stage, Topology};
    use prophet_host::{UiTopology, WireAlgorithm, WireTailPolicy};
    use std::collections::BTreeMap;

    struct DummyInitCtx {
        latency: AtomicU32,
    }

    impl InitContext<Prophet> for DummyInitCtx {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn execute(&self, _task: BgTask) {}
        fn set_latency_samples(&self, samples: u32) {
            self.latency.store(samples, Ordering::Relaxed);
        }
        fn set_current_voice_capacity(&self, _capacity: u32) {}
    }

    struct DummyProcessCtx {
        events: std::collections::VecDeque<PluginNoteEvent<Prophet>>,
        latency: AtomicU32,
        bg_tasks: Mutex<Vec<BgTask>>,
    }

    impl DummyProcessCtx {
        fn new() -> Self {
            Self {
                events: std::collections::VecDeque::new(),
                latency: AtomicU32::new(0),
                bg_tasks: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessHostHook for DummyProcessCtx {
        fn next_event(&mut self) -> Option<PluginNoteEvent<Prophet>> {
            self.events.pop_front()
        }
        fn execute_background(&self, task: BgTask) {
            self.bg_tasks.lock().unwrap().push(task);
        }
        fn set_latency_samples(&self, samples: u32) {
            self.latency.store(samples, Ordering::Relaxed);
        }
    }

    #[test]
    fn param_map_includes_86_params_and_persist_field() {
        let params = SynthParams::default();
        let map = params.param_map();
        assert_eq!(map.len(), 86);
        assert_eq!(map[0].0, "reverb_enabled");
        assert_eq!(map[1].0, "reverb_kill_tail");
        assert_eq!(map[2].0, "p0");
        assert_eq!(map[84].0, "p82");
        assert_eq!(map[85].0, "p83");
    }

    #[test]
    fn rc1_legacy_filter_state_migration_infers_p83_and_empty_preset_v2() {
        let mut params = BTreeMap::new();
        params.insert("p42".to_owned(), ParamValue::F32(0.3));
        let mut state = PluginState {
            version: "1.0.0".to_owned(),
            params,
            fields: BTreeMap::new(),
        };
        Prophet::filter_state(&mut state);
        match state.params.get("p83") {
            Some(ParamValue::Bool(true)) => {}
            other => panic!("Expected inferred p83=true from p42=0.3, got {other:?}"),
        }
        assert_eq!(state.fields.get("prophet_preset_v2").map(String::as_str), Some("\"\""));

        // Explicit p83=false with p42=0.3 must NOT be overwritten.
        let mut v2_params = BTreeMap::new();
        v2_params.insert("p42".to_owned(), ParamValue::F32(0.3));
        v2_params.insert("p83".to_owned(), ParamValue::Bool(false));
        let mut v2_state = PluginState {
            version: "1.0.0".to_owned(),
            params: v2_params,
            fields: BTreeMap::new(),
        };
        Prophet::filter_state(&mut v2_state);
        match v2_state.params.get("p83") {
            Some(ParamValue::Bool(false)) => {}
            other => panic!("Expected preserved explicit p83=false, got {other:?}"),
        }
    }

    #[test]
    fn compact_v2_persist_roundtrip_and_crc_under_64kb() {
        let p = Prophet::default();
        {
            let mut ctrl = p.ctrl.lock().unwrap();
            let reply = ctrl.dispatch(UiCommand::SetPatch {
                topology: UiTopology::Vector4,
                routes: [prophet_host::UiRoute::Frequency, prophet_host::UiRoute::Phase],
                ratios: [1.0, 2.0],
                stages: [
                    prophet_host::UiStage { target: prophet_host::UiPoint { x: 1.0, y: 0.0 }, millis: 50 },
                    prophet_host::UiStage { target: prophet_host::UiPoint { x: 0.0, y: 1.0 }, millis: 60 },
                    prophet_host::UiStage { target: prophet_host::UiPoint { x: -1.0, y: 0.0 }, millis: 70 },
                    prophet_host::UiStage { target: prophet_host::UiPoint { x: 0.0, y: -1.0 }, millis: 80 },
                    prophet_host::UiStage { target: prophet_host::UiPoint { x: 0.0, y: 0.0 }, millis: 90 },
                ],
                modulation: [
                    prophet_host::UiPoint { x: 0.1, y: 0.0 },
                    prophet_host::UiPoint { x: 0.0, y: 0.1 },
                    prophet_host::UiPoint { x: -0.1, y: 0.0 },
                ],
                loop_held: true,
            });
            assert!(reply.ok);
            let rev_reply = ctrl.dispatch(UiCommand::SetReverb {
                algorithm: Some(WireAlgorithm::DiffuseV1),
                enabled: None,
                tail: None,
            });
            assert!(rev_reply.ok);
            Prophet::sync_persist_from_ctrl(&ctrl, &p.persist);
        }

        let serialized_field = p.params.preset_v2.map(|s| s.clone());
        assert!(!serialized_field.is_empty());
        assert!(serialized_field.len() < 8192, "Default bank v2 JSON unexpectedly large: {}", serialized_field.len());
        let decoded = Preset::decode(serialized_field.as_bytes()).expect("CRC-32 preset decode failed");
        assert_eq!(decoded.sound.patch.topology, Topology::Vector4);
        assert_eq!(decoded.sound.algorithm, Algorithm::DiffuseV1);
        assert!(decoded.sound.patch.loop_held);
        assert!(decoded.bank.is_none());

        // Restore into a second isolated instance cleanly via PersistentField::set + initialize().
        let mut p2 = Prophet::default();
        p2.params.preset_v2.set(serialized_field);
        let io = AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        };
        let cfg = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: Some(64),
            max_buffer_size: 256,
            process_mode: ProcessMode::Realtime,
        };
        let mut init_ctx = DummyInitCtx {
            latency: AtomicU32::new(999),
        };
        assert!(p2.initialize(&io, &cfg, &mut init_ctx));
        assert_eq!(init_ctx.latency.load(Ordering::Relaxed), 16);
        assert_eq!(p2.rt_slot.slot.engine.patch().topology, Topology::Vector4);
        assert_eq!(p2.rt_slot.slot.engine.reverb_state().algorithm, Algorithm::DiffuseV1);

        // Two instances must remain isolated.
        assert_eq!(
            p2.rt_slot.slot.engine.patch().stages[0],
            Stage {
                target: Point { x: 1.0, y: 0.0 },
                millis: 50,
            }
        );
        assert_eq!(
            p2.rt_slot.slot.engine.patch().routes,
            [Route::Frequency, Route::Phase]
        );
    }

    #[test]
    fn oversampling_latency_and_dynamic_process_status_and_poly_pressure_work() {
        let mut p = Prophet::default();
        let io = AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        };
        let cfg = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: Some(64),
            max_buffer_size: 256,
            process_mode: ProcessMode::Realtime,
        };
        let mut init_ctx = DummyInitCtx {
            latency: AtomicU32::new(999),
        };
        assert!(p.initialize(&io, &cfg, &mut init_ctx));
        assert_eq!(init_ctx.latency.load(Ordering::Relaxed), 16);
        assert_eq!(p.engine_latency(), 16);

        let mut l = [0.0f32; 128];
        let mut r = [0.0f32; 128];
        let mut ctx = DummyProcessCtx::new();

        // Silent idle start -> ProcessStatus::Normal
        {
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(128, |s| {
                    s.push(&mut l);
                    s.push(&mut r);
                });
            }
            assert_eq!(p.process_with_hook(&mut buf, &mut ctx), ProcessStatus::Normal);
        }

        // NoteOn + PolyPressure + Choke
        ctx.events.push_back(NoteEvent::NoteOn {
            timing: 8,
            voice_id: Some(7),
            channel: 0,
            note: 60,
            velocity: 0.8,
        });
        ctx.events.push_back(NoteEvent::PolyPressure {
            timing: 16,
            voice_id: Some(7),
            channel: 0,
            note: 60,
            pressure: 0.6,
        });
        {
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(128, |s| {
                    s.push(&mut l);
                    s.push(&mut r);
                });
            }
            assert_eq!(p.process_with_hook(&mut buf, &mut ctx), ProcessStatus::KeepAlive);
            assert!(l[..8].iter().all(|s| *s == 0.0));
            let energy: f32 = l.iter().map(|s| s * s).sum();
            assert!(energy > 0.001, "Expected audible energy after NoteOn");
        }

        // Release note via Choke BEFORE swapping engine and verify KeepAlive/Tail transition
        ctx.events.push_back(NoteEvent::Choke {
            timing: 0,
            voice_id: Some(7),
            channel: 0,
            note: 60,
        });
        {
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(128, |s| {
                    s.push(&mut l);
                    s.push(&mut r);
                });
            }
            let st = p.process_with_hook(&mut buf, &mut ctx);
            assert!(matches!(st, ProcessStatus::KeepAlive | ProcessStatus::Tail(_)));
        }

        // Trigger OS swap to 4x via control plane (like UI) and execute swap in audio thread
        {
            let mut ctrl = p.ctrl.lock().unwrap();
            let rep = ctrl.dispatch(UiCommand::SetOversampling { factor: 4 });
            assert!(rep.ok);
        }
        {
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(128, |s| {
                    s.push(&mut l);
                    s.push(&mut r);
                });
            }
            let _ = p.process_with_hook(&mut buf, &mut ctx);
            assert_eq!(p.engine_latency(), 24);
            assert_eq!(ctx.latency.load(Ordering::Relaxed), 24);
            let tasks = ctx.bg_tasks.lock().unwrap();
            assert!(!tasks.is_empty());
        }

        // Also test Tail -> Normal via Kill-tail bypass when idle on second clean instance
        let mut p_kill = Prophet::default();
        assert!(p_kill.initialize(&io, &cfg, &mut DummyInitCtx { latency: AtomicU32::new(0) }));
        {
            let mut ctrl = p_kill.ctrl.lock().unwrap();
            let _ = ctrl.dispatch(UiCommand::SetReverb {
                algorithm: None,
                enabled: Some(false),
                tail: Some(WireTailPolicy::Kill),
            });
        }
        {
            let mut buf = Buffer::default();
            unsafe {
                buf.set_slices(128, |s| {
                    s.push(&mut l);
                    s.push(&mut r);
                });
            }
            let mut kctx = DummyProcessCtx::new();
            assert_eq!(p_kill.process_with_hook(&mut buf, &mut kctx), ProcessStatus::Normal);
        }
    }

    struct DummyGuiCtx {
        resizes: AtomicU32,
        param_sets: Mutex<Vec<(ParamPtr, f32)>>,
    }

    impl DummyGuiCtx {
        fn new() -> Self {
            Self {
                resizes: AtomicU32::new(0),
                param_sets: Mutex::new(Vec::new()),
            }
        }
    }

    impl GuiContext for DummyGuiCtx {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            self.resizes.fetch_add(1, Ordering::Relaxed);
            true
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            self.param_sets.lock().unwrap().push((param, normalized));
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> PluginState {
            PluginState {
                version: "1.0.0".to_owned(),
                params: BTreeMap::new(),
                fields: BTreeMap::new(),
            }
        }
        fn set_state(&self, _state: PluginState) {}
    }

    #[test]
    fn editor_and_bridge_sync_params_and_zoom() {
        let mut p = Prophet::default();
        let io = AudioIOLayout {
            main_input_channels: None,
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        };
        let cfg = BufferConfig {
            sample_rate: 48_000.0,
            min_buffer_size: Some(64),
            max_buffer_size: 256,
            process_mode: ProcessMode::Realtime,
        };
        let mut init_ctx = DummyInitCtx {
            latency: AtomicU32::new(0),
        };
        assert!(p.initialize(&io, &cfg, &mut init_ctx));

        let gui_ctx = Arc::new(DummyGuiCtx::new());
        let editor = ProphetEditor {
            params: p.params.clone(),
            ctrl: p.ctrl.clone(),
            persist: p.persist.clone(),
            pending_bg_reconfig: p.pending_bg_reconfig.clone(),
            zoom_percent: p.zoom_percent.clone(),
        };
        assert_eq!(editor.size(), (EDITOR_LOGICAL_WIDTH, EDITOR_LOGICAL_HEIGHT));

        let bridge = PluginHostBridge {
            params: p.params.clone(),
            ctrl: p.ctrl.clone(),
            persist: p.persist.clone(),
            gui_ctx: gui_ctx.clone(),
            setter_lock: Mutex::new(()),
            pending_bg_reconfig: p.pending_bg_reconfig.clone(),
            zoom_percent: p.zoom_percent.clone(),
        };

        // 1. UI zoom updates Editor::size() and notifies GuiContext::request_resize
        let z_rep = bridge.dispatch_command(UiCommand::HostControl {
            op: "setUiZoom".to_owned(),
            enabled: None,
            device: None,
            sample_rate: None,
            buffer_frames: Some(150),
            midi_port: None,
        });
        assert!(z_rep.ok);
        assert_eq!(gui_ctx.resizes.load(Ordering::Relaxed), 1);
        assert_eq!(editor.size(), (1770, 1080));

        // 2. UI parameter change (FilterCutoff=23 -> 1200 Hz) schedules GuiContext param update
        let p_rep = bridge.dispatch_command(UiCommand::Param {
            id: ParamId::FilterCutoff as u16,
            value: 1200.0,
        });
        assert!(p_rep.ok, "FilterCutoff Param command failed: {}", p_rep.message);
        assert!(!gui_ctx.param_sets.lock().unwrap().is_empty());

        // 3. Reverting zoom via on_zoom_changed clamps cleanly
        bridge.on_zoom_changed(1.0);
        assert_eq!(editor.size(), (EDITOR_LOGICAL_WIDTH, EDITOR_LOGICAL_HEIGHT));
        assert!(!bridge.snapshot_json().is_empty());

        // 4. Host notification updates Editor shadow state cleanly
        editor.param_values_changed();
        editor.param_value_changed("reverb_enabled", 0.0);
    }
}
