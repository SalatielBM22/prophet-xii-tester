// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Roland Juno-106 Dual Panasonic MN3009 BBD Stereo Chorus (I, II, I+II)
// Mathematical grounding:
// - YKChorus / TAL-NoiseMaker Chorus Engine (https://github.com/SpotlightKid/ykchorus)
// - Dual 256-stage MN3009 Bucket Brigade Device delay lines with 4-point Cubic Hermite interpolation
// - Pre/De-emphasis 4-pole Butterworth Low-Pass anti-aliasing filters (~8.0 kHz)
// - Modes: I (0.513 Hz, 1.66..5.35 ms), II (0.863 Hz, 1.66..5.35 ms), I+II (8.65 Hz, 3.5 +/- 0.2 ms)

use crate::osc_cem3340::ProphetNoiseGen;
use core::f32::consts::PI;

/// Pre-allocated BBD delay buffer size per channel (`2048` samples supports >10.6 ms at 192 kHz).
pub const MAX_BBD_BUFFER_LEN: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JunoChorusMode {
    Off,
    ModeI,
    ModeII,
    ModeIPlusII,
}

/// 4-Pole Cascaded TPT Butterworth Low-Pass Filter for BBD Anti-Aliasing & Reconstruction (~8 kHz).
#[derive(Clone, Copy, Debug, Default)]
pub struct Butterworth4PoleLp {
    s1: f32,
    s2: f32,
    s3: f32,
    s4: f32,
    g: f32,
    r1: f32,
    r2: f32,
}

impl Butterworth4PoleLp {
    pub fn new(sample_rate: f32, cutoff_hz: f32) -> Self {
        let mut flt = Self::default();
        flt.set_cutoff(sample_rate, cutoff_hz);
        flt
    }

    #[inline]
    pub fn set_cutoff(&mut self, sample_rate: f32, cutoff_hz: f32) {
        let sr = sample_rate.max(8000.0);
        let fc = cutoff_hz.clamp(1000.0, 0.45 * sr);
        self.g = (PI * fc / sr).tan();
        // 4th-order Butterworth damping factors: 2*cos(pi/8) = 1.847759, 2*cos(3*pi/8) = 0.765367
        self.r1 = 0.765_366_85;
        self.r2 = 1.847_759_1;
    }

    #[inline(always)]
    pub fn process(&mut self, x: f32) -> f32 {
        let g = self.g;
        // Biquad TPT Section 1
        let h1 = 1.0 / (1.0 + self.r1 * g + g * g);
        let hp1 = (x - (self.r1 + g) * self.s1 - self.s2) * h1;
        let bp1 = g * hp1 + self.s1;
        self.s1 = g * hp1 + bp1;
        let lp1 = g * bp1 + self.s2;
        self.s2 = g * bp1 + lp1;

        // Biquad TPT Section 2
        let h2 = 1.0 / (1.0 + self.r2 * g + g * g);
        let hp2 = (lp1 - (self.r2 + g) * self.s3 - self.s4) * h2;
        let bp2 = g * hp2 + self.s3;
        self.s3 = g * hp2 + bp2;
        let lp2 = g * bp2 + self.s4;
        self.s4 = g * bp2 + lp2;

        for s in [&mut self.s1, &mut self.s2, &mut self.s3, &mut self.s4] {
            if s.abs() < 1e-20 {
                *s = 0.;
            }
        }
        if lp2.abs() < 1e-20 {
            0.
        } else {
            lp2
        }
    }
}

/// 4-point Cubic Hermite Fractional Delay Interpolation.
#[inline(always)]
pub fn hermite_interp_4pt(ym1: f32, y0: f32, y1: f32, y2: f32, frac: f32) -> f32 {
    let c0 = y0;
    let c1 = 0.5 * (y1 - ym1);
    let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
    let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);
    ((c3 * frac + c2) * frac + c1) * frac + c0
}

