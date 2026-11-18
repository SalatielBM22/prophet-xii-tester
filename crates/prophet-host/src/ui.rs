// SPDX-License-Identifier: GPL-3.0-only
//! Unified typed `UiTransport` wire types shared identically across:
//! - B1 Android WebView bridge (`Native.dispatch` / `Native.poll`)
//! - B2 VST3 / CLAP embedded GUI
//! - B3 LV2 UI & Native Desktop Standalone window
//! - B4 React frontend (`ui/src/protocol/`)

use crate::{
    analysis::FilterResponseReport,
    preset::{FactoryPresetEntry, Performance},
    queue::PhysicalMidiSource,
    wire::{SnapshotPod, WireAlgorithm, WireSnapshot, WireStatus, WireTailPolicy},
};
use prophet_control::{
    music::{Kind, Order, Pattern, PatternBank, Step},
    port::{Action, Command, Status, GLIDE_ENABLED},
    state::SoundState,
};
use prophet_dsp::{
    fx::reverb::Algorithm,
    vector::{Patch, Point, Route, Stage, Topology},
    KernelTelemetrySnapshot,
};
use prophet_params::{ParamId, NUM_PARAMS};
use serde::{Deserialize, Serialize};

pub const PARAM_UI_COUNT: usize = NUM_PARAMS + 1; // p0..p82 + p83 (Glide Enabled)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiTopology {
    Classic2,
    Extended3,
    Extended4,
    Vector4,
}

impl From<Topology> for UiTopology {
    fn from(t: Topology) -> Self {
        match t {
            Topology::Classic2 => Self::Classic2,
            Topology::Extended3 => Self::Extended3,
            Topology::Extended4 => Self::Extended4,
            Topology::Vector4 => Self::Vector4,
        }
    }
}

