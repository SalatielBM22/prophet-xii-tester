// SPDX-License-Identifier: GPL-3.0-only
//! Platform-independent Android host controller uniting `prophet-host` EnginePort v1 session,
//! AAudio diagnostics/probe telemetry, AudioFocus smooth attenuation, USB MIDI 1.0 packet parsing,
//! and strict off-by-default developer bridge policy.

use crate::probe::{
    AndroidAudioDiagnostics, AudioFormatChoice, LifecycleState, PerformanceChoice,
    ProbeAttemptRecord, SharingChoice,
};
use prophet_control::{
    port::Prepare,
    state::SoundState,
};
use prophet_host::{
    create_host_session,
    queue::{CanonicalMidiArbiter, CcLearnRouter, PhysicalMidiSource},
    ui::{UiCommand, UiCommandReply},
    wire::WireStatus,
    HostControlPlane, RtHostSlot,
};
use prophet_params::ParamId;
use std::sync::{
    atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering},
    Arc,
};

pub const ANDROID_CAPABILITIES: &[&str] = &[
    "enginePortV1",
    "glideEnabled",
    "reverbState",
    "selectIdleReverb",
    "oscillatorPatch",
    "noteScheduler",
    "vectorXY",
];

#[derive(Debug)]
pub struct SharedRtMetrics {
    pub callback_invocations: AtomicU64,
    pub frames_rendered: AtomicU64,
    pub error_code: AtomicI32,
    pub focus_target_gain_bits: AtomicU32,
    pub last_cpu_pct_bits: AtomicU32,
}

impl Default for SharedRtMetrics {
    fn default() -> Self {
        Self {
            callback_invocations: AtomicU64::new(0),
            frames_rendered: AtomicU64::new(0),
            error_code: AtomicI32::new(0),
            focus_target_gain_bits: AtomicU32::new(1.0f32.to_bits()),
            last_cpu_pct_bits: AtomicU32::new(f32::NAN.to_bits()),
        }
    }
}

impl SharedRtMetrics {
    pub fn set_focus_gain(&self, muted: bool, gain: f32) {
        let clamped = if muted {
            0.0
        } else if gain.is_finite() {
            gain.clamp(0.0, 1.0)
        } else {
            1.0
        };
        self.focus_target_gain_bits
            .store(clamped.to_bits(), Ordering::Relaxed);
    }

    pub fn focus_gain(&self) -> f32 {
        f32::from_bits(self.focus_target_gain_bits.load(Ordering::Relaxed))
    }

