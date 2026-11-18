// SPDX-License-Identifier: GPL-3.0-only
//! Android AAudio configuration probe matrix, result-code diagnostics, and lifecycle states.

use serde::{Deserialize, Serialize};

pub const AAUDIO_OK: i32 = 0;
pub const AAUDIO_DIRECTION_OUTPUT: i32 = 0;

pub const AAUDIO_FORMAT_INVALID: i32 = -1;
pub const AAUDIO_FORMAT_UNSPECIFIED: i32 = 0;
pub const AAUDIO_FORMAT_PCM_I16: i32 = 1;
pub const AAUDIO_FORMAT_PCM_FLOAT: i32 = 2;

pub const AAUDIO_SHARING_MODE_EXCLUSIVE: i32 = 0;
pub const AAUDIO_SHARING_MODE_SHARED: i32 = 1;

pub const AAUDIO_PERFORMANCE_MODE_NONE: i32 = 10;
pub const AAUDIO_PERFORMANCE_MODE_POWER_SAVING: i32 = 11;
pub const AAUDIO_PERFORMANCE_MODE_LOW_LATENCY: i32 = 12;

pub const AAUDIO_USAGE_MEDIA: i32 = 1;
pub const AAUDIO_CONTENT_TYPE_MUSIC: i32 = 2;

pub const AAUDIO_CALLBACK_RESULT_CONTINUE: i32 = 0;
pub const AAUDIO_CALLBACK_RESULT_STOP: i32 = 1;

