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
/// Context of one callback whose processing exceeded its own audio duration.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissRecord {
    /// Monotonic time since the processor was created.
    pub at_ns: u64,
    pub frames: usize,
    pub processing_ns: u64,
    pub interval_ns: u64,
    pub active_voices: u32,
}
pub const LOAD_BUCKETS: usize = 64;
/// Width of one load histogram bucket, in percent of the callback budget.
pub const LOAD_BUCKET_PCT: f32 = 2.5;
/// Whole-session callback timing, kept allocation-free on the audio thread.
///
/// `load` is processing time over the callback's own audio duration, so a deadline
/// miss is load above 100 %. A late wakeup is a callback interval longer than 1.5x
/// the previous callback's audio: the OS scheduled us late, whatever the DSP cost.
/// Separating the two tells DSP overload apart from scheduler or driver stalls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionTiming {
    pub callbacks: u64,
    /// The last bucket is open-ended (>= 157.5 %).
    pub load_hist: [u64; LOAD_BUCKETS],
    pub late_wakeups: u64,
    pub frames_min: usize,
    pub frames_max: usize,
    pub misses_with_voices: u64,
    pub misses_idle: u64,
    pub first_miss: Option<MissRecord>,
    pub last_miss: Option<MissRecord>,
    pub worst_miss: Option<MissRecord>,
}
impl Default for SessionTiming {
    fn default() -> Self {
        Self {
            callbacks: 0,
            load_hist: [0; LOAD_BUCKETS],
            late_wakeups: 0,
            frames_min: 0,
            frames_max: 0,
            misses_with_voices: 0,
            misses_idle: 0,
            first_miss: None,
            last_miss: None,
            worst_miss: None,
        }
    }
}
impl SessionTiming {
    /// Upper edge of the histogram bucket holding quantile `q` (0..=1), in percent.
    pub fn load_percentile(&self, q: f64) -> f32 {
        if self.callbacks == 0 {
            return 0.0;
        }
        let rank = ((self.callbacks as f64 * q.clamp(0.0, 1.0)).ceil() as u64).max(1);
        let mut seen = 0;
        for (i, n) in self.load_hist.iter().enumerate() {
            seen += n;
            if seen >= rank {
                return (i + 1) as f32 * LOAD_BUCKET_PCT;
            }
        }
        LOAD_BUCKETS as f32 * LOAD_BUCKET_PCT
    }
    pub fn deadline_misses(&self) -> u64 {
        self.misses_with_voices + self.misses_idle
    }
    fn record(&mut self, frames: usize, load: f32, miss: MissRecord) {
        self.callbacks += 1;
        let bucket = ((load / LOAD_BUCKET_PCT).max(0.0) as usize).min(LOAD_BUCKETS - 1);
        self.load_hist[bucket] += 1;
        self.frames_min = if self.callbacks == 1 { frames } else { self.frames_min.min(frames) };
        self.frames_max = self.frames_max.max(frames);
        if load > 100.0 {
            if miss.active_voices > 0 {
                self.misses_with_voices += 1;
            } else {
                self.misses_idle += 1;
            }
            self.first_miss.get_or_insert(miss);
            self.last_miss = Some(miss);
            if self.worst_miss.is_none_or(|w| {
                // Compare overload ratios without division by zero frames.
                miss.processing_ns as u128 * w.frames.max(1) as u128
                    > w.processing_ns as u128 * miss.frames.max(1) as u128
            }) {
                self.worst_miss = Some(miss);
            }
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
    pub session: SessionTiming,
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
    session: SessionTiming,
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
            session: SessionTiming::default(),
        }
    }
    pub fn session_timing(&self) -> &SessionTiming {
        &self.session
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
            let expected_ns = self.callback_frames as f64 * 1e9 / self.engine.kernel.sample_rate as f64;
            if self.callback_frames > 0 && self.callback_interval_ns as f64 > 1.5 * expected_ns {
                self.session.late_wakeups += 1;
            }
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
                    Command::Param { id, .. } if id < NUM_PARAMS => {
                        self.parameter_command_ids[id] = stamp.id
                    }
                    Command::Preset(_) | Command::Snapshot(_) => {
                        self.parameter_command_ids.fill(stamp.id)
                    }
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
                parameter_command_ids: self.parameter_command_ids,
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
                session: self.session,
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
        let miss = MissRecord {
            at_ns: start.duration_since(self.origin).as_nanos() as u64,
            frames,
            processing_ns: self.processing_ns,
            interval_ns: self.callback_interval_ns,
            active_voices: self.engine.kernel.last_telemetry.active_voice_mask.count_ones(),
        };
        self.session.record(frames, load, miss);
    }
}

pub mod transfer {
    use prophet_analysis::{
        log_frequencies, Error, RequestToken, TransferAnalyzer, TransferMetadata, TransferPoint,
    };
    use prophet_dsp::vcf_moog::DualVcfStage;
    use prophet_params::ParamId::*;
    use prophet_params::NUM_PARAMS;

