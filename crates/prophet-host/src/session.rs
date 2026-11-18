// SPDX-License-Identifier: GPL-3.0-only
//! Unified real-time (`RtHostSlot`) and off-audio control plane (`HostControlPlane`) shared across
//! Android, VST3/CLAP, LV2, and Desktop Standalone hosts.
//!
//! Strictly separates real-time audio execution (zero allocations, zero locks, zero JSON,
//! zero drops of `Box<PreparedEngine>`) from off-thread preset preparation, bounded VCF
//! validity analysis, MIDI arbitration/Learn, and UI snapshot serialization.

use crate::{
    analysis::{measure_filter_response, FilterResponseReport},
    preset::{self, HostPresetError, Performance, Preset},
    queue::{
        BlockSummary, CanonicalMidiArbiter, CcLearnRouter, PhysicalMidiSource, SerialIssuer,
        ORIGIN_HARDWARE_MIDI, ORIGIN_LOCAL_UI, ORIGIN_MUSIC_SCHEDULER, STAGED_CAPACITY,
    },
    runtime::{create_swap_pair, AudioEngineSlot, WorkerSwapCoordinator},
    ui::{
        apply_ui_patch_fields, performance_record, ui_parameters_from_pod, ui_to_rt_command,
        UiCommand, UiCommandReply, UiKernelTelemetry, UiMidiLearnState, UiMusicState, UiNotice,
        UiPatchState, UiPattern, UiPresetSummary, UiSnapshot,
    },
    wire::{SnapshotPod, WireTailPolicy},
};
use prophet_control::{
    music::{MusicError, Order, PatternBank, Step},
    port::{Ack, Action, Command, EnginePort, PortError, Prepare, Status, GLIDE_ENABLED, ORIGINS},
    state::SoundState,
    PreparedEngine,
};
use prophet_dsp::{fx::reverb::Algorithm, vector::Patch};
use prophet_params::{ParamId, PARAM_SPECS};
use rtrb::{Consumer, Producer, RingBuffer};

const DEFAULT_SNAPSHOT_INTERVAL_DIVISOR: u64 = 25;
const RT_STACK_CHUNK_FRAMES: usize = 1024;

#[derive(Clone, Copy, Debug)]
pub enum RtMusicOp {
    StartFromZero,
    StartGrid { step: u32, frame: u64 },
    Seek { frame: u64 },
    Stop,
    SetPattern(u8),
    SetPatternLen { pattern: u8, len: u8 },
    SetStep {
        pattern: u8,
        step_index: u8,
        step: Step,
    },
    SetTempo(u32),
    SetSwing(u8),
    SetArpEnabled(bool),
    SetArpOrder(Order),
    SetArpOctaves(u8),
    SetArpLatch(bool),
    SetArpSeed(u32),
    ArpKey { key: u8, on: bool },
}

#[derive(Clone, Copy, Debug)]
pub enum RtMessage {
    Port(Command),
    SetPatch { req_id: u64, patch: Patch },
    SelectReverb { req_id: u64, algorithm: Algorithm },
    SetDriveMakeup(bool),
    Music(RtMusicOp),
    RequestSnapshot,
}

#[derive(Clone, Copy, Debug)]
pub enum RtNotice {
    PatchResult {
        req_id: u64,
        status: Status,
        patch: Patch,
    },
    ReverbResult {
        req_id: u64,
        status: Status,
        algorithm: Algorithm,
    },
    MusicErr(MusicError),
    Ack(Ack),
}

/// Real-time audio thread host wrapper: zero allocations, zero locks, zero deallocations.
pub struct RtHostSlot {
    pub slot: AudioEngineSlot,
    rt_rx: Consumer<RtMessage>,
    telemetry_tx: Producer<SnapshotPod>,
    notice_tx: Producer<RtNotice>,
    staged: [Command; STAGED_CAPACITY],
    staged_len: usize,
    frames_since_publish: u64,
    force_publish: bool,
    scratch_l: [f32; RT_STACK_CHUNK_FRAMES],
    scratch_r: [f32; RT_STACK_CHUNK_FRAMES],
}

// `PreparedEngine`, `rtrb` endpoints, and arrays are `Send`; ownership moves exclusively onto
// the audio callback thread once started.
unsafe impl Send for RtHostSlot {}

impl RtHostSlot {
    pub fn drain_rt_messages(&mut self, serials: Option<&SerialIssuer>) -> bool {
        let mut swapped = self.slot.poll_swap(serials);
        let _ = &mut swapped;
        self.staged_len = 0;

        while !self.rt_rx.is_empty() {
            // Stop before overflowing staged commands in case of massive MIDI bursts.
            if self.staged_len >= STAGED_CAPACITY {
                break;
            }
            let Ok(msg) = self.rt_rx.pop() else {
                break;
            };
            match msg {
                RtMessage::Port(cmd) => {
                    let mut forward = true;
                    if let Action::Midi { status, data1, data2, .. } = cmd.action {
                        if let Some(m) = self.slot.music.as_mut() {
                            if m.arp_enabled && cmd.origin != ORIGIN_MUSIC_SCHEDULER {
                                let kind = status & 0xf0;
                                if kind == 0x90 || kind == 0x80 {
                                    let on = kind == 0x90 && data2 > 0;
                                    let _ = m.arp.key(data1, on);
                                    if m.enabled && on {
                                        // Arpeggiator owns note-on triggers while running so raw
                                        // un-arpeggiated chord tones do not double-trigger.
                                        forward = false;
                                    }
                                }
                            }
                        }
                    }
                    if forward {
                        self.staged[self.staged_len] = cmd;
                        self.staged_len += 1;
                    }
                }
                RtMessage::SetPatch { req_id, patch } => {
                    let res = self.slot.engine.set_patch(patch);
                    let status = match res {
                        Ok(()) => Status::Applied,
                        Err(s) => s,
                    };
                    let _ = self.notice_tx.push(RtNotice::PatchResult {
                        req_id,
                        status,
                        patch: self.slot.engine.patch(),
                    });
                    self.force_publish = true;
                }
                RtMessage::SelectReverb { req_id, algorithm } => {
                    let res = self.slot.engine.select_reverb(algorithm);
                    let status = match res {
                        Ok(()) => Status::Applied,
                        Err(s) => s,
                    };
                    let _ = self.notice_tx.push(RtNotice::ReverbResult {
                        req_id,
                        status,
                        algorithm: self.slot.engine.reverb_state().algorithm,
                    });
                    self.force_publish = true;
                }
                RtMessage::SetDriveMakeup(enabled) => {
                    self.slot.engine.set_drive_makeup(enabled);
                    self.force_publish = true;
                }
                RtMessage::Music(op) => {
                    self.apply_music_op(op);
                    self.force_publish = true;
                }
                RtMessage::RequestSnapshot => {
                    self.force_publish = true;
                }
            }
        }
        swapped
    }

