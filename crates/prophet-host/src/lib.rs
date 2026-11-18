// SPDX-License-Identifier: GPL-3.0-only
//! Shared PROPHET-XII host adapter crate consuming Agent A's EnginePort v1 contract,
//! preset codec, musical clock, and bounded VCF validity analyzer.

pub mod analysis;
pub mod preset;
pub mod queue;
pub mod runtime;
pub mod wire;

pub use analysis::{measure_filter_response, FilterPoint, FilterResponseReport, FilterValidity};
pub use preset::{
    decode_and_prepare, encode_from_engine, factory_preset, factory_presets, FactoryPresetEntry,
    HostPresetError, PreparedPresetBundle,
};
pub use queue::{
    BlockScheduler, BlockSummary, CanonicalMidiArbiter, CcLearnRouter, MusicTransport,
    PhysicalMidiSource, SerialIssuer, ORIGIN_DAW_HOST, ORIGIN_HARDWARE_MIDI, ORIGIN_LOCAL_UI,
    ORIGIN_MUSIC_SCHEDULER, ORIGIN_REMOTE_WS, ORIGIN_TEST_HARNESS, ORIGIN_USB_MIDI,
    ORIGIN_WEB_MIDI,
};
pub use runtime::{
    create_swap_pair, AudioEngineSlot, PreparedSwap, RetiredResource, WorkerSwapCoordinator,
};
pub use wire::{
    snapshot_from_engine, EngineMessage, SnapshotPod, WireAck, WireAction, WireAlgorithm,
    WireBlock, WireCommand, WireFx, WireNote, WireSnapshot, WireStatus, WireTailPolicy,
    WireValidationError, WireVoice,
};
