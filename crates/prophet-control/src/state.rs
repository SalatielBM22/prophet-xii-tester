//! Validated capture and recall at a stopped transport boundary.
use crate::{port::*, PreparedEngine};
use prophet_dsp::{
    fx::{
        reverb::{Algorithm, ReverbPort, TailState},
        solaris_reverb::ReverbTailPolicy,
    },
    vector::Patch,
};
use prophet_params::{ParamId, NUM_PARAMS, PARAM_SPECS};

#[derive(Clone, Debug)]
pub struct SoundState {
    pub values: [f32; NUM_PARAMS],
    pub glide: bool,
    pub patch: Patch,
    pub algorithm: Algorithm,
    pub reverb_enabled: bool,
    pub kill_tail: bool,
    pub drive_makeup: bool,
}
impl SoundState {
    pub fn from_rc1(values: [f32; NUM_PARAMS]) -> Result<Self, Status> {
        let state = Self {
            glide: values[ParamId::GlideTime as usize] > 0.0005,
            values,
            patch: Default::default(),
            algorithm: Algorithm::LegacyRC1,
            reverb_enabled: true,
            kill_tail: false,
            drive_makeup: false,
        };
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<(), Status> {
        if !self.patch.valid()
            || !matches!(
                self.values[ParamId::OversamplingMode as usize],
                1. | 2. | 4.
            )
        {
            return Err(Status::Invalid);
        }
        for (v, p) in self.values.iter().zip(PARAM_SPECS.iter()) {
            if !(p.min..=p.max).contains(v) || p.integer && v.fract() != 0. {
                return Err(Status::Invalid);
            }
        }
        Ok(())
    }
    /// Prepare a replacement instance. A failed decode or prepare leaves the
    /// playing instance, held notes and tail under the caller's ownership.
    pub fn prepare(&self, sample_rate: u32, max_frames: u16) -> Result<PreparedEngine, Status> {
        self.validate()?;
        let mut engine = PreparedEngine::prepare(Prepare {
            sample_rate,
            max_frames,
            oversampling: self.values[ParamId::OversamplingMode as usize] as u8,
        })
        .map_err(|_| Status::Invalid)?;
        engine.set_patch(self.patch)?;
        engine.select_reverb(self.algorithm)?;
        let bank = &engine.synth.ipc.params;
        for (param, v) in PARAM_SPECS.iter().zip(self.values) {
            bank.set(param.id, v);
        }
        bank.set_glide_enabled(self.glide);
        bank.set_reverb_bypass(
            self.reverb_enabled,
            if self.kill_tail {
                ReverbTailPolicy::Kill
            } else {
                ReverbTailPolicy::Spill
            },
        );
        engine.synth.sync_parameters();
        engine.set_drive_makeup(self.drive_makeup);
        Ok(engine)
    }
}
impl PreparedEngine {
    pub fn select_reverb(&mut self, a: Algorithm) -> Result<(), Status> {
        if self.synth.kernel.reverb.select_idle(a) {
            Ok(())
        } else {
            Err(Status::Unsupported)
        }
    }
    pub fn reverb_state(&self) -> TailState {
        self.synth
            .kernel
            .reverb
            .state(&self.synth.kernel.reverb_params)
    }
    pub fn sound_state(&self) -> SoundState {
        let s = self.reverb_state();
        SoundState {
            values: core::array::from_fn(|i| self.synth.ipc.params.get(PARAM_SPECS[i].id)),
            glide: self.synth.ipc.params.glide_enabled(),
            patch: self.patch(),
            algorithm: s.algorithm,
            reverb_enabled: s.enabled,
            kill_tail: s.policy == ReverbTailPolicy::Kill,
            drive_makeup: self.synth.kernel.voice_params.drive_makeup,
        }
    }
}
