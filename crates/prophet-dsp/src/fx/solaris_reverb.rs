// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — "SOLARIS-CLOUDBURST" Hybrid Ambient Reverb
// Mathematical grounding:
// - Stage 1: Tempo-syncable Pre-Delay (0..500 ms) + 4-stage Modulated Cascade Allpass Diffuser
// - Stage 2: 48-Band Constant-Q Log-Spaced Harmonic Ensemble Engine (80 Hz..12 kHz) with
//            per-band Envelope Followers (15 ms attack / 120 ms release) & 3-phase Solina string
//            harmonic partial synthesizers (2nd & 3rd harmonics)
// - Stage 3: 8-Channel Mutually-Prime Feedback Delay Network (FDN) with Orthogonal 8x8 Hadamard
//            Matrix, Dual Granular Shimmer Pitch-Shifter (+12 / +7 semitones), and Continuous
//            5-Vowel Formant Filter (A, E, I, O, U) + Freeze
// - References: CloudSeed (https://github.com/ValdemarOrn/CloudSeed) & Dattorro / Signalsmith

use core::f32::consts::PI;

pub const NUM_ENSEMBLE_BANDS: usize = 48;
pub const FDN_CHANNELS: usize = 8;
/// Pre-delay buffer supporting up to 500 ms at 192 kHz (131072 power-of-2 ring buffer)
pub const MAX_PREDELAY_LEN: usize = 131072;
pub const CASCADE_AP_LEN: usize = 8192;
pub const FDN_BUFFER_LEN: usize = 32768;
pub const SHIMMER_BUF_LEN: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnsembleMode {
    Off,
    MezzoPiano, // mp: subtle lush harmonic halo
    Forte,      // forte: rich polyphonic string orchestra synthesis
}

/// Spill renders remaining tails; Kill fades them before invalidating history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReverbTailPolicy {
    Spill,
    Kill,
}

/// Parameters for the Solaris-Cloudburst Hybrid Reverb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SolarisReverbParams {
    pub enabled: bool,
    pub bypass_tail: ReverbTailPolicy,
    pub pre_delay_ms: f32,
    pub cascade_diffusion: f32,
    pub ensemble_mode: EnsembleMode,
    pub ensemble_blend: f32,
    pub decay_sec: f32,
    pub shimmer_amount: f32,
    /// Pitch shift mode: +12 semitones + optional +7 fifth harmonic
    pub shimmer_fifth_blend: f32,
    /// Continuous vowel morph across [0.0 = A, 0.25 = E, 0.5 = I, 0.75 = O, 1.0 = U]
    pub vowel_morph: f32,
    pub vowel_mix: f32,
    pub high_cut_hz: f32,
    pub low_cut_hz: f32,
    pub mod_depth: f32,
    pub mod_speed_hz: f32,
    pub freeze: bool,
    pub wet_mix: f32,
    pub shimmer_semitones: f32,
    pub tempo_bpm: f32,
    pub predelay_sync: bool,
    pub predelay_beats: f32,
}

impl Default for SolarisReverbParams {
    fn default() -> Self {
        Self {
            enabled: true,
            bypass_tail: ReverbTailPolicy::Spill,
            pre_delay_ms: 28.0,
            cascade_diffusion: 0.72,
            ensemble_mode: EnsembleMode::MezzoPiano,
            ensemble_blend: 0.35,
            decay_sec: 4.8,
            shimmer_amount: 0.22,
            shimmer_fifth_blend: 0.30,
            vowel_morph: 0.15,
            vowel_mix: 0.30,
            high_cut_hz: 7500.0,
            low_cut_hz: 110.0,
            mod_depth: 0.45,
            mod_speed_hz: 0.65,
            freeze: false,
            wet_mix: 0.32,
            shimmer_semitones: 12.0,
            tempo_bpm: 120.0,
            predelay_sync: false,
            predelay_beats: 0.25,
        }
    }
}

/// Wrap a phase value near `[0.0, 1.0)` without invoking libm `fmodf`.
#[inline(always)]
fn wrap_01(mut p: f32) -> f32 {
    if p >= 1.0 {
        p -= 1.0;
        if p >= 1.0 {
            p -= p.floor();
        }
    } else if p < 0.0 {
        p += 1.0;
        if p < 0.0 {
            p -= p.floor();
        }
        if p >= 1.0 {
            p = 0.0;
        }
    }
    p
}

/// Single Band of the 48-Band Constant-Q Cloudburst Harmonic Ensemble Analyzer & Exciter.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnsembleBandChannel {
    /// TPT SVF states for Constant-Q bandpass analysis filter
    ic1eq: f32,
    ic2eq: f32,
    g: f32,
    a1: f32,
    a2: f32,
    k_inv_q: f32,
    /// Envelope follower state (15 ms attack, 120 ms release)
    env: f32,
    /// Instantaneous phase of 2nd & 3rd harmonic partial oscillators
    phase_h2: f32,
    phase_h3: f32,
    dt_h2: f32,
    dt_h3: f32,
}

impl EnsembleBandChannel {
    pub fn configure(&mut self, sample_rate: f32, center_freq_hz: f32) {
        let sr = sample_rate.max(8000.0);
        let fc = center_freq_hz.clamp(60.0, 0.44 * sr);
        // Constant-Q resonator with Q = 11.5 across 48 log-spaced bands (80 Hz .. 12 kHz)
        let g = (PI * fc / sr).tan();
        let k_inv_q = 1.0 / 11.5;
        let a1 = 1.0 / (1.0 + g * (g + k_inv_q));
        self.g = g;
        self.k_inv_q = k_inv_q;
        self.a1 = a1;
        self.a2 = g * a1;
        self.dt_h2 = 2.0 * fc / sr;
        self.dt_h3 = 3.0 * fc / sr;
    }

