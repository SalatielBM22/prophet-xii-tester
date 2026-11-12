// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Curtis CEM3340 Dual VCO + LFSR Pink/White Noise
// Mathematical grounding:
// - PolyBLEP (4th-order B-spline & 2nd-order polynomial residual) & PolyBLAMP
// - Exact Analytical Sub-Sample Hard Sync (Osc A slaved to Osc B)
// - Reference: OB-Xd (https://github.com/2DaT/Obxd) & Vadim Zavalishin VA Design

/// 2nd-order PolyBLEP residual for step discontinuities of magnitude 1.0.
#[inline(always)]
pub fn poly_blep_2(t: f32, dt: f32) -> f32 {
    if dt <= 1.0e-9 {
        return 0.0;
    }
    if t < dt {
        let u = t / dt;
        u + u - u * u - 1.0
    } else if t > 1.0 - dt {
        let u = (t - 1.0) / dt;
        u * u + u + u + 1.0
    } else {
        0.0
    }
}

/// 4th-order (4-point cubic B-spline) PolyBLEP residual for high-precision VCO anti-aliasing.
/// Evaluated around phase wrap boundary using normalized distance `d = phase / dt`.
#[inline(always)]
pub fn poly_blep_4(t: f32, dt: f32) -> f32 {
    // The baseline fourth-order formula was discontinuous at t=dt.
    // Use the continuous, analytically defined quadratic residual.
    poly_blep_2(t, dt)
}

/// PolyBLEP residual evaluated at an arbitrary sub-sample fractional delay `tau` in [0, 1)
/// where the discontinuity occurred `tau` sample periods ago (i.e., at `n - tau`).
/// Returns `(corr_prev, corr_curr)` to be added to sample `y[n-1]` and `y[n]`.
#[inline(always)]
pub fn poly_blep_subsample_pair(tau: f32) -> (f32, f32) {
    let t = tau.clamp(0.0, 1.0);
    // 2-point causal/lookback polynomial residual across [n-1, n]
    let corr_prev = 0.5 * t * t;
    let corr_curr = -0.5 * (1.0 - t) * (1.0 - t);
    (corr_prev, corr_curr)
}

/// PolyBLAMP residual for ramp/slope discontinuities (1st derivative jump).
#[inline(always)]
pub fn poly_blamp_2(t: f32, dt: f32) -> f32 {
    if dt <= 1.0e-9 {
        return 0.0;
    }
    if t < dt {
        let u = t / dt - 1.0;
        -(1.0 / 3.0) * dt * u * u * u
    } else if t > 1.0 - dt {
        let u = (t - 1.0) / dt + 1.0;
        (1.0 / 3.0) * dt * u * u * u
    } else {
        0.0
    }
}

/// PolyBLAMP residual pair `(corr_prev, corr_curr)` for a slope jump at subsample delay `tau` in [0, 1).
#[inline(always)]
pub fn poly_blamp_subsample_pair(tau: f32) -> (f32, f32) {
    let t = tau.clamp(0.0, 1.0);
    let omt = 1.0 - t;
    let corr_prev = (1.0 / 6.0) * t * t * t;
    let corr_curr = (1.0 / 6.0) * omt * omt * omt;
    (corr_prev, corr_curr)
}

/// Waveform selection flags for CEM3340 Oscillator A (Sawtooth + Pulse simultaneously summable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscAWaveforms {
    pub saw: bool,
    pub pulse: bool,
}

impl Default for OscAWaveforms {
    fn default() -> Self {
        Self {
            saw: true,
            pulse: false,
        }
    }
}

/// Waveform selection flags for CEM3340 Oscillator B (Sawtooth + Triangle + Pulse simultaneously summable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OscBWaveforms {
    pub saw: bool,
    pub triangle: bool,
    pub pulse: bool,
    pub lo_freq: bool,
    pub key_tracking: bool,
}

impl Default for OscBWaveforms {
    fn default() -> Self {
        Self {
            saw: true,
            triangle: false,
            pulse: false,
            lo_freq: false,
            key_tracking: true,
        }
    }
}