impl From<UiTopology> for Topology {
    fn from(t: UiTopology) -> Self {
        match t {
            UiTopology::Classic2 => Self::Classic2,
            UiTopology::Extended3 => Self::Extended3,
            UiTopology::Extended4 => Self::Extended4,
            UiTopology::Vector4 => Self::Vector4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiRoute {
    Sum,
    Frequency,
    Phase,
    Sync,
    Ring,
}

impl From<Route> for UiRoute {
    fn from(r: Route) -> Self {
        match r {
            Route::Sum => Self::Sum,
            Route::Frequency => Self::Frequency,
            Route::Phase => Self::Phase,
            Route::Sync => Self::Sync,
            Route::Ring => Self::Ring,
        }
    }
}

impl From<UiRoute> for Route {
    fn from(r: UiRoute) -> Self {
        match r {
            UiRoute::Sum => Self::Sum,
            UiRoute::Frequency => Self::Frequency,
            UiRoute::Phase => Self::Phase,
            UiRoute::Sync => Self::Sync,
            UiRoute::Ring => Self::Ring,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPoint {
    pub x: f32,
    pub y: f32,
}

impl From<Point> for UiPoint {
    fn from(p: Point) -> Self {
        Self { x: p.x, y: p.y }
    }
}

impl From<UiPoint> for Point {
    fn from(p: UiPoint) -> Self {
        Self { x: p.x, y: p.y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiStage {
    pub target: UiPoint,
    pub millis: u16,
}

impl From<Stage> for UiStage {
    fn from(s: Stage) -> Self {
        Self {
            target: s.target.into(),
            millis: s.millis,
        }
    }
}

impl From<UiStage> for Stage {
    fn from(s: UiStage) -> Self {
        Self {
            target: s.target.into(),
            millis: s.millis,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPatchState {
    pub topology: UiTopology,
    pub routes: [UiRoute; 2],
    pub ratios: [f32; 2],
    pub position: UiPoint,
    pub weights: [f32; 4],
    pub stages: [UiStage; 5],
    pub modulation: [UiPoint; 3],
    pub loop_held: bool,
    pub idle_for_topology_change: bool,
}

impl UiPatchState {
    pub fn from_pod(pod: &SnapshotPod) -> Self {
        let p = pod.patch;
        Self {
            topology: p.topology.into(),
            routes: p.routes.map(UiRoute::from),
            ratios: p.ratios,
            position: p.position.into(),
            weights: pod.resting_or_active_vector_weights(),
            stages: p.stages.map(UiStage::from),
            modulation: p.modulation.map(UiPoint::from),
            loop_held: p.loop_held,
            idle_for_topology_change: !pod
                .voices
                .iter()
                .any(|v| v.active || v.gate || v.releasing),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiArpOrder {
    Up,
    Down,
    UpDown,
    AsPlayed,
    Random,
    Chord,
}

impl From<Order> for UiArpOrder {
    fn from(o: Order) -> Self {
        match o {
            Order::Up => Self::Up,
            Order::Down => Self::Down,
            Order::UpDown => Self::UpDown,
            Order::AsPlayed => Self::AsPlayed,
            Order::Random => Self::Random,
            Order::Chord => Self::Chord,
        }
    }
}

impl From<UiArpOrder> for Order {
    fn from(o: UiArpOrder) -> Self {
        match o {
            UiArpOrder::Up => Self::Up,
            UiArpOrder::Down => Self::Down,
            UiArpOrder::UpDown => Self::UpDown,
            UiArpOrder::AsPlayed => Self::AsPlayed,
            UiArpOrder::Random => Self::Random,
            UiArpOrder::Chord => Self::Chord,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UiStepKind {
    Rest,
    Note,
    Tie,
}

impl From<Kind> for UiStepKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Rest => Self::Rest,
            Kind::Note => Self::Note,
            Kind::Tie => Self::Tie,
        }
    }
}

impl From<UiStepKind> for Kind {
    fn from(k: UiStepKind) -> Self {
        match k {
            UiStepKind::Rest => Self::Rest,
            UiStepKind::Note => Self::Note,
            UiStepKind::Tie => Self::Tie,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiStep {
    pub kind: UiStepKind,
    pub key: u8,
    pub velocity: u8,
    pub accent: bool,
    pub glide: bool,
    pub gate: u8,
    pub ratchet: u8,
}

impl From<Step> for UiStep {
    fn from(s: Step) -> Self {
        Self {
            kind: s.kind.into(),
            key: s.key,
            velocity: s.velocity,
            accent: s.accent,
            glide: s.glide,
            gate: s.gate,
            ratchet: s.ratchet,
        }
    }
}

impl From<UiStep> for Step {
    fn from(s: UiStep) -> Self {
        Self {
            kind: s.kind.into(),
            key: s.key,
            velocity: s.velocity,
            accent: s.accent,
            glide: s.glide,
            gate: s.gate,
            ratchet: s.ratchet,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPattern {
    pub index: u8,
    pub len: u8,
    pub steps: Vec<UiStep>,
}

impl UiPattern {
    pub fn from_bank(bank: &PatternBank, index: usize) -> Self {
        let idx = index.min(63);
        let pat: &Pattern = &bank.0[idx];
        Self {
            index: idx as u8,
            len: pat.len,
            steps: pat.steps.iter().copied().map(UiStep::from).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMusicState {
    pub playing: bool,
    pub active_pattern: u8,
    pub pattern_len: u8,
    pub step_index: u64,
    pub tempo_milli: u32,
    pub swing: u8,
    pub arp_enabled: bool,
    pub arp_order: UiArpOrder,
    pub arp_octaves: u8,
    pub arp_latch: bool,
    pub arp_seed: u32,
    pub current_pattern: UiPattern,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMidiLearnState {
    pub learning_target: Option<u16>,
    pub cc_map: Vec<Option<u16>>,
    pub physical_source: PhysicalMidiSource,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiEnvelopeTelemetry {
    pub stage: u8,
    pub elapsed_samples: u64,
    pub progress: f32,
    pub level: f32,
    pub timed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiKernelTelemetry {
    pub active_voice_mask: u16,
    pub voice_vca_levels: Vec<f32>,
    pub voice_filt_env_levels: Vec<f32>,
    pub voice_midi_notes: Vec<u8>,
    pub amp_envelopes: Vec<UiEnvelopeTelemetry>,
    pub filter_envelopes: Vec<UiEnvelopeTelemetry>,
    pub vector_weights: Vec<Option<[f32; 4]>>,
}

impl UiKernelTelemetry {
    pub fn from_pod(pod: &SnapshotPod) -> Self {
        let k: &KernelTelemetrySnapshot = &pod.kernel_telemetry;
        let map_env = |e: &prophet_dsp::env_cem3310::EnvelopeTelemetry| UiEnvelopeTelemetry {
            stage: e.stage,
            elapsed_samples: e.elapsed_samples,
            progress: e.progress,
            level: e.level,
            timed: e.timed,
        };
        Self {
            active_voice_mask: k.active_voice_mask,
            voice_vca_levels: k.voice_vca_levels.to_vec(),
            voice_filt_env_levels: k.voice_filt_env_levels.to_vec(),
            voice_midi_notes: k.voice_midi_notes.to_vec(),
            amp_envelopes: k.amp_envelopes.iter().map(map_env).collect(),
            filter_envelopes: k.filter_envelopes.iter().map(map_env).collect(),
            vector_weights: pod.vector_weights.to_vec(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiPresetSummary {
    pub id: usize,
    pub name: String,
    pub category: String,
    pub glide: bool,
}

impl From<&FactoryPresetEntry> for UiPresetSummary {
    fn from(e: &FactoryPresetEntry) -> Self {
        Self {
            id: e.id,
            name: e.name.clone(),
            category: e.category.clone(),
            glide: e.glide,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiNotice {
    pub seq: u64,
    pub ok: bool,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSnapshot {
    pub version: u16,
    pub backend: String,
    pub ready: bool,
    pub engine_port: WireSnapshot,
    pub parameters: Vec<f32>,
    pub scope: Vec<f32>,
    pub patch: UiPatchState,
    pub kernel_telemetry: UiKernelTelemetry,
    pub music: Option<UiMusicState>,
    pub midi_learn: UiMidiLearnState,
    pub drive_makeup: bool,
    pub reverb_bypass_effective: bool,
    pub reverb_tail_effective: WireTailPolicy,
    pub presets: Vec<UiPresetSummary>,
    pub filter_analysis: Option<FilterResponseReport>,
    pub last_notice: Option<UiNotice>,
    pub diagnostics: Option<serde_json::Value>,
}

/// Canonical EnginePort v1 + Host Worker command envelope accepted across all UI transports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UiCommand {
    /// Parameter change (`id` in `0..=83`, where `83 = Glide Enabled`).
    Param { id: u16, value: f32 },
    /// Live MIDI event routed on `ORIGIN_LOCAL_UI` (never echoed when displaying EnginePort notes).
    Midi {
        status: u8,
        data1: u8,
        data2: u8,
        #[serde(default)]
        note_id: Option<u32>,
    },
    /// Vector4 XY point inside diamond `|x| + |y| <= 1`.
    VectorXY { x: f32, y: f32 },
    /// Release notes held by local UI (or all origins if `all_origins` is true).
    Panic {
        #[serde(default)]
        all_origins: bool,
    },
    /// Load one of the 10 immutable factory presets off-audio and swap safely.
    FactoryPreset { index: usize },
    /// Decode, CRC-validate, prepare, and swap a v1/v2 `prophet_preset` JSON payload off-audio.
    PresetLoadJson { json: String },
    /// Encode current state via `prophet_preset` v2 codec with CRC-32 off-audio.
    PresetSaveJson { name: String, category: String },
    /// Switch active oversampling (`1 | 2 | 4`) off-audio via `SoundState::prepare`.
    SetOversampling { factor: u8 },
    /// Configure oscillator topology, routing, ratios, vector env, and modulation depths.
    SetPatch {
        topology: UiTopology,
        routes: [UiRoute; 2],
        ratios: [f32; 2],
        stages: [UiStage; 5],
        modulation: [UiPoint; 3],
        loop_held: bool,
    },
    /// Configure reverb algorithm (`LegacyRC1 | DiffuseV1`, idle-only switch) and tail policy.
    SetReverb {
        #[serde(default)]
        algorithm: Option<WireAlgorithm>,
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        tail: Option<WireTailPolicy>,
    },
    /// Toggle drive makeup gain normalization cleanly off-audio.
    SetDriveMakeup { enabled: bool },
    /// MIDI Learn lifecycle backed by `CcLearnRouter`.
    MidiLearn {
        action: String,
        #[serde(default)]
        param_id: Option<u16>,
        #[serde(default)]
        cc: Option<u8>,
    },
    /// Select canonical physical MIDI controller owner (`native | browser`) with `PanicOrigin` handoff.
    SetPhysicalMidiSource { source: PhysicalMidiSource },
    /// Sequencer / Arpeggiator runtime control matching `prophet_control::music`.
    MusicControl {
        op: String,
        #[serde(default)]
        pattern: Option<u8>,
        #[serde(default)]
        pattern_len: Option<u8>,
        #[serde(default)]
        step_index: Option<u8>,
        #[serde(default)]
        step: Option<UiStep>,
        #[serde(default)]
        tempo_milli: Option<u32>,
        #[serde(default)]
        swing: Option<u8>,
        #[serde(default)]
        arp_enabled: Option<bool>,
        #[serde(default)]
        arp_order: Option<UiArpOrder>,
        #[serde(default)]
        arp_octaves: Option<u8>,
        #[serde(default)]
        arp_latch: Option<bool>,
        #[serde(default)]
        arp_seed: Option<u32>,
        #[serde(default)]
        grid_step: Option<u32>,
        #[serde(default)]
        frame: Option<u64>,
    },
    /// Request bounded off-thread VCF analysis backed by `prophet_analysis_core::validity::measure`.
    AnalyzeFilter {
        #[serde(default)]
        sample_rate: Option<f64>,
    },
    /// Host platform audio/device/developer controls (start/stop/reconnect/dev-bridge/device-select).
    HostControl {
        op: String,
        #[serde(default)]
        enabled: Option<bool>,
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        sample_rate: Option<u32>,
        #[serde(default)]
        buffer_frames: Option<u32>,
        #[serde(default)]
        midi_port: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiCommandReply {
    pub ok: bool,
    pub status: Option<WireStatus>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_analysis: Option<FilterResponseReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl UiCommandReply {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            status: Some(WireStatus::Applied),
            message: message.into(),
            swap_id: None,
            preset_json: None,
            filter_analysis: None,
            data: None,
        }
    }

    pub fn err(status: Option<Status>, message: impl Into<String>) -> Self {
        Self {
            ok: false,
            status: status.map(WireStatus::from),
            message: message.into(),
            swap_id: None,
            preset_json: None,
            filter_analysis: None,
            data: None,
        }
    }
}

/// Converts a `UiCommand` targeting the real-time stream into a single `EnginePort` `Command`.
pub fn ui_to_rt_command(cmd: &UiCommand, origin: u8, serial: u32, offset: u16) -> Option<Command> {
    match cmd {
        UiCommand::Param { id, value } => {
            if *id > GLIDE_ENABLED || !value.is_finite() {
                return None;
            }
            if *id == ParamId::OversamplingMode as u16 {
                return None;
            }
            Some(Command {
                offset,
                origin,
                serial,
                action: Action::Parameter {
                    id: *id,
                    value: *value,
                },
            })
        }
        UiCommand::Midi {
            status,
            data1,
            data2,
            note_id,
        } => {
            if !(128..=239).contains(status) || *data1 > 127 || *data2 > 127 {
                return None;
            }
            let resolved_id = note_id.unwrap_or_else(|| u32::from(*data1));
            Some(Command {
                offset,
                origin,
                serial,
                action: Action::Midi {
                    status: *status,
                    data1: *data1,
                    data2: *data2,
                    note_id: resolved_id,
                },
            })
        }
        UiCommand::VectorXY { x, y } => {
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            Some(Command {
                offset,
                origin,
                serial,
                action: Action::VectorXY { x: *x, y: *y },
            })
        }
        UiCommand::Panic { .. } => Some(Command {
            offset,
            origin,
            serial,
            action: Action::PanicOrigin,
        }),
        _ => None,
    }
}

/// Shadow copy of `SoundState` metadata that matches what the running/target engine expects.
pub fn apply_ui_patch_fields(
    state: &mut SoundState,
    topology: UiTopology,
    routes: [UiRoute; 2],
    ratios: [f32; 2],
    stages: [UiStage; 5],
    modulation: [UiPoint; 3],
    loop_held: bool,
) -> Result<(), &'static str> {
    let mut candidate = Patch {
        topology: topology.into(),
        routes: routes.map(Route::from),
        ratios,
        position: state.patch.position,
        modulation: modulation.map(Point::from),
        stages: stages.map(Stage::from),
        loop_held,
    };
    if !candidate.position.valid() {
        candidate.position = Point::default();
    }
    if !candidate.valid() {
        return Err("Invalid Vector4/oscillator patch parameters or coordinates outside diamond |x|+|y|<=1");
    }
    state.patch = candidate;
    Ok(())
}

/// Helper converting `SnapshotPod` + `MusicShadow` into full UI parameters array `[p0..p82, p83]`.
pub fn ui_parameters_from_pod(pod: &SnapshotPod) -> Vec<f32> {
    let mut out = Vec::with_capacity(PARAM_UI_COUNT);
    out.extend_from_slice(&pod.values);
    out.push(if pod.glide_enabled { 1.0 } else { 0.0 });
    out
}

/// Builds a `Performance` record matching `prophet_preset` from scalar shadow fields.
pub fn performance_record(
    tempo_milli: u32,
    swing: u8,
    pattern: u8,
    order: Order,
    octaves: u8,
    latch: bool,
    seed: u32,
) -> Performance {
    let mode = match order {
        Order::Up => 0,
        Order::Down => 1,
        Order::UpDown => 2,
        Order::AsPlayed => 3,
        Order::Random => 4,
        Order::Chord => 5,
    };
    Performance {
        tempo_milli,
        swing,
        pattern,
        mode,
        octaves,
        latch,
        seed,
    }
}

/// Maps host-level algorithm enum cleanly.
pub fn algorithm_name(alg: Algorithm) -> &'static str {
    match alg {
        Algorithm::LegacyRC1 => "LegacyRC1",
        Algorithm::DiffuseV1 => "DiffuseV1",
    }
}
