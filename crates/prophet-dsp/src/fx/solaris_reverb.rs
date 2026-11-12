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

use core::f32::consts::{PI, TAU};

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
pub enum ReverbTailPolicy { Spill, Kill }

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

/// Single Band of the 48-Band Constant-Q Cloudburst Harmonic Ensemble Analyzer & Exciter.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnsembleBandChannel {
    /// TPT SVF states for Constant-Q bandpass analysis filter
    ic1eq: f32,
    ic2eq: f32,
    g: f32,
    k_inv_q: f32,
    /// Envelope follower state (15 ms attack, 120 ms release)
    env: f32,
    /// Instantaneous phase of fundamental & 2nd/3rd harmonic partial oscillators
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
        self.g = (PI * fc / sr).tan();
        self.k_inv_q = 1.0 / 11.5;
        self.dt_h2 = 2.0 * fc / sr;
        self.dt_h3 = 3.0 * fc / sr;
    }

    #[inline(always)]
    pub fn process(
        &mut self,
        input: f32,
        att_coeff: f32,
        rel_coeff: f32,
        solina_mod_slow: f32,
        solina_mod_fast: f32,
        forte_boost: f32,
    ) -> f32 {
        // 1. Constant-Q TPT Bandpass filter extraction
        let a1 = 1.0 / (1.0 + self.g * (self.g + self.k_inv_q));
        let a2 = self.g * a1;
        let v3 = input - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + self.g * v1;
        self.ic1eq = flush_tiny(2.0 * v1 - self.ic1eq);
        self.ic2eq = flush_tiny(2.0 * v2 - self.ic2eq);
        let bp = v1 * self.k_inv_q;

        // 2. Dynamic Envelope Follower (15 ms attack, 120 ms release)
        let rect = bp.abs();
        if rect > self.env {
            self.env += (rect - self.env) * att_coeff;
        } else {
            self.env += (rect - self.env) * rel_coeff;
        }

        self.env = flush_tiny(self.env);
        // 3. Harmonic Partial Synthesis (2nd and 3rd harmonics with Solina 3-phase modulation)
        let freq_mod = 1.0 + 0.0035 * solina_mod_slow + 0.0012 * solina_mod_fast;
        self.phase_h2 = (self.phase_h2 + self.dt_h2 * freq_mod).rem_euclid(1.0);
        self.phase_h3 = (self.phase_h3 + self.dt_h3 * freq_mod).rem_euclid(1.0);

        // Fast parabolic sine approximation on [0, 1)
        let s2 = fast_unit_sine(self.phase_h2);
        let s3 = fast_unit_sine(self.phase_h3);

        // Non-linear soft expansion in Forte mode
        let env_shaped = self.env * (1.0 + forte_boost * (self.env * 2.5).min(1.5));
        env_shaped
            * (if self.dt_h2 * freq_mod < 0.45 {
                0.72 * s2
            } else {
                0.0
            } + if self.dt_h3 * freq_mod < 0.45 {
                0.28 * s3
            } else {
                0.0
            })
    }
}

#[inline(always)]
fn fast_unit_sine(phase_0_to_1: f32) -> f32 {
    let x = phase_0_to_1 * 2.0 - 1.0;
    let y = 4.0 * x * (1.0 - x.abs());
    0.225 * (y * y.abs() - y) + y
}