/// Roland Juno-106 Stereo BBD Chorus Processor.
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct Juno106BbdChorus {
    pub mode: JunoChorusMode,
    pub wet_mix: f32,
    /// Optional BBD analog hiss level in linear amplitude (0.0 = -inf dB, 0.0018 = -55 dB)
    pub bbd_noise_level: f32,
    pub sample_rate: f32,
    lfo_phase: f32,
    delay_l: [f32; MAX_BBD_BUFFER_LEN],
    delay_r: [f32; MAX_BBD_BUFFER_LEN],
    write_pos: usize,
    pre_filter_l: Butterworth4PoleLp,
    pre_filter_r: Butterworth4PoleLp,
    post_filter_l: Butterworth4PoleLp,
    post_filter_r: Butterworth4PoleLp,
    emphasis_lp: [f32; 2],
    deemphasis: [f32; 2],
    dc_prev: [f32; 2],
    dc_out: [f32; 2],
    emphasis_coeff: f32,
    dc_coeff: f32,
    noise_l: ProphetNoiseGen,
    noise_r: ProphetNoiseGen,
}

impl Juno106BbdChorus {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.max(8000.0);
        Self {
            mode: JunoChorusMode::ModeI,
            wet_mix: 0.5,
            bbd_noise_level: 0.0,
            sample_rate: sr,
            lfo_phase: 0.0,
            delay_l: [0.0; MAX_BBD_BUFFER_LEN],
            delay_r: [0.0; MAX_BBD_BUFFER_LEN],
            write_pos: 0,
            pre_filter_l: Butterworth4PoleLp::new(sr, 8200.0),
            pre_filter_r: Butterworth4PoleLp::new(sr, 8200.0),
            post_filter_l: Butterworth4PoleLp::new(sr, 7800.0),
            post_filter_r: Butterworth4PoleLp::new(sr, 7800.0),
            emphasis_lp: [0.0; 2],
            deemphasis: [0.0; 2],
            dc_prev: [0.0; 2],
            dc_out: [0.0; 2],
            emphasis_coeff: 1.0 - (-2.0 * PI * 1200.0 / sr).exp(),
            dc_coeff: (-2.0 * PI * 10.0 / sr).exp(),
            noise_l: ProphetNoiseGen::new(0xA5A5_1234),
            noise_r: ProphetNoiseGen::new(0x5A5A_8765),
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sr = sample_rate.max(8000.0);
        self.sample_rate = sr;
        self.emphasis_coeff = 1.0 - (-2.0 * PI * 1200.0 / sr).exp();
        self.dc_coeff = (-2.0 * PI * 10.0 / sr).exp();
        self.pre_filter_l.set_cutoff(sr, 8200.0);
        self.pre_filter_r.set_cutoff(sr, 8200.0);
        self.post_filter_l.set_cutoff(sr, 7800.0);
        self.post_filter_r.set_cutoff(sr, 7800.0);
    }

    pub fn reset(&mut self) {
        self.delay_l.fill(0.0);
        self.delay_r.fill(0.0);
        self.write_pos = 0;
    }

    #[inline(always)]
    fn read_hermite(buf: &[f32; MAX_BBD_BUFFER_LEN], write_pos: usize, delay_samples: f32) -> f32 {
        let d = delay_samples.clamp(2.0, (MAX_BBD_BUFFER_LEN - 4) as f32);
        let d_int = d.floor() as usize;
        let frac = d - (d_int as f32);

        let mask = MAX_BBD_BUFFER_LEN - 1;
        let idx0 = (write_pos + MAX_BBD_BUFFER_LEN - d_int) & mask;
        let idxm1 = (idx0 + 1) & mask;
        let idx1 = (idx0 + MAX_BBD_BUFFER_LEN - 1) & mask;
        let idx2 = (idx0 + MAX_BBD_BUFFER_LEN - 2) & mask;

        hermite_interp_4pt(buf[idxm1], buf[idx0], buf[idx1], buf[idx2], frac)
    }

    /// Asymmetric BBD MOSFET bucket charge-transfer saturation.
    #[inline(always)]
    fn bbd_saturate(x: f32) -> f32 {
        let xc = x.clamp(-1.8, 1.8);
        xc - 0.085 * xc * xc - 0.095 * xc * xc * xc
    }

