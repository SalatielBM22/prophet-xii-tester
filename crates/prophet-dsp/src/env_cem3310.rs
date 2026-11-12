// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Curtis CEM3310 Exponential RC ADSR Envelope & Global LFO
// Mathematical grounding:
// - True RC charging curve toward 1.2 * V_max with comparator trip at 1.0
// - Exponential Decay/Release with time constants from 1 ms to 25 s
// - Per-voice deterministic RC tolerance dispersion (+/- 4%) per OB-Xd (https://github.com/2DaT/Obxd)

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Curtis CEM3310 Voltage-Controlled ADSR Envelope Generator.
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct Cem3310Envelope {
    pub stage: EnvStage,
    pub level: f32,
    pub stage_elapsed_samples: u64,
    pub sample_rate: f32,
    /// Attack time in seconds [0.001, 25.0]
    pub attack_sec: f32,
    /// Decay time in seconds [0.001, 25.0]
    pub decay_sec: f32,
    /// Sustain level in [0.0, 1.0]
    pub sustain_level: f32,
    /// Release time in seconds [0.001, 25.0]
    pub release_sec: f32,
    /// Deterministic per-voice RC dispersion factor (e.g. 0.96 ..= 1.04)
    pub rc_dispersion: f32,
    cached_dispersion: f32,
    attack_coeff: f32,
    decay_coeff: f32,
    release_coeff: f32,
}

impl Cem3310Envelope {
    /// Asymptotic target voltage ratio for CEM3310 Attack comparator (`1.2 * V_max`).
    pub const ATTACK_TARGET: f32 = 1.2;
    /// Small undershoot offset so Decay/Release reach target in finite RC time.
    pub const EXP_EPSILON: f32 = 0.0015;

    pub fn new(sample_rate: f32, rc_dispersion: f32) -> Self {
        let mut env = Self {
            stage: EnvStage::Idle,
            level: 0.0,
            stage_elapsed_samples: 0,
            sample_rate: sample_rate.max(8000.0),
            attack_sec: 0.005,
            decay_sec: 0.25,
            sustain_level: 0.7,
            release_sec: 0.4,
            rc_dispersion: rc_dispersion.clamp(0.90, 1.10),
            cached_dispersion: 0.0,
            attack_coeff: 0.0,
            decay_coeff: 0.0,
            release_coeff: 0.0,
        };
        env.recalculate_coefficients();
        env
    }