    fn apply_music_op(&mut self, op: RtMusicOp) {
        let Some(m) = self.slot.music.as_mut() else {
            return;
        };
        let res = match op {
            RtMusicOp::StartFromZero => m.start_from_zero(),
            RtMusicOp::StartGrid { step, frame } => m.start_grid(step, frame),
            RtMusicOp::Seek { frame } => m.seek(frame),
            RtMusicOp::Stop => {
                m.stop();
                Ok(())
            }
            RtMusicOp::SetPattern(p) => {
                m.active_pattern = usize::from(p.min(63));
                Ok(())
            }
            RtMusicOp::SetPatternLen { pattern, len } => {
                if (1..=32).contains(&len) {
                    m.bank.0[usize::from(pattern.min(63))].len = len;
                    Ok(())
                } else {
                    Err(MusicError::Configuration)
                }
            }
            RtMusicOp::SetStep {
                pattern,
                step_index,
                step,
            } => {
                if step.valid() && usize::from(step_index) < 32 {
                    m.bank.0[usize::from(pattern.min(63))].steps[usize::from(step_index)] = step;
                    Ok(())
                } else {
                    Err(MusicError::Configuration)
                }
            }
            RtMusicOp::SetTempo(tempo_milli) => m.set_tempo(tempo_milli),
            RtMusicOp::SetSwing(swing) => m.set_swing(swing),
            RtMusicOp::SetArpEnabled(enabled) => {
                m.arp_enabled = enabled;
                if !enabled && m.enabled {
                    // Flush current arp notes before continuing pattern on step key cleanly.
                    m.scheduler.stop();
                    let _ = m.scheduler.seek(m.current_frame);
                }
                Ok(())
            }
            RtMusicOp::SetArpOrder(order) => {
                m.arp.order = order;
                Ok(())
            }
            RtMusicOp::SetArpOctaves(oct) => {
                if m.arp.set_octaves(oct) {
                    m.arp_octaves = oct;
                    Ok(())
                } else {
                    Err(MusicError::Configuration)
                }
            }
            RtMusicOp::SetArpLatch(latch) => {
                m.arp.set_latch(latch);
                m.arp_latch = latch;
                Ok(())
            }
            RtMusicOp::SetArpSeed(seed) => {
                m.arp.seed = seed;
                Ok(())
            }
            RtMusicOp::ArpKey { key, on } => {
                if m.arp.key(key, on) {
                    Ok(())
                } else {
                    Err(MusicError::Configuration)
                }
            }
        };
        if let Err(err) = res {
            let _ = self.notice_tx.push(RtNotice::MusicErr(err));
        }
    }

    pub fn flush_stop_before_suspend(&mut self) -> Result<(), PortError> {
        if let Some(m) = self.slot.music.as_mut() {
            m.flush_stop_before_suspend(&mut self.slot.engine)?;
        }
        Ok(())
    }

