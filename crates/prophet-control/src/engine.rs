use crate::{port::*, ProphetXiiPlugin};
use core::sync::atomic::Ordering;
use prophet_dsp::{KernelTelemetrySnapshot, MAX_BLOCK_SIZE};
use prophet_params::{ParamId, PARAM_SPECS};

pub struct PreparedEngine {
    pub(crate) synth: Box<ProphetXiiPlugin>,
    config: Prepare,
    serials: [u32; ORIGINS],
    frames: u64,
}
pub use prophet_dsp::vector::{Patch, Point, Topology};
impl PreparedEngine {
    /// Configure at prepare/recall, before starting voices.
    pub fn set_patch(&mut self, patch: Patch) -> Result<(), Status> {
        if !patch.valid() {
            return Err(Status::Invalid);
        }
        if self
            .synth
            .kernel
            .allocator
            .voices
            .iter()
            .any(|v| v.is_active())
        {
            return Err(Status::Unsupported);
        }
        self.synth.kernel.voice_params.oscillator_patch = patch;
        Ok(())
    }
    pub fn patch(&self) -> Patch {
        self.synth.kernel.voice_params.oscillator_patch
    }

    /// Render an optional scheduler lane alongside ordered EnginePort commands.
    /// Reserve a separate origin for this lane. Host commands win ties in time.
    pub fn process_music(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: &[Command],
        acks: &mut [Ack],
        origin: u8,
        music: &[crate::music::MusicEvent],
        results: &mut [Status],
    ) -> Result<(), PortError> {
        use crate::music::Trigger;
        validate_block(self.config, left.len(), right.len(), events, acks.len())?;
        if origin as usize >= ORIGINS || events.iter().any(|e| e.origin == origin) {
            return Err(PortError::Origin);
        }
        if music.len() > MAX_EVENTS - events.len() || results.len() < music.len() {
            return Err(PortError::Capacity);
        }
        if music.iter().any(|e| e.offset as usize >= left.len())
            || music.windows(2).any(|p| p[0].offset > p[1].offset)
        {
            return Err(PortError::Offset);
        }
        let (mut host, mut seq, mut pos) = (0, 0, 0);
        while host < events.len() || seq < music.len() {
            let h = events.get(host).map_or(u16::MAX, |e| e.offset);
            let m = music.get(seq).map_or(u16::MAX, |e| e.offset);
            let at = h.min(m) as usize;
            self.render(&mut left[pos..at], &mut right[pos..at]);
            pos = at;
            if h <= m {
                let e = events[host];
                let status = self.apply(e);
                acks[host] = Ack {
                    origin: e.origin,
                    serial: e.serial,
                    status,
                    revision: self.revision(),
                };
                host += 1;
            } else {
                results[seq] = match music[seq].trigger {
                    Trigger::Release => {
                        self.synth.release_source(origin, None);
                        Status::Applied
                    }
                    Trigger::Note {
                        key,
                        velocity,
                        glide,
                    } => {
                        if velocity == 0 || velocity > 127 || key > 127 {
                            Status::Invalid
                        } else {
                            let stored = self.synth.ipc.params.get(ParamId::GlideTime);
                            let original = self.synth.kernel.voice_params.glide_time_sec;
                            self.synth.kernel.voice_params.glide_time_sec =
                                if glide { stored } else { 0. };
                            let status =
                                self.synth
                                    .midi_owned(origin, 0x90, key, velocity, u32::from(key));
                            self.synth.kernel.voice_params.glide_time_sec = original;
                            if status == Status::Applied {
                                let allocator = &mut self.synth.kernel.allocator;
                                for v in &mut allocator.voices {
                                    if v.gate && v.note_age == allocator.event_counter {
                                        v.step_glide = glide;
                                    }
                                }
                            }
                            status
                        }
                    }
                };
                seq += 1;
            }
        }
        self.render(&mut left[pos..], &mut right[pos..]);
        self.frames = self.frames.saturating_add(left.len() as u64);
        Ok(())
    }
    pub fn set_drive_makeup(&mut self, enabled: bool) {
        self.synth.kernel.voice_params.drive_makeup = enabled;
    }
    pub fn revision(&self) -> u64 {
        self.synth.ipc.params.revision.load(Ordering::Relaxed)
    }
    pub fn rendered_frames(&self) -> u64 {
        self.frames
    }
    pub fn latency_samples(&self) -> u32 {
        self.synth.kernel.allocator.voices[0]
            .oversampler
            .factor
            .latency_samples()
    }
    pub fn telemetry(&self) -> KernelTelemetrySnapshot {
        self.synth.kernel.telemetry_snapshot()
    }
    pub fn notes(&self) -> impl Iterator<Item = &crate::Note> {
        self.synth.held_notes()
    }
    pub fn configuration(&self) -> Prepare {
        self.config
    }
    pub fn parameter(&self, id: u16) -> Option<f32> {
        if id == GLIDE_ENABLED {
            return Some(f32::from(self.synth.ipc.params.glide_enabled()));
        }
        ParamId::from_index(usize::from(id)).map(|p| self.synth.ipc.params.get(p))
    }
    fn apply(&mut self, e: Command) -> Status {
        let previous = &mut self.serials[usize::from(e.origin)];
        if e.serial <= *previous {
            return Status::Duplicate;
        }
        *previous = e.serial;
        match e.action {
            Action::Midi {
                status,
                data1,
                data2,
                note_id,
            } => {
                let result = self
                    .synth
                    .midi_owned(e.origin, status, data1, data2, note_id);
                if result == Status::Applied && matches!(status & 0xf0, 0xb0 | 0xe0) {
                    self.synth.sync_parameters();
                }
                result
            }
            Action::PanicOrigin => {
                self.synth.release_source(e.origin, None);
                Status::Applied
            }
            Action::Parameter { id, value } => {
                let bank = &self.synth.ipc.params;
                if id == GLIDE_ENABLED && (value == 0.0 || value == 1.0) {
                    bank.set_glide_enabled(value == 1.0);
                } else if let Some(spec) = PARAM_SPECS.get(usize::from(id)) {
                    if spec.id == ParamId::OversamplingMode {
                        return Status::Unsupported;
                    }
                    if !value.is_finite()
                        || !(spec.min..=spec.max).contains(&value)
                        || spec.integer && value != value.round()
                    {
                        return Status::Invalid;
                    }
                    bank.set(spec.id, value);
                } else if id == GLIDE_ENABLED {
                    return Status::Invalid;
                } else {
                    return Status::Unsupported;
                }
                self.synth.sync_parameters();
                Status::Applied
            }
            Action::VectorXY { x, y } => {
                let patch = &mut self.synth.kernel.voice_params.oscillator_patch;
                if patch.topology != Topology::Vector4 {
                    return Status::Unsupported;
                }
                let pos = Point { x, y };
                if !pos.valid() {
                    return Status::Invalid;
                }
                patch.position = pos;
                Status::Applied
            }
        }
    }
    fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        for (l, r) in l
            .chunks_mut(MAX_BLOCK_SIZE)
            .zip(r.chunks_mut(MAX_BLOCK_SIZE))
        {
            self.synth.kernel.process_block(l, r, l.len());
        }
    }
}
impl EnginePort for PreparedEngine {
    fn prepare(config: Prepare) -> Result<Self, PortError> {
        if !config.valid() {
            return Err(PortError::Configuration);
        }
        let mut synth = Box::new(ProphetXiiPlugin::new(config.sample_rate as f32));
        synth
            .ipc
            .params
            .set(ParamId::OversamplingMode, config.oversampling as f32);
        synth.sync_parameters();
        synth.kernel.output_policy = prophet_dsp::OutputPolicy::FloatBus;
        Ok(Self {
            synth,
            config,
            serials: [0; ORIGINS],
            frames: 0,
        })
    }
    fn process(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        events: &[Command],
        acks: &mut [Ack],
    ) -> Result<(), PortError> {
        validate_block(self.config, l.len(), r.len(), events, acks.len())?;
        let mut pos = 0;
        for (e, ack) in events.iter().zip(acks.iter_mut()) {
            let at = usize::from(e.offset);
            self.render(&mut l[pos..at], &mut r[pos..at]);
            let status = self.apply(*e);
            *ack = Ack {
                origin: e.origin,
                serial: e.serial,
                revision: self.revision(),
                status,
            };
            pos = at;
        }
        self.render(&mut l[pos..], &mut r[pos..]);
        self.frames = self.frames.saturating_add(l.len() as u64);
        Ok(())
    }
}