    #[derive(Clone, Debug, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct FilterTransfer {
        pub sample_rate: f64,
        pub hz: Vec<f64>,
        pub db: Vec<f64>,
        pub phase: Vec<Option<f64>>,
        pub fft_size: usize,
        pub impulse_samples: usize,
        pub tail_converged: bool,
        pub tail_relative_rms: f64,
        pub bin_hz: f64,
    }

    pub fn tail_converged(impulse: &[f64]) -> bool {
        let energy = impulse.iter().map(|x| x * x).sum::<f64>();
        if energy == 0.0 {
            return true;
        }
        let tail = &impulse[impulse.len() * 7 / 8..];
        let tail_energy = tail.iter().map(|x| x * x).sum::<f64>();
        let rel = (tail_energy / tail.len() as f64 / (energy / impulse.len() as f64)).sqrt();
        rel < 1e-4
    }

    pub fn compute_filter_transfer(
        values: &[f32; NUM_PARAMS],
        sample_rate: f64,
        token: Option<&RequestToken>,
    ) -> Result<FilterTransfer, Error> {
        let check = || {
            if token.is_some_and(|t| !t.is_current()) {
                Err(Error::Superseded)
            } else {
                Ok(())
            }
        };
        check()?;

        if !sample_rate.is_finite() || !(8000.0..=384000.0).contains(&sample_rate) {
            return Err(Error::InvalidConfig);
        }

        let mut f = DualVcfStage::new(sample_rate as f32, 1.0);
        f.set_params(
            values[FilterCutoff as usize],
            values[FilterResonance as usize],
            values[FilterDrive as usize],
            values[MoogBassComp as usize],
            values[FilterMorph as usize],
        );

        let mut freq_grid = [0.0f64; 256];
        let upper = (sample_rate * 0.49).min(20000.0);
        log_frequencies(sample_rate, 20.0, upper, &mut freq_grid)?;

        let mut impulse = Vec::with_capacity(4096);
        let lengths = [4096usize, 8192, 16384, 32768, 65536, 131072];
        for (idx, &target_len) in lengths.iter().enumerate() {
            check()?;
            let start = impulse.len();
            for i in start..target_len {
                if i % 1024 == 0 {
                    check()?;
                }
                let x = if i == 0 { 0.0001f32 } else { 0.0f32 };
                impulse.push(f.process_sample(x) as f64 / 0.0001);
            }
            if tail_converged(&impulse) || idx + 1 == lengths.len() {
                break;
            }
        }

        check()?;
        let fft_size = impulse.len() * 2;
        let mut analyzer = TransferAnalyzer::new(fft_size)?;
        let mut points = vec![TransferPoint::default(); 256];
        let meta: TransferMetadata = analyzer.analyze(
            &impulse,
            sample_rate,
            &freq_grid,
            -100.0,
            &mut points,
            token,
        )?;

        Ok(FilterTransfer {
            sample_rate: meta.sample_rate,
            hz: points.iter().map(|p| p.hz).collect(),
            db: points.iter().map(|p| p.db).collect(),
            phase: points.iter().map(|p| p.phase).collect(),
            fft_size: meta.fft_size,
            impulse_samples: meta.impulse_samples,
            tail_converged: meta.tail_converged,
            tail_relative_rms: meta.tail_relative_rms,
            bin_hz: meta.bin_hz,
        })
    }
}

#[cfg(test)]
mod session_timing_tests {
    use super::*;

    fn miss(frames: usize, processing_us: u64, voices: u32) -> MissRecord {
        MissRecord {
            at_ns: processing_us,
            frames,
            processing_ns: processing_us * 1000,
            interval_ns: 0,
            active_voices: voices,
        }
    }

    #[test]
    fn whole_session_percentiles_come_from_the_load_histogram() {
        let mut s = SessionTiming::default();
        for _ in 0..98 {
            s.record(480, 10.0, miss(480, 1000, 2));
        }
        s.record(480, 60.0, miss(480, 6000, 12));
        s.record(480, 130.0, miss(480, 13000, 12));
        assert_eq!(s.callbacks, 100);
        assert_eq!(s.load_percentile(0.5), 12.5);
        assert_eq!(s.load_percentile(0.99), 62.5);
        assert_eq!(s.load_percentile(1.0), 132.5);
        assert_eq!(s.deadline_misses(), 1);
    }

    #[test]
    fn misses_split_by_voice_activity_and_keep_first_last_worst() {
        let mut s = SessionTiming::default();
        s.record(480, 120.0, miss(480, 12000, 0));
        s.record(96, 300.0, miss(96, 6000, 12));
        s.record(480, 101.0, miss(480, 10100, 12));
        assert_eq!((s.misses_idle, s.misses_with_voices), (1, 2));
        assert_eq!(s.first_miss.unwrap().active_voices, 0);
        assert_eq!(s.last_miss.unwrap().processing_ns, 10_100_000);
        // 6 ms over 96 frames (2 ms of audio) is the worst overload ratio.
        assert_eq!(s.worst_miss.unwrap().frames, 96);
        assert_eq!((s.frames_min, s.frames_max), (96, 480));
    }

    #[test]
    fn overload_beyond_the_histogram_lands_in_the_open_bucket() {
        let mut s = SessionTiming::default();
        s.record(480, 5000.0, miss(480, 500_000, 12));
        assert_eq!(s.load_percentile(1.0), LOAD_BUCKETS as f32 * LOAD_BUCKET_PCT);
    }
}
