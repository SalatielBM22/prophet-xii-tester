// SPDX-License-Identifier: GPL-3.0-only
use prophet_control::ProphetXiiPlugin;
use prophet_dsp::{KernelTelemetrySnapshot, MAX_BLOCK_SIZE};
use prophet_params::{ParamId, NUM_PARAMS};
use rtrb::{Consumer, Producer};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Param {
        id: usize,
        value: f32,
    },
    Midi {
        status: u8,
        data1: u8,
        data2: u8,
    },
    Preset(usize),
    Snapshot([f32; NUM_PARAMS]),
    Learn(usize),
    CancelLearn,
    Unlearn(usize),
    Remap {
        cc: usize,
        id: usize,
    },
    ReverbBypass {
        enabled: bool,
        tail: prophet_dsp::fx::solaris_reverb::ReverbTailPolicy,
    },
    Panic,
    Trimpot {
        voice: usize,
        field: usize,
        value: f32,
    },
}
/// One producer stamps entry into the RT queue; ingress and RT queue waits are distinct.
#[derive(Clone, Copy, Debug)]
pub struct StampedCommand {
    pub command: Command,
    pub id: u64,
    pub received: Instant,
    pub queued: Instant,
}
enum CommandInput {
    Plain(Consumer<Command>),
    Stamped(Consumer<StampedCommand>),
}
impl CommandInput {
    fn pop(&mut self) -> Option<(Command, Option<StampedCommand>)> {
        match self {
            Self::Plain(q) => q.pop().ok().map(|c| (c, None)),
            Self::Stamped(q) => q.pop().ok().map(|c| (c.command, Some(c))),
        }
    }
    fn slots(&self) -> usize {
        match self {
            Self::Plain(q) => q.slots(),
            Self::Stamped(q) => q.slots(),
        }
    }
}
/// Contiguous mono acquisition, not a periodically sampled scope. Sequence
/// advances even when the bounded transport is full, making gaps detectable.
#[derive(Clone, Copy)]
pub struct CaptureChunk {
    pub sequence: u64,
    pub first_frame: u64,
    pub sample_rate: f32,
    pub len: usize,
    pub samples: [f32; MAX_BLOCK_SIZE],
}
#[derive(Clone, Copy)]
pub struct Telemetry {
    pub kernel: KernelTelemetrySnapshot,
    pub scope: [f32; 1024],
    pub cpu: f32,
    pub peak_cpu: f32,
    pub deadline_misses: u64,
    pub frames: u64,
    pub sample_rate: f32,
    pub params: [f32; NUM_PARAMS],
    pub trimpots: [[f32; 4]; 12],
    pub late_commands: u64,
    pub dropped_telemetry: u64,
    pub revision: u64,
    pub parameter_command_ids: [u64; NUM_PARAMS],
    pub learn_active: bool,
    pub learn_target: u32,
    pub cc_map: [u32; 128],
    pub reverb_enabled: bool,
    pub reverb_kill: bool,
    pub reverb_dormant: bool,
    pub reverb_enable_gain: f32,
    pub sequence: u64,
    pub monotonic_ns: u64,
    pub callback_frames: usize,
    pub processing_ns: u64,
    pub callback_interval_ns: u64,
    pub timing_ns: [u64; 128],
    pub timing_count: usize,
    pub last_command_id: u64,
    pub applied_frame: u64,
    pub control_wait_us: u64,
    pub relay_wait_us: u64,
    pub rt_queue_wait_us: u64,
    pub queue_depth: usize,
    pub peak_queue_depth: usize,
    pub dropped_capture: u64,
    pub backend_buffered_ns: Option<u64>,
    pub invalid_callbacks: u64,
}
pub struct AudioProcessor {
    pub engine: ProphetXiiPlugin,
    commands: CommandInput,
    telemetry: Producer<Telemetry>,
    scope: [f32; 1024],
    scope_pos: usize,
    pub frames: u64,
    last_publish: u64,
    cpu: f32,
    peak_cpu: f32,
    deadline_misses: u64,
    late_commands: u64,
    dropped_telemetry: u64,
    pub underruns: Arc<AtomicU64>,
    capture: Option<Producer<CaptureChunk>>,
    capture_sequence: u64,
    dropped_capture: u64,
    origin: Instant,
    previous_callback: Option<Instant>,
    callback_interval_ns: u64,
    callback_frames: usize,
    processing_ns: u64,
    timing_ns: [u64; 128],
    timing_count: usize,
    telemetry_sequence: u64,
    last_command_id: u64,
    parameter_command_ids: [u64; NUM_PARAMS],
    applied_frame: u64,
    control_wait_us: u64,
    relay_wait_us: u64,
    rt_queue_wait_us: u64,
    peak_queue_depth: usize,
    pub backend_buffered_ns: Option<u64>,
    invalid_callbacks: u64,
}
impl AudioProcessor {
    pub fn new(
        sr: f32,
        commands: Consumer<Command>,
        telemetry: Producer<Telemetry>,
        underruns: Arc<AtomicU64>,
    ) -> Self {
        Self::with_queue(sr, CommandInput::Plain(commands), telemetry, underruns)
    }
    pub fn new_stamped(
        sr: f32,
        commands: Consumer<StampedCommand>,
        telemetry: Producer<Telemetry>,
        underruns: Arc<AtomicU64>,
    ) -> Self {
        Self::with_queue(sr, CommandInput::Stamped(commands), telemetry, underruns)
    }
    fn with_queue(
        sr: f32,
        commands: CommandInput,
        telemetry: Producer<Telemetry>,
        underruns: Arc<AtomicU64>,
    ) -> Self {
        assert!(sr.is_finite() && (8000.0..=192000.0).contains(&sr));
        Self {
            engine: ProphetXiiPlugin::new(sr),
            commands,
            telemetry,
            scope: [0.0; 1024],
            scope_pos: 0,
            frames: 0,
            last_publish: 0,
            cpu: 0.0,
            peak_cpu: 0.0,
            deadline_misses: 0,
            late_commands: 0,
            dropped_telemetry: 0,
            underruns,
            capture: None,
            capture_sequence: 0,
            dropped_capture: 0,
            origin: Instant::now(),
            previous_callback: None,
            callback_interval_ns: 0,
            callback_frames: 0,
            processing_ns: 0,
            timing_ns: [0; 128],
            timing_count: 0,
            telemetry_sequence: 0,
            last_command_id: 0,
            parameter_command_ids: [0; NUM_PARAMS],
            applied_frame: 0,
            control_wait_us: 0,
            relay_wait_us: 0,
            rt_queue_wait_us: 0,
            peak_queue_depth: 0,
            backend_buffered_ns: None,
            invalid_callbacks: 0,
        }
    }
    pub fn attach_capture(&mut self, producer: Producer<CaptureChunk>) {
        self.capture = Some(producer);
    }
    pub fn command(&mut self, c: Command) {
        match c {
            Command::Param { id, value } => {
                if let Some(id) = ParamId::from_index(id) {
                    self.engine.ipc.params.set(id, value)
                }
            }
            Command::Midi {
                status,
                data1,
                data2,
            } => {
                self.engine.sync_parameters();
                self.engine.handle_midi_message(status, data1, data2);
            }
            Command::Preset(i) => {
                self.engine.panic();
                self.engine.load_preset(i)
            }
            Command::Snapshot(values) => {
                for (i, v) in values.into_iter().enumerate() {
                    self.engine
                        .ipc
                        .params
                        .set(ParamId::from_index(i).unwrap(), v)
                }
            }
            Command::Learn(id) => {
                self.engine.ipc.params.begin_learn(id);
            }
            Command::CancelLearn => self.engine.ipc.params.cancel_learn(),
            Command::Unlearn(id) => {
                self.engine.ipc.params.unlearn(id);
            }
            Command::Remap { cc, id } => {
                self.engine.ipc.params.remap(cc, id);
            }
            Command::ReverbBypass { enabled, tail } => {
                self.engine.ipc.params.set_reverb_bypass(enabled, tail)
            }
            Command::Panic => self.engine.panic(),
            Command::Trimpot {
                voice,
                field,
                value,
            } => {
                if voice < 12 && value.is_finite() {
                    let p = &self.engine.ipc.params;
                    match field {
                        0 => p.voice_vco_a_cents[voice].store(value.clamp(-6.0, 6.0)),
                        1 => p.voice_vco_b_cents[voice].store(value.clamp(-6.0, 6.0)),
                        2 => p.voice_vcf_pct[voice].store(value.clamp(-0.05, 0.05)),
                        3 => p.voice_env_rc_pct[voice].store(value.clamp(-0.1, 0.1)),
                        _ => {}
                    }
                }
            }
        }
    }
    /// Called only by the audio backend. Bounded control draining, no heap allocation or locks.
    pub fn process<T: cpal::Sample + cpal::FromSample<f32>>(
        &mut self,
        output: &mut [T],
        channels: usize,
    ) {
        let start = Instant::now();
        if channels == 0 || output.is_empty() || output.len() % channels != 0 {
            output.fill(T::from_sample(0.0));
            self.invalid_callbacks += 1;
            return;
        }
        if let Some(previous) = self.previous_callback {
            self.callback_interval_ns = start.duration_since(previous).as_nanos() as u64;
        }
        self.previous_callback = Some(start);
        self.peak_queue_depth = self.peak_queue_depth.max(self.commands.slots());
        for _ in 0..256 {
            let Some((command, stamp)) = self.commands.pop() else {
                break;
            };
            self.command(command);
            if let Some(stamp) = stamp {
                match command {
                    Command::Param { id, .. } if id < NUM_PARAMS => self.parameter_command_ids[id] = stamp.id,
                    Command::Preset(_) | Command::Snapshot(_) => self.parameter_command_ids.fill(stamp.id),
                    _ => {}
                }
                self.last_command_id = stamp.id;
                self.applied_frame = self.frames;
                self.control_wait_us = stamp.received.elapsed().as_micros() as u64;
                self.relay_wait_us = stamp.queued.duration_since(stamp.received).as_micros() as u64;
                self.rt_queue_wait_us = stamp.queued.elapsed().as_micros() as u64;
            }
        }
        if self.commands.slots() > 0 {
            self.late_commands += 1;
        }
        let mut l = [0.0; MAX_BLOCK_SIZE];
        let mut r = [0.0; MAX_BLOCK_SIZE];
        let frames = output.len() / channels;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(MAX_BLOCK_SIZE);
            self.engine.process_audio_block(&mut l[..n], &mut r[..n]);
            for i in 0..n {
                self.scope[self.scope_pos] = l[i];
                self.scope_pos = (self.scope_pos + 1) % 1024;
                for c in 0..channels {
                    output[(pos + i) * channels + c] = T::from_sample(if c == 0 {
                        l[i]
                    } else if c == 1 {
                        r[i]
                    } else {
                        0.0
                    });
                }
            }
            if let Some(capture) = self.capture.as_mut() {
                let chunk = CaptureChunk {
                    sequence: self.capture_sequence,
                    first_frame: self.frames + pos as u64,
                    sample_rate: self.engine.kernel.sample_rate,
                    len: n,
                    samples: l,
                };
                if capture.push(chunk).is_err() {
                    self.dropped_capture += 1;
                }
                self.capture_sequence = self.capture_sequence.wrapping_add(1);
            }
            pos += n;
        }
        self.frames += frames as u64;
        if self.frames - self.last_publish >= self.engine.kernel.sample_rate as u64 / 25 {
            let mut scope = [0.0; 1024];
            for (i, x) in scope.iter_mut().enumerate() {
                *x = self.scope[(self.scope_pos + i) % 1024];
            }
            let p = &self.engine.ipc.params;
            let t = Telemetry {
                kernel: self.engine.kernel.telemetry_snapshot(),
                scope,
                cpu: self.cpu,
                peak_cpu: self.peak_cpu,
                deadline_misses: self.deadline_misses,
                frames: self.frames,
                sample_rate: self.engine.kernel.sample_rate,
                params: core::array::from_fn(|i| p.get(ParamId::from_index(i).unwrap())),
                trimpots: core::array::from_fn(|i| {
                    [
                        p.voice_vco_a_cents[i].load(),
                        p.voice_vco_b_cents[i].load(),
                        p.voice_vcf_pct[i].load(),
                        p.voice_env_rc_pct[i].load(),
                    ]
                }),
                late_commands: self.late_commands,
                dropped_telemetry: self.dropped_telemetry,
                revision: p.revision.load(Ordering::Relaxed),
                parameter_command_ids:self.parameter_command_ids,
                learn_active: p.midi_learn_active.load(Ordering::Relaxed),
                learn_target: p.midi_learn_target_param.load(Ordering::Relaxed),
                cc_map: core::array::from_fn(|i| p.cc_map[i].load(Ordering::Relaxed)),
                reverb_enabled: self.engine.kernel.reverb_params.enabled,
                reverb_kill: self.engine.kernel.reverb_params.bypass_tail
                    == prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Kill,
                reverb_dormant: self.engine.kernel.reverb.is_dormant(),
                reverb_enable_gain: self.engine.kernel.reverb.enable_gain(),
                sequence: self.telemetry_sequence,
                monotonic_ns: self.origin.elapsed().as_nanos() as u64,
                callback_frames: self.callback_frames,
                processing_ns: self.processing_ns,
                callback_interval_ns: self.callback_interval_ns,
                timing_ns: self.timing_ns,
                timing_count: self.timing_count.min(128),
                last_command_id: self.last_command_id,
                applied_frame: self.applied_frame,
                control_wait_us: self.control_wait_us,
                relay_wait_us: self.relay_wait_us,
                rt_queue_wait_us: self.rt_queue_wait_us,
                queue_depth: self.commands.slots(),
                peak_queue_depth: self.peak_queue_depth,
                dropped_capture: self.dropped_capture,
                backend_buffered_ns: self.backend_buffered_ns,
                invalid_callbacks: self.invalid_callbacks,
            };
            if self.telemetry.push(t).is_err() {
                self.dropped_telemetry += 1;
            }
            self.telemetry_sequence = self.telemetry_sequence.wrapping_add(1);
            self.last_publish = self.frames;
        }
        // Includes bounded control draining, capture and telemetry publication.
        self.processing_ns = start.elapsed().as_nanos() as u64;
        self.callback_frames = frames;
        self.timing_ns[self.timing_count % 128] = self.processing_ns;
        self.timing_count = self.timing_count.wrapping_add(1);
        let load = self.processing_ns as f32 * 1e-9
            / (frames as f32 / self.engine.kernel.sample_rate)
            * 100.0;
        self.cpu = self.cpu * 0.9 + load * 0.1;
        self.peak_cpu = self.peak_cpu.max(load);
        if load > 100.0 {
            self.deadline_misses += 1;
        }
    }
}
