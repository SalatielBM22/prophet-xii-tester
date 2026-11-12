// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Curtis CEM3320 4-Pole OTA Cascade ZDF/TPT Filter
// Mathematical grounding:
// - Vadim Zavalishin, "The Art of VA Filter Design" (https://noisehack.com/research/VAFilterDesign_1.0.3.pdf)
// - Surge XT sst-filters (https://github.com/surge-synthesizer/sst-filters)
// - Sequential Circuits Prophet-5/10 Rev3 CEM3320 topology with 5 Hz AC-coupled feedback loop

use core::f32::consts::PI;

/// Fast and accurate Padé rational approximation of `tanh(x)` for real-time audio saturation,
/// with saturation at `+/- 1.0` for `|x| >= 3.0`.
#[inline(always)]
pub fn fast_tanh(x: f32) -> f32 {
    let xc = x.clamp(-3.0, 3.0);
    let x2 = xc * xc;
    xc * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Exact derivative of the clamped rational fast_tanh, not the true sech^2.
/// Keeping residual and derivative consistent matters for Newton convergence.
#[inline(always)]
pub fn fast_sech2(x: f32) -> f32 {
    if x.abs() >= 3.0 {
        return 0.0;
    }
    let q = 27.0 + 9.0 * x * x;
    9.0 * (x * x - 9.0).powi(2) / (q * q)
}

/// Computes `(fast_tanh(x), fast_sech2(x))` sharing a single reciprocal `1 / (27 + 9x^2)`.
#[inline(always)]
pub fn fast_tanh_and_sech2(x: f32) -> (f32, f32) {
    let x2 = x * x;
    if x2 >= 9.0 {
        return (if x >= 0.0 { 1.0 } else { -1.0 }, 0.0);
    }
    let inv_q = 1.0 / (27.0 + 9.0 * x2);
    let phi = x * (27.0 + x2) * inv_q;
    let num = (9.0 - x2) * inv_q;
    let dphi = 9.0 * num * num;
    (phi, dphi)
}

/// Curtis CEM3320 4-Pole Low-Pass ZDF/TPT Filter (topology-inspired VA model, not circuit calibrated).
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct Cem3320Filter {
    /// Integrator states s1..s4 of the 4 cascaded OTA poles
    pub s: [f32; 4],
    /// State of the 5 Hz high-pass DC-blocking capacitor in the CEM3320 resonance loop
    pub dc_block_state: f32,
    pub sample_rate: f32,
    pub cutoff_hz: f32,
    pub resonance: f32,
    pub drive: f32,
    /// Per-voice cutoff tolerance multiplier (e.g. 0.98 ..= 1.02)
    pub cutoff_dispersion: f32,
    g: f32,
    big_g: f32,
    dc_g: f32,
    cached_tan_fc: f32,
    cached_tan_sr: f32,
}

impl Cem3320Filter {
    pub fn new(sample_rate: f32, cutoff_dispersion: f32) -> Self {
        let mut flt = Self {
            s: [0.0; 4],
            dc_block_state: 0.0,
            sample_rate: sample_rate.max(8000.0),
            cutoff_hz: 2000.0,
            resonance: 0.0,
            drive: 1.0,
            cutoff_dispersion: cutoff_dispersion.clamp(0.95, 1.05),
            g: 0.1,
            big_g: 0.09,
            dc_g: 0.0003,
            cached_tan_fc: -1.0,
            cached_tan_sr: -1.0,
        };
        flt.update_sample_rate(sample_rate);
        flt.set_params(2000.0, 0.0, 1.0);
        flt
    }

    #[inline]
    pub fn reset(&mut self) {
        self.s = [0.0; 4];
        self.dc_block_state = 0.0;
    }

    #[inline]
    pub fn update_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.sample_rate = sr;
        // 5 Hz 1-pole TPT high-pass coefficient in feedback loop
        let g_hp = (PI * 5.0 / sr).tan();
        self.dc_g = g_hp / (1.0 + g_hp);
        self.set_params(self.cutoff_hz, self.resonance, self.drive);
    }

    #[inline(always)]
    pub fn set_params(&mut self, cutoff_hz: f32, resonance: f32, drive: f32) {
        let fc = (cutoff_hz * self.cutoff_dispersion).clamp(15.0, 0.46 * self.sample_rate);
        self.cutoff_hz = fc;
        self.resonance = resonance.clamp(0.0, 1.0);
        self.drive = drive.clamp(0.25, 4.0);

        if fc != self.cached_tan_fc || self.sample_rate != self.cached_tan_sr {
            // Pre-warped bilinear integrator gain g = tan(pi * fc / fs)
            let g = (PI * fc / self.sample_rate).tan();
            self.g = g;
            self.big_g = g / (1.0 + g);
            self.cached_tan_fc = fc;
            self.cached_tan_sr = self.sample_rate;
        }
    }

    /// Processes a single audio sample through the 4-pole ZDF/TPT CEM3320 OTA cascade.
    ///
    /// Mathematical derivation (Vadim Zavalishin TPT):
    /// Each trapezoidal integrator satisfies:
    ///   `v_i = G * (x_i - s_i)`, `y_i = v_i + s_i`, `s_{i,next} = y_i + v_i`
    /// Consequently:
    ///   `y_4 = G^4 * u + S`, where `S = (1-G)*(G^3 * s_1 + G^2 * s_2 + G * s_3 + s_4)`
    /// And the non-linear differential feedback limiter with 5 Hz AC coupling satisfies:
    ///   `u = x_in - k * tanh(drive * HP_5Hz(y_4))`
    /// Solved algebraically with Newton-Raphson zero-delay feedback iteration.
    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        let g1 = self.big_g;
        let g2 = g1 * g1;
        let g3 = g2 * g1;
        let g4 = g2 * g2;

        // Composite historical state contribution S at output of 4th pole
        let big_s = (1.0 - g1) * (g3 * self.s[0] + g2 * self.s[1] + g1 * self.s[2] + self.s[3]);

        // Global feedback gain k in [0.0, 4.5] (self-oscillation threshold at k = 4.0)
        let drv = self.drive;
        let inv_drv = 1.0 / drv;
        let input = fast_tanh(input * drv) * inv_drv;
        let k = 4.5 * self.resonance;

        // 5 Hz TPT high-pass transfer on y4: y_hp = (1 - dc_g) * (y4 - dc_block_state)
        let hp_gain = 1.0 - self.dc_g;

        // Initial linear ZDF estimate for y4
        let linear_denom = 1.0 + k * g4 * hp_gain;
        let mut y4 = (g4 * input + big_s) / linear_denom;

        // At most 6 Newton iterations for the rational nonlinear feedback root:
        // F(y4) = y4 - G^4 * (input - k * inv_drv * tanh(drv * hp_gain * (y4 - dc_block_state))) - S = 0
        let g4k = g4 * k;
        for _ in 0..6 {
            let y_hp = hp_gain * (y4 - self.dc_block_state);
            let (phi, dphi) = fast_tanh_and_sech2(drv * y_hp);
            let u = input - k * (phi * inv_drv);
            let f_val = y4 - g4 * u - big_s;
            let df_dy4 = 1.0 + g4k * hp_gain * dphi;
            y4 -= f_val / df_dy4;
            if f_val.abs() < 1.0e-7 {
                break;
            }
        }

        // Compute final feedback signal u and update 5 Hz DC-blocker TPT state
        let v_hp = self.dc_g * (y4 - self.dc_block_state);
        let y_lp_dc = v_hp + self.dc_block_state;
        self.dc_block_state = y_lp_dc + v_hp;
        let y_hp_final = y4 - y_lp_dc;

        let u_final = input - k * (fast_tanh(drv * y_hp_final) * inv_drv);

        // Cascade through the 4 OTA trapezoidal integrators
        let v0 = g1 * (u_final - self.s[0]);
        let y0 = v0 + self.s[0];
        self.s[0] = y0 + v0;

        let v1 = g1 * (y0 - self.s[1]);
        let y1 = v1 + self.s[1];
        self.s[1] = y1 + v1;

        let v2 = g1 * (y1 - self.s[2]);
        let y2 = v2 + self.s[2];
        self.s[2] = y2 + v2;

        let v3 = g1 * (y2 - self.s[3]);
        let y3 = v3 + self.s[3];
        self.s[3] = y3 + v3;

        for v in &mut self.s {
            if v.abs() < 1.0e-20 {
                *v = 0.0;
            }
        }
        if self.dc_block_state.abs() < 1.0e-20 {
            self.dc_block_state = 0.0;
        }
        y3
    }
}
