// SPDX-License-Identifier: GPL-3.0-only
//! EnginePort v1 wire JSON types, strict validation against `docs/wo-a/control-v1.schema.json`,
//! and zero-allocation audio-thread snapshot POD capture.

use prophet_control::{
    port::{Ack, Action, Command, Prepare, Status, GLIDE_ENABLED, MAX_EVENTS, ORIGINS, VERSION},
    PreparedEngine, VoiceState,
};
use prophet_dsp::{
    fx::{
        reverb::{Algorithm, TailState},
        solaris_reverb::ReverbTailPolicy,
    },
    vector::{Patch, Point, Topology},
    KernelTelemetrySnapshot,
};
use prophet_params::{ParamId, NUM_PARAMS, PARAM_SPECS};
use serde::{Deserialize, Serialize};

pub const CAP_ENGINE_PORT_V1: u32 = 1 << 0;
pub const CAP_GLIDE_ENABLED: u32 = 1 << 1;
pub const CAP_REVERB_STATE: u32 = 1 << 2;
pub const CAP_SELECT_IDLE_REVERB: u32 = 1 << 3;
pub const CAP_OSCILLATOR_PATCH: u32 = 1 << 4;
pub const CAP_NOTE_SCHEDULER: u32 = 1 << 5;
pub const CAP_VECTOR_XY: u32 = 1 << 6;

const ENGINE_CAP_TABLE: &[(u32, &str)] = &[
    (CAP_ENGINE_PORT_V1, "enginePortV1"),
    (CAP_GLIDE_ENABLED, "glideEnabled"),
    (CAP_REVERB_STATE, "reverbState"),
    (CAP_SELECT_IDLE_REVERB, "selectIdleReverb"),
    (CAP_OSCILLATOR_PATCH, "oscillatorPatch"),
    (CAP_NOTE_SCHEDULER, "noteScheduler"),
    (CAP_VECTOR_XY, "vectorXY"),
];

pub fn encode_capabilities_mask<'a>(caps: impl IntoIterator<Item = &'a str>) -> u32 {
    let mut mask = 0u32;
    for c in caps {
        for &(bit, name) in ENGINE_CAP_TABLE {
            if c == name {
                mask |= bit;
            }
        }
    }
    mask
}