    #[inline(always)]
    pub fn process(
        &mut self,
        input: f32,
        att_coeff: f32,
        rel_coeff: f32,
        freq_mod: f32,
        forte_boost: f32,
    ) -> f32 {
        // 1. Constant-Q TPT Bandpass filter extraction (precomputed a1, a2)
        let v3 = input - self.ic2eq;
        let v1 = self.a1 * self.ic1eq + self.a2 * v3;
        let v2 = self.ic2eq + self.g * v1;
        self.ic1eq = flush_tiny(2.0 * v1 - self.ic1eq);
        self.ic2eq = flush_tiny(2.0 * v2 - self.ic2eq);
        let bp = v1 * self.k_inv_q;

        // 2. Dynamic Envelope Follower (15 ms attack, 120 ms release)
        let rect = bp.abs();
        let coeff = if rect > self.env { att_coeff } else { rel_coeff };
        self.env = flush_tiny(self.env + (rect - self.env) * coeff);

        // 3. Harmonic Partial Synthesis (2nd and 3rd harmonics with Solina 3-phase modulation)
        let inc2 = self.dt_h2 * freq_mod;
        let inc3 = self.dt_h3 * freq_mod;
        let mut harmonic = 0.0f32;
        if inc2 < 0.45 {
            let mut p2 = self.phase_h2 + inc2;
            if p2 >= 1.0 {
                p2 -= 1.0;
            }
            self.phase_h2 = p2;
            harmonic += 0.72 * fast_unit_sine(p2);
        }
        if inc3 < 0.45 {
            let mut p3 = self.phase_h3 + inc3;
            if p3 >= 1.0 {
                p3 -= 1.0;
            }
            self.phase_h3 = p3;
            harmonic += 0.28 * fast_unit_sine(p3);
        }

        // Non-linear soft expansion in Forte mode
        let env_shaped = self.env * (1.0 + forte_boost * (self.env * 2.5).min(1.5));
        env_shaped * harmonic
    }
}

/// Structure-of-Arrays 3-lane x 16-band layout matching the 48-band logarithmic Ensemble bank
/// (`band k = group * 3 + lane`), enabling branch-free auto-vectorized processing per lane.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
struct EnsembleBank {
    ic1eq: [[f32; 16]; 3],
    ic2eq: [[f32; 16]; 3],
    g: [[f32; 16]; 3],
    a1: [[f32; 16]; 3],
    a2: [[f32; 16]; 3],
    env: [[f32; 16]; 3],
    phase_h2: [[f32; 16]; 3],
    phase_h3: [[f32; 16]; 3],
    dt_h2: [[f32; 16]; 3],
    dt_h3: [[f32; 16]; 3],
}

impl Default for EnsembleBank {
    fn default() -> Self {
        Self {
            ic1eq: [[0.0; 16]; 3],
            ic2eq: [[0.0; 16]; 3],
            g: [[0.0; 16]; 3],
            a1: [[0.0; 16]; 3],
            a2: [[0.0; 16]; 3],
            env: [[0.0; 16]; 3],
            phase_h2: [[0.0; 16]; 3],
            phase_h3: [[0.0; 16]; 3],
            dt_h2: [[0.0; 16]; 3],
            dt_h3: [[0.0; 16]; 3],
        }
    }
}

impl EnsembleBank {
    fn clear_and_configure(&mut self, sample_rate: f32) {
        *self = Self::default();
        self.configure(sample_rate);
    }

    fn configure(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        let ratio = 12000.0f32 / 80.0f32;
        let k_inv_q = 1.0f32 / 11.5;
        for k in 0..NUM_ENSEMBLE_BANDS {
            let frac = (k as f32) / ((NUM_ENSEMBLE_BANDS - 1) as f32);
            let fc = (80.0 * ratio.powf(frac)).clamp(60.0, 0.44 * sr);
            let g = (PI * fc / sr).tan();
            let a1 = 1.0 / (1.0 + g * (g + k_inv_q));
            let lane = k % 3;
            let idx = k / 3;
            self.g[lane][idx] = g;
            self.a1[lane][idx] = a1;
            self.a2[lane][idx] = g * a1;
            self.dt_h2[lane][idx] = 2.0 * fc / sr;
            self.dt_h3[lane][idx] = 3.0 * fc / sr;
        }
    }

    #[inline(always)]
    fn process_lane(
        &mut self,
        lane: usize,
        input: f32,
        att_coeff: f32,
        rel_coeff: f32,
        freq_mod: f32,
        forte_boost: f32,
    ) -> f32 {
        let k_inv_q = 1.0f32 / 11.5;
        let ic1_arr = &mut self.ic1eq[lane];
        let ic2_arr = &mut self.ic2eq[lane];
        let env_arr = &mut self.env[lane];
        let ph2_arr = &mut self.phase_h2[lane];
        let ph3_arr = &mut self.phase_h3[lane];
        let g_arr = &self.g[lane];
        let a1_arr = &self.a1[lane];
        let a2_arr = &self.a2[lane];
        let dt2_arr = &self.dt_h2[lane];
        let dt3_arr = &self.dt_h3[lane];

        let mut lanes_out = [0.0f32; 16];
        for i in 0..16 {
            let ic1 = ic1_arr[i];
            let ic2 = ic2_arr[i];
            let v3 = input - ic2;
            let v1 = a1_arr[i] * ic1 + a2_arr[i] * v3;
            let v2 = ic2 + g_arr[i] * v1;
            let next_ic1 = 2.0 * v1 - ic1;
            let next_ic2 = 2.0 * v2 - ic2;
            ic1_arr[i] = if next_ic1.abs() < 1.0e-20 { 0.0 } else { next_ic1 };
            ic2_arr[i] = if next_ic2.abs() < 1.0e-20 { 0.0 } else { next_ic2 };
            let bp = v1 * k_inv_q;

            let rect = bp.abs();
            let prev_env = env_arr[i];
            let coeff = if rect > prev_env { att_coeff } else { rel_coeff };
            let next_env = prev_env + (rect - prev_env) * coeff;
            let next_env = if next_env.abs() < 1.0e-20 { 0.0 } else { next_env };
            env_arr[i] = next_env;

            let inc2 = dt2_arr[i] * freq_mod;
            let inc3 = dt3_arr[i] * freq_mod;
            let m2 = if inc2 < 0.45 { 1.0f32 } else { 0.0f32 };
            let m3 = if inc3 < 0.45 { 1.0f32 } else { 0.0f32 };

            let p2_raw = ph2_arr[i] + inc2 * m2;
            let p2 = if p2_raw >= 1.0 { p2_raw - 1.0 } else { p2_raw };
            ph2_arr[i] = p2;

            let p3_raw = ph3_arr[i] + inc3 * m3;
            let p3 = if p3_raw >= 1.0 { p3_raw - 1.0 } else { p3_raw };
            ph3_arr[i] = p3;

            let harmonic =
                (0.72 * m2) * fast_unit_sine(p2) + (0.28 * m3) * fast_unit_sine(p3);
            let boost = (next_env * 2.5).min(1.5);
            let env_shaped = next_env * (1.0 + forte_boost * boost);
            lanes_out[i] = env_shaped * harmonic;
        }
        let s0 = lanes_out[0] + lanes_out[4] + lanes_out[8] + lanes_out[12];
        let s1 = lanes_out[1] + lanes_out[5] + lanes_out[9] + lanes_out[13];
        let s2 = lanes_out[2] + lanes_out[6] + lanes_out[10] + lanes_out[14];
        let s3 = lanes_out[3] + lanes_out[7] + lanes_out[11] + lanes_out[15];
        (s0 + s2) + (s1 + s3)
    }
}