    /// Processes a stereo sample pair `(left, right)` through the Juno-106 Dual MN3009 BBD Chorus.
    #[inline]
    pub fn process_stereo(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.mode == JunoChorusMode::Off || self.wet_mix <= 1.0e-5 {
            return (in_l, in_r);
        }

        // Select authentic Juno-106 LFO rate and delay sweep window
        let (lfo_freq_hz, min_delay_ms, max_delay_ms) = match self.mode {
            JunoChorusMode::Off => (0.513, 1.66, 5.35),
            JunoChorusMode::ModeI => (0.513, 1.66, 5.35),
            JunoChorusMode::ModeII => (0.863, 1.66, 5.35),
            JunoChorusMode::ModeIPlusII => (8.65, 3.30, 3.70), // Rotary ensemble resistor switch
        };

        let dt = lfo_freq_hz / self.sample_rate;
        let p = self.lfo_phase;
        self.lfo_phase = (p + dt).rem_euclid(1.0);

        // Triangle LFO in [0.0, 1.0] for Left, inverted 180 degrees (1.0 - tri_l) for Right
        let tri_l = if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p };
        let tri_r = if self.mode == JunoChorusMode::ModeIPlusII {
            // In Mode I+II, both channels share the fast vibrato with slight quadrature offset
            let p_r = (p + 0.25).rem_euclid(1.0);
            if p_r < 0.5 {
                2.0 * p_r
            } else {
                2.0 - 2.0 * p_r
            }
        } else {
            1.0 - tri_l
        };

        let delay_ms_l = min_delay_ms + tri_l * (max_delay_ms - min_delay_ms);
        let delay_ms_r = min_delay_ms + tri_r * (max_delay_ms - min_delay_ms);

        let delay_samp_l = delay_ms_l * 0.001 * self.sample_rate;
        let delay_samp_r = delay_ms_r * 0.001 * self.sample_rate;

        // 1. Pre-filtering (Anti-Aliasing before MN3009 BBD sampling) + Asymmetric BBD Saturation
        let c = self.emphasis_coeff;
        self.emphasis_lp[0] += c * (in_l - self.emphasis_lp[0]);
        self.emphasis_lp[1] += c * (in_r - self.emphasis_lp[1]);
        let pre_l = Self::bbd_saturate(
            self.pre_filter_l
                .process(in_l + 0.5 * (in_l - self.emphasis_lp[0])),
        );
        let pre_r = Self::bbd_saturate(
            self.pre_filter_r
                .process(in_r + 0.5 * (in_r - self.emphasis_lp[1])),
        );

        self.delay_l[self.write_pos] = pre_l;
        self.delay_r[self.write_pos] = pre_r;

        // 2. Interpolated BBD Readout + MN3009 Pink Hiss Floor
        let raw_wet_l = Self::read_hermite(&self.delay_l, self.write_pos, delay_samp_l)
            + self.noise_l.next_pink() * self.bbd_noise_level;
        let raw_wet_r = Self::read_hermite(&self.delay_r, self.write_pos, delay_samp_r)
            + self.noise_r.next_pink() * self.bbd_noise_level;

        self.write_pos = (self.write_pos + 1) & (MAX_BBD_BUFFER_LEN - 1);

        // 3. Post-filtering (Reconstruction Low-Pass at 7.8 kHz)
        let mut wet = [
            self.post_filter_l.process(raw_wet_l),
            self.post_filter_r.process(raw_wet_r),
        ];
        for i in 0..2 {
            self.deemphasis[i] += c * (wet[i] - self.deemphasis[i]);
            wet[i] = (2.0 * wet[i] + self.deemphasis[i]) / 3.0;
            let dc = wet[i] - self.dc_prev[i] + self.dc_coeff * self.dc_out[i];
            self.dc_prev[i] = wet[i];
            self.dc_out[i] = if dc.abs() < 1e-20 { 0. } else { dc };
            if self.deemphasis[i].abs() < 1e-20 {
                self.deemphasis[i] = 0.;
            }
            if self.emphasis_lp[i].abs() < 1e-20 {
                self.emphasis_lp[i] = 0.;
            }
            wet[i] = self.dc_out[i];
        }
        let (wet_l, wet_r) = (wet[0], wet[1]);

        let mix = self.wet_mix.clamp(0.0, 1.0);
        (
            in_l * (1.0 - 0.45 * mix) + wet_l * mix,
            in_r * (1.0 - 0.45 * mix) + wet_r * mix,
        )
    }
}