    pub fn store_cpu_pct(&self, pct: f32) {
        if pct.is_finite() {
            self.last_cpu_pct_bits.store(pct.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn last_cpu_pct(&self) -> Option<f32> {
        let v = f32::from_bits(self.last_cpu_pct_bits.load(Ordering::Relaxed));
        v.is_finite().then_some(v)
    }
}

/// Pure-Rust Android USB/BLE MIDI 1.0 byte-stream decoder handling single messages, running status,
/// interleaved single-byte realtime (`0xF8..=0xFF`), and SysEx framing cleanly.
#[derive(Clone, Debug, Default)]
pub struct Midi1StreamDecoder {
    running_status: u8,
    expected_data: u8,
    data_count: u8,
    data: [u8; 2],
    in_sysex: bool,
}

impl Midi1StreamDecoder {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn feed<F: FnMut(u8, u8, u8)>(&mut self, bytes: &[u8], mut on_msg: F) {
        for &b in bytes {
            if b >= 0xf8 {
                // Single-byte system realtime ignores running status & SysEx cleanly.
                continue;
            }
            if b == 0xf0 {
                self.in_sysex = true;
                self.running_status = 0;
                self.data_count = 0;
                self.expected_data = 0;
                continue;
            }
            if b == 0xf7 {
                self.in_sysex = false;
                continue;
            }
            if b >= 0x80 {
                self.in_sysex = false;
                self.data_count = 0;
                if (0x80..=0xef).contains(&b) {
                    self.running_status = b;
                    self.expected_data = match b & 0xf0 {
                        0xc0 | 0xd0 => 1,
                        _ => 2,
                    };
                } else {
                    self.running_status = 0;
                    self.expected_data = 0;
                }
                continue;
            }
            if self.in_sysex || self.running_status == 0 {
                continue;
            }
            let idx = usize::from(self.data_count);
            if idx < 2 {
                self.data[idx] = b;
                self.data_count += 1;
            }
            if self.data_count >= self.expected_data {
                let d2 = if self.expected_data == 2 {
                    self.data[1]
                } else {
                    0
                };
                on_msg(self.running_status, self.data[0], d2);
                self.data_count = 0;
            }
        }
    }
}

pub struct AndroidController {
    pub control: HostControlPlane,
    pub diagnostics: AndroidAudioDiagnostics,
    pub metrics: Arc<SharedRtMetrics>,
    pub midi_decoder: Midi1StreamDecoder,
    pub standby_rt: Option<RtHostSlot>,
}

impl AndroidController {
    pub fn new(initial_sr: u32) -> Result<Self, prophet_control::port::PortError> {
        let sr = if (8000..=192_000).contains(&initial_sr) {
            initial_sr
        } else {
            48_000
        };
        let (rt_slot, ctrl) = create_host_session(
            "android-aaudio",
            ANDROID_CAPABILITIES.to_vec(),
            Prepare {
                sample_rate: sr,
                max_frames: 1024,
                oversampling: 2,
            },
            true,
        )?;
        Ok(Self {
            control: ctrl,
            diagnostics: AndroidAudioDiagnostics::default(),
            metrics: Arc::new(SharedRtMetrics::default()),
            midi_decoder: Midi1StreamDecoder::default(),
            standby_rt: Some(rt_slot),
        })
    }

    /// Rebuilds the audio session pair cleanly before opening an AAudio stream while preserving
    /// the current `SoundState`, `PatternBank`, `Performance`, `CcLearnRouter`, and `CanonicalMidiArbiter`.
    pub fn prepare_stream_session(&mut self, sample_rate: u32) -> Result<RtHostSlot, String> {
        self.control.poll();
        let sr = sample_rate.clamp(8000, 192_000);
        let os = match self.control.sound.values[ParamId::OversamplingMode as usize] as u8 {
            1 => 1,
            4 => 4,
            _ => 2,
        };
        let (mut new_rt, mut new_ctrl) = create_host_session(
            "android-aaudio",
            ANDROID_CAPABILITIES.to_vec(),
            Prepare {
                sample_rate: sr,
                max_frames: 1024,
                oversampling: os,
            },
            true,
        )
        .map_err(|e| format!("Engine prepare error: {e:?}"))?;

        // Preserve user sound/music/midi state across stream open/reconnection.
        let prev_sound: SoundState = self.control.sound.clone();
        let prev_bank = self.control.music_bank.clone();
        let perf = self.control.current_performance();
        if let Ok(engine) = prev_sound.prepare(sr, 1024) {
            let perf_pair = perf.prepare(sr).ok();
            let _ = new_ctrl.worker.submit_prepared(
                engine,
                Some(prev_bank.clone()),
                perf_pair,
                Some(perf),
            );
            new_rt.slot.poll_swap(None);
        }
        new_ctrl.sound = prev_sound;
        new_ctrl.music_bank = prev_bank;
        new_ctrl.music_tempo_milli = self.control.music_tempo_milli;
        new_ctrl.music_swing = self.control.music_swing;
        new_ctrl.music_active_pattern = self.control.music_active_pattern;
        new_ctrl.music_arp_enabled = self.control.music_arp_enabled;
        new_ctrl.music_arp_order = self.control.music_arp_order;
        new_ctrl.music_arp_octaves = self.control.music_arp_octaves;
        new_ctrl.music_arp_latch = self.control.music_arp_latch;
        new_ctrl.music_arp_seed = self.control.music_arp_seed;
        new_ctrl.cc_router = CcLearnRouter {
            cc_map: self.control.cc_router.cc_map,
            learning_target: self.control.cc_router.learning_target,
        };
        new_ctrl.arbiter = CanonicalMidiArbiter::new(self.control.arbiter.source());
        new_ctrl.filter_analysis = self.control.filter_analysis.clone();
        new_ctrl.last_notice = self.control.last_notice.clone();

        self.standby_rt = None;
        self.control = new_ctrl;
        self.metrics = Arc::new(SharedRtMetrics::default());
        Ok(new_rt)
    }

    pub fn record_probe_attempt(&mut self, rec: ProbeAttemptRecord) {
        self.diagnostics.probe_history.push(rec);
    }

    pub fn mark_starting(&mut self) {
        self.diagnostics.lifecycle = LifecycleState::Starting;
        self.diagnostics.probe_history.clear();
        self.diagnostics.last_error_code = None;
        self.diagnostics.last_error_text = None;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mark_ready(
        &mut self,
        sample_rate: i32,
        channels: i32,
        format: AudioFormatChoice,
        performance: PerformanceChoice,
        sharing: SharingChoice,
        frames_per_burst: i32,
        buffer_capacity: i32,
        buffer_size: i32,
        mmap_used: Option<bool>,
    ) {
        self.diagnostics.lifecycle = LifecycleState::Ready;
        self.diagnostics.sample_rate = Some(sample_rate);
        self.diagnostics.channel_count = Some(channels);
        self.diagnostics.format = Some(format);
        self.diagnostics.performance_mode = Some(performance);
        self.diagnostics.sharing_mode = Some(sharing);
        self.diagnostics.frames_per_burst = (frames_per_burst > 0).then_some(frames_per_burst);
        self.diagnostics.buffer_capacity_frames = (buffer_capacity > 0).then_some(buffer_capacity);
        self.diagnostics.buffer_size_frames = (buffer_size > 0).then_some(buffer_size);
        self.diagnostics.mmap_used = mmap_used;
        self.diagnostics.last_error_code = None;
        self.diagnostics.last_error_text = None;
        self.control
            .set_audio_running(true, Some(sample_rate.max(8000) as u32), Some(1024));
    }

    pub fn mark_failed(&mut self, code: i32, text: impl Into<String>) {
        self.diagnostics.lifecycle = LifecycleState::Failed;
        self.diagnostics.last_error_code = Some(code);
        self.diagnostics.last_error_text = Some(text.into());
        self.control.set_audio_running(false, None, None);
    }

    pub fn mark_stopped(&mut self) {
        self.diagnostics.lifecycle = LifecycleState::Stopped;
        self.control.set_audio_running(false, None, None);
    }

    pub fn update_xruns(&mut self, xruns: Option<i32>) {
        self.diagnostics.xruns = xruns;
    }

    pub fn set_focus_state(&mut self, state: &str, muted: bool, gain: f32) {
        self.diagnostics.focus_state = state.to_owned();
        let resolved_gain = if muted { 0.0 } else { gain.clamp(0.0, 1.0) };
        self.diagnostics.focus_gain = resolved_gain;
        self.metrics.set_focus_gain(muted, resolved_gain);
    }

    pub fn set_route_info(&mut self, route: impl Into<String>, usb_devices: Vec<String>) {
        self.diagnostics.route = route.into();
        self.diagnostics.usb_midi_devices = usb_devices;
    }

    pub fn ingest_usb_midi(&mut self, packet: &[u8]) -> usize {
        let mut accepted = 0usize;
        let ctrl = &mut self.control;
        self.midi_decoder.feed(packet, |status, data1, data2| {
            if let Ok(true) =
                ctrl.dispatch_hardware_midi(PhysicalMidiSource::Native, status, data1, data2, None, 0)
            {
                accepted += 1;
            }
        });
        accepted
    }

    pub fn refresh_diagnostics_into_control(&mut self) {
        self.diagnostics.callback_invocations =
            self.metrics.callback_invocations.load(Ordering::Relaxed);
        self.diagnostics.frames_rendered = self.metrics.frames_rendered.load(Ordering::Relaxed);
        let err = self.metrics.error_code.load(Ordering::Relaxed);
        if err != 0 {
            self.diagnostics.last_error_code = Some(err);
            self.diagnostics.last_error_text =
                Some(crate::probe::aaudio_error_name(err).to_owned());
        }
        self.control.diagnostics = serde_json::to_value(&self.diagnostics).ok();
    }

    pub fn snapshot_json(&mut self) -> String {
        self.refresh_diagnostics_into_control();
        let snap = self.control.snapshot();
        serde_json::to_string(&snap)
            .unwrap_or_else(|_| "{\"version\":1,\"ready\":false}".to_owned())
    }

    pub fn diagnostics_json(&mut self) -> String {
        self.refresh_diagnostics_into_control();
        serde_json::to_string(&self.diagnostics).unwrap_or_else(|_| "{}".to_owned())
    }

    pub fn dispatch_json(&mut self, cmd_json: &str) -> (String, Option<String>) {
        let parsed: Result<UiCommand, _> = serde_json::from_str(cmd_json);
        let cmd = match parsed {
            Ok(c) => c,
            Err(e) => {
                let err = UiCommandReply {
                    ok: false,
                    status: Some(WireStatus::Invalid),
                    message: format!("Malformed UiCommand JSON: {e}"),
                    swap_id: None,
                    preset_json: None,
                    filter_analysis: None,
                    data: None,
                };
                return (
                    serde_json::to_string(&err).unwrap_or_else(|_| "{\"ok\":false}".into()),
                    None,
                );
            }
        };

        if let UiCommand::HostControl { op, enabled, .. } = &cmd {
            match op.as_str() {
                "toggleDevBridge" => {
                    let next = enabled.unwrap_or(!self.diagnostics.dev_bridge_enabled);
                    self.diagnostics.dev_bridge_enabled = next;
                    let reply = UiCommandReply::ok(if next {
                        "Developer LAN bridge enabled (explicit opt-in)"
                    } else {
                        "Developer LAN bridge disabled"
                    });
                    return (
                        serde_json::to_string(&reply).unwrap_or_else(|_| "{\"ok\":true}".into()),
                        Some(if next {
                            "DEV_BRIDGE_ON".into()
                        } else {
                            "DEV_BRIDGE_OFF".into()
                        }),
                    );
                }
                "retryAudio" | "startAudio" | "stopAudio" | "reconnectAudio" => {
                    let reply = UiCommandReply::ok(format!("Scheduled Android host control: {op}"));
                    return (
                        serde_json::to_string(&reply).unwrap_or_else(|_| "{\"ok\":true}".into()),
                        Some(op.clone()),
                    );
                }
                _ => {}
            }
        }

        let reply = self.control.dispatch(cmd);
        let json = serde_json::to_string(&reply).unwrap_or_else(|_| "{\"ok\":false}".into());
        (json, None)
    }
}