#[inline(always)]
fn fast_unit_sine(phase_0_to_1: f32) -> f32 {
    let x = phase_0_to_1 * 2.0 - 1.0;
    let y = 4.0 * x * (1.0 - x.abs());
    0.225 * (y * y.abs() - y) + y
}

/// Dual-Head Root-Hann-Windowed Granular Pitch Shifter (+12 Semitones Octave & +7 Semitones Fifth).
#[derive(Clone, Debug)]
pub struct GranularPitchShifter {
    buf: [f32; SHIMMER_BUF_LEN],
    write_pos: usize,
    phasor: f32,
    grain_size: f32,
    filled: u64,
}

impl GranularPitchShifter {
    pub fn new() -> Self {
        Self {
            buf: [0.0; SHIMMER_BUF_LEN],
            write_pos: 0,
            phasor: 0.0,
            grain_size: 2048.0,
            filled: 0,
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        // Preserve ~42.67 ms grain duration at 48 kHz across sample rates (bounded for SHIMMER_BUF_LEN = 8192).
        self.grain_size = (2048.0 * (sr / 48000.0)).clamp(512.0, (SHIMMER_BUF_LEN - 256) as f32);
    }

    #[inline(always)]
    fn read_linear(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (SHIMMER_BUF_LEN - 4) as f32);
        let d_int = d as usize;
        let frac = d - (d_int as f32);
        let mask = SHIMMER_BUF_LEN - 1;
        let idx0 = (self.write_pos + SHIMMER_BUF_LEN - d_int) & mask;
        let idx1 = (idx0 + SHIMMER_BUF_LEN - 1) & mask;
        let a = if self.filled >= d_int as u64 {
            self.buf[idx0]
        } else {
            0.0
        };
        let b = if self.filled > d_int as u64 {
            self.buf[idx1]
        } else {
            0.0
        };
        a * (1.0 - frac) + b * frac
    }

    /// Pitch-shifts `input` by `pitch_ratio` (e.g. `2.0` for +12 st, `1.4983` for +7 st).
    #[inline(always)]
    pub fn process(&mut self, input: f32, pitch_ratio: f32) -> f32 {
        self.buf[self.write_pos] = flush_tiny(input);
        self.write_pos = (self.write_pos + 1) & (SHIMMER_BUF_LEN - 1);
        self.filled = self.filled.saturating_add(1);

        let rate = (1.0 - pitch_ratio) / self.grain_size;
        self.phasor = wrap_01(self.phasor + rate);
        let phasor_b = if self.phasor >= 0.5 {
            self.phasor - 0.5
        } else {
            self.phasor + 0.5
        };

        let delay_a = self.phasor * self.grain_size + 64.0;
        let delay_b = phasor_b * self.grain_size + 64.0;

        // Complementary Hann crossfade: win_a + win_b == 1.0 everywhere, ensuring
        // strict non-expansive operator gain (||P|| <= 1.0) inside recursive FDN loops.
        let s = (PI * self.phasor).sin();
        let win_a = s * s;
        let win_b = 1.0 - win_a;

        self.read_linear(delay_a) * win_a + self.read_linear(delay_b) * win_b
    }
}

/// Solaris Continuous 5-Vowel Formant Filter (A, E, I, O, U) using 3 Parallel TPT Bandpass Resonators.
#[derive(Clone, Copy, Debug, Default)]
pub struct SolarisVowelFilter {
    ic1: [f32; 3],
    ic2: [f32; 3],
    cached_morph: f32,
    cached_sr: f32,
    gs: [f32; 3],
    a1s: [f32; 3],
    a2s: [f32; 3],
}

impl SolarisVowelFilter {
    /// Formant frequency table (F1, F2, F3 in Hz) for Vowels [A, E, I, O, U] (Peterson & Barney).
    const FORMANTS: [[f32; 3]; 5] = [
        [730.0, 1090.0, 2440.0], // A
        [530.0, 1840.0, 2480.0], // E
        [270.0, 2290.0, 3010.0], // I
        [570.0, 840.0, 2410.0],  // O
        [300.0, 870.0, 2240.0],  // U
    ];

    const GAINS: [f32; 3] = [1.0, 0.55, 0.30];
    const KS: [f32; 3] = [1.0 / 7.5, 1.0 / 9.0, 1.0 / 11.0];

