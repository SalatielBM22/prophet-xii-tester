// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Moog Transistor Ladder 24 dB/oct ZDF Filter & Dual VCF Stage
// Mathematical grounding:
// - Antti Huovilainen & Vesa Välimäki non-linear BJT differential pair model
// - Surge XT sst-filters VintageLadder (https://github.com/surge-synthesizer/sst-filters)
// - Vadim Zavalishin TPT + Mystran local pivot linearization + 2-step Newton-Raphson

use crate::vcf_cem3320::{fast_sech2, fast_tanh, Cem3320Filter};
use core::f32::consts::PI;

/// Normalized, physically inspired ladder ODE with per-stage rational saturation.
/// Drive has no measured mapping to BJT thermal voltage or control current.
/// and optional Passband Bass Compensation.
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct MoogLadderFilter {
    /// TPT integrator states s1..s4
    pub s: [f32; 4],
    /// Previous output y4 for initial Newton-Raphson seed
    pub prev_y4: f32,
    pub sample_rate: f32,
    pub cutoff_hz: f32,
    pub resonance: f32,
    /// Dimensionless saturation drive; not calibrated to 1/(2 V_T).
    pub drive: f32,
    /// Bass compensation amount in [0.0, 1.0] (restores 6..12 dB passband loss at high Q)
    pub bass_compensation: f32,
    /// Deterministic per-voice cutoff tolerance multiplier
    pub cutoff_dispersion: f32,
    g: f32,
    big_g: f32,
}

impl MoogLadderFilter {
    pub fn new(sample_rate: f32, cutoff_dispersion: f32) -> Self {
        let mut flt = Self {
            s: [0.0; 4],
            prev_y4: 0.0,
            sample_rate: sample_rate.max(8000.0),
            cutoff_hz: 2000.0,
            resonance: 0.0,
            drive: 1.15,
            bass_compensation: 0.5,
            cutoff_dispersion: cutoff_dispersion.clamp(0.95, 1.05),
            g: 0.1,
            big_g: 0.09,
        };
        flt.set_params(2000.0, 0.0, 1.15, 0.5);
        flt
    }

    #[inline]
    pub fn reset(&mut self) {
        self.s = [0.0; 4];
        self.prev_y4 = 0.0;
    }

    #[inline]
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(8000.0);
        self.set_params(
            self.cutoff_hz,
            self.resonance,
            self.drive,
            self.bass_compensation,
        );
    }

    #[inline(always)]
    pub fn set_params(
        &mut self,
        cutoff_hz: f32,
        resonance: f32,
        drive: f32,
        bass_compensation: f32,
    ) {
        let fc = (cutoff_hz * self.cutoff_dispersion).clamp(15.0, 0.46 * self.sample_rate);
        self.cutoff_hz = fc;
        self.resonance = resonance.clamp(0.0, 1.0);
        self.drive = drive.clamp(0.25, 4.0);
        self.bass_compensation = bass_compensation.clamp(0.0, 1.0);

        let g = (PI * fc / self.sample_rate).tan();
        self.g = g;
        self.big_g = g / (1.0 + g);
    }

    /// Evaluates the 4-stage non-linear BJT transistor ladder forward path for a trial feedback `y4_est`
    /// without mutating state, returning `(y4_actual, dy4_dy4est)` for Newton-Raphson ZDF polishing.
    ///
    /// In the Huovilainen / Välimäki transistor ladder model:
    /// Each BJT differential pair saturates according to `tanh(x / (2 * V_T))`.
    // Trapezoidal solution of dy/dt = wc*(sat(x)-sat(y)).
    // Each implicit stage is solved before the global zero-delay feedback root.
    #[inline]
    fn stage(&self, input: f32, state: f32) -> (f32, f32) {
        let d = self.drive;
        let rhs = state + self.g * fast_tanh(d * input) / d;
        let mut y = rhs / (1.0 + self.g);
        for _ in 0..8 {
            let f = y + self.g * fast_tanh(d * y) / d - rhs;
            y -= f / (1.0 + self.g * fast_sech2(d * y));
            if f.abs() < 1.0e-7 {
                break;
            }
        }
        let derivative = self.g * fast_sech2(d * input) / (1.0 + self.g * fast_sech2(d * y));
        (y, derivative)
    }
    #[inline]
    fn evaluate(&self, input: f32, trial: f32, k: f32, comp: f32) -> ([f32; 4], f32) {
        let mut x = input * comp - k * trial;
        let mut derivative = -k;
        let mut ys = [0.0; 4];
        for i in 0..4 {
            let (y, dy) = self.stage(x, self.s[i]);
            ys[i] = y;
            x = y;
            derivative *= dy;
        }
        (ys, derivative)
    }
    #[inline]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let k = 4.35 * self.resonance;
        let comp = 1.0 + k * self.bass_compensation * 0.65;
        let mut trial = self.prev_y4;
        for _ in 0..8 {
            let (ys, d) = self.evaluate(input, trial, k, comp);
            let residual = trial - ys[3];
            trial -= residual / (1.0 - d);
            if residual.abs() < 1.0e-7 {
                break;
            }
        }
        let (ys, _) = self.evaluate(input, trial, k, comp);
        for i in 0..4 {
            self.s[i] = 2.0 * ys[i] - self.s[i];
            if self.s[i].abs() < 1.0e-20 {
                self.s[i] = 0.0;
            }
        }
        self.prev_y4 = ys[3];
        ys[3]
    }
}