    #[inline]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
        self.recalculate_coefficients();
    }

    #[inline]
    pub fn set_adsr(&mut self, attack_sec: f32, decay_sec: f32, sustain: f32, release_sec: f32) {
        if self.cached_dispersion == self.rc_dispersion
            && self.attack_sec == attack_sec.clamp(0.001, 25.0)
            && self.decay_sec == decay_sec.clamp(0.001, 25.0)
            && self.sustain_level == sustain.clamp(0.0, 1.0)
            && self.release_sec == release_sec.clamp(0.001, 25.0)
        {
            return;
        }
        self.attack_sec = attack_sec.clamp(0.001, 25.0);
        self.decay_sec = decay_sec.clamp(0.001, 25.0);
        self.sustain_level = sustain.clamp(0.0, 1.0);
        self.release_sec = release_sec.clamp(0.001, 25.0);
        self.recalculate_coefficients();
    }

    #[inline]
    pub fn recalculate_coefficients(&mut self) {
        self.cached_dispersion = self.rc_dispersion;
        let sr = self.sample_rate;
        let disp = self.rc_dispersion;

        // Attack reaches 1.0 when charging toward 1.2:
        // 1.0 = 1.2 * (1 - exp(-t_attack / tau)) => t_attack / tau = ln(1.2 / 0.2) = ln(6) = 1.7917595
        let att_samples = (self.attack_sec * disp * sr).max(1.0);
        self.attack_coeff = 1.0 - (-1.791_759_5 / att_samples).exp();

        // Decay & Release reach within EXP_EPSILON (~6.5 time constants for 99.85% settling)
        let dec_samples = (self.decay_sec * disp * sr).max(1.0);
        self.decay_coeff = 1.0 - (-4.8 / dec_samples).exp();

        let rel_samples = (self.release_sec * disp * sr).max(1.0);
        self.release_coeff = 1.0 - (-4.8 / rel_samples).exp();
    }

    /// Triggers Note On. Preserves current capacitor voltage `self.level` to prevent clicks.
    #[inline(always)]
    pub fn note_on(&mut self, reset_to_zero: bool) {
        if reset_to_zero {
            self.level = 0.0;
        }
        self.stage = EnvStage::Attack;
        self.stage_elapsed_samples = 0;
    }

    /// Triggers Note Off, transitioning immediately from current voltage into Release stage.
    #[inline(always)]
    pub fn note_off(&mut self) {
        if self.stage != EnvStage::Idle {
            if self.stage != EnvStage::Release {
                self.stage_elapsed_samples = 0;
            }
            self.stage = EnvStage::Release;
        }
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        self.stage != EnvStage::Idle
    }

    /// Advances the CEM3310 RC state machine by 1 sample and returns output in `[0.0, 1.0]`.
    #[inline(always)]
    pub fn process_sample(&mut self) -> f32 {
        let previous_stage = self.stage;
        match self.stage {
            EnvStage::Idle => {
                self.level = 0.0;
            }
            EnvStage::Attack => {
                self.level += (Self::ATTACK_TARGET - self.level) * self.attack_coeff;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                }
            }
            EnvStage::Decay => {
                let target = self.sustain_level - Self::EXP_EPSILON;
                self.level += (target - self.level) * self.decay_coeff;
                if self.level <= self.sustain_level {
                    self.level = self.sustain_level;
                    self.stage = EnvStage::Sustain;
                }
            }
            EnvStage::Sustain => {
                self.level += (self.sustain_level - self.level) * 0.02;
            }
            EnvStage::Release => {
                let target = -Self::EXP_EPSILON;
                self.level += (target - self.level) * self.release_coeff;
                if self.level <= 0.0001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                }
            }
        }
        if self.stage != previous_stage {
            self.stage_elapsed_samples = 0;
        } else if self.stage != EnvStage::Idle {
            self.stage_elapsed_samples = self.stage_elapsed_samples.saturating_add(1);
        }
        self.level.clamp(0.0, 1.0)
    }

    /// Progress from elapsed samples and remaining RC threshold time, not amplitude.
    /// Sustain has no finite end: progress=0 and timed=false.
    pub fn telemetry(&self) -> EnvelopeTelemetry {
        let (distance, threshold, coefficient, timed) = match self.stage {
            EnvStage::Attack => (
                Self::ATTACK_TARGET - self.level,
                Self::ATTACK_TARGET - 1.0,
                self.attack_coeff,
                true,
            ),
            EnvStage::Decay => (
                self.level - self.sustain_level + Self::EXP_EPSILON,
                Self::EXP_EPSILON,
                self.decay_coeff,
                true,
            ),
            EnvStage::Release => (
                self.level + Self::EXP_EPSILON,
                0.0001 + Self::EXP_EPSILON,
                self.release_coeff,
                true,
            ),
            _ => (0.0, 1.0, 0.0, false),
        };
        let remaining = if timed && distance > threshold {
            ((distance / threshold).ln() / -(1.0 - coefficient).ln()).max(0.0)
        } else {
            0.0
        };
        let elapsed = self.stage_elapsed_samples as f32;
        EnvelopeTelemetry {
            stage: self.stage as u8,
            elapsed_samples: self.stage_elapsed_samples,
            level: self.level,
            progress: if timed {
                elapsed / (elapsed + remaining).max(1.0)
            } else {
                0.0
            },
            timed,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct EnvelopeTelemetry {
    pub stage: u8,
    pub elapsed_samples: u64,
    pub level: f32,
    pub progress: f32,
    pub timed: bool,
}

/// Prophet-10 Rev3 Global LFO + Mod Wheel Routing Engine.
/// Supports simultaneous combination of Saw, Triangle, and Square waveforms.
#[derive(Clone, Debug)]
pub struct ProphetGlobalLfo {
    pub phase: f32,
    pub sample_rate: f32,
    pub freq_hz: f32,
    pub wave_saw: bool,
    pub wave_tri: bool,
    pub wave_square: bool,
    pub initial_amount: f32,
    pub mod_wheel: f32,
    pub dest_osc_a_freq: bool,
    pub dest_osc_b_freq: bool,
    pub dest_pw_ab: bool,
    pub dest_filter_cutoff: bool,
}

impl ProphetGlobalLfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            sample_rate: sample_rate.max(8000.0),
            freq_hz: 2.5,
            wave_saw: false,
            wave_tri: true,
            wave_square: false,
            initial_amount: 0.0,
            mod_wheel: 0.0,
            dest_osc_a_freq: false,
            dest_osc_b_freq: false,
            dest_pw_ab: false,
            dest_filter_cutoff: false,
        }
    }

    #[inline(always)]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
    }

    /// Advances the LFO by 1 sample and returns `(raw_lfo, scaled_mod_signal)`.
    #[inline(always)]
    pub fn process_sample(&mut self) -> (f32, f32) {
        let dt = (self.freq_hz.clamp(0.04, 30.0)) / self.sample_rate;
        let p = self.phase;
        self.phase = (p + dt).rem_euclid(1.0);

        let mut sum = 0.0;
        let mut count = 0.0;
        if self.wave_saw {
            sum += 2.0 * p - 1.0;
            count += 1.0;
        }
        if self.wave_tri {
            let tri = if p < 0.5 {
                4.0 * p - 1.0
            } else {
                3.0 - 4.0 * p
            };
            sum += tri;
            count += 1.0;
        }
        if self.wave_square {
            sum += if p < 0.5 { 1.0 } else { -1.0 };
            count += 1.0;
        }
        let raw = if count > 0.0 { sum / count } else { 0.0 };
        let depth = (self.initial_amount + self.mod_wheel).clamp(0.0, 1.0);
        (raw, raw * depth)
    }
}