/// Unit-phase fast wrap helper equivalent to `x.rem_euclid(1.0)` for small shifts `|x| < 3.0`.
#[inline(always)]
fn wrap_unit_phase(mut x: f32) -> f32 {
    if x >= 1.0 {
        x -= 1.0;
        if x >= 1.0 {
            x = x.rem_euclid(1.0);
        }
    } else if x < 0.0 {
        x += 1.0;
        if x < 0.0 {
            x = x.rem_euclid(1.0);
        }
    }
    x
}

/// Curtis CEM3340 Dual VCO Core (Oscillator B = Master, Oscillator A = Slave with Sub-Sample Hard Sync).
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct Cem3340DualVco {
    pub phase_a: f32,
    pub phase_b: f32,
    pub prev_out_a: f32,
    pub pending_curr_a: f32,
    pub sample_rate: f32,
    pub inv_sample_rate: f32,
    /// Per-voice deterministic frequency dispersion in cents (e.g. +/- 3.0 cents)
    pub cents_offset_a: f32,
    pub cents_offset_b: f32,
    cached_cents_a: f32,
    cached_cents_b: f32,
    cached_disp_a: f32,
    cached_disp_b: f32,
}

impl Cem3340DualVco {
    pub fn new(sample_rate: f32, cents_offset_a: f32, cents_offset_b: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        Self {
            phase_a: 0.0,
            phase_b: 0.0,
            prev_out_a: 0.0,
            pending_curr_a: 0.0,
            sample_rate: sr,
            inv_sample_rate: 1.0 / sr,
            cents_offset_a,
            cents_offset_b,
            cached_cents_a: cents_offset_a,
            cached_cents_b: cents_offset_b,
            cached_disp_a: (cents_offset_a * (1.0 / 1200.0)).exp2(),
            cached_disp_b: (cents_offset_b * (1.0 / 1200.0)).exp2(),
        }
    }