/// Dual-Head Hann-Windowed Granular Pitch Shifter (+12 Semitones Octave & +7 Semitones Fifth).
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

    #[inline(always)]
    fn read_linear(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, (SHIMMER_BUF_LEN - 4) as f32);
        let d_int = d as usize;
        let frac = d - (d_int as f32);
        let mask = SHIMMER_BUF_LEN - 1;
        let idx0 = (self.write_pos + SHIMMER_BUF_LEN - d_int) & mask;
        let idx1 = (idx0 + SHIMMER_BUF_LEN - 1) & mask;
        let a = if self.filled >= d_int as u64 { self.buf[idx0] } else { 0.0 };
        let b = if self.filled > d_int as u64 { self.buf[idx1] } else { 0.0 };
        a * (1.0 - frac) + b * frac
    }

    /// Pitch-shifts `input` by `pitch_ratio` (e.g. `2.0` for +12 st, `1.4983` for +7 st).
    #[inline(always)]
    pub fn process(&mut self, input: f32, pitch_ratio: f32) -> f32 {
        self.buf[self.write_pos] = flush_tiny(input);
        self.write_pos = (self.write_pos + 1) & (SHIMMER_BUF_LEN - 1);
        self.filled = self.filled.saturating_add(1);

        let rate = (1.0 - pitch_ratio) / self.grain_size;
        self.phasor = (self.phasor + rate).rem_euclid(1.0);
        let phasor_b = (self.phasor + 0.5).rem_euclid(1.0);

        let delay_a = self.phasor * self.grain_size + 64.0;
        let delay_b = phasor_b * self.grain_size + 64.0;

        // Complementary Hann crossfade windows: sin^2(pi * p) + sin^2(pi * (p + 0.5)) == 1.0
        let win_a_root = (PI * self.phasor).sin();
        let win_b_root = (PI * phasor_b).sin();
        let win_a = win_a_root * win_a_root;
        let win_b = win_b_root * win_b_root;

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

    #[inline(always)]
    pub fn process(&mut self, input: f32, vowel_morph: f32, sample_rate: f32) -> f32 {
        let pos = (vowel_morph.clamp(0.0, 1.0)) * 4.0;
        let idx0 = (pos.floor() as usize).min(4);
        let idx1 = (idx0 + 1).min(4);
        let frac = pos - (idx0 as f32);

        if self.cached_sr != sample_rate || self.cached_morph != vowel_morph {
            for i in 0..3 {
                let freq = Self::FORMANTS[idx0][i] * (1.0 - frac) + Self::FORMANTS[idx1][i] * frac;
                self.gs[i] = (PI * freq.clamp(150.0, 0.42 * sample_rate) / sample_rate).tan();
            }
            self.cached_sr = sample_rate;
            self.cached_morph = vowel_morph;
        }
        let gains = [1.0, 0.55, 0.30];
        let qs = [7.5, 9.0, 11.0];
        let mut sum = 0.0;

        for f_idx in 0..3 {
            let freq =
                Self::FORMANTS[idx0][f_idx] * (1.0 - frac) + Self::FORMANTS[idx1][f_idx] * frac;
            let _ = freq;
            let g = self.gs[f_idx];
            let k = 1.0 / qs[f_idx];
            let a1 = 1.0 / (1.0 + g * (g + k));
            let a2 = g * a1;
            let v3 = input - self.ic2[f_idx];
            let v1 = a1 * self.ic1[f_idx] + a2 * v3;
            let v2 = self.ic2[f_idx] + g * v1;
            self.ic1[f_idx] = flush_tiny(2.0 * v1 - self.ic1[f_idx]);
            self.ic2[f_idx] = flush_tiny(2.0 * v2 - self.ic2[f_idx]);
            sum += (v1 * k) * gains[f_idx];
        }
        sum / 1.85
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
    ensemble_bands: [EnsembleBandChannel; NUM_ENSEMBLE_BANDS],
    env_att_coeff: f32,
    env_rel_coeff: f32,
    solina_phase_slow: f32,
    solina_phase_fast: f32,

    // Stage 3: 8-Channel Hadamard FDN Tank + Shimmer + Vowel Formant Filter
    fdn_bufs: [Box<[f32]>; FDN_CHANNELS],
    fdn_write_pos: usize,
    fdn_base_delays: [f32; FDN_CHANNELS],
    fdn_lp_states: [f32; FDN_CHANNELS],
    fdn_hp_states: [f32; FDN_CHANNELS],
    mod_phases: [f32; FDN_CHANNELS],

    shimmer_oct: GranularPitchShifter,
    shimmer_fifth: GranularPitchShifter,
    vowel_l: SolarisVowelFilter,
    vowel_r: SolarisVowelFilter,
    vowel_feedback: [SolarisVowelFilter; 8],
    diff_phases: [f32; 4],
    random: [u32; 8],
    random_target: [f32; 8],
    random_state: [f32; 8],
    sample_counter: u64,
    cached_params: Option<[f32; 3]>,
    coefficients: [f32; 10],
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
            ensemble_bands: [EnsembleBandChannel::default(); NUM_ENSEMBLE_BANDS],
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
            vowel_l: SolarisVowelFilter::default(),
            vowel_r: SolarisVowelFilter::default(),
            vowel_feedback: [SolarisVowelFilter::default(); 8],
            diff_phases: [0.0, 0.23, 0.47, 0.71],
            random: std::array::from_fn(|i| 0x13579u32.wrapping_mul(i as u32 + 1)),
            random_target: [0.0; 8],
            random_state: [0.0; 8],
            sample_counter: 0,
            cached_params: None,
            coefficients: [0.0; 10],
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

    pub fn is_dormant(&self) -> bool { self.dormant }
    pub fn enable_gain(&self) -> f32 { self.enable_gain }

    fn clear_history(&mut self) {
        // Read guards invalidate old storage until overwritten. No 1.7 MB
        // memset, allocation or delay-history scan in the callback.
        self.predelay_pos = 0;
        self.cascade_pos = [0; 4];
        self.fdn_write_pos = 0;
        self.fdn_lp_states = [0.0; 8];
        self.fdn_hp_states = [0.0; 8];
        self.ensemble_bands = [EnsembleBandChannel::default(); 48];
        for shifter in [&mut self.shimmer_oct, &mut self.shimmer_fifth] {
            shifter.write_pos = 0;
            shifter.phasor = 0.0;
            shifter.filled = 0;
        }
        self.vowel_l = SolarisVowelFilter::default();
        self.vowel_r = SolarisVowelFilter::default();
        self.vowel_feedback = [SolarisVowelFilter::default(); 8];
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
        let ratio = 12000.0f32 / 80.0f32;
        for (k, band) in self.ensemble_bands.iter_mut().enumerate() {
            band.configure(self.sample_rate, 80.0 * ratio.powf(k as f32 / 47.0));
        }
    }
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.sample_rate = sr;
        self.cached_params = None;

        // 15 ms Attack & 120 ms Release for the 48-Band Ensemble Envelope Followers
        self.env_att_coeff = 1.0 - (-1.0 / (0.015 * sr)).exp();
        self.env_rel_coeff = 1.0 - (-1.0 / (0.120 * sr)).exp();

        // Logarithmically distribute 48 bands from 80 Hz to 12,000 Hz:
        // f_k = 80 * (12000 / 80)^(k / 47)
        let ratio = 12000.0f32 / 80.0f32;
        for k in 0..NUM_ENSEMBLE_BANDS {
            let frac = (k as f32) / ((NUM_ENSEMBLE_BANDS - 1) as f32);
            let fc = 80.0 * ratio.powf(frac);
            self.ensemble_bands[k].configure(sr, fc);
        }

        // Scale prime FDN delay lengths to current sample rate
        let sr_scale = sr / 48000.0;
        for i in 0..FDN_CHANNELS {
            self.fdn_base_delays[i] = ((Self::PRIME_DELAYS_48K[i] as f32) * sr_scale)
                .clamp(64.0, (FDN_BUFFER_LEN - 128) as f32);
        }
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
        let a = if filled > di as u64 { buf[read_idx] } else { 0.0 };
        let b = if filled > di as u64 + 1 { buf[(read_idx + CASCADE_AP_LEN - 1) & mask] } else { 0.0 };
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
        if !params.enabled && self.enable_gain == 0.0
            && params.bypass_tail == ReverbTailPolicy::Kill && !self.dormant {
            self.clear_history();
        }
        let mono_in = 0.5 * (in_l + in_r) * self.enable_gain;
        if self.dormant && (mono_in == 0.0 || params.freeze) {
            return self.mix(in_l, in_r, 0.0, 0.0, params);
        }
        self.dormant = false;

        let coefficient_key = [params.high_cut_hz, params.low_cut_hz, params.decay_sec];
        if self.cached_params != Some(coefficient_key) {
            self.coefficients[0] =
                (-TAU * params.high_cut_hz.clamp(800.0, 18000.0) / self.sample_rate).exp();
            self.coefficients[1] =
                (-TAU * params.low_cut_hz.clamp(20.0, 1000.0) / self.sample_rate).exp();
            for i in 0..8 {
                self.coefficients[i + 2] = 10.0f32
                    .powf(
                        -3.0 * self.fdn_base_delays[i]
                            / self.sample_rate
                            / params.decay_sec.clamp(0.2, 60.0),
                    )
                    .min(0.9985);
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
        } else { 0.0 };
        self.predelay_pos = (self.predelay_pos + 1) & mask_pd;

        let sr_scale = self.sample_rate / 48000.0;
        let diff_g = (params.cascade_diffusion * 0.75).clamp(0.0, 0.78);
        let mut cascaded = pd_out;
        for i in 0..4 {
            self.diff_phases[i] =
                (self.diff_phases[i] + (0.13 + 0.03 * i as f32) / self.sample_rate).fract();
            let d_len = (Self::CASCADE_DELAYS_48K[i] as f32) * sr_scale
                + (TAU * self.diff_phases[i]).sin() * params.mod_depth * 0.0007 * self.sample_rate;
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
        let mut ensemble_sig = 0.0f32;
        if params.ensemble_mode != EnsembleMode::Off && params.ensemble_blend > 1.0e-4 {
            // Advance Solina 3-phase chorus LFOs (0.6 Hz slow + 5.8 Hz fast)
            self.solina_phase_slow =
                (self.solina_phase_slow + 0.6 / self.sample_rate).rem_euclid(1.0);
            self.solina_phase_fast =
                (self.solina_phase_fast + 5.8 / self.sample_rate).rem_euclid(1.0);

            let forte_boost = match params.ensemble_mode {
                EnsembleMode::Off => 0.0,
                EnsembleMode::MezzoPiano => 0.35,
                EnsembleMode::Forte => 1.25,
            };

            // Exactly three phase pairs are shared by all 48 bands (#1).
            let slow: [f32; 3] = core::array::from_fn(|i|
                (TAU * (self.solina_phase_slow + i as f32 / 3.0)).sin());
            let fast: [f32; 3] = core::array::from_fn(|i|
                (TAU * (self.solina_phase_fast + i as f32 / 3.0)).sin());
            let mut band_sum = 0.0f32;
            for (i, band) in self.ensemble_bands.iter_mut().enumerate() {
                let mod_slow = slow[i % 3];
                let mod_fast = fast[i % 3];
                band_sum += band.process(
                    cascaded,
                    self.env_att_coeff,
                    self.env_rel_coeff,
                    mod_slow,
                    mod_fast,
                    forte_boost,
                );
            }
            ensemble_sig = band_sum * 0.22 * params.ensemble_blend;
        }

        let tank_excitation = if params.freeze {
            0.0
        } else {
            cascaded + ensemble_sig
        };

        // --- STAGE 3: 8-Channel Mutually-Prime FDN Tank with Hadamard Mixing ---
        let mut fdn_reads = [0.0f32; FDN_CHANNELS];
        let mask_fdn = FDN_BUFFER_LEN - 1;
        let mod_inc = (params.mod_speed_hz.clamp(0.05, 5.0)) / self.sample_rate;
        let mod_exc_samples = if params.freeze {
            0.0
        } else {
            params.mod_depth.clamp(0.0, 1.0) * 18.0 * sr_scale
        };

        for ch in 0..FDN_CHANNELS {
            self.mod_phases[ch] =
                (self.mod_phases[ch] + mod_inc * (1.0 + 0.07 * (ch as f32))).rem_euclid(1.0);
            if self.sample_counter % 1024 == 0 {
                let mut x = self.random[ch];
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                self.random[ch] = x;
                self.random_target[ch] = x as f32 / u32::MAX as f32 * 2.0 - 1.0;
            }
            self.random_state[ch] +=
                (self.random_target[ch] - self.random_state[ch]) * (0.8 / self.sample_rate);
            let lfo_offset = ((TAU * self.mod_phases[ch]).sin() * 0.75
                + self.random_state[ch] * 0.25)
                * mod_exc_samples;
            let total_delay = if params.freeze {
                self.fdn_base_delays[ch].round()
            } else {
                (self.fdn_base_delays[ch] + lfo_offset).clamp(16.0, (FDN_BUFFER_LEN - 16) as f32)
            };

            let d_int = total_delay as usize;
            let frac = total_delay - (d_int as f32);
            let r0 = (self.fdn_write_pos + FDN_BUFFER_LEN - d_int) & mask_fdn;
            let r1 = (r0 + FDN_BUFFER_LEN - 1) & mask_fdn;
            let a = if self.sample_counter > d_int as u64 { self.fdn_bufs[ch][r0] } else { 0.0 };
            let b = if self.sample_counter > d_int as u64 + 1 { self.fdn_bufs[ch][r1] } else { 0.0 };
            fdn_reads[ch] = a * (1.0 - frac) + b * frac;
        }

        // Orthogonal 8x8 Hadamard scatter matrix
        let mixed = hadamard_8x8(fdn_reads);

        // Dual Granular Pitch-Shifted Shimmer (+12 st & +7 st) inside the FDN feedback loop
        let shimmer_in = 0.5 * (mixed[0] + mixed[4]);
        let shim_12 = self.shimmer_oct.process(
            shimmer_in,
            (params.shimmer_semitones.clamp(-12.0, 12.0) / 12.0).exp2(),
        );
        let shim_07 = self.shimmer_fifth.process(shimmer_in, 1.498_307_1);
        let shimmer_amount = if params.freeze {
            0.0
        } else {
            params.shimmer_amount.clamp(0.0, 0.85)
        };
        let shimmer_fb = (shim_12 * (1.0 - params.shimmer_fifth_blend)
            + shim_07 * params.shimmer_fifth_blend)
            * shimmer_amount;

        // High-Cut & Low-Cut Damping coefficients
        let lp_coeff = self.coefficients[0];
        let hp_coeff = self.coefficients[1];

        for ch in 0..FDN_CHANNELS {
            let fb_gain = if params.freeze {
                1.0
            } else {
                // RT60 attenuation per delay line length: 10^(-3 * delay_sec / RT60)
                self.coefficients[ch + 2]
            };

            let sig_in = (mixed[ch] * (1.0 - shimmer_amount)
                + if ch % 2 == 0 {
                    shimmer_fb * 0.25
                } else {
                    -shimmer_fb * 0.25
                })
                * fb_gain
                + tank_excitation * 0.35;

            let sig_in = if params.vowel_mix > 0.0001 && !params.freeze {
                let v =
                    self.vowel_feedback[ch].process(sig_in, params.vowel_morph, self.sample_rate);
                sig_in * (1.0 - params.vowel_mix) + v * params.vowel_mix
            } else {
                sig_in
            };
            let damped = if params.freeze {
                sig_in
            } else {
                // 1-pole High-Cut + Low-Cut damping inside loop
                self.fdn_lp_states[ch] =
                    flush_tiny(sig_in * (1.0 - lp_coeff) + self.fdn_lp_states[ch] * lp_coeff);
                let lp_out = self.fdn_lp_states[ch];
                self.fdn_hp_states[ch] =
                    flush_tiny(lp_out * (1.0 - hp_coeff) + self.fdn_hp_states[ch] * hp_coeff);
                lp_out - self.fdn_hp_states[ch]
            };

            // Soft tank saturation preventing runaway energy at high shimmer
            self.fdn_bufs[ch][self.fdn_write_pos] = if params.freeze {
                damped.clamp(-4.0, 4.0)
            } else {
                flush_tiny(crate::vcf_cem3320::fast_tanh(damped * 0.85) / 0.85)
            };
        }

        self.fdn_write_pos = (self.fdn_write_pos + 1) & mask_fdn;

        // Stereo decorrelated multi-tap readout from the 8 FDN lines
        let raw_tank_l =
            0.45 * (fdn_reads[0] - fdn_reads[2] + fdn_reads[4] - fdn_reads[6]) + ensemble_sig * 0.5;
        let raw_tank_r =
            0.45 * (fdn_reads[1] - fdn_reads[3] + fdn_reads[5] - fdn_reads[7]) + ensemble_sig * 0.5;

        let (wet_l, wet_r) = (raw_tank_l, raw_tank_r);

        let activity = mono_in.abs().max(cascaded.abs()).max(ensemble_sig.abs())
            .max(fdn_reads.iter().fold(0.0f32, |a, x| a.max(x.abs())))
            .max(wet_l.abs()).max(wet_r.abs());
        if !params.freeze && activity < 1.0e-20 {
            self.quiet_samples += 1;
            // Include a full longest storage horizon; silence before predelay
            // or cancellation of a single output is not evidence of an empty tank.
            if self.quiet_samples > MAX_PREDELAY_LEN + CASCADE_AP_LEN * 4 + SHIMMER_BUF_LEN {
                self.clear_history();
            }
        } else { self.quiet_samples = 0; }
        self.mix(in_l, in_r, wet_l, wet_r, params)
    }

    fn mix(&self, l: f32, r: f32, wet_l: f32, wet_r: f32, p: &SolarisReverbParams) -> (f32, f32) {
        let wet = p.wet_mix.clamp(0.0, 1.0);
        let wet_gain = wet * if p.bypass_tail == ReverbTailPolicy::Kill { self.enable_gain } else { 1.0 };
        let dry_gain = 1.0 - wet * self.enable_gain;
        (flush_tiny(l * dry_gain + wet_l * wet_gain), flush_tiny(r * dry_gain + wet_r * wet_gain))
    }
}

/// -400 dBFS floor prevents gradual underflow in recursive FX states.
#[inline(always)]
fn flush_tiny(x: f32) -> f32 { if x.abs() < 1.0e-20 { 0.0 } else { x } }
