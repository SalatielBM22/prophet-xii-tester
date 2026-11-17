//! Prepared effect slot. Both profiles reuse a single allocated delay bank.
use super::solaris_reverb::{ReverbTailPolicy, SolarisCloudburstReverb, SolarisReverbParams};

pub type LegacyRC1 = SolarisCloudburstReverb;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Algorithm {
    #[default]
    LegacyRC1 = 0,
    DiffuseV1 = 1,
}
#[derive(Clone, Copy, Debug)]
pub struct TailState {
    pub algorithm: Algorithm,
    pub version: u16,
    pub latency_samples: u32,
    pub enabled: bool,
    pub policy: ReverbTailPolicy,
    pub frozen: bool,
    pub dormant: bool,
}
pub trait ReverbPort: Sized {
    fn prepare(sample_rate: f32, algorithm: Algorithm) -> Option<Self>;
    fn process_stereo(&mut self, left: f32, right: f32, p: &SolarisReverbParams) -> (f32, f32);
    fn reset(&mut self);
    fn state(&self, p: &SolarisReverbParams) -> TailState;
}
pub struct ReverbSlot {
    tank: LegacyRC1,
    algorithm: Algorithm,
}
impl ReverbSlot {
    pub fn new(sr: f32) -> Self {
        Self {
            tank: LegacyRC1::new(sr),
            algorithm: Algorithm::LegacyRC1,
        }
    }
    pub fn is_dormant(&self) -> bool {
        self.tank.is_dormant()
    }
    pub fn enable_gain(&self) -> f32 {
        self.tank.enable_gain()
    }
    pub fn set_sample_rate(&mut self, sr: f32) {
        self.tank.set_sample_rate(sr)
    }
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }
    /// Select only while the tank is empty. A busy tail remains intact.
    pub fn select_idle(&mut self, algorithm: Algorithm) -> bool {
        if self.algorithm != algorithm && !self.is_dormant() {
            return false;
        }
        self.algorithm = algorithm;
        true
    }
}
impl ReverbPort for ReverbSlot {
    fn prepare(sr: f32, algorithm: Algorithm) -> Option<Self> {
        if !(8000.0..=192000.0).contains(&sr) {
            return None;
        }
        let mut slot = Self::new(sr);
        slot.algorithm = algorithm;
        Some(slot)
    }
    fn reset(&mut self) {
        self.tank.reset()
    }
    fn state(&self, p: &SolarisReverbParams) -> TailState {
        TailState {
            algorithm: self.algorithm,
            version: 1,
            latency_samples: 0,
            enabled: p.enabled,
            policy: p.bypass_tail,
            frozen: p.freeze,
            dormant: self.is_dormant(),
        }
    }
    fn process_stereo(&mut self, l: f32, r: f32, p: &SolarisReverbParams) -> (f32, f32) {
        match self.algorithm {
            Algorithm::LegacyRC1 => self.tank.process_stereo(l, r, p),
            Algorithm::DiffuseV1 => self.tank.process_profile::<true>(l, r, p),
        }
    }
}