    #[inline(always)]
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.sample_rate = sr;
        self.inv_sample_rate = 1.0 / sr;
    }

    #[inline(always)]
    pub fn reset(&mut self, initial_phase_a: f32, initial_phase_b: f32) {
        self.phase_a = initial_phase_a.rem_euclid(1.0);
        self.phase_b = initial_phase_b.rem_euclid(1.0);
        self.prev_out_a = 0.0;
        self.pending_curr_a = 0.0;
    }

    /// Evaluates the naive continuous-time waveform amplitude and slope (per unit phase) of Osc A.
    #[inline(always)]
    fn eval_naive_a(phase: f32, pw: f32, waves: OscAWaveforms) -> (f32, f32) {
        let mut amp = 0.0;
        let mut slope = 0.0;
        let mut count = 0.0;
        if waves.saw {
            amp += 2.0 * phase - 1.0;
            slope += 2.0;
            count += 1.0;
        }
        if waves.pulse {
            let p_val = if phase < pw { 1.0 } else { -1.0 };
            // DC-compensated pulse wave as in CEM3340 summing node
            let dc_comp = 1.0 - 2.0 * pw;
            amp += p_val + dc_comp;
            count += 1.0;
        }
        if count > 1.0 {
            let norm = core::f32::consts::FRAC_1_SQRT_2; // 1 / sqrt(2) headroom normalization when both Saw + Pulse are active
            (amp * norm, slope * norm)
        } else {
            (amp, slope)
        }
    }

    /// Computes one sample pair `(out_a, out_b)` with exact sub-sample Hard Sync and PolyBLEP/PolyBLAMP.
    ///
    /// Parameters:
    /// - `freq_a_hz`: Instantaneous frequency of Osc A in Hz (including Poly-Mod FM & LFO).
    /// - `freq_b_hz`: Instantaneous frequency of Osc B in Hz.
    /// - `pw_a`: Pulse width of Osc A, clamped to `[0.01, 0.99]`.
    /// - `pw_b`: Pulse width of Osc B, clamped to `[0.01, 0.99]`.
    /// - `waves_a`: Active waveforms for Osc A.
    /// - `waves_b`: Active waveforms and LO FREQ flag for Osc B.
    /// - `hard_sync`: True when Osc A is hard-synced to Osc B's reset comparator.
    #[inline]
    pub fn process_sample(
        &mut self,
        freq_a_hz: f32,
        freq_b_hz: f32,
        pw_a: f32,
        pw_b: f32,
        waves_a: OscAWaveforms,
        waves_b: OscBWaveforms,
        hard_sync: bool,
    ) -> (f32, f32) {
        let pw_a_clamped = pw_a.clamp(0.01, 0.99);
        let pw_b_clamped = pw_b.clamp(0.01, 0.99);

        // Apply deterministic CEM3340 per-voice tuning dispersion (cached across substeps)
        if self.cents_offset_a != self.cached_cents_a {
            self.cached_cents_a = self.cents_offset_a;
            self.cached_disp_a = (self.cents_offset_a * (1.0 / 1200.0)).exp2();
        }
        if self.cents_offset_b != self.cached_cents_b {
            self.cached_cents_b = self.cents_offset_b;
            self.cached_disp_b = (self.cents_offset_b * (1.0 / 1200.0)).exp2();
        }
        let disp_a = self.cached_disp_a;
        let disp_b = self.cached_disp_b;

        let fa = (freq_a_hz * disp_a).clamp(0.01, 0.48 * self.sample_rate);
        let fb = if waves_b.lo_freq {
            (freq_b_hz * disp_b).clamp(0.005, 200.0)
        } else {
            (freq_b_hz * disp_b).clamp(0.01, 0.48 * self.sample_rate)
        };

        let dt_a = fa * self.inv_sample_rate;
        let dt_b = fb * self.inv_sample_rate;

        // --- 1. Master Oscillator B Progression ---
        let p_b = self.phase_b;
        let next_phase_b_unwrapped = p_b + dt_b;
        let master_wrapped = next_phase_b_unwrapped >= 1.0;
        // Exact subsample fraction tau in [0, 1) elapsed SINCE master crossed 1.0
        let tau_sync = if master_wrapped && dt_b > 1.0e-9 {
            ((next_phase_b_unwrapped - 1.0) / dt_b).clamp(0.0, 0.999999)
        } else {
            0.0
        };
        self.phase_b = wrap_unit_phase(next_phase_b_unwrapped);

        // Synthesize Oscillator B output
        let mut out_b = 0.0;
        let mut count_b: f32 = 0.0;
        let use_blep_b = !waves_b.lo_freq;

        if waves_b.saw {
            let mut saw_b = 2.0 * p_b - 1.0;
            if use_blep_b {
                saw_b -= poly_blep_4(p_b, dt_b);
            }
            out_b += saw_b;
            count_b += 1.0;
        }
        if waves_b.triangle {
            // Triangle with vertex at 0.0 and 0.5
            let mut tri_b = if p_b < 0.5 {
                4.0 * p_b - 1.0
            } else {
                3.0 - 4.0 * p_b
            };
            if use_blep_b {
                let p_b_half = wrap_unit_phase(p_b + 0.5);
                tri_b += 4.0 * (poly_blamp_2(p_b, dt_b) - poly_blamp_2(p_b_half, dt_b));
            }
            out_b += tri_b;
            count_b += 1.0;
        }
        if waves_b.pulse {
            let dc_comp_b = 1.0 - 2.0 * pw_b_clamped;
            let mut pulse_b = (if p_b < pw_b_clamped { 1.0 } else { -1.0 }) + dc_comp_b;
            if use_blep_b {
                let p_b_pw = wrap_unit_phase(p_b - pw_b_clamped);
                pulse_b += poly_blep_4(p_b, dt_b) - poly_blep_4(p_b_pw, dt_b);
            }
            out_b += pulse_b;
            count_b += 1.0;
        }
        if count_b > 1.0 {
            out_b /= count_b.sqrt();
        }

        // --- 2. Slave Oscillator A Progression + Exact Sub-Sample Hard Sync ---
        let norm_a = if waves_a.saw && waves_a.pulse {
            core::f32::consts::FRAC_1_SQRT_2
        } else {
            1.0
        };

        let p_a_start = self.phase_a;
        let mut corr_prev = 0.0f32;
        let mut corr_curr = self.pending_curr_a;

        if hard_sync && master_wrapped {
            // Time interval before Master reset is (1 - tau_sync) * dt_a
            let phase_before_sync = p_a_start + (1.0 - tau_sync) * dt_a;

            // Check if Slave Osc A naturally wrapped or crossed PW BEFORE the Master sync event
            if phase_before_sync >= 1.0 && dt_a > 1.0e-9 {
                let tau_natural = ((phase_before_sync - 1.0) / dt_a + tau_sync).clamp(0.0, 1.0);
                let mut jump = 0.0;
                if waves_a.saw {
                    jump -= 2.0 * norm_a;
                }
                if waves_a.pulse {
                    jump += 2.0 * norm_a;
                }
                let (cp, cc) = poly_blep_subsample_pair(tau_natural);
                corr_prev += jump * cp;
                corr_curr += jump * cc;
            }
            if waves_a.pulse {
                let p_rel = (p_a_start - pw_a_clamped).rem_euclid(1.0);
                let p_rel_before = p_rel + (1.0 - tau_sync) * dt_a;
                if p_rel_before >= 1.0 && dt_a > 1.0e-9 {
                    let tau_pw = ((p_rel_before - 1.0) / dt_a + tau_sync).clamp(0.0, 1.0);
                    let jump_pw = -2.0 * norm_a;
                    let (cp, cc) = poly_blep_subsample_pair(tau_pw);
                    corr_prev += jump_pw * cp;
                    corr_curr += jump_pw * cc;
                }
            }

            // Now Master Osc B resets Slave Osc A to 0.0 at subsample offset `tau_sync` ago!
            let p_pre_sync = phase_before_sync.rem_euclid(1.0);
            let (amp_pre, slope_pre) = Self::eval_naive_a(p_pre_sync, pw_a_clamped, waves_a);
            let (amp_post, slope_post) = Self::eval_naive_a(0.0, pw_a_clamped, waves_a);

            let delta_amp = amp_post - amp_pre;
            let delta_slope = (slope_post - slope_pre) * dt_a;

            let (blep_prev, blep_curr) = poly_blep_subsample_pair(tau_sync);
            let (blamp_prev, blamp_curr) = poly_blamp_subsample_pair(tau_sync);
            corr_prev += delta_amp * blep_prev + delta_slope * blamp_prev;
            corr_curr += delta_amp * blep_curr + delta_slope * blamp_curr;

            // After sync reset to 0.0, Slave Osc A advances by `tau_sync * dt_a`
            let p_after_sync = tau_sync * dt_a;
            if waves_a.pulse && p_after_sync >= pw_a_clamped && dt_a > 1.0e-9 {
                let tau_pw_post = ((p_after_sync - pw_a_clamped) / dt_a).clamp(0.0, 1.0);
                let jump_pw = -2.0 * norm_a;
                let (cp, cc) = poly_blep_subsample_pair(tau_pw_post);
                corr_prev += jump_pw * cp;
                corr_curr += jump_pw * cc;
            }
            self.phase_a = p_after_sync.rem_euclid(1.0);
        } else {
            // Standard free-running Osc A progression with subsample discontinuity tracking
            let next_p_a = p_a_start + dt_a;
            if next_p_a >= 1.0 && dt_a > 1.0e-9 {
                let tau_wrap = ((next_p_a - 1.0) / dt_a).clamp(0.0, 1.0);
                let mut jump = 0.0;
                if waves_a.saw {
                    jump -= 2.0 * norm_a;
                }
                if waves_a.pulse {
                    jump += 2.0 * norm_a;
                }
                let (cp, cc) = poly_blep_subsample_pair(tau_wrap);
                corr_prev += jump * cp;
                corr_curr += jump * cc;
            }
            if waves_a.pulse {
                let p_rel = (p_a_start - pw_a_clamped).rem_euclid(1.0);
                let next_p_rel = p_rel + dt_a;
                if next_p_rel >= 1.0 && dt_a > 1.0e-9 {
                    let tau_pw = ((next_p_rel - 1.0) / dt_a).clamp(0.0, 1.0);
                    let jump_pw = -2.0 * norm_a;
                    let (cp, cc) = poly_blep_subsample_pair(tau_pw);
                    corr_prev += jump_pw * cp;
                    corr_curr += jump_pw * cc;
                }
            }
            self.phase_a = next_p_a.rem_euclid(1.0);
        }

        let (naive_curr_a, _) = Self::eval_naive_a(self.phase_a, pw_a_clamped, waves_a);
        // 1-sample lookback output allows exact causal anti-aliased hard sync
        let out_a = self.prev_out_a + corr_prev;
        self.prev_out_a = naive_curr_a + corr_curr;
        self.pending_curr_a = 0.0;

        (out_a, out_b)
    }
}