    /// Processes planar stereo buffers with combined `extra_cmds` (e.g. DAW host events) and queued
    /// non-RT `RtMessage` commands, maintaining zero heap allocations.
    #[allow(clippy::too_many_arguments)]
    pub fn process_planar_f32(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        extra_cmds: &[Command],
        xruns: Option<u32>,
        cpu_percent: Option<f32>,
        serials: Option<&SerialIssuer>,
    ) -> Result<BlockSummary, PortError> {
        self.drain_rt_messages(serials);

        let total = left.len();
        if total != right.len() {
            left.fill(0.0);
            right.fill(0.0);
            return Err(PortError::Block);
        }
        for c in extra_cmds {
            if usize::from(c.offset) > total {
                left.fill(0.0);
                right.fill(0.0);
                return Err(PortError::Offset);
            }
            if usize::from(c.origin) >= ORIGINS || c.serial == 0 {
                left.fill(0.0);
                right.fill(0.0);
                return Err(PortError::Origin);
            }
        }

        // Staged control-plane events go at offset 0 of this block; extra_cmds keep host offsets.
        for &c in extra_cmds {
            if self.staged_len < STAGED_CAPACITY {
                self.staged[self.staged_len] = c;
                self.staged_len += 1;
            }
        }

        let summary = if total == 0 {
            let mut empty_l = [];
            let mut empty_r = [];
            self.slot.process_block(
                &mut empty_l,
                &mut empty_r,
                &self.staged[..self.staged_len],
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?
        } else {
            let mut combined = [Command {
                offset: 0,
                origin: 0,
                serial: 0,
                action: Action::PanicOrigin,
            }; STAGED_CAPACITY];
            let staged_n = self.staged_len;
            combined[..staged_n].copy_from_slice(&self.staged[..staged_n]);
            self.slot.process_block(
                left,
                right,
                &combined[..staged_n],
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?
        };

        self.maybe_publish_snapshot(total as u64, xruns, cpu_percent, summary.host_acks > 0);
        Ok(summary)
    }

    /// Processes arbitrary-length stereo interleaved `f32` audio callbacks (e.g. AAudio/CPAL) in
    /// bounded `1024`-frame chunks with zero allocations or locks.
    pub fn process_interleaved_f32(
        &mut self,
        interleaved: &mut [f32],
        frames: usize,
        xruns: Option<u32>,
        cpu_percent: Option<f32>,
        serials: Option<&SerialIssuer>,
    ) -> Result<BlockSummary, PortError> {
        if interleaved.len() < frames.saturating_mul(2) {
            interleaved.fill(0.0);
            return Err(PortError::Block);
        }
        self.drain_rt_messages(serials);

        let mut staged_copy = [Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        }; STAGED_CAPACITY];
        let staged_n = self.staged_len;
        staged_copy[..staged_n].copy_from_slice(&self.staged[..staged_n]);

        if frames == 0 {
            let mut el = [];
            let mut er = [];
            let sum = self.slot.process_block(
                &mut el,
                &mut er,
                &staged_copy[..staged_n],
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?;
            self.maybe_publish_snapshot(0, xruns, cpu_percent, false);
            return Ok(sum);
        }

        let mut agg = BlockSummary::default();
        let mut pos = 0usize;
        while pos < frames {
            let chunk = (frames - pos).min(RT_STACK_CHUNK_FRAMES);
            let cmds_slice: &[Command] = if pos == 0 { &staged_copy[..staged_n] } else { &[] };
            let sum = self.slot.process_block(
                &mut self.scratch_l[..chunk],
                &mut self.scratch_r[..chunk],
                cmds_slice,
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?;
            for i in 0..chunk {
                let l = self.scratch_l[i];
                let r = self.scratch_r[i];
                let out_idx = (pos + i) * 2;
                interleaved[out_idx] = if l.is_finite() { l } else { 0.0 };
                interleaved[out_idx + 1] = if r.is_finite() { r } else { 0.0 };
            }
            agg.frames_rendered += sum.frames_rendered;
            agg.sub_blocks += sum.sub_blocks;
            agg.host_acks += sum.host_acks;
            agg.music_results += sum.music_results;
            agg.shifted_overflow_events += sum.shifted_overflow_events;
            agg.carried_out = sum.carried_out;
            pos += chunk;
        }
        self.maybe_publish_snapshot(frames as u64, xruns, cpu_percent, agg.host_acks > 0);
        Ok(agg)
    }

    /// Zero-allocation interleaved `i16` fallback renderer for Android AAudio HALs or 16-bit
    /// consumer devices that refuse `AAUDIO_FORMAT_PCM_FLOAT`.
    pub fn process_interleaved_i16(
        &mut self,
        interleaved: &mut [i16],
        frames: usize,
        xruns: Option<u32>,
        cpu_percent: Option<f32>,
        serials: Option<&SerialIssuer>,
    ) -> Result<BlockSummary, PortError> {
        if interleaved.len() < frames.saturating_mul(2) {
            interleaved.fill(0);
            return Err(PortError::Block);
        }
        self.drain_rt_messages(serials);

        let mut staged_copy = [Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        }; STAGED_CAPACITY];
        let staged_n = self.staged_len;
        staged_copy[..staged_n].copy_from_slice(&self.staged[..staged_n]);

        if frames == 0 {
            let mut el = [];
            let mut er = [];
            let sum = self.slot.process_block(
                &mut el,
                &mut er,
                &staged_copy[..staged_n],
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?;
            self.maybe_publish_snapshot(0, xruns, cpu_percent, false);
            return Ok(sum);
        }

        let mut agg = BlockSummary::default();
        let mut pos = 0usize;
        while pos < frames {
            let chunk = (frames - pos).min(RT_STACK_CHUNK_FRAMES);
            let cmds_slice: &[Command] = if pos == 0 { &staged_copy[..staged_n] } else { &[] };
            let sum = self.slot.process_block(
                &mut self.scratch_l[..chunk],
                &mut self.scratch_r[..chunk],
                cmds_slice,
                |ack| {
                    let _ = self.notice_tx.push(RtNotice::Ack(ack));
                },
                |_| {},
            )?;
            for i in 0..chunk {
                let l = self.scratch_l[i].clamp(-1.0, 1.0);
                let r = self.scratch_r[i].clamp(-1.0, 1.0);
                let out_idx = (pos + i) * 2;
                interleaved[out_idx] = if l.is_finite() {
                    (l * 32767.0).round() as i16
                } else {
                    0
                };
                interleaved[out_idx + 1] = if r.is_finite() {
                    (r * 32767.0).round() as i16
                } else {
                    0
                };
            }
            agg.frames_rendered += sum.frames_rendered;
            agg.sub_blocks += sum.sub_blocks;
            agg.host_acks += sum.host_acks;
            agg.music_results += sum.music_results;
            agg.shifted_overflow_events += sum.shifted_overflow_events;
            agg.carried_out = sum.carried_out;
            pos += chunk;
        }
        self.maybe_publish_snapshot(frames as u64, xruns, cpu_percent, agg.host_acks > 0);
        Ok(agg)
    }

    fn maybe_publish_snapshot(
        &mut self,
        frames: u64,
        xruns: Option<u32>,
        cpu_percent: Option<f32>,
        had_commands: bool,
    ) {
        self.frames_since_publish = self.frames_since_publish.saturating_add(frames);
        let sr = u64::from(self.slot.engine.configuration().sample_rate.max(8000));
        let cadence = (sr / DEFAULT_SNAPSHOT_INTERVAL_DIVISOR).max(256);
        if (self.force_publish || had_commands || self.frames_since_publish >= cadence)
            && self.telemetry_tx.slots() > 0
        {
            let pod = self.slot.capture_snapshot(xruns, cpu_percent);
            if self.telemetry_tx.push(pod).is_ok() {
                self.frames_since_publish = 0;
                self.force_publish = false;
            }
        }
    }
}

/// Non-RT unified host controller consuming `prophet-host` + `UiCommand` / `UiSnapshot`.
pub struct HostControlPlane {
    pub backend: String,
    pub ready: bool,
    pub host_capabilities: Vec<&'static str>,
    pub worker: WorkerSwapCoordinator,
    pub rt_tx: Producer<RtMessage>,
    pub telemetry_rx: Consumer<SnapshotPod>,
    pub notice_rx: Consumer<RtNotice>,
    pub serials: SerialIssuer,
    pub arbiter: CanonicalMidiArbiter,
    pub cc_router: CcLearnRouter,
    pub sound: SoundState,
    pub music_bank: Box<PatternBank>,
    pub music_tempo_milli: u32,
    pub music_swing: u8,
    pub music_active_pattern: u8,
    pub music_arp_enabled: bool,
    pub music_arp_order: Order,
    pub music_arp_octaves: u8,
    pub music_arp_latch: bool,
    pub music_arp_seed: u32,
    pub latest_pod: SnapshotPod,
    pub presets: Vec<UiPresetSummary>,
    pub filter_analysis: Option<FilterResponseReport>,
    pub last_notice: Option<UiNotice>,
    pub diagnostics: Option<serde_json::Value>,
    pending_swap_id: Option<u64>,
    last_observed_swap_id: u64,
    notice_seq: u64,
    req_seq: u64,
    stored_reverb_wet: f32,
    reverb_resync_pending: bool,
    audio_running: bool,
    sample_rate: u32,
    max_frames: u16,
}

pub fn create_host_session(
    backend: impl Into<String>,
    host_capabilities: Vec<&'static str>,
    initial_config: Prepare,
    with_music: bool,
) -> Result<(RtHostSlot, HostControlPlane), PortError> {
    let initial = PreparedEngine::prepare(initial_config)?;
    let initial_sound = initial.sound_state();
    let initial_pod = SnapshotPod::from_engine(&initial, 0, [0.0; 1024], None, None, 0, false, 0);
    let (slot, worker) = create_swap_pair(initial, with_music, 8);
    let (rt_tx, rt_rx) = RingBuffer::new(512);
    let (telemetry_tx, telemetry_rx) = RingBuffer::new(16);
    let (notice_tx, notice_rx) = RingBuffer::new(128);

    let rt_slot = RtHostSlot {
        slot,
        rt_rx,
        telemetry_tx,
        notice_tx,
        staged: [Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        }; STAGED_CAPACITY],
        staged_len: 0,
        frames_since_publish: 0,
        force_publish: true,
        scratch_l: [0.0; RT_STACK_CHUNK_FRAMES],
        scratch_r: [0.0; RT_STACK_CHUNK_FRAMES],
    };

    let stored_reverb_wet = initial_sound.values[ParamId::ReverbWet as usize];
    let presets = preset::factory_presets()
        .iter()
        .map(UiPresetSummary::from)
        .collect();

    let ctrl = HostControlPlane {
        backend: backend.into(),
        ready: false,
        host_capabilities,
        worker,
        rt_tx,
        telemetry_rx,
        notice_rx,
        serials: SerialIssuer::new(),
        arbiter: CanonicalMidiArbiter::default(),
        cc_router: CcLearnRouter::default(),
        sound: initial_sound,
        music_bank: Box::<PatternBank>::default(),
        music_tempo_milli: 120_000,
        music_swing: 0,
        music_active_pattern: 0,
        music_arp_enabled: false,
        music_arp_order: Order::Up,
        music_arp_octaves: 1,
        music_arp_latch: false,
        music_arp_seed: 1,
        latest_pod: initial_pod,
        presets,
        filter_analysis: None,
        last_notice: None,
        diagnostics: None,
        pending_swap_id: None,
        last_observed_swap_id: 0,
        notice_seq: 0,
        req_seq: 0,
        stored_reverb_wet,
        reverb_resync_pending: false,
        audio_running: false,
        sample_rate: initial_config.sample_rate,
        max_frames: initial_config.max_frames,
    };

    Ok((rt_slot, ctrl))
}

impl HostControlPlane {
    pub fn set_audio_running(&mut self, running: bool, sample_rate: Option<u32>, max_frames: Option<u16>) {
        self.audio_running = running;
        self.ready = running;
        if let Some(sr) = sample_rate {
            self.sample_rate = sr;
        }
        if let Some(mf) = max_frames {
            self.max_frames = mf;
        }
        if running {
            let _ = self.rt_tx.push(RtMessage::RequestSnapshot);
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn max_frames(&self) -> u16 {
        self.max_frames
    }

    fn emit_notice(&mut self, ok: bool, code: impl Into<String>, message: impl Into<String>) {
        self.notice_seq += 1;
        self.last_notice = Some(UiNotice {
            seq: self.notice_seq,
            ok,
            code: code.into(),
            message: message.into(),
        });
    }

    fn sync_from_pod(&mut self, pod: &SnapshotPod) {
        self.sound.values = pod.values;
        self.sound.glide = pod.glide_enabled;
        self.sound.patch = pod.patch;
        self.sound.algorithm = pod.reverb.algorithm;
        if !self.reverb_resync_pending {
            self.sound.reverb_enabled = pod.reverb.enabled;
            self.sound.kill_tail = pod.reverb.policy == prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Kill;
        }
        if self.sound.reverb_enabled {
            self.stored_reverb_wet = pod.values[ParamId::ReverbWet as usize];
        }
        self.sample_rate = pod.config.sample_rate;
        self.max_frames = pod.config.max_frames;
        self.music_active_pattern = pod.active_pattern;
    }

    pub fn sync_pod_from_sound(&mut self) {
        self.latest_pod.values = self.sound.values;
        self.latest_pod.glide_enabled = self.sound.glide;
        self.latest_pod.patch = self.sound.patch;
        self.latest_pod.reverb.algorithm = self.sound.algorithm;
        self.latest_pod.reverb.enabled = self.sound.reverb_enabled;
        self.latest_pod.reverb.policy = if self.sound.kill_tail {
            prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Kill
        } else {
            prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Spill
        };
        let os = self.sound.values[ParamId::OversamplingMode as usize] as u8;
        let factor = match os {
            1 => prophet_dsp::oversample::OversampleFactor::X1,
            4 => prophet_dsp::oversample::OversampleFactor::X4,
            _ => prophet_dsp::oversample::OversampleFactor::X2,
        };
        self.latest_pod.config.oversampling = factor as u8;
        self.latest_pod.latency_samples = factor.latency_samples();
        self.latest_pod.active_pattern = self.music_active_pattern;
    }

    /// Drains retired engines, latest `SnapshotPod`s, and RT notifications. Call before rendering UI
    /// or dispatching structural reconfigurations.
    pub fn poll(&mut self) -> usize {
        self.worker.collect_retired();
        let mut newest: Option<SnapshotPod> = None;
        while let Ok(pod) = self.telemetry_rx.pop() {
            newest = Some(pod);
        }
        if let Some(pod) = newest {
            self.latest_pod = pod;
            if pod.active_swap_id != self.last_observed_swap_id {
                self.last_observed_swap_id = pod.active_swap_id;
                if self.pending_swap_id == Some(pod.active_swap_id) {
                    self.pending_swap_id = None;
                }
            }
            if self.pending_swap_id.is_none() {
                self.sync_from_pod(&pod);
            }
        }

        let mut handled_notifs = 0usize;
        while let Ok(n) = self.notice_rx.pop() {
            handled_notifs += 1;
            match n {
                RtNotice::PatchResult {
                    req_id: _,
                    status,
                    patch,
                } => {
                    self.sound.patch = patch;
                    self.latest_pod.patch = patch;
                    if status == Status::Applied {
                        self.emit_notice(true, "patchApplied", "Oscillator topology / Vector4 patch updated");
                    } else if status == Status::Unsupported {
                        self.emit_notice(
                            false,
                            "patchBusyVoices",
                            "Cannot change oscillator topology while voices are sounding; wait for idle voices",
                        );
                    } else {
                        self.emit_notice(
                            false,
                            "patchInvalid",
                            "Rejected invalid oscillator topology or Vector4 parameters",
                        );
                    }
                }
                RtNotice::ReverbResult {
                    req_id: _,
                    status,
                    algorithm,
                } => {
                    self.sound.algorithm = algorithm;
                    self.latest_pod.reverb.algorithm = algorithm;
                    if status == Status::Applied {
                        self.emit_notice(
                            true,
                            "reverbSelected",
                            format!("Reverb algorithm switched to {:?}", algorithm),
                        );
                    } else {
                        self.emit_notice(
                            false,
                            "reverbActiveTail",
                            "Cannot switch reverb algorithm while tank tail is active; wait until dormant",
                        );
                    }
                }
                RtNotice::MusicErr(err) => {
                    self.emit_notice(false, "musicError", format!("Sequencer/Arp error: {:?}", err));
                }
                RtNotice::Ack(ack) => {
                    if ack.status == Status::Invalid {
                        self.emit_notice(false, "commandInvalid", "Engine rejected invalid command payload");
                    } else if ack.status == Status::Unsupported {
                        self.emit_notice(
                            false,
                            "commandUnsupported",
                            "Engine rejected unsupported live command",
                        );
                    }
                }
            }
        }

        // Complete CCR-01 idle reverb_enabled/kill_tail resync safely when voices & tank permit.
        if self.reverb_resync_pending && self.pending_swap_id.is_none() {
            let voices_idle = !self
                .latest_pod
                .voices
                .iter()
                .any(|v| v.active || v.gate || v.releasing);
            let tank_ready = self.latest_pod.reverb.dormant
                || (!self.sound.reverb_enabled && self.sound.kill_tail);
            if voices_idle && tank_ready {
                let mut target = self.sound.clone();
                if !target.reverb_enabled {
                    target.values[ParamId::ReverbWet as usize] = 0.0;
                }
                if let Ok(swap_id) =
                    self.worker
                        .prepare_reconfig(&target, self.sample_rate, self.max_frames)
                {
                    self.pending_swap_id = Some(swap_id);
                    self.reverb_resync_pending = false;
                }
            }
        }
        handled_notifs
    }

    pub fn is_voices_idle(&self) -> bool {
        !self
            .latest_pod
            .voices
            .iter()
            .any(|v| v.active || v.gate || v.releasing)
    }

    pub fn dispatch_hardware_midi(
        &mut self,
        source: PhysicalMidiSource,
        status: u8,
        data1: u8,
        data2: u8,
        explicit_note_id: Option<u32>,
        offset: u16,
    ) -> Result<bool, Status> {
        if !self.arbiter.accepts(source) {
            return Ok(false);
        }
        let note_id = explicit_note_id.unwrap_or(u32::from(data1));
        let Some(action) = self.cc_router.translate_midi(status, data1, data2, note_id) else {
            return Ok(false);
        };
        let cmd = Command {
            offset,
            origin: ORIGIN_HARDWARE_MIDI,
            serial: self.serials.next_serial(ORIGIN_HARDWARE_MIDI),
            action,
        };
        self.rt_tx
            .push(RtMessage::Port(cmd))
            .map_err(|_| Status::Capacity)?;
        Ok(true)
    }

    pub fn current_performance(&self) -> Performance {
        performance_record(
            self.music_tempo_milli,
            self.music_swing,
            self.music_active_pattern,
            self.music_arp_order,
            self.music_arp_octaves,
            self.music_arp_latch,
            self.music_arp_seed,
        )
    }

    pub fn snapshot(&mut self) -> UiSnapshot {
        self.poll();
        let pod = &self.latest_pod;
        let wire = pod.to_wire_snapshot(&self.host_capabilities, true);
        let patch = UiPatchState::from_pod(pod);
        let kernel_telemetry = UiKernelTelemetry::from_pod(pod);
        let parameters = ui_parameters_from_pod(pod);
        let music = Some(UiMusicState {
            playing: pod.scheduler_playing,
            active_pattern: self.music_active_pattern,
            pattern_len: self.music_bank.0[usize::from(self.music_active_pattern.min(63))].len,
            step_index: pod.scheduler_step,
            tempo_milli: self.music_tempo_milli,
            swing: self.music_swing,
            arp_enabled: self.music_arp_enabled,
            arp_order: self.music_arp_order.into(),
            arp_octaves: self.music_arp_octaves,
            arp_latch: self.music_arp_latch,
            arp_seed: self.music_arp_seed,
            current_pattern: UiPattern::from_bank(&self.music_bank, usize::from(self.music_active_pattern)),
        });
        let midi_learn = UiMidiLearnState {
            learning_target: self.cc_router.learning_target,
            cc_map: self.cc_router.cc_map.to_vec(),
            physical_source: self.arbiter.source(),
        };
        let reverb_tail_effective = if self.sound.kill_tail {
            WireTailPolicy::Kill
        } else {
            WireTailPolicy::Spill
        };
        UiSnapshot {
            version: 1,
            backend: self.backend.clone(),
            ready: self.ready,
            engine_port: wire,
            parameters,
            scope: pod.scope.to_vec(),
            patch,
            kernel_telemetry,
            music,
            midi_learn,
            drive_makeup: self.sound.drive_makeup,
            reverb_bypass_effective: self.sound.reverb_enabled,
            reverb_tail_effective,
            presets: self.presets.clone(),
            filter_analysis: self.filter_analysis.clone(),
            last_notice: self.last_notice.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub fn dispatch(&mut self, cmd: UiCommand) -> UiCommandReply {
        self.poll();
        match cmd {
            UiCommand::Param { id, value } => self.handle_param(id, value),
            UiCommand::Midi {
                status,
                data1,
                data2,
                note_id,
            } => self.handle_ui_midi(status, data1, data2, note_id),
            UiCommand::VectorXY { x, y } => self.handle_vector_xy(x, y),
            UiCommand::Panic { all_origins } => self.handle_panic(all_origins),
            UiCommand::FactoryPreset { index } => self.handle_factory_preset(index),
            UiCommand::PresetLoadJson { json } => self.handle_preset_load_json(&json),
            UiCommand::PresetSaveJson { name, category } => {
                self.handle_preset_save_json(name, category)
            }
            UiCommand::SetOversampling { factor } => self.handle_set_oversampling(factor),
            UiCommand::SetPatch {
                topology,
                routes,
                ratios,
                stages,
                modulation,
                loop_held,
            } => self.handle_set_patch(topology, routes, ratios, stages, modulation, loop_held),
            UiCommand::SetReverb {
                algorithm,
                enabled,
                tail,
            } => self.handle_set_reverb(algorithm, enabled, tail),
            UiCommand::SetDriveMakeup { enabled } => {
                self.sound.drive_makeup = enabled;
                if self.rt_tx.push(RtMessage::SetDriveMakeup(enabled)).is_err() {
                    return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
                }
                UiCommandReply::ok("Drive makeup updated")
            }
            UiCommand::MidiLearn {
                action,
                param_id,
                cc,
            } => self.handle_midi_learn(&action, param_id, cc),
            UiCommand::SetPhysicalMidiSource { source } => {
                if let Some(panic_cmd) = self.arbiter.switch_source(source, &self.serials, 0) {
                    let _ = self.rt_tx.push(RtMessage::Port(panic_cmd));
                }
                UiCommandReply::ok("Physical MIDI owner updated")
            }
            UiCommand::MusicControl {
                op,
                pattern,
                pattern_len,
                step_index,
                step,
                tempo_milli,
                swing,
                arp_enabled,
                arp_order,
                arp_octaves,
                arp_latch,
                arp_seed,
                grid_step,
                frame,
            } => self.handle_music_control(
                &op,
                pattern,
                pattern_len,
                step_index,
                step,
                tempo_milli,
                swing,
                arp_enabled,
                arp_order,
                arp_octaves,
                arp_latch,
                arp_seed,
                grid_step,
                frame,
            ),
            UiCommand::AnalyzeFilter { sample_rate } => {
                let sr = sample_rate.unwrap_or_else(|| f64::from(self.sample_rate));
                match measure_filter_response(&self.sound.values, sr, None, None) {
                    Ok(report) => {
                        self.filter_analysis = Some(report.clone());
                        let mut reply = UiCommandReply::ok(report.reason.clone());
                        reply.filter_analysis = Some(report);
                        reply
                    }
                    Err(e) => UiCommandReply::err(
                        Some(Status::Invalid),
                        format!("VCF analysis failed: {:?}", e),
                    ),
                }
            }
            UiCommand::HostControl { op, .. } => UiCommandReply::err(
                Some(Status::Unsupported),
                format!("HostControl '{op}' must be handled by the platform host adapter"),
            ),
        }
    }

    fn handle_param(&mut self, id: u16, value: f32) -> UiCommandReply {
        if !value.is_finite() {
            return UiCommandReply::err(Some(Status::Invalid), "Non-finite parameter value");
        }
        if id == ParamId::OversamplingMode as u16 {
            return UiCommandReply::err(
                Some(Status::Unsupported),
                "Use SetOversampling for off-thread engine re-preparation",
            );
        }
        if id == GLIDE_ENABLED {
            if value != 0.0 && value != 1.0 {
                return UiCommandReply::err(Some(Status::Invalid), "p83 Glide Enabled must be 0 or 1");
            }
            self.sound.glide = value == 1.0;
        } else if let Some(spec) = PARAM_SPECS.get(usize::from(id)) {
            if !(spec.min..=spec.max).contains(&value) || (spec.integer && value.fract() != 0.0) {
                return UiCommandReply::err(
                    Some(Status::Invalid),
                    format!("Parameter p{id} ({}) out of valid range", spec.name),
                );
            }
            if id == ParamId::ReverbWet as u16 && self.sound.reverb_enabled {
                self.stored_reverb_wet = value;
            }
            self.sound.values[usize::from(id)] = value;
        } else {
            return UiCommandReply::err(
                Some(Status::Unsupported),
                format!("Unknown parameter ID {id}"),
            );
        }
        if !self.audio_running {
            self.sync_pod_from_sound();
        }

        let serial = self.serials.next_serial(ORIGIN_LOCAL_UI);
        let Some(port_cmd) = ui_to_rt_command(&UiCommand::Param { id, value }, ORIGIN_LOCAL_UI, serial, 0)
        else {
            return UiCommandReply::err(Some(Status::Invalid), "Failed to convert UI param command");
        };
        if self.rt_tx.push(RtMessage::Port(port_cmd)).is_err() {
            return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
        }
        UiCommandReply::ok("Parameter queued")
    }

    fn handle_ui_midi(
        &mut self,
        status: u8,
        data1: u8,
        data2: u8,
        note_id: Option<u32>,
    ) -> UiCommandReply {
        if !(128..=239).contains(&status) || data1 > 127 || data2 > 127 {
            return UiCommandReply::err(Some(Status::Invalid), "Malformed MIDI bytes");
        }
        let nid = note_id.unwrap_or(u32::from(data1));
        let Some(action) = self.cc_router.translate_midi(status, data1, data2, nid) else {
            return UiCommandReply::ok("Unmapped CC ignored");
        };
        let cmd = Command {
            offset: 0,
            origin: ORIGIN_LOCAL_UI,
            serial: self.serials.next_serial(ORIGIN_LOCAL_UI),
            action,
        };
        if self.rt_tx.push(RtMessage::Port(cmd)).is_err() {
            return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
        }
        UiCommandReply::ok("MIDI queued")
    }

    fn handle_vector_xy(&mut self, x: f32, y: f32) -> UiCommandReply {
        if !x.is_finite() || !y.is_finite() || x.abs() + y.abs() > 1.0 {
            return UiCommandReply::err(
                Some(Status::Invalid),
                "Vector4 XY must satisfy |x| + |y| <= 1",
            );
        }
        if self.sound.patch.topology != prophet_dsp::vector::Topology::Vector4 {
            return UiCommandReply::err(
                Some(Status::Unsupported),
                "VectorXY requires Vector4 oscillator topology",
            );
        }
        self.sound.patch.position = prophet_dsp::vector::Point { x, y };
        let cmd = Command {
            offset: 0,
            origin: ORIGIN_LOCAL_UI,
            serial: self.serials.next_serial(ORIGIN_LOCAL_UI),
            action: Action::VectorXY { x, y },
        };
        if self.rt_tx.push(RtMessage::Port(cmd)).is_err() {
            return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
        }
        UiCommandReply::ok("VectorXY queued")
    }

    fn handle_panic(&mut self, all_origins: bool) -> UiCommandReply {
        if all_origins {
            for orig in 0..(ORIGINS as u8) {
                let cmd = Command {
                    offset: 0,
                    origin: orig,
                    serial: self.serials.next_serial(orig),
                    action: Action::PanicOrigin,
                };
                if self.rt_tx.push(RtMessage::Port(cmd)).is_err() {
                    return UiCommandReply::err(Some(Status::Capacity), "RT queue full during panic");
                }
            }
            let _ = self.rt_tx.push(RtMessage::Music(RtMusicOp::Stop));
        } else {
            let cmd = Command {
                offset: 0,
                origin: ORIGIN_LOCAL_UI,
                serial: self.serials.next_serial(ORIGIN_LOCAL_UI),
                action: Action::PanicOrigin,
            };
            if self.rt_tx.push(RtMessage::Port(cmd)).is_err() {
                return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
            }
        }
        UiCommandReply::ok("Panic queued")
    }

    fn handle_factory_preset(&mut self, index: usize) -> UiCommandReply {
        let Some(p) = preset::factory_preset(index) else {
            return UiCommandReply::err(
                Some(Status::Invalid),
                format!("Unknown factory preset index {index}"),
            );
        };
        match self
            .worker
            .prepare_reconfig(&p.sound, self.sample_rate, self.max_frames)
        {
            Ok(swap_id) => {
                self.sound = p.sound;
                self.stored_reverb_wet = self.sound.values[ParamId::ReverbWet as usize];
                self.reverb_resync_pending = false;
                self.pending_swap_id = Some(swap_id);
                if !self.audio_running {
                    self.sync_pod_from_sound();
                }
                self.emit_notice(true, "factoryPreset", format!("Loaded factory preset: {}", p.name));
                let mut reply = UiCommandReply::ok(format!("Loaded factory preset {}", p.name));
                reply.swap_id = Some(swap_id);
                reply
            }
            Err(st) => UiCommandReply::err(Some(st), "Failed to prepare factory preset"),
        }
    }

    fn handle_preset_load_json(&mut self, json: &str) -> UiCommandReply {
        let bundle = match preset::decode_and_prepare(
            json.as_bytes(),
            self.sample_rate,
            self.max_frames,
        ) {
            Ok(b) => b,
            Err(e) => {
                let msg = e.describe().to_owned();
                self.emit_notice(false, "presetLoadFailed", msg.clone());
                return UiCommandReply::err(Some(Status::Invalid), msg);
            }
        };
        let preset_name = bundle.preset.name.clone();
        let next_sound = bundle.preset.sound.clone();
        let next_bank = bundle.bank.clone();
        let next_perf = bundle.preset.performance;
        match self.worker.submit_prepared(
            bundle.engine,
            bundle.bank,
            bundle.performance,
            bundle.preset.performance,
        ) {
            Ok(swap_id) => {
                self.sound = next_sound;
                self.stored_reverb_wet = self.sound.values[ParamId::ReverbWet as usize];
                self.reverb_resync_pending = false;
                if let Some(bank) = next_bank {
                    self.music_bank = bank;
                }
                if let Some(perf) = next_perf {
                    self.music_active_pattern = perf.pattern;
                    self.music_tempo_milli = perf.tempo_milli;
                    self.music_swing = perf.swing;
                    self.music_arp_octaves = perf.octaves;
                    self.music_arp_latch = perf.latch;
                    self.music_arp_seed = perf.seed;
                    self.music_arp_order = match perf.mode {
                        1 => Order::Down,
                        2 => Order::UpDown,
                        3 => Order::AsPlayed,
                        4 => Order::Random,
                        5 => Order::Chord,
                        _ => Order::Up,
                    };
                }
                self.pending_swap_id = Some(swap_id);
                if !self.audio_running {
                    self.sync_pod_from_sound();
                }
                self.emit_notice(true, "presetLoaded", format!("Loaded preset: {}", preset_name));
                let mut reply = UiCommandReply::ok(format!("Loaded preset {}", preset_name));
                reply.swap_id = Some(swap_id);
                reply
            }
            Err(st) => {
                self.emit_notice(false, "presetSwapFailed", "Worker swap queue full");
                UiCommandReply::err(Some(st), "Worker swap queue full")
            }
        }
    }

    fn handle_preset_save_json(&mut self, name: String, category: String) -> UiCommandReply {
        let p = Preset {
            name: if name.trim().is_empty() {
                "Untitled Patch".to_owned()
            } else {
                name
            },
            category: if category.trim().is_empty() {
                "User".to_owned()
            } else {
                category
            },
            sound: self.sound.clone(),
            bank: Some(self.music_bank.clone()),
            performance: Some(self.current_performance()),
        };
        match p.encode() {
            Ok(bytes) => {
                let json = String::from_utf8_lossy(&bytes).into_owned();
                let mut reply = UiCommandReply::ok("Encoded preset v2 with CRC-32");
                reply.preset_json = Some(json);
                reply
            }
            Err(e) => {
                let host_err = HostPresetError::Codec(e);
                UiCommandReply::err(Some(Status::Invalid), host_err.describe())
            }
        }
    }

    fn handle_set_oversampling(&mut self, factor: u8) -> UiCommandReply {
        if !matches!(factor, 1 | 2 | 4) {
            return UiCommandReply::err(
                Some(Status::Invalid),
                "Oversampling factor must be 1, 2, or 4",
            );
        }
        let mut target = self.sound.clone();
        target.values[ParamId::OversamplingMode as usize] = f32::from(factor);
        match self
            .worker
            .prepare_reconfig(&target, self.sample_rate, self.max_frames)
        {
            Ok(swap_id) => {
                self.sound = target;
                self.pending_swap_id = Some(swap_id);
                if !self.audio_running {
                    self.sync_pod_from_sound();
                }
                self.emit_notice(true, "oversamplingSwapped", format!("Oversampling prepared at {factor}x"));
                let mut reply = UiCommandReply::ok(format!("Prepared {factor}x oversampling swap"));
                reply.swap_id = Some(swap_id);
                reply
            }
            Err(st) => UiCommandReply::err(Some(st), "Failed to prepare oversampling swap"),
        }
    }

    fn handle_set_patch(
        &mut self,
        topology: crate::ui::UiTopology,
        routes: [crate::ui::UiRoute; 2],
        ratios: [f32; 2],
        stages: [crate::ui::UiStage; 5],
        modulation: [crate::ui::UiPoint; 3],
        loop_held: bool,
    ) -> UiCommandReply {
        let mut candidate = self.sound.clone();
        if let Err(msg) =
            apply_ui_patch_fields(&mut candidate, topology, routes, ratios, stages, modulation, loop_held)
        {
            self.emit_notice(false, "patchInvalid", msg);
            return UiCommandReply::err(Some(Status::Invalid), msg);
        }
        if !self.audio_running {
            self.sound.patch = candidate.patch;
            self.latest_pod.patch = candidate.patch;
            return UiCommandReply::ok("Oscillator patch stored while audio stopped");
        }
        if !self.is_voices_idle() {
            let msg = "Oscillator topology cannot change while voices are sounding; release all keys first";
            self.emit_notice(false, "patchBusyVoices", msg);
            return UiCommandReply::err(Some(Status::Unsupported), msg);
        }
        self.req_seq += 1;
        let req_id = self.req_seq;
        if self
            .rt_tx
            .push(RtMessage::SetPatch {
                req_id,
                patch: candidate.patch,
            })
            .is_err()
        {
            return UiCommandReply::err(Some(Status::Capacity), "RT command queue full");
        }
        self.sound.patch = candidate.patch;
        UiCommandReply::ok("Patch update submitted")
    }

    fn handle_set_reverb(
        &mut self,
        algorithm: Option<crate::wire::WireAlgorithm>,
        enabled: Option<bool>,
        tail: Option<WireTailPolicy>,
    ) -> UiCommandReply {
        if let Some(alg_wire) = algorithm {
            let target_alg: Algorithm = alg_wire.into();
            if target_alg != self.sound.algorithm {
                if self.audio_running && !self.latest_pod.reverb.dormant {
                    let msg = "Reverb tank has an active tail; cannot switch algorithm until dormant";
                    self.emit_notice(false, "reverbActiveTail", msg);
                    return UiCommandReply::err(Some(Status::Unsupported), msg);
                }
                if !self.audio_running {
                    self.sound.algorithm = target_alg;
                    self.latest_pod.reverb.algorithm = target_alg;
                } else {
                    self.req_seq += 1;
                    let req_id = self.req_seq;
                    if self
                        .rt_tx
                        .push(RtMessage::SelectReverb {
                            req_id,
                            algorithm: target_alg,
                        })
                        .is_err()
                    {
                        return UiCommandReply::err(Some(Status::Capacity), "RT queue full");
                    }
                    self.sound.algorithm = target_alg;
                }
            }
        }

        if let Some(en) = enabled {
            if en != self.sound.reverb_enabled {
                self.sound.reverb_enabled = en;
                self.reverb_resync_pending = true;
                let wet_target = if en { self.stored_reverb_wet } else { 0.0 };
                let cmd = Command {
                    offset: 0,
                    origin: ORIGIN_LOCAL_UI,
                    serial: self.serials.next_serial(ORIGIN_LOCAL_UI),
                    action: Action::Parameter {
                        id: ParamId::ReverbWet as u16,
                        value: wet_target,
                    },
                };
                let _ = self.rt_tx.push(RtMessage::Port(cmd));
            }
        }

        if let Some(tp) = tail {
            let kill = tp == WireTailPolicy::Kill;
            if kill != self.sound.kill_tail {
                self.sound.kill_tail = kill;
                self.reverb_resync_pending = true;
            }
        }
        if !self.audio_running {
            self.sync_pod_from_sound();
        }

        UiCommandReply::ok("Reverb configuration updated")
    }

    fn handle_midi_learn(&mut self, action: &str, param_id: Option<u16>, cc: Option<u8>) -> UiCommandReply {
        match action {
            "begin" => {
                let Some(id) = param_id else {
                    return UiCommandReply::err(Some(Status::Invalid), "Missing paramId for MIDI Learn");
                };
                if self.cc_router.begin_learn(id) {
                    UiCommandReply::ok(format!("MIDI Learn armed for p{id}"))
                } else {
                    UiCommandReply::err(Some(Status::Unsupported), "Parameter cannot be MIDI-learned")
                }
            }
            "cancel" => {
                self.cc_router.cancel_learn();
                UiCommandReply::ok("MIDI Learn cancelled")
            }
            "unlearn" => {
                let Some(id) = param_id else {
                    return UiCommandReply::err(Some(Status::Invalid), "Missing paramId");
                };
                if self.cc_router.unlearn(id) {
                    UiCommandReply::ok(format!("MIDI CC unlearned from p{id}"))
                } else {
                    UiCommandReply::err(Some(Status::Invalid), "Invalid parameter ID")
                }
            }
            "remap" => match (cc, param_id) {
                (Some(c), Some(id)) => {
                    if self.cc_router.remap(c, id) {
                        UiCommandReply::ok(format!("Mapped CC {c} to p{id}"))
                    } else {
                        UiCommandReply::err(
                            Some(Status::Invalid),
                            "Reserved/invalid MIDI CC or unlearnable parameter",
                        )
                    }
                }
                _ => UiCommandReply::err(Some(Status::Invalid), "Missing cc or paramId"),
            },
            _ => UiCommandReply::err(Some(Status::Invalid), format!("Unknown MIDI learn op '{action}'")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_music_control(
        &mut self,
        op: &str,
        pattern: Option<u8>,
        pattern_len: Option<u8>,
        step_index: Option<u8>,
        step: Option<crate::ui::UiStep>,
        tempo_milli: Option<u32>,
        swing: Option<u8>,
        arp_enabled: Option<bool>,
        arp_order: Option<crate::ui::UiArpOrder>,
        arp_octaves: Option<u8>,
        arp_latch: Option<bool>,
        arp_seed: Option<u32>,
        grid_step: Option<u32>,
        frame: Option<u64>,
    ) -> UiCommandReply {
        let mut ops: Vec<RtMusicOp> = Vec::with_capacity(6);
        match op {
            "start" => ops.push(RtMusicOp::StartFromZero),
            "startGrid" => ops.push(RtMusicOp::StartGrid {
                step: grid_step.unwrap_or(0),
                frame: frame.unwrap_or(0),
            }),
            "seek" => ops.push(RtMusicOp::Seek {
                frame: frame.unwrap_or(0),
            }),
            "stop" => ops.push(RtMusicOp::Stop),
            "setPattern" => {
                let p = pattern.unwrap_or(self.music_active_pattern).min(63);
                self.music_active_pattern = p;
                ops.push(RtMusicOp::SetPattern(p));
            }
            "setPatternLen" => {
                let p = pattern.unwrap_or(self.music_active_pattern).min(63);
                let len = pattern_len.unwrap_or(32);
                if !(1..=32).contains(&len) {
                    return UiCommandReply::err(Some(Status::Invalid), "Pattern length must be 1..=32");
                }
                self.music_bank.0[usize::from(p)].len = len;
                ops.push(RtMusicOp::SetPatternLen { pattern: p, len });
            }
            "setStep" => {
                let p = pattern.unwrap_or(self.music_active_pattern).min(63);
                let idx = step_index.unwrap_or(0);
                let Some(s) = step.map(Step::from) else {
                    return UiCommandReply::err(Some(Status::Invalid), "Missing step payload");
                };
                if usize::from(idx) >= 32 || !s.valid() {
                    return UiCommandReply::err(Some(Status::Invalid), "Step index or step fields out of range");
                }
                self.music_bank.0[usize::from(p)].steps[usize::from(idx)] = s;
                ops.push(RtMusicOp::SetStep {
                    pattern: p,
                    step_index: idx,
                    step: s,
                });
            }
            "setTempo" => {
                let tm = tempo_milli.unwrap_or(self.music_tempo_milli);
                if !(20_000..=400_000).contains(&tm) {
                    return UiCommandReply::err(
                        Some(Status::Invalid),
                        "Tempo must be 20.0..=400.0 BPM (20_000..=400_000 milli-BPM)",
                    );
                }
                self.music_tempo_milli = tm;
                ops.push(RtMusicOp::SetTempo(tm));
            }
            "setSwing" => {
                let sw = swing.unwrap_or(self.music_swing);
                if sw > 75 {
                    return UiCommandReply::err(Some(Status::Invalid), "Swing must be 0..=75%");
                }
                self.music_swing = sw;
                ops.push(RtMusicOp::SetSwing(sw));
            }
            "setArp" => {
                if let Some(en) = arp_enabled {
                    self.music_arp_enabled = en;
                    ops.push(RtMusicOp::SetArpEnabled(en));
                }
                if let Some(ord) = arp_order {
                    let o: Order = ord.into();
                    self.music_arp_order = o;
                    ops.push(RtMusicOp::SetArpOrder(o));
                }
                if let Some(oct) = arp_octaves {
                    if !(1..=4).contains(&oct) {
                        return UiCommandReply::err(Some(Status::Invalid), "Arp octaves must be 1..=4");
                    }
                    self.music_arp_octaves = oct;
                    ops.push(RtMusicOp::SetArpOctaves(oct));
                }
                if let Some(latch) = arp_latch {
                    self.music_arp_latch = latch;
                    ops.push(RtMusicOp::SetArpLatch(latch));
                }
                if let Some(seed) = arp_seed {
                    self.music_arp_seed = seed;
                    ops.push(RtMusicOp::SetArpSeed(seed));
                }
            }
            "arpKey" => {
                let k = step_index.unwrap_or(60).min(127);
                let on = arp_enabled.unwrap_or(true);
                ops.push(RtMusicOp::ArpKey { key: k, on });
            }
            _ => {
                return UiCommandReply::err(
                    Some(Status::Invalid),
                    format!("Unknown MusicControl op '{op}'"),
                )
            }
        }
        for mop in ops {
            if self.rt_tx.push(RtMessage::Music(mop)).is_err() {
                return UiCommandReply::err(Some(Status::Capacity), "RT queue full");
            }
        }
        UiCommandReply::ok("Music transport updated")
    }
}