    #[inline(always)]
    pub fn process(&mut self, input: f32, vowel_morph: f32, sample_rate: f32) -> f32 {
        let clamped_morph = vowel_morph.clamp(0.0, 1.0);
        if self.cached_sr != sample_rate || self.cached_morph != clamped_morph {
            let pos = clamped_morph * 4.0;
            let idx0 = (pos.floor() as usize).min(4);
            let idx1 = (idx0 + 1).min(4);
            let frac = pos - (idx0 as f32);
            for i in 0..3 {
                let freq = Self::FORMANTS[idx0][i] * (1.0 - frac) + Self::FORMANTS[idx1][i] * frac;
                let g = (PI * freq.clamp(150.0, 0.42 * sample_rate) / sample_rate).tan();
                let a1 = 1.0 / (1.0 + g * (g + Self::KS[i]));
                self.gs[i] = g;
                self.a1s[i] = a1;
                self.a2s[i] = g * a1;
            }
            self.cached_sr = sample_rate;
            self.cached_morph = clamped_morph;
        }
        let mut sum = 0.0;

        for f_idx in 0..3 {
            let g = self.gs[f_idx];
            let k = Self::KS[f_idx];
            let a1 = self.a1s[f_idx];
            let a2 = self.a2s[f_idx];
            let v3 = input - self.ic2[f_idx];
            let v1 = a1 * self.ic1[f_idx] + a2 * v3;
            let v2 = self.ic2[f_idx] + g * v1;
            self.ic1[f_idx] = flush_tiny(2.0 * v1 - self.ic1[f_idx]);
            self.ic2[f_idx] = flush_tiny(2.0 * v2 - self.ic2[f_idx]);
            sum += (v1 * k) * Self::GAINS[f_idx];
        }
        flush_tiny(sum / 1.85)
    }
}

/// Orthogonal 8x8 Fast Walsh-Hadamard Mixing Matrix (Normalized by `1 / sqrt(8)`).
#[inline(always)]
pub fn hadamard_8x8(x: [f32; FDN_CHANNELS]) -> [f32; FDN_CHANNELS] {
    // Stage 1
    let a0 = x[0] + x[1];
    let a1 = x[0] - x[1];
    let a2 = x[2] + x[3];
    let a3 = x[2] - x[3];
    let a4 = x[4] + x[5];
    let a5 = x[4] - x[5];
    let a6 = x[6] + x[7];
    let a7 = x[6] - x[7];

    // Stage 2
    let b0 = a0 + a2;
    let b1 = a1 + a3;
    let b2 = a0 - a2;
    let b3 = a1 - a3;
    let b4 = a4 + a6;
    let b5 = a5 + a7;
    let b6 = a4 - a6;
    let b7 = a5 - a7;

    // Stage 3 * (1 / sqrt(8) = 0.35355339)
    const NORM: f32 = 0.353_553_39;
    [
        (b0 + b4) * NORM,
        (b1 + b5) * NORM,
        (b2 + b6) * NORM,
        (b3 + b7) * NORM,
        (b0 - b4) * NORM,
        (b1 - b5) * NORM,
        (b2 - b6) * NORM,
        (b3 - b7) * NORM,
    ]
}

/// Complete "SOLARIS-CLOUDBURST" Hybrid Ambient Reverb Engine.
/// All delay lines are pre-allocated at struct initialization to uphold zero-allocation RT safety.
#[repr(C, align(32))]
#[derive(Clone)]
pub struct SolarisCloudburstReverb {
    pub sample_rate: f32,
    // Stage 1: Pre-Delay & 4-stage Cascade Modulated Allpass Diffuser
    predelay_buf: Box<[f32]>,
    predelay_pos: usize,
    cascade_bufs: [Box<[f32]>; 4],
    cascade_pos: [usize; 4],

    // Stage 2: 48-Band Constant-Q Harmonic Ensemble (80 Hz .. 12 kHz)
    ensemble_bank: EnsembleBank,
    env_att_coeff: f32,
    env_rel_coeff: f32,
    solina_phase_slow: f32,
    solina_phase_fast: f32,

    // Stage 3: 8-Channel Hadamard FDN Tank + Shimmer + Post Vowel Formant Filter
    fdn_bufs: [Box<[f32]>; FDN_CHANNELS],
    fdn_write_pos: usize,
    fdn_base_delays: [f32; FDN_CHANNELS],
    fdn_lp_states: [f32; FDN_CHANNELS],
    fdn_hp_states: [f32; FDN_CHANNELS],
    mod_phases: [f32; FDN_CHANNELS],

    shimmer_oct: GranularPitchShifter,
    shimmer_fifth: GranularPitchShifter,
    shimmer_was_active: bool,
    vowel_l: SolarisVowelFilter,
    vowel_r: SolarisVowelFilter,
    post_lp_l: f32,
    post_lp_r: f32,
    post_hp_l: f32,
    post_hp_r: f32,
    diff_phases: [f32; 4],
    random: [u32; 8],
    random_target: [f32; 8],
    random_state: [f32; 8],
    sample_counter: u64,
    cached_params: Option<[f32; 3]>,
    coeff_fb: [f32; FDN_CHANNELS],
    coeff_shelf_alpha: [f32; FDN_CHANNELS],
    coeff_shelf_g1: f32,
    coeff_dc_g1: f32,
    coeff_post_hp_g1: f32,
    coeff_post_lp_g1: f32,
    shelf_active: bool,
    enable_gain: f32,
    quiet_samples: usize,
    dormant: bool,
}