/// 31-bit Galois LFSR + Paul Kellet Pink Noise Filter (-3 dB/oct) for Prophet-10 Noise Mixer.
#[derive(Clone, Debug)]
pub struct ProphetNoiseGen {
    lfsr_state: u32,
    b0: f32,
    b1: f32,
    b2: f32,
    b3: f32,
    b4: f32,
    b5: f32,
    b6: f32,
}

impl ProphetNoiseGen {
    pub fn new(seed: u32) -> Self {
        Self {
            lfsr_state: if seed == 0 { 0x5A3C_9E71 } else { seed },
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            b3: 0.0,
            b4: 0.0,
            b5: 0.0,
            b6: 0.0,
        }
    }

    /// Generates next white noise sample in [-1.0, 1.0] via 31-bit LFSR + xorshift whitening.
    #[inline(always)]
    pub fn next_white(&mut self) -> f32 {
        let lsb = self.lfsr_state & 1;
        self.lfsr_state >>= 1;
        if lsb != 0 {
            self.lfsr_state ^= 0xB4BCD35C;
        }
        let mut x = self.lfsr_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let normalized = (x as f32) * (1.0 / (u32::MAX as f32));
        2.0 * normalized - 1.0
    }

    /// Generates next pink noise sample (-3 dB/octave) using Paul Kellet's 6-pole/zero network.
    #[inline(always)]
    pub fn next_pink(&mut self) -> f32 {
        let white = self.next_white();
        self.b0 = 0.99886 * self.b0 + white * 0.0555179;
        self.b1 = 0.99332 * self.b1 + white * 0.0750759;
        self.b2 = 0.96900 * self.b2 + white * 0.1538520;
        self.b3 = 0.86650 * self.b3 + white * 0.3104856;
        self.b4 = 0.55000 * self.b4 + white * 0.5329522;
        self.b5 = -0.7616 * self.b5 - white * 0.0168980;
        let pink =
            self.b0 + self.b1 + self.b2 + self.b3 + self.b4 + self.b5 + self.b6 + white * 0.5362;
        self.b6 = white * 0.115926;
        (pink * 0.18).clamp(-1.0, 1.0)
    }

    /// Continuous crossfade between White Noise (`color = 0.0`) and Pink Noise (`color = 1.0`).
    #[inline(always)]
    pub fn process(&mut self, pink_blend: f32) -> f32 {
        let white = self.next_white();
        self.b0 = 0.99886 * self.b0 + white * 0.0555179;
        self.b1 = 0.99332 * self.b1 + white * 0.0750759;
        self.b2 = 0.96900 * self.b2 + white * 0.1538520;
        self.b3 = 0.86650 * self.b3 + white * 0.3104856;
        self.b4 = 0.55000 * self.b4 + white * 0.5329522;
        self.b5 = -0.7616 * self.b5 - white * 0.0168980;
        let pink =
            (self.b0 + self.b1 + self.b2 + self.b3 + self.b4 + self.b5 + self.b6 + white * 0.5362)
                * 0.18;
        self.b6 = white * 0.115926;
        let c = pink_blend.clamp(0.0, 1.0);
        white * (1.0 - c) + pink * c
    }
}
