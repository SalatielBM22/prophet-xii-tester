// SPDX-License-Identifier: GPL-3.0-only
//! Worker-only native v1/v2 preset validation, encoding/decoding, and immutable factory presets.

#[path = "../../prophet-preset/src/lib.rs"]
pub mod codec;

pub use codec::{Error as PresetError, Performance, Preset};

use prophet_control::{
    music::{Arpeggiator, NoteScheduler, PatternBank},
    port::Status,
    PreparedEngine,
};

#[derive(Clone, Debug)]
pub struct FactoryPresetEntry {
    pub id: usize,
    pub name: String,
    pub category: String,
    pub values: [f32; prophet_params::NUM_PARAMS],
    pub glide: bool,
}

const FACTORY_BYTES: [&[u8]; 10] = [
    include_bytes!("../../../presets/00-factory.json"),
    include_bytes!("../../../presets/01-factory.json"),
    include_bytes!("../../../presets/02-factory.json"),
    include_bytes!("../../../presets/03-factory.json"),
    include_bytes!("../../../presets/04-factory.json"),
    include_bytes!("../../../presets/05-factory.json"),
    include_bytes!("../../../presets/06-factory.json"),
    include_bytes!("../../../presets/07-factory.json"),
    include_bytes!("../../../presets/08-factory.json"),
    include_bytes!("../../../presets/09-factory.json"),
];

pub fn factory_preset(index: usize) -> Option<Preset> {
    let bytes = FACTORY_BYTES.get(index)?;
    Preset::decode(bytes).ok()
}

pub fn factory_presets() -> Vec<FactoryPresetEntry> {
    FACTORY_BYTES
        .iter()
        .enumerate()
        .filter_map(|(id, bytes)| {
            let decoded = Preset::decode(bytes).ok()?;
            Some(FactoryPresetEntry {
                id,
                name: decoded.name,
                category: decoded.category,
                values: decoded.sound.values,
                glide: decoded.sound.glide,
            })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum HostPresetError {
    Codec(PresetError),
    PrepareFailed(Status),
}

impl From<PresetError> for HostPresetError {
    fn from(e: PresetError) -> Self {
        Self::Codec(e)
    }
}

impl HostPresetError {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Codec(PresetError::Size) => "Preset exceeds 512 KiB size limit",
            Self::Codec(PresetError::Json) => "Malformed or unknown field in preset JSON",
            Self::Codec(PresetError::Version) => "Unsupported PROPHET-XII preset format or version",
            Self::Codec(PresetError::Checksum) => "Preset payload CRC-32 checksum mismatch",
            Self::Codec(PresetError::Parameters) => "Invalid parameter value or SoundState out of range",
            Self::Codec(PresetError::Algorithm) => "Unsupported oscillator, route, or reverb algorithm revision",
            Self::Codec(PresetError::Pattern) => "Invalid sequencer pattern or performance data",
            Self::PrepareFailed(_) => "Failed to prepare DSP engine for decoded SoundState",
        }
    }
}

pub struct PreparedPresetBundle {
    pub preset: Preset,
    pub engine: PreparedEngine,
    pub bank: Option<Box<PatternBank>>,
    pub performance: Option<(NoteScheduler, Arpeggiator)>,
}

/// Validates and prepares a replacement `PreparedEngine` (and optional music transport state)
/// strictly off the audio thread. On error, callers retain the prior active engine untouched.
pub fn decode_and_prepare(
    bytes: &[u8],
    sample_rate: u32,
    max_frames: u16,
) -> Result<PreparedPresetBundle, HostPresetError> {
    let mut preset = Preset::decode(bytes)?;
    let engine = preset
        .sound
        .prepare(sample_rate, max_frames)
        .map_err(HostPresetError::PrepareFailed)?;
    let performance = match preset.performance {
        Some(perf) => Some(perf.prepare(sample_rate)?),
        None => None,
    };
    let bank = preset.bank.take();
    Ok(PreparedPresetBundle {
        preset,
        engine,
        bank,
        performance,
    })
}

pub fn encode_from_engine(
    name: impl Into<String>,
    category: impl Into<String>,
    engine: &PreparedEngine,
    bank: Option<Box<PatternBank>>,
    performance: Option<Performance>,
) -> Result<Vec<u8>, HostPresetError> {
    let preset = Preset {
        name: name.into(),
        category: category.into(),
        sound: engine.sound_state(),
        bank,
        performance,
    };
    preset.encode().map_err(HostPresetError::Codec)
}