impl SolarisCloudburstReverb {
    /// Mutually exclusive prime delay lengths in samples at 48 kHz reference rate (~29 ms to ~89 ms).
    const PRIME_DELAYS_48K: [usize; FDN_CHANNELS] =
        [1399, 1697, 2029, 2411, 2803, 3259, 3761, 4273];
    const CASCADE_DELAYS_48K: [usize; 4] = [193, 349, 577, 911];

    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        let mut rev = Self {
            sample_rate: sr,
            predelay_buf: vec![0.0; MAX_PREDELAY_LEN].into_boxed_slice(),
            predelay_pos: 0,
            cascade_bufs: core::array::from_fn(|_| vec![0.0; CASCADE_AP_LEN].into_boxed_slice()),
            cascade_pos: [0; 4],
            ensemble_bank: EnsembleBank::default(),
            env_att_coeff: 0.0,
            env_rel_coeff: 0.0,
            solina_phase_slow: 0.0,
            solina_phase_fast: 0.0,
            fdn_bufs: core::array::from_fn(|_| vec![0.0; FDN_BUFFER_LEN].into_boxed_slice()),
            fdn_write_pos: 0,
            fdn_base_delays: [1500.0; FDN_CHANNELS],
            fdn_lp_states: [0.0; FDN_CHANNELS],
            fdn_hp_states: [0.0; FDN_CHANNELS],
            mod_phases: [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875],
            shimmer_oct: GranularPitchShifter::new(),
            shimmer_fifth: GranularPitchShifter::new(),
            shimmer_was_active: false,
            vowel_l: SolarisVowelFilter::default(),
            vowel_r: SolarisVowelFilter::default(),
            post_lp_l: 0.0,
            post_lp_r: 0.0,
            post_hp_l: 0.0,
            post_hp_r: 0.0,
            diff_phases: [0.0, 0.23, 0.47, 0.71],
            random: std::array::from_fn(|i| 0x13579u32.wrapping_mul(i as u32 + 1)),
            random_target: [0.0; 8],
            random_state: [0.0; 8],
            sample_counter: 0,
            cached_params: None,
            coeff_fb: [0.9; FDN_CHANNELS],
            coeff_shelf_alpha: [1.0; FDN_CHANNELS],
            coeff_shelf_g1: 0.5,
            coeff_dc_g1: 0.0001,
            coeff_post_hp_g1: 0.002,
            coeff_post_lp_g1: 0.5,
            shelf_active: false,
            enable_gain: 1.0,
            quiet_samples: 0,
            dormant: true,
        };
        rev.set_sample_rate(sr);
        rev
    }

    pub fn reset(&mut self) {
        self.clear_history();
        self.set_sample_rate(self.sample_rate);
    }

    pub fn is_dormant(&self) -> bool {
        self.dormant
    }
    pub fn enable_gain(&self) -> f32 {
        self.enable_gain
    }

    fn clear_history(&mut self) {
        // Read guards invalidate old storage until overwritten. No 1.7 MB
        // memset, allocation or delay-history scan in the callback.
        self.predelay_pos = 0;
        self.cascade_pos = [0; 4];
        self.fdn_write_pos = 0;
        self.fdn_lp_states = [0.0; 8];
        self.fdn_hp_states = [0.0; 8];
        self.ensemble_bank.clear_and_configure(self.sample_rate);
        for shifter in [&mut self.shimmer_oct, &mut self.shimmer_fifth] {
            shifter.write_pos = 0;
            shifter.phasor = 0.0;
            shifter.filled = 0;
            shifter.set_sample_rate(self.sample_rate);
        }
        self.shimmer_was_active = false;
        self.vowel_l = SolarisVowelFilter::default();
        self.vowel_r = SolarisVowelFilter::default();
        self.post_lp_l = 0.0;
        self.post_lp_r = 0.0;
        self.post_hp_l = 0.0;
        self.post_hp_r = 0.0;
        self.diff_phases = [0.0, 0.23, 0.47, 0.71];
        self.sample_counter = 0;
        self.random_target = [0.0; 8];
        self.random_state = [0.0; 8];
        self.random = core::array::from_fn(|i| 0x13579u32.wrapping_mul(i as u32 + 1));
        self.solina_phase_slow = 0.0;
        self.solina_phase_fast = 0.0;
        self.mod_phases = [0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875];
        self.quiet_samples = 0;
        self.dormant = true;
    }
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.sample_rate = sr;
        self.cached_params = None;

        // 15 ms Attack & 120 ms Release for the 48-Band Ensemble Envelope Followers
        self.env_att_coeff = 1.0 - (-1.0 / (0.015 * sr)).exp();
        self.env_rel_coeff = 1.0 - (-1.0 / (0.120 * sr)).exp();

        // Logarithmically distribute 48 bands from 80 Hz to 12,000 Hz across 3 Solina lanes
        self.ensemble_bank.configure(sr);

        // Scale granular shimmer grain size cleanly to current sample rate
        self.shimmer_oct.set_sample_rate(sr);
        self.shimmer_fifth.set_sample_rate(sr);

        // Scale prime FDN delay lengths to current sample rate
        let sr_scale = sr / 48000.0;
        for i in 0..FDN_CHANNELS {
            self.fdn_base_delays[i] = ((Self::PRIME_DELAYS_48K[i] as f32) * sr_scale)
                .clamp(64.0, (FDN_BUFFER_LEN - 128) as f32);
        }

        // Fixed 1.5 Hz TPT DC blocker inside feedback loop (blocks DC under saturation without
        // causing broadband low/mid RT60 droop across 48/96 kHz).
        let g_dc = (PI * 1.5 / sr).tan();
        self.coeff_dc_g1 = g_dc / (1.0 + g_dc);
    }

    #[inline(always)]
    fn process_allpass_diffuser(
        buf: &mut [f32],
        pos: &mut usize,
        delay_samples: f32,
        input: f32,
        g: f32,
        filled: u64,
    ) -> f32 {
        let mask = CASCADE_AP_LEN - 1;
        let d = delay_samples.clamp(2.0, (CASCADE_AP_LEN - 2) as f32);
        let di = d as usize;
        let read_idx = (*pos + CASCADE_AP_LEN - di) & mask;
        let frac = d - di as f32;
        let a = if filled > di as u64 {
            buf[read_idx]
        } else {
            0.0
        };
        let b = if filled > di as u64 + 1 {
            buf[(read_idx + CASCADE_AP_LEN - 1) & mask]
        } else {
            0.0
        };
        let delayed = a * (1.0 - frac) + b * frac;
        let v = input - g * delayed;
        buf[*pos] = flush_tiny(v);
        *pos = (*pos + 1) & mask;
        delayed + g * v
    }

    /// Processes a stereo sample pair `(in_l, in_r)` through the Solaris-Cloudburst Reverb.
    #[inline]
    pub fn process_stereo(
        &mut self,
        in_l: f32,
        in_r: f32,
        params: &SolarisReverbParams,
    ) -> (f32, f32) {
        let step = 1.0 / (0.005 * self.sample_rate);
        let target = if params.enabled { 1.0 } else { 0.0 };
        self.enable_gain += (target - self.enable_gain).clamp(-step, step);
        if !params.enabled
            && self.enable_gain == 0.0
            && params.bypass_tail == ReverbTailPolicy::Kill
            && !self.dormant
        {
            self.clear_history();
        }
        // At zero wet, advance the existing tail without feeding hidden new
        // excitation. A fresh dry instrument can remain dormant (#1).
        let mono_in = if params.freeze || params.wet_mix <= 0.0 {
            0.0
        } else {
            0.5 * (in_l + in_r) * self.enable_gain
        };
        if self.dormant && (mono_in == 0.0 || params.freeze) {
            return self.mix(in_l, in_r, 0.0, 0.0, params);
        }
        self.dormant = false;

        let coefficient_key = [params.high_cut_hz, params.low_cut_hz, params.decay_sec];
        if self.cached_params != Some(coefficient_key) {
            let sr = self.sample_rate;
            let hc = params.high_cut_hz.clamp(800.0, 18000.0);
            let lc = params.low_cut_hz.clamp(20.0, 1000.0);
            let decay = params.decay_sec.clamp(0.2, 60.0);

            let g_hi = (PI * hc / sr).tan();
            let g_lo = (PI * lc / sr).tan();
            self.coeff_post_lp_g1 = g_hi / (1.0 + g_hi);
            self.coeff_post_hp_g1 = g_lo / (1.0 + g_lo);

            // Jot proportional HF damping shelf inside FDN loop:
            // vanishes continuously at high_cut_hz >= 17.9 kHz for true broadband neutral RT60,
            // and scales strictly with each channel's delay length when high_cut_hz is lowered.
            let air_u = ((18000.0 - hc) / 17200.0).clamp(0.0, 1.0);
            let shelf_active = hc < 17900.0;
            self.shelf_active = shelf_active;
            self.coeff_shelf_g1 = self.coeff_post_lp_g1;
            let hi_decay = (decay * (1.0 - 0.35 * air_u)).max(0.15);
            if !shelf_active {
                self.fdn_lp_states = [0.0; FDN_CHANNELS];
            }

            for i in 0..FDN_CHANNELS {
                let tau = self.fdn_base_delays[i] / sr;
                let fb = 10.0f32.powf(-3.0 * tau / decay).min(0.99999);
                self.coeff_fb[i] = fb;
                self.coeff_shelf_alpha[i] = if shelf_active {
                    let fb_hi = 10.0f32.powf(-3.0 * tau / hi_decay).min(fb);
                    (fb_hi / fb).clamp(0.05, 1.0)
                } else {
                    1.0
                };
            }
            self.cached_params = Some(coefficient_key);
        }
        self.sample_counter = self.sample_counter.wrapping_add(1);

        // --- STAGE 1: Pre-Delay (0..500 ms) & 4-Stage Cascade Diffuser ---
        let mask_pd = MAX_PREDELAY_LEN - 1;
        self.predelay_buf[self.predelay_pos] = mono_in;
        let pd_ms = if params.predelay_sync {
            60000.0 / params.tempo_bpm.clamp(20.0, 300.0) * params.predelay_beats
        } else {
            params.pre_delay_ms
        };
        let pd_samples = ((pd_ms.clamp(0.0, 500.0) * 0.001 * self.sample_rate) as usize)
            .min(MAX_PREDELAY_LEN - 2);
        let pd_out = if self.sample_counter > pd_samples as u64 {
            self.predelay_buf[(self.predelay_pos + MAX_PREDELAY_LEN - pd_samples) & mask_pd]
        } else {
            0.0
        };
        self.predelay_pos = (self.predelay_pos + 1) & mask_pd;

        let sr_scale = self.sample_rate / 48000.0;
        let diff_g = (params.cascade_diffusion * 0.75).clamp(0.0, 0.78);
        let mut cascaded = pd_out;
        let mod_scale = params.mod_depth * 0.0007 * self.sample_rate;
        let inv_sr = 1.0 / self.sample_rate;
        for i in 0..4 {
            let mut p = self.diff_phases[i] + (0.13 + 0.03 * i as f32) * inv_sr;
            if p >= 1.0 {
                p -= 1.0;
            }
            self.diff_phases[i] = p;
            let d_len =
                (Self::CASCADE_DELAYS_48K[i] as f32) * sr_scale + fast_unit_sine(p) * mod_scale;
            cascaded = Self::process_allpass_diffuser(
                &mut self.cascade_bufs[i],
                &mut self.cascade_pos[i],
                d_len,
                cascaded,
                diff_g,
                self.sample_counter,
            );
        }

        // --- STAGE 2: 48-Band Constant-Q Harmonic Ensemble (Cloudburst) ---
        let mut ensemble_l = 0.0f32;
        let mut ensemble_r = 0.0f32;
        if params.ensemble_mode != EnsembleMode::Off && params.ensemble_blend > 1.0e-4 {
            // Advance Solina 3-phase chorus LFOs (0.6 Hz slow + 5.8 Hz fast)
            self.solina_phase_slow = wrap_01(self.solina_phase_slow + 0.6 * inv_sr);
            self.solina_phase_fast = wrap_01(self.solina_phase_fast + 5.8 * inv_sr);

            let forte_boost = match params.ensemble_mode {
                EnsembleMode::Off => 0.0,
                EnsembleMode::MezzoPiano => 0.35,
                EnsembleMode::Forte => 1.25,
            };

            // Exactly three phase pairs are shared by all 48 bands (#1), hoisted once outside loop.
            let mut freq_mods = [1.0f32; 3];
            for phase_idx in 0..3 {
                let offset = phase_idx as f32 * (1.0 / 3.0);
                let slow = fast_unit_sine(wrap_01(self.solina_phase_slow + offset));
                let fast = fast_unit_sine(wrap_01(self.solina_phase_fast + offset));
                freq_mods[phase_idx] = 1.0 + 0.0035 * slow + 0.0012 * fast;
            }
            let att = self.env_att_coeff;
            let rel = self.env_rel_coeff;
            let lane0 = self
                .ensemble_bank
                .process_lane(0, cascaded, att, rel, freq_mods[0], forte_boost);
            let lane1 = self
                .ensemble_bank
                .process_lane(1, cascaded, att, rel, freq_mods[1], forte_boost);
            let lane2 = self
                .ensemble_bank
                .process_lane(2, cascaded, att, rel, freq_mods[2], forte_boost);
            // 3-phase Solina stereo spread (weights sum to 1.0 across L+R)
            let sum_l = 0.80 * lane0 + 0.50 * lane1 + 0.20 * lane2;
            let sum_r = 0.20 * lane0 + 0.50 * lane1 + 0.80 * lane2;
            let gain = 0.22 * params.ensemble_blend;
            ensemble_l = sum_l * gain;
            ensemble_r = sum_r * gain;
        }

        let tank_excitation = if params.freeze {
            0.0
        } else {
            cascaded + ensemble_l + ensemble_r
        };

        // --- STAGE 3: 8-Channel Mutually-Prime FDN Tank with Hadamard Mixing ---
        let mut fdn_reads = [0.0f32; FDN_CHANNELS];
        let mask_fdn = FDN_BUFFER_LEN - 1;
        let mod_inc = (params.mod_speed_hz.clamp(0.05, 5.0)) * inv_sr;
        let mod_exc_samples = if params.freeze {
            0.0
        } else {
            params.mod_depth.clamp(0.0, 1.0) * 18.0 * sr_scale
        };
        let rand_slew = 0.8 * inv_sr;
        let update_rng = (self.sample_counter & 1023) == 0;

        for ch in 0..FDN_CHANNELS {
            let mut ph = self.mod_phases[ch] + mod_inc * (1.0 + 0.07 * (ch as f32));
            if ph >= 1.0 {
                ph -= 1.0;
            }
            self.mod_phases[ch] = ph;
            if update_rng {
                let mut x = self.random[ch];
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.random[ch] = x;
                self.random_target[ch] = x as f32 * (2.0 / u32::MAX as f32) - 1.0;
            }
            self.random_state[ch] += (self.random_target[ch] - self.random_state[ch]) * rand_slew;
            let total_delay = if params.freeze {
                self.fdn_base_delays[ch].round()
            } else {
                let lfo_offset =
                    (fast_unit_sine(ph) * 0.75 + self.random_state[ch] * 0.25) * mod_exc_samples;
                (self.fdn_base_delays[ch] + lfo_offset).clamp(16.0, (FDN_BUFFER_LEN - 16) as f32)
            };

            let d_int = total_delay as usize;
            let frac = total_delay - (d_int as f32);
            let r0 = (self.fdn_write_pos + FDN_BUFFER_LEN - d_int) & mask_fdn;
            let r1 = (r0 + FDN_BUFFER_LEN - 1) & mask_fdn;
            let a = if self.sample_counter > d_int as u64 {
                self.fdn_bufs[ch][r0]
            } else {
                0.0
            };
            let b = if self.sample_counter > d_int as u64 + 1 {
                self.fdn_bufs[ch][r1]
            } else {
                0.0
            };
            fdn_reads[ch] = a * (1.0 - frac) + b * frac;
        }

        // Orthogonal 8x8 Hadamard scatter matrix
        let mixed = hadamard_8x8(fdn_reads);

        // Dual Granular Pitch-Shifted Shimmer (+12 st & +7 st) inside the FDN feedback loop
        // Mixed smoothly on channel 0 with strict contraction bound |(1-g)*m0 + g*P(m0)| <= |m0|
        // so RT60 tracks decay_sec cleanly across 0.5..60 s with zero limit cycles.
        let shimmer_amount = if params.freeze {
            0.0
        } else {
            params.shimmer_amount.clamp(0.0, 0.85)
        };
        let mut feedback_vec = mixed;
        let mut shimmer_wet_tap = 0.0f32;
        if shimmer_amount > 1.0e-4 {
            self.shimmer_was_active = true;
            let shimmer_in = mixed[0];
            let shim_12 = self.shimmer_oct.process(
                shimmer_in,
                (params.shimmer_semitones.clamp(-12.0, 12.0) / 12.0).exp2(),
            );
            let shim_07 = self.shimmer_fifth.process(shimmer_in, 1.498_307_1);
            let fifth = params.shimmer_fifth_blend.clamp(0.0, 1.0);
            let u_shifted = shim_12 * (1.0 - fifth) + shim_07 * fifth;
            let g_fb = shimmer_amount * 0.28;
            feedback_vec[0] = (1.0 - g_fb) * mixed[0] + g_fb * u_shifted;
            shimmer_wet_tap = u_shifted * (0.22 * shimmer_amount);
        } else if self.shimmer_was_active {
            self.shimmer_was_active = false;
            for shifter in [&mut self.shimmer_oct, &mut self.shimmer_fifth] {
                shifter.write_pos = 0;
                shifter.phasor = 0.0;
                shifter.filled = 0;
            }
        }

        let dc_g1 = self.coeff_dc_g1;
        let shelf_g1 = self.coeff_shelf_g1;
        let shelf_active = self.shelf_active;
        let exc = tank_excitation * 0.35;

        for ch in 0..FDN_CHANNELS {
            let fb_gain = if params.freeze {
                1.0
            } else {
                // RT60 attenuation per delay line length: 10^(-3 * delay_sec / RT60)
                self.coeff_fb[ch]
            };

            let sig_in = feedback_vec[ch] * fb_gain + exc;

            let damped = if params.freeze {
                sig_in
            } else {
                // Unity-passband 1.5 Hz TPT DC guard + delay-calibrated Jot HF shelf
                let v_dc = (sig_in - self.fdn_hp_states[ch]) * dc_g1;
                let lp_dc = v_dc + self.fdn_hp_states[ch];
                self.fdn_hp_states[ch] = flush_tiny(lp_dc + v_dc);
                let after_dc = sig_in - lp_dc;
                if shelf_active {
                    let v_hi = (after_dc - self.fdn_lp_states[ch]) * shelf_g1;
                    let lp_hi = v_hi + self.fdn_lp_states[ch];
                    self.fdn_lp_states[ch] = flush_tiny(lp_hi + v_hi);
                    let alpha = self.coeff_shelf_alpha[ch];
                    flush_tiny(alpha * after_dc + (1.0 - alpha) * lp_hi)
                } else {
                    flush_tiny(after_dc)
                }
            };

            // Soft tank saturation preventing runaway energy at extreme drive/shimmer
            self.fdn_bufs[ch][self.fdn_write_pos] = if params.freeze {
                damped.clamp(-4.0, 4.0)
            } else {
                flush_tiny(crate::vcf_cem3320::fast_tanh(damped * 0.85) / 0.85)
            };
        }

        self.fdn_write_pos = (self.fdn_write_pos + 1) & mask_fdn;

        // Stereo decorrelated multi-tap readout from the 8 FDN lines + stereo ensemble halo + shimmer tap
        let raw_tank_l = 0.45 * (fdn_reads[0] - fdn_reads[2] + fdn_reads[4] - fdn_reads[6])
            + ensemble_l
            + shimmer_wet_tap;
        let raw_tank_r = 0.45 * (fdn_reads[1] - fdn_reads[3] + fdn_reads[5] - fdn_reads[7])
            + ensemble_r
            - shimmer_wet_tap;

        // Stage 3b: Continuous 5-Vowel Formant Filter on the stereo wet readout (never collapsing tank RT60)
        let (vow_l, vow_r) = if params.vowel_mix > 1.0e-4 {
            let vm = params.vowel_mix.clamp(0.0, 1.0);
            let fl = self.vowel_l.process(raw_tank_l, params.vowel_morph, self.sample_rate);
            let fr = self.vowel_r.process(raw_tank_r, params.vowel_morph, self.sample_rate);
            // 1.65x formant gain keeps peak resonator gain (1.65/1.85 = 0.892) strictly passive (<1.0)
            // while maintaining clear vowel spectral contour against diffuse wet energy.
            let dry_v = 1.0 - vm;
            let wet_v = vm * 1.65;
            (
                raw_tank_l * dry_v + fl * wet_v,
                raw_tank_r * dry_v + fr * wet_v,
            )
        } else {
            (raw_tank_l, raw_tank_r)
        };

        // Post-tank tonal High-Cut & Low-Cut TPT filters on stereo wet bus
        let (wet_l, wet_r) = if params.freeze {
            (vow_l, vow_r)
        } else {
            let post_hp_g1 = self.coeff_post_hp_g1;
            let v_hpl = (vow_l - self.post_hp_l) * post_hp_g1;
            let lp_hpl = v_hpl + self.post_hp_l;
            self.post_hp_l = flush_tiny(lp_hpl + v_hpl);
            let hp_l = vow_l - lp_hpl;

            let v_hpr = (vow_r - self.post_hp_r) * post_hp_g1;
            let lp_hpr = v_hpr + self.post_hp_r;
            self.post_hp_r = flush_tiny(lp_hpr + v_hpr);
            let hp_r = vow_r - lp_hpr;

            let post_lp_g1 = self.coeff_post_lp_g1;
            let v_lpl = (hp_l - self.post_lp_l) * post_lp_g1;
            let lp_l = flush_tiny(v_lpl + self.post_lp_l);
            self.post_lp_l = flush_tiny(lp_l + v_lpl);

            let v_lpr = (hp_r - self.post_lp_r) * post_lp_g1;
            let lp_r = flush_tiny(v_lpr + self.post_lp_r);
            self.post_lp_r = flush_tiny(lp_r + v_lpr);
            (lp_l, lp_r)
        };

        let activity = mono_in
            .abs()
            .max(cascaded.abs())
            .max(ensemble_l.abs())
            .max(ensemble_r.abs())
            .max(fdn_reads.iter().fold(0.0f32, |a, x| a.max(x.abs())))
            .max(wet_l.abs())
            .max(wet_r.abs())
            .max(self.post_lp_l.abs())
            .max(self.post_lp_r.abs())
            .max(self.post_hp_l.abs())
            .max(self.post_hp_r.abs());
        if !params.freeze && activity < 1.0e-20 {
            self.quiet_samples += 1;
            // Include a full longest storage horizon; silence before predelay
            // or cancellation of a single output is not evidence of an empty tank.
            if self.quiet_samples > MAX_PREDELAY_LEN + CASCADE_AP_LEN * 4 + SHIMMER_BUF_LEN {
                self.clear_history();
            }
        } else {
            self.quiet_samples = 0;
        }
        self.mix(in_l, in_r, wet_l, wet_r, params)
    }

    fn mix(&self, l: f32, r: f32, wet_l: f32, wet_r: f32, p: &SolarisReverbParams) -> (f32, f32) {
        let wet = p.wet_mix.clamp(0.0, 1.0);
        let wet_gain = wet
            * if p.bypass_tail == ReverbTailPolicy::Kill {
                self.enable_gain
            } else {
                1.0
            };
        let dry_gain = 1.0 - wet * self.enable_gain;
        (
            flush_tiny(l * dry_gain + wet_l * wet_gain),
            flush_tiny(r * dry_gain + wet_r * wet_gain),
        )
    }
}

/// -400 dBFS floor prevents gradual underflow in recursive FX states.
#[inline(always)]
fn flush_tiny(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}