pub fn intersect_capabilities(engine_mask: u32, host_supported: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for &(bit, name) in ENGINE_CAP_TABLE {
        if (engine_mask & bit) != 0 && host_supported.contains(&name) {
            out.push(name.to_owned());
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireValidationError {
    UnsupportedVersion(u16),
    InvalidFrames(u16),
    TooManyCommands(usize),
    InvalidOffset { index: usize, offset: u16, frames: u16 },
    UnsortedOffset { index: usize, prev: u16, offset: u16 },
    InvalidOrigin(u8),
    InvalidSerial(u32),
    InvalidMidi,
    InvalidParameterId(u16),
    NonFiniteValue,
    InvalidVectorCoordinates,
    InvalidDecimalU64(String),
    InvalidSampleRate(u32),
    InvalidMaxFrames(u16),
    InvalidLatency(u32),
    TooManyNotes(usize),
    InvalidNote,
    TooManyVoices(usize),
    InvalidVoice,
    TooManyCapabilities(usize),
    InvalidCpuPercent,
}

pub fn is_valid_u64_decimal(s: &str) -> bool {
    if s == "0" {
        return true;
    }
    !s.is_empty()
        && s.len() <= 20
        && !s.starts_with('0')
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<u64>().is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireStatus {
    Applied,
    Duplicate,
    Capacity,
    Unsupported,
    Invalid,
}

impl From<Status> for WireStatus {
    fn from(s: Status) -> Self {
        match s {
            Status::Applied => Self::Applied,
            Status::Duplicate => Self::Duplicate,
            Status::Capacity => Self::Capacity,
            Status::Unsupported => Self::Unsupported,
            Status::Invalid => Self::Invalid,
        }
    }
}

impl From<WireStatus> for Status {
    fn from(s: WireStatus) -> Self {
        match s {
            WireStatus::Applied => Self::Applied,
            WireStatus::Duplicate => Self::Duplicate,
            WireStatus::Capacity => Self::Capacity,
            WireStatus::Unsupported => Self::Unsupported,
            WireStatus::Invalid => Self::Invalid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WireAlgorithm {
    LegacyRC1,
    DiffuseV1,
}

impl From<Algorithm> for WireAlgorithm {
    fn from(a: Algorithm) -> Self {
        match a {
            Algorithm::LegacyRC1 => Self::LegacyRC1,
            Algorithm::DiffuseV1 => Self::DiffuseV1,
        }
    }
}

impl From<WireAlgorithm> for Algorithm {
    fn from(a: WireAlgorithm) -> Self {
        match a {
            WireAlgorithm::LegacyRC1 => Self::LegacyRC1,
            WireAlgorithm::DiffuseV1 => Self::DiffuseV1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WireTailPolicy {
    Spill,
    Kill,
}

impl From<ReverbTailPolicy> for WireTailPolicy {
    fn from(p: ReverbTailPolicy) -> Self {
        match p {
            ReverbTailPolicy::Spill => Self::Spill,
            ReverbTailPolicy::Kill => Self::Kill,
        }
    }
}

impl From<WireTailPolicy> for ReverbTailPolicy {
    fn from(p: WireTailPolicy) -> Self {
        match p {
            WireTailPolicy::Spill => Self::Spill,
            WireTailPolicy::Kill => Self::Kill,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WireAction {
    #[serde(rename = "midi")]
    Midi {
        status: u8,
        data1: u8,
        data2: u8,
        #[serde(rename = "noteId")]
        note_id: u32,
    },
    #[serde(rename = "parameter")]
    Parameter { id: u16, value: f32 },
    #[serde(rename = "vectorXY")]
    VectorXY { x: f32, y: f32 },
    #[serde(rename = "panicOrigin")]
    PanicOrigin,
}

impl From<Action> for WireAction {
    fn from(a: Action) -> Self {
        match a {
            Action::Midi {
                status,
                data1,
                data2,
                note_id,
            } => Self::Midi {
                status,
                data1,
                data2,
                note_id,
            },
            Action::Parameter { id, value } => Self::Parameter { id, value },
            Action::VectorXY { x, y } => Self::VectorXY { x, y },
            Action::PanicOrigin => Self::PanicOrigin,
        }
    }
}

impl From<WireAction> for Action {
    fn from(a: WireAction) -> Self {
        match a {
            WireAction::Midi {
                status,
                data1,
                data2,
                note_id,
            } => Self::Midi {
                status,
                data1,
                data2,
                note_id,
            },
            WireAction::Parameter { id, value } => Self::Parameter { id, value },
            WireAction::VectorXY { x, y } => Self::VectorXY { x, y },
            WireAction::PanicOrigin => Self::PanicOrigin,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireCommand {
    pub offset: u16,
    pub origin: u8,
    pub serial: u32,
    pub action: WireAction,
}

impl From<Command> for WireCommand {
    fn from(c: Command) -> Self {
        Self {
            offset: c.offset,
            origin: c.origin,
            serial: c.serial,
            action: c.action.into(),
        }
    }
}

impl From<WireCommand> for Command {
    fn from(c: WireCommand) -> Self {
        Self {
            offset: c.offset,
            origin: c.origin,
            serial: c.serial,
            action: c.action.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireBlock {
    pub version: u16,
    pub frames: u16,
    pub commands: Vec<WireCommand>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireAck {
    pub version: u16,
    pub origin: u8,
    pub serial: u32,
    pub status: WireStatus,
    pub revision: String,
}

impl WireAck {
    pub fn from_port(ack: Ack) -> Self {
        Self {
            version: VERSION,
            origin: ack.origin,
            serial: ack.serial,
            status: ack.status.into(),
            revision: ack.revision.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireNote {
    pub origin: u8,
    pub channel: u8,
    pub key: u8,
    pub note_id: u32,
    pub pressed: bool,
    pub sustained: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireVoice {
    pub index: u8,
    pub key: u8,
    pub gate: bool,
    pub releasing: bool,
    pub detune_semis: f32,
    pub pan: f32,
}

impl From<VoiceState> for WireVoice {
    fn from(v: VoiceState) -> Self {
        Self {
            index: v.index,
            key: v.key.min(127),
            gate: v.gate,
            releasing: v.releasing,
            detune_semis: if v.detune_semis.is_finite() {
                v.detune_semis
            } else {
                0.0
            },
            pan: if v.pan.is_finite() {
                v.pan.clamp(-1.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireFx {
    pub algorithm: WireAlgorithm,
    pub version: u16,
    pub enabled: bool,
    pub tail: WireTailPolicy,
    pub frozen: bool,
    pub dormant: bool,
}

impl From<TailState> for WireFx {
    fn from(s: TailState) -> Self {
        Self {
            algorithm: s.algorithm.into(),
            version: s.version,
            enabled: s.enabled,
            tail: s.policy.into(),
            frozen: s.frozen,
            dormant: s.dormant,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireSnapshot {
    pub version: u16,
    pub revision: String,
    pub rendered_frames: String,
    pub sample_rate: u32,
    pub max_frames: u16,
    pub latency_samples: u32,
    pub notes: Vec<WireNote>,
    pub voices: Vec<WireVoice>,
    pub fx: WireFx,
    pub capabilities: Vec<String>,
    pub xruns: Option<u32>,
    pub cpu_percent: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EngineMessage {
    Block(WireBlock),
    Ack(WireAck),
    Snapshot(WireSnapshot),
}

impl EngineMessage {
    pub fn validate(&self) -> Result<(), WireValidationError> {
        match self {
            EngineMessage::Block(b) => b.validate(),
            EngineMessage::Ack(a) => a.validate(),
            EngineMessage::Snapshot(s) => s.validate(),
        }
    }
}

impl WireBlock {
    pub fn validate(&self) -> Result<(), WireValidationError> {
        if self.version != VERSION {
            return Err(WireValidationError::UnsupportedVersion(self.version));
        }
        if self.frames > 1024 {
            return Err(WireValidationError::InvalidFrames(self.frames));
        }
        if self.commands.len() > MAX_EVENTS {
            return Err(WireValidationError::TooManyCommands(self.commands.len()));
        }
        let mut prev_offset = 0u16;
        for (i, c) in self.commands.iter().enumerate() {
            if c.offset >= self.frames || c.offset > 1023 {
                return Err(WireValidationError::InvalidOffset {
                    index: i,
                    offset: c.offset,
                    frames: self.frames,
                });
            }
            if c.offset < prev_offset {
                return Err(WireValidationError::UnsortedOffset {
                    index: i,
                    prev: prev_offset,
                    offset: c.offset,
                });
            }
            prev_offset = c.offset;
            if usize::from(c.origin) >= ORIGINS {
                return Err(WireValidationError::InvalidOrigin(c.origin));
            }
            if c.serial == 0 {
                return Err(WireValidationError::InvalidSerial(c.serial));
            }
            match c.action {
                WireAction::Midi {
                    status,
                    data1,
                    data2,
                    ..
                } => {
                    if !(128..=239).contains(&status) || data1 > 127 || data2 > 127 {
                        return Err(WireValidationError::InvalidMidi);
                    }
                }
                WireAction::Parameter { id, value } => {
                    if id > GLIDE_ENABLED {
                        return Err(WireValidationError::InvalidParameterId(id));
                    }
                    if !value.is_finite() {
                        return Err(WireValidationError::NonFiniteValue);
                    }
                }
                WireAction::VectorXY { x, y } => {
                    if !x.is_finite() || !y.is_finite() {
                        return Err(WireValidationError::NonFiniteValue);
                    }
                    if !(-1.0..=1.0).contains(&x) || !(-1.0..=1.0).contains(&y) {
                        return Err(WireValidationError::InvalidVectorCoordinates);
                    }
                }
                WireAction::PanicOrigin => {}
            }
        }
        Ok(())
    }

    pub fn validate_strict_diamond(&self) -> Result<(), WireValidationError> {
        self.validate()?;
        for c in &self.commands {
            if let WireAction::VectorXY { x, y } = c.action {
                if !(Point { x, y }).valid() {
                    return Err(WireValidationError::InvalidVectorCoordinates);
                }
            }
        }
        Ok(())
    }
}

impl WireAck {
    pub fn validate(&self) -> Result<(), WireValidationError> {
        if self.version != VERSION {
            return Err(WireValidationError::UnsupportedVersion(self.version));
        }
        if usize::from(self.origin) >= ORIGINS {
            return Err(WireValidationError::InvalidOrigin(self.origin));
        }
        if self.serial == 0 {
            return Err(WireValidationError::InvalidSerial(self.serial));
        }
        if !is_valid_u64_decimal(&self.revision) {
            return Err(WireValidationError::InvalidDecimalU64(
                self.revision.clone(),
            ));
        }
        Ok(())
    }
}

impl WireSnapshot {
    pub fn validate(&self) -> Result<(), WireValidationError> {
        if self.version != VERSION {
            return Err(WireValidationError::UnsupportedVersion(self.version));
        }
        if !is_valid_u64_decimal(&self.revision) {
            return Err(WireValidationError::InvalidDecimalU64(
                self.revision.clone(),
            ));
        }
        if !is_valid_u64_decimal(&self.rendered_frames) {
            return Err(WireValidationError::InvalidDecimalU64(
                self.rendered_frames.clone(),
            ));
        }
        if !(8000..=192000).contains(&self.sample_rate) {
            return Err(WireValidationError::InvalidSampleRate(self.sample_rate));
        }
        if !(1..=1024).contains(&self.max_frames) {
            return Err(WireValidationError::InvalidMaxFrames(self.max_frames));
        }
        if !matches!(self.latency_samples, 0 | 16 | 24) {
            return Err(WireValidationError::InvalidLatency(self.latency_samples));
        }
        if self.notes.len() > 128 {
            return Err(WireValidationError::TooManyNotes(self.notes.len()));
        }
        for n in &self.notes {
            if usize::from(n.origin) >= ORIGINS || n.channel >= 16 || n.key > 127 {
                return Err(WireValidationError::InvalidNote);
            }
        }
        if self.voices.len() > 12 {
            return Err(WireValidationError::TooManyVoices(self.voices.len()));
        }
        for v in &self.voices {
            if v.index >= 12
                || v.key > 127
                || !v.detune_semis.is_finite()
                || !v.pan.is_finite()
                || !(-1.0..=1.0).contains(&v.pan)
            {
                return Err(WireValidationError::InvalidVoice);
            }
        }
        if self.fx.version != 1 {
            return Err(WireValidationError::UnsupportedVersion(self.fx.version));
        }
        if self.capabilities.len() > 32 {
            return Err(WireValidationError::TooManyCapabilities(
                self.capabilities.len(),
            ));
        }
        if let Some(cpu) = self.cpu_percent {
            if !cpu.is_finite() || cpu < 0.0 {
                return Err(WireValidationError::InvalidCpuPercent);
            }
        }
        Ok(())
    }
}

/// Zero-allocation audio-thread capture POD suitable for pushing through `rtrb`.
/// JSON serialization and capability intersection occur outside the audio callback.
#[derive(Clone, Copy, Debug)]
pub struct SnapshotPod {
    pub config: Prepare,
    pub revision: u64,
    pub rendered_frames: u64,
    pub latency_samples: u32,
    pub notes: [WireNote; 128],
    pub note_count: u8,
    pub voices: [VoiceState; 12],
    pub vector_weights: [Option<[f32; 4]>; 12],
    pub patch: Patch,
    pub values: [f32; NUM_PARAMS],
    pub glide_enabled: bool,
    pub reverb: TailState,
    pub engine_caps_mask: u32,
    pub kernel_telemetry: KernelTelemetrySnapshot,
    pub scope: [f32; 1024],
    pub xruns: Option<u32>,
    pub cpu_percent: Option<f32>,
    pub active_pattern: u8,
    pub scheduler_playing: bool,
    pub scheduler_step: u64,
}

impl SnapshotPod {
    pub fn from_engine(
        engine: &PreparedEngine,
        scope: [f32; 1024],
        xruns: Option<u32>,
        cpu_percent: Option<f32>,
        active_pattern: u8,
        scheduler_playing: bool,
        scheduler_step: u64,
    ) -> Self {
        let mut notes = [WireNote::default(); 128];
        let mut note_count = 0usize;
        for note in engine.notes() {
            if note_count >= 128 {
                break;
            }
            notes[note_count] = WireNote {
                origin: note.origin,
                channel: note.channel,
                key: note.key,
                note_id: note.note_id,
                pressed: note.pressed,
                sustained: !note.pressed,
            };
            note_count += 1;
        }
        let values = core::array::from_fn(|i| {
            engine
                .parameter(i as u16)
                .unwrap_or(PARAM_SPECS[i].min)
        });
        let glide_enabled = engine
            .parameter(GLIDE_ENABLED)
            .map(|v| v >= 0.5)
            .unwrap_or_else(|| values[ParamId::GlideTime as usize] > 0.0005);
        Self {
            config: engine.configuration(),
            revision: engine.revision(),
            rendered_frames: engine.rendered_frames(),
            latency_samples: engine.latency_samples(),
            notes,
            note_count: note_count as u8,
            voices: engine.voices(),
            vector_weights: engine.vector_weights(),
            patch: engine.patch(),
            values,
            glide_enabled,
            reverb: engine.reverb_state(),
            engine_caps_mask: encode_capabilities_mask(engine.capabilities()),
            kernel_telemetry: engine.telemetry(),
            scope,
            xruns,
            cpu_percent,
            active_pattern,
            scheduler_playing,
            scheduler_step,
        }
    }

    pub fn to_wire_snapshot(&self, host_supported_caps: &[&str], active_voices_only: bool) -> WireSnapshot {
        let notes = self.notes[..usize::from(self.note_count)].to_vec();
        let voices: Vec<WireVoice> = self
            .voices
            .iter()
            .copied()
            .filter(|v| !active_voices_only || v.active || v.gate || v.releasing)
            .map(WireVoice::from)
            .collect();
        WireSnapshot {
            version: VERSION,
            revision: self.revision.to_string(),
            rendered_frames: self.rendered_frames.to_string(),
            sample_rate: self.config.sample_rate,
            max_frames: self.config.max_frames,
            latency_samples: self.latency_samples,
            notes,
            voices,
            fx: self.reverb.into(),
            capabilities: intersect_capabilities(self.engine_caps_mask, host_supported_caps),
            xruns: self.xruns,
            cpu_percent: self.cpu_percent,
        }
    }

    pub fn resting_or_active_vector_weights(&self) -> [f32; 4] {
        if self.patch.topology == Topology::Vector4 {
            for w in self.vector_weights.iter().flatten() {
                return *w;
            }
        }
        self.patch.position.weights()
    }
}

pub fn snapshot_from_engine(
    engine: &PreparedEngine,
    host_supported_caps: &[&str],
    xruns: Option<u32>,
    cpu_percent: Option<f32>,
    active_voices_only: bool,
) -> WireSnapshot {
    let pod = SnapshotPod::from_engine(engine, [0.0; 1024], xruns, cpu_percent, 0, false, 0);
    pod.to_wire_snapshot(host_supported_caps, active_voices_only)
}