pub const AAUDIO_ERROR_DISCONNECTED: i32 = -899;
pub const AAUDIO_ERROR_ILLEGAL_ARGUMENT: i32 = -898;
pub const AAUDIO_ERROR_INTERNAL: i32 = -896;
pub const AAUDIO_ERROR_INVALID_STATE: i32 = -895;
pub const AAUDIO_ERROR_INVALID_HANDLE: i32 = -892;
pub const AAUDIO_ERROR_UNIMPLEMENTED: i32 = -890;
pub const AAUDIO_ERROR_UNAVAILABLE: i32 = -889;
pub const AAUDIO_ERROR_NO_FREE_HANDLES: i32 = -888;
pub const AAUDIO_ERROR_NO_MEMORY: i32 = -887;
pub const AAUDIO_ERROR_NULL: i32 = -886;
pub const AAUDIO_ERROR_TIMEOUT: i32 = -885;
pub const AAUDIO_ERROR_WOULD_BLOCK: i32 = -884;
pub const AAUDIO_ERROR_INVALID_FORMAT: i32 = -883;
pub const AAUDIO_ERROR_OUT_OF_RANGE: i32 = -882;
pub const AAUDIO_ERROR_NO_SERVICE: i32 = -881;
pub const AAUDIO_ERROR_INVALID_RATE: i32 = -880;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LifecycleState {
    Stopped,
    Starting,
    Ready,
    Failed,
    Reconnecting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormatChoice {
    Unspecified,
    I16,
    Float,
}

impl AudioFormatChoice {
    pub const fn to_aaudio(self) -> i32 {
        match self {
            Self::Unspecified => AAUDIO_FORMAT_UNSPECIFIED,
            Self::I16 => AAUDIO_FORMAT_PCM_I16,
            Self::Float => AAUDIO_FORMAT_PCM_FLOAT,
        }
    }

    pub const fn from_aaudio(code: i32) -> Option<Self> {
        match code {
            AAUDIO_FORMAT_UNSPECIFIED => Some(Self::Unspecified),
            AAUDIO_FORMAT_PCM_I16 => Some(Self::I16),
            AAUDIO_FORMAT_PCM_FLOAT => Some(Self::Float),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PerformanceChoice {
    None,
    PowerSaving,
    LowLatency,
}

impl PerformanceChoice {
    pub const fn to_aaudio(self) -> i32 {
        match self {
            Self::None => AAUDIO_PERFORMANCE_MODE_NONE,
            Self::PowerSaving => AAUDIO_PERFORMANCE_MODE_POWER_SAVING,
            Self::LowLatency => AAUDIO_PERFORMANCE_MODE_LOW_LATENCY,
        }
    }

    pub const fn from_aaudio(code: i32) -> Option<Self> {
        match code {
            AAUDIO_PERFORMANCE_MODE_NONE => Some(Self::None),
            AAUDIO_PERFORMANCE_MODE_POWER_SAVING => Some(Self::PowerSaving),
            AAUDIO_PERFORMANCE_MODE_LOW_LATENCY => Some(Self::LowLatency),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SharingChoice {
    Exclusive,
    Shared,
}

impl SharingChoice {
    pub const fn to_aaudio(self) -> i32 {
        match self {
            Self::Exclusive => AAUDIO_SHARING_MODE_EXCLUSIVE,
            Self::Shared => AAUDIO_SHARING_MODE_SHARED,
        }
    }

    pub const fn from_aaudio(code: i32) -> Option<Self> {
        match code {
            AAUDIO_SHARING_MODE_EXCLUSIVE => Some(Self::Exclusive),
            AAUDIO_SHARING_MODE_SHARED => Some(Self::Shared),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeStep {
    /// Requested sample rate (`0` lets AAudio HAL choose device native rate).
    pub sample_rate_hz: i32,
    pub format: AudioFormatChoice,
    pub performance: PerformanceChoice,
    pub sharing: SharingChoice,
    pub rationale: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeAttemptRecord {
    pub step_index: usize,
    pub requested_sr: i32,
    pub requested_format: AudioFormatChoice,
    pub requested_performance: PerformanceChoice,
    pub requested_sharing: SharingChoice,
    pub open_result: i32,
    pub open_result_name: String,
    pub start_result: Option<i32>,
    pub negotiated_sr: Option<i32>,
    pub negotiated_channels: Option<i32>,
    pub negotiated_format: Option<AudioFormatChoice>,
    pub negotiated_performance: Option<PerformanceChoice>,
    pub negotiated_sharing: Option<SharingChoice>,
    pub frames_per_burst: Option<i32>,
    pub buffer_capacity_frames: Option<i32>,
    pub buffer_size_frames: Option<i32>,
    pub mmap_used: Option<bool>,
    pub accepted: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidAudioDiagnostics {
    pub lifecycle: LifecycleState,
    pub sample_rate: Option<i32>,
    pub channel_count: Option<i32>,
    pub format: Option<AudioFormatChoice>,
    pub performance_mode: Option<PerformanceChoice>,
    pub sharing_mode: Option<SharingChoice>,
    pub frames_per_burst: Option<i32>,
    pub buffer_capacity_frames: Option<i32>,
    pub buffer_size_frames: Option<i32>,
    pub mmap_used: Option<bool>,
    pub xruns: Option<i32>,
    pub callback_invocations: u64,
    pub frames_rendered: u64,
    pub last_error_code: Option<i32>,
    pub last_error_text: Option<String>,
    pub focus_state: String,
    pub focus_gain: f32,
    pub route: String,
    pub usb_midi_devices: Vec<String>,
    pub dev_bridge_enabled: bool,
    pub probe_history: Vec<ProbeAttemptRecord>,
}

impl Default for AndroidAudioDiagnostics {
    fn default() -> Self {
        Self {
            lifecycle: LifecycleState::Stopped,
            sample_rate: None,
            channel_count: None,
            format: None,
            performance_mode: None,
            sharing_mode: None,
            frames_per_burst: None,
            buffer_capacity_frames: None,
            buffer_size_frames: None,
            mmap_used: None,
            xruns: None,
            callback_invocations: 0,
            frames_rendered: 0,
            last_error_code: None,
            last_error_text: None,
            focus_state: "NONE".to_owned(),
            focus_gain: 1.0,
            route: "unknown".to_owned(),
            usb_midi_devices: Vec::new(),
            dev_bridge_enabled: false,
            probe_history: Vec::new(),
        }
    }
}

/// Builds a prioritized fallback probe matrix ordering low-latency float first and falling back
/// cleanly across HAL native rate (`0`), `PCM_I16`, `PerformanceMode::None`, and `48000`/`44100`.
pub fn build_probe_plan(hint_sample_rate: i32) -> Vec<ProbeStep> {
    let valid_hint = (8000..=192_000).contains(&hint_sample_rate).then_some(hint_sample_rate);
    let mut plan: Vec<ProbeStep> = Vec::with_capacity(8);

    let mut push_unique = |step: ProbeStep| {
        if !plan.iter().any(|s| {
            s.sample_rate_hz == step.sample_rate_hz
                && s.format == step.format
                && s.performance == step.performance
                && s.sharing == step.sharing
        }) {
            plan.push(step);
        }
    };

    if let Some(sr) = valid_hint {
        push_unique(ProbeStep {
            sample_rate_hz: sr,
            format: AudioFormatChoice::Float,
            performance: PerformanceChoice::LowLatency,
            sharing: SharingChoice::Shared,
            rationale: "Hinted native sample rate, Float, LowLatency, Shared",
        });
    }
    push_unique(ProbeStep {
        sample_rate_hz: 0,
        format: AudioFormatChoice::Float,
        performance: PerformanceChoice::LowLatency,
        sharing: SharingChoice::Shared,
        rationale: "Unspecified HAL rate, Float, LowLatency, Shared",
    });
    if let Some(sr) = valid_hint {
        push_unique(ProbeStep {
            sample_rate_hz: sr,
            format: AudioFormatChoice::I16,
            performance: PerformanceChoice::LowLatency,
            sharing: SharingChoice::Shared,
            rationale: "Hinted rate with PCM_I16 fallback for 16-bit fast paths",
        });
    }
    push_unique(ProbeStep {
        sample_rate_hz: 0,
        format: AudioFormatChoice::I16,
        performance: PerformanceChoice::LowLatency,
        sharing: SharingChoice::Shared,
        rationale: "Unspecified HAL rate, PCM_I16, LowLatency, Shared",
    });
    push_unique(ProbeStep {
        sample_rate_hz: 0,
        format: AudioFormatChoice::Float,
        performance: PerformanceChoice::None,
        sharing: SharingChoice::Shared,
        rationale: "Native rate Float with PerformanceMode::None (non-fast AudioTrack path)",
    });
    push_unique(ProbeStep {
        sample_rate_hz: 0,
        format: AudioFormatChoice::I16,
        performance: PerformanceChoice::None,
        sharing: SharingChoice::Shared,
        rationale: "Native rate PCM_I16 with PerformanceMode::None universal fallback",
    });
    push_unique(ProbeStep {
        sample_rate_hz: 48_000,
        format: AudioFormatChoice::I16,
        performance: PerformanceChoice::None,
        sharing: SharingChoice::Shared,
        rationale: "Conservative 48kHz PCM_I16 fallback",
    });
    push_unique(ProbeStep {
        sample_rate_hz: 44_100,
        format: AudioFormatChoice::I16,
        performance: PerformanceChoice::None,
        sharing: SharingChoice::Shared,
        rationale: "Conservative 44.1kHz PCM_I16 fallback",
    });

    plan
}

pub fn is_reconnectable_aaudio_error(code: i32) -> bool {
    matches!(
        code,
        AAUDIO_ERROR_DISCONNECTED | AAUDIO_ERROR_TIMEOUT | AAUDIO_ERROR_UNAVAILABLE
    )
}

pub fn aaudio_error_name(code: i32) -> &'static str {
    match code {
        AAUDIO_OK => "AAUDIO_OK",
        AAUDIO_ERROR_DISCONNECTED => "AAUDIO_ERROR_DISCONNECTED",
        AAUDIO_ERROR_ILLEGAL_ARGUMENT => "AAUDIO_ERROR_ILLEGAL_ARGUMENT",
        AAUDIO_ERROR_INTERNAL => "AAUDIO_ERROR_INTERNAL",
        AAUDIO_ERROR_INVALID_STATE => "AAUDIO_ERROR_INVALID_STATE",
        AAUDIO_ERROR_INVALID_HANDLE => "AAUDIO_ERROR_INVALID_HANDLE",
        AAUDIO_ERROR_UNIMPLEMENTED => "AAUDIO_ERROR_UNIMPLEMENTED",
        AAUDIO_ERROR_UNAVAILABLE => "AAUDIO_ERROR_UNAVAILABLE",
        AAUDIO_ERROR_NO_FREE_HANDLES => "AAUDIO_ERROR_NO_FREE_HANDLES",
        AAUDIO_ERROR_NO_MEMORY => "AAUDIO_ERROR_NO_MEMORY",
        AAUDIO_ERROR_NULL => "AAUDIO_ERROR_NULL",
        AAUDIO_ERROR_TIMEOUT => "AAUDIO_ERROR_TIMEOUT",
        AAUDIO_ERROR_WOULD_BLOCK => "AAUDIO_ERROR_WOULD_BLOCK",
        AAUDIO_ERROR_INVALID_FORMAT => "AAUDIO_ERROR_INVALID_FORMAT",
        AAUDIO_ERROR_OUT_OF_RANGE => "AAUDIO_ERROR_OUT_OF_RANGE",
        AAUDIO_ERROR_NO_SERVICE => "AAUDIO_ERROR_NO_SERVICE",
        AAUDIO_ERROR_INVALID_RATE => "AAUDIO_ERROR_INVALID_RATE",
        -1 => "HOST_LOCK_OR_PRECONDITION_ERROR",
        -2 => "HOST_NEGOTIATION_MISMATCH",
        -3 => "HOST_ENGINE_PREPARE_ERROR",
        _ => "AAUDIO_ERROR_UNKNOWN",
    }
}