/// Dual VCF Stage allowing continuous morphing (`morph in [0.0, 1.0]`) or discrete switching
/// between Mode 1 (Prophet-10 Rev3 Curtis CEM3320) and Mode 2 (Moog 24 dB/oct Transistor Ladder).
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct DualVcfStage {
    pub cem3320: Cem3320Filter,
    pub moog_ladder: MoogLadderFilter,
    /// `0.0` = 100% CEM3320, `1.0` = 100% Moog Ladder, intermediate = linear output blend (no phase/energy equivalence asserted)
    pub topology_morph: f32,
}

impl DualVcfStage {
    pub fn new(sample_rate: f32, cutoff_dispersion: f32) -> Self {
        Self {
            cem3320: Cem3320Filter::new(sample_rate, cutoff_dispersion),
            moog_ladder: MoogLadderFilter::new(sample_rate, cutoff_dispersion),
            topology_morph: 0.0,
        }
    }

    #[inline]
    pub fn reset(&mut self) {
        self.cem3320.reset();
        self.moog_ladder.reset();
    }

    #[inline]
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        self.cem3320.update_sample_rate(sample_rate);
        self.moog_ladder.update_sample_rate(sample_rate);
    }

    #[inline(always)]
    pub fn set_params(
        &mut self,
        cutoff_hz: f32,
        resonance: f32,
        drive: f32,
        bass_compensation: f32,
        topology_morph: f32,
    ) {
        let next = topology_morph.clamp(0.0, 1.0);
        // Endpoint-inactive branches are not integrated. Reset on re-entry,
        // rather than exposing a tail frozen at an unrelated earlier input.
        // The kernel's existing 5 ms morph smoother performs the crossfade.
        if self.topology_morph <= 0.0001 && next > 0.0001 {
            self.moog_ladder.reset();
        }
        if self.topology_morph >= 0.9999 && next < 0.9999 {
            self.cem3320.reset();
        }
        self.topology_morph = next;
        self.cem3320.set_params(cutoff_hz, resonance, drive);
        self.moog_ladder
            .set_params(cutoff_hz, resonance, drive, bass_compensation);
    }

    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let m = self.topology_morph;
        if m <= 0.0001 {
            self.cem3320.process_sample(input)
        } else if m >= 0.9999 {
            self.moog_ladder.process_sample(input)
        } else {
            let out_cem = self.cem3320.process_sample(input);
            let out_moog = self.moog_ladder.process_sample(input);
            // Linear convex blend. Prewarping alone establishes neither phase
            // agreement nor equal power for these different nonlinear systems.
            out_cem * (1.0 - m) + out_moog * m
        }
    }
}
