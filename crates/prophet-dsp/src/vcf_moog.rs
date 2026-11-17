// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Moog Transistor Ladder 24 dB/oct ZDF Filter & Dual VCF Stage
// Mathematical grounding:
// - Antti Huovilainen & Vesa Välimäki non-linear BJT differential pair model
// - Surge XT sst-filters VintageLadder (https://github.com/surge-synthesizer/sst-filters)
// - Vadim Zavalishin TPT + Mystran local pivot linearization + 2-step Newton-Raphson

use crate::vcf_cem3320::{fast_sech2, fast_tanh, fast_tanh_and_sech2, Cem3320Filter};
use core::f32::consts::PI;

/// Normalized, physically inspired ladder ODE with per-stage rational saturation.
/// Drive has no measured mapping to BJT thermal voltage or control current.
/// and optional Passband Bass Compensation.
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct MoogLadderFilter {
    /// TPT integrator states s1..s4
    pub s: [f32; 4],
    /// Previous output y4
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
    inv_drive: f32,
    cached_tan_fc: f32,
    cached_tan_sr: f32,
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
            inv_drive: 1.0 / 1.15,
            cached_tan_fc: -1.0,
            cached_tan_sr: -1.0,
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
        let d = drive.clamp(0.25, 4.0);
        if d != self.drive {
            self.drive = d;
            self.inv_drive = 1.0 / d;
        }
        self.bass_compensation = bass_compensation.clamp(0.0, 1.0);

        if fc != self.cached_tan_fc || self.sample_rate != self.cached_tan_sr {
            let g = (PI * fc / self.sample_rate).tan();
            self.g = g;
            self.big_g = g / (1.0 + g);
            self.cached_tan_fc = fc;
            self.cached_tan_sr = self.sample_rate;
        }
    }

    #[inline(never)]
    #[cold]
    fn fallback_nested_newton(&mut self, input: f32) -> f32 {
        let d = self.drive;
        let inv_d = self.inv_drive;
        let g = self.g;
        let eval_stage = |inp: f32, state: f32| -> (f32, f32) {
            let rhs = state + g * fast_tanh(d * inp) * inv_d;
            let mut y = rhs / (1.0 + g);
            for _ in 0..8 {
                let (p, dp) = fast_tanh_and_sech2(d * y);
                let f = y + g * p * inv_d - rhs;
                y -= f / (1.0 + g * dp);
                if f.abs() < 1.0e-7 {
                    break;
                }
            }
            let derivative = g * fast_sech2(d * inp) / (1.0 + g * fast_sech2(d * y));
            (y, derivative)
        };
        let eval_all = |trial: f32, s: &[f32; 4], k: f32, comp: f32| -> ([f32; 4], f32) {
            let mut x = input * comp - k * trial;
            let mut derivative = -k;
            let mut ys = [0.0; 4];
            for i in 0..4 {
                let (y, dy) = eval_stage(x, s[i]);
                ys[i] = y;
                x = y;
                derivative *= dy;
            }
            (ys, derivative)
        };
        let k = 4.35 * self.resonance;
        let comp = 1.0 + k * self.bass_compensation * 0.65;
        let mut trial = self.prev_y4;
        for _ in 0..8 {
            let (ys, deriv) = eval_all(trial, &self.s, k, comp);
            let residual = trial - ys[3];
            trial -= residual / (1.0 - deriv);
            if residual.abs() < 1.0e-7 {
                break;
            }
        }
        let (ys, _) = eval_all(trial, &self.s, k, comp);
        for i in 0..4 {
            self.s[i] = 2.0 * ys[i] - self.s[i];
            if self.s[i].abs() < 1.0e-20 {
                self.s[i] = 0.0;
            }
        }
        self.prev_y4 = ys[3];
        ys[3]
    }

    /// Evaluates `(fast_tanh(x), fast_sech2(x))` across `[u_in, u0, u1, u2, u3]` without inner branches
    /// so the compiler vectorizes reciprocals on x86_64/ARM64.
    #[inline(always)]
    fn eval_tanh_sech2_5(xs: [f32; 5]) -> ([f32; 5], [f32; 5]) {
        let mut phi = [0.0f32; 5];
        let mut dphi = [0.0f32; 5];
        for i in 0..5 {
            let xc = xs[i].clamp(-3.0, 3.0);
            let x2 = xc * xc;
            let inv_q = 1.0 / (27.0 + 9.0 * x2);
            let p = xc * (27.0 + x2) * inv_q;
            let num = (9.0 - x2) * inv_q;
            phi[i] = p;
            dphi[i] = 9.0 * num * num;
        }
        (phi, dphi)
    }

    /// Solves the 4-stage nonlinear implicit trapezoidal ladder equations simultaneously
    /// using an exact analytical O(1) solve of the 4x4 bordered-bidiagonal Newton Jacobian
    /// in scaled `u = drive * y` coordinates, seeded from the linear ZDF solution.
    #[inline]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let d = self.drive;
        let inv_d = self.inv_drive;
        let k = 4.35 * self.resonance;
        let comp = 1.0 + k * self.bass_compensation * 0.65;
        let g = self.g;

        let x_d = d * (input * comp);
        let sd0 = d * self.s[0];
        let sd1 = d * self.s[1];
        let sd2 = d * self.s[2];
        let sd3 = d * self.s[3];

        if x_d == 0.0 && sd0 == 0.0 && sd1 == 0.0 && sd2 == 0.0 && sd3 == 0.0 {
            self.prev_y4 = 0.0;
            return 0.0;
        }

        // Analytical 4-stage linear ZDF seed
        let g1 = self.big_g;
        let g2 = g1 * g1;
        let g3 = g2 * g1;
        let g4 = g2 * g2;
        let one_minus_g = 1.0 - g1;
        let big_s = one_minus_g * (g3 * sd0 + g2 * sd1 + g1 * sd2 + sd3);
        let u3_lin = (g4 * x_d + big_s) / (1.0 + k * g4);
        let u_in_lin = x_d - k * u3_lin;
        let (mut u0, mut u1, mut u2, mut u3) = if u_in_lin.abs() < 0.75 && sd0.abs() < 0.75 {
            let s0 = g1 * u_in_lin + one_minus_g * sd0;
            let s1 = g1 * s0 + one_minus_g * sd1;
            let s2 = g1 * s1 + one_minus_g * sd2;
            let s3 = g1 * s2 + one_minus_g * sd3;
            (s0, s1, s2, s3)
        } else {
            let s0 = (g1 * fast_tanh(u_in_lin) + one_minus_g * sd0).clamp(-3.5, 3.5);
            let s1 = (g1 * fast_tanh(s0) + one_minus_g * sd1).clamp(-3.5, 3.5);
            let s2 = (g1 * fast_tanh(s1) + one_minus_g * sd2).clamp(-3.5, 3.5);
            let s3 = (g1 * fast_tanh(s2) + one_minus_g * sd3).clamp(-3.5, 3.5);
            (s0, s1, s2, s3)
        };

        let u_scale = x_d
            .abs()
            .max(sd0.abs())
            .max(sd1.abs())
            .max(sd2.abs())
            .max(sd3.abs())
            .max(u3.abs());
        let tol = (1.0e-7 * d).min(1.0e-6 * u_scale).max(1.0e-10);
        // Quadratic Newton bound scaled by the system's O(g) nonlinearity:
        // ||F''|| <= O(g), so |du| <= 3.5e-4 / sqrt(max(g, 0.1)) guarantees post-step
        // residual < 1e-7 across normal audio frequencies without extra Jacobian passes.
        let step_tol = (3.5e-4f32 / g.max(0.1).sqrt())
            .min(1.1e-3)
            .min(0.05 * u_scale.max(1.0e-6))
            .max(1.0e-7);
        let mut converged = false;

        for _iter in 0..10 {
            let ([p_in, p0, p1, p2, p3], [dp_in, dp0, dp1, dp2, dp3]) =
                Self::eval_tanh_sech2_5([x_d - k * u3, u0, u1, u2, u3]);

            let f0 = u0 + g * p0 - sd0 - g * p_in;
            let f1 = u1 + g * p1 - sd1 - g * p0;
            let f2 = u2 + g * p2 - sd2 - g * p1;
            let f3 = u3 + g * p3 - sd3 - g * p2;

            let max_res = f0.abs().max(f1.abs()).max(f2.abs()).max(f3.abs());
            if max_res <= tol {
                converged = true;
                break;
            }

            let m_in = g * dp_in;
            let m0 = g * dp0;
            let m1 = g * dp1;
            let m2 = g * dp2;
            let m3 = g * dp3;

            let inv_d0 = 1.0 / (1.0 + m0);
            let inv_d1 = 1.0 / (1.0 + m1);
            let inv_d2 = 1.0 / (1.0 + m2);
            let d3 = 1.0 + m3;

            let a0 = -f0 * inv_d0;
            let b0 = (k * m_in) * inv_d0;

            let a1 = (-f1 + m0 * a0) * inv_d1;
            let b1 = (m0 * b0) * inv_d1;

            let a2 = (-f2 + m1 * a1) * inv_d2;
            let b2 = (m1 * b1) * inv_d2;

            let du3 = (-f3 + m2 * a2) / (d3 + m2 * b2);
            let du2 = a2 - b2 * du3;
            let du1 = a1 - b1 * du3;
            let du0 = a0 - b0 * du3;

            u0 += du0;
            u1 += du1;
            u2 += du2;
            u3 += du3;

            if du0
                .abs()
                .max(du1.abs())
                .max(du2.abs())
                .max(du3.abs())
                <= step_tol
            {
                converged = true;
                break;
            }
        }

        if !converged {
            return self.fallback_nested_newton(input);
        }

        let ys = [u0 * inv_d, u1 * inv_d, u2 * inv_d, u3 * inv_d];
        for i in 0..4 {
            let next_s = 2.0 * ys[i] - self.s[i];
            self.s[i] = if next_s.abs() < 1.0e-20 { 0.0 } else { next_s };
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
        if next < 0.9999 {
            self.cem3320.set_params(cutoff_hz, resonance, drive);
        }
        if next > 0.0001 {
            self.moog_ladder
                .set_params(cutoff_hz, resonance, drive, bass_compensation);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_stage(g: f32, d: f32, input: f32, state: f32) -> (f32, f32) {
        let rhs = state + g * fast_tanh(d * input) / d;
        let mut y = rhs / (1.0 + g);
        for _ in 0..8 {
            let f = y + g * fast_tanh(d * y) / d - rhs;
            y -= f / (1.0 + g * fast_sech2(d * y));
            if f.abs() < 1.0e-7 {
                break;
            }
        }
        let derivative = g * fast_sech2(d * input) / (1.0 + g * fast_sech2(d * y));
        (y, derivative)
    }

    fn ref_evaluate(
        s: &[f32; 4],
        g: f32,
        d: f32,
        input: f32,
        trial: f32,
        k: f32,
        comp: f32,
    ) -> ([f32; 4], f32) {
        let mut x = input * comp - k * trial;
        let mut derivative = -k;
        let mut ys = [0.0; 4];
        for i in 0..4 {
            let (y, dy) = ref_stage(g, d, x, s[i]);
            ys[i] = y;
            x = y;
            derivative *= dy;
        }
        (ys, derivative)
    }

    struct RefMoog {
        s: [f32; 4],
        prev_y4: f32,
        sr: f32,
        g: f32,
        res: f32,
        drive: f32,
        bass: f32,
    }

    impl RefMoog {
        fn new(sr: f32) -> Self {
            Self {
                s: [0.0; 4],
                prev_y4: 0.0,
                sr,
                g: (PI * 2000.0 / sr).tan(),
                res: 0.0,
                drive: 1.15,
                bass: 0.5,
            }
        }
        fn set_params(&mut self, fc: f32, res: f32, drive: f32, bass: f32) {
            let fc = fc.clamp(15.0, 0.46 * self.sr);
            self.res = res.clamp(0.0, 1.0);
            self.drive = drive.clamp(0.25, 4.0);
            self.bass = bass.clamp(0.0, 1.0);
            self.g = (PI * fc / self.sr).tan();
        }
        fn process_sample(&mut self, input: f32) -> f32 {
            let k = 4.35 * self.res;
            let comp = 1.0 + k * self.bass * 0.65;
            let mut trial = self.prev_y4;
            for _ in 0..8 {
                let (ys, deriv) = ref_evaluate(&self.s, self.g, self.drive, input, trial, k, comp);
                let residual = trial - ys[3];
                trial -= residual / (1.0 - deriv);
                if residual.abs() < 1.0e-7 {
                    break;
                }
            }
            let (ys, _) = ref_evaluate(&self.s, self.g, self.drive, input, trial, k, comp);
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

    #[test]
    fn fast_moog_solver_matches_reference_across_sweeps_and_saturation() {
        let mut max_diff = 0.0f32;
        for sr in [44100.0f32, 48000.0, 96000.0, 192000.0] {
            for &drive in &[0.25f32, 1.15, 2.5, 4.0] {
                for &res in &[0.0f32, 0.5, 0.92, 1.0] {
                    let mut fast = MoogLadderFilter::new(sr, 1.0);
                    let mut oracle = RefMoog::new(sr);
                    for i in 0..2000 {
                        let fc = 25.0 * (sr * 0.44 / 25.0).powf((i % 500) as f32 / 499.0);
                        fast.set_params(fc, res, drive, 0.65);
                        oracle.set_params(fc, res, drive, 0.65);
                        // Mix continuous waves, step jumps (sawtooth edge), and hot peaks
                        let x = if i % 67 == 0 {
                            1.5
                        } else if i % 67 == 33 {
                            -1.5
                        } else {
                            ((i as f32) * 0.113).sin() * 0.9 + ((i as f32) * 0.37).cos() * 0.4
                        };
                        let y_fast = fast.process_sample(x);
                        let y_ref = oracle.process_sample(x);
                        let diff = (y_fast - y_ref).abs();
                        if diff > max_diff {
                            max_diff = diff;
                        }
                        assert!(
                            diff < 2.0e-5,
                            "Moog mismatch at sr={sr} drive={drive} res={res} i={i} fc={fc}: fast={y_fast} ref={y_ref} diff={diff}"
                        );
                    }
                }
            }
        }
        assert!(max_diff < 2.0e-5, "max_diff={max_diff}");
    }
}
