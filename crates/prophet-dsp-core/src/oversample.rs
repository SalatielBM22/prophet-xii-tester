// SPDX-License-Identifier: GPL-3.0-only
// Linear-phase, Blackman-windowed half-band FIR. See docs/DSP.md for latency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OversampleFactor {
    X1 = 1,
    X2 = 2,
    X4 = 4,
}
impl OversampleFactor {
    pub fn as_usize(self) -> usize {
        self as usize
    }
    pub fn latency_samples(self) -> u32 {
        match self {
            Self::X1 => 0,
            Self::X2 => 16,
            Self::X4 => 24,
        }
    }
}
const TAPS: [f32; 65] = [
    2.7048772605e-34,
    -8.9378515703e-06,
    2.0116390918e-19,
    8.8329163001e-05,
    -2.8512501234e-19,
    -2.7686815975e-04,
    -8.3724601096e-19,
    6.2518079606e-04,
    -1.2950905815e-18,
    -1.2067459471e-03,
    7.9116920362e-18,
    2.1198655264e-03,
    -3.3541486188e-18,
    -3.4903237743e-03,
    4.8443045227e-18,
    5.4772919000e-03,
    -6.6268362171e-18,
    -8.2875682607e-03,
    8.6467504390e-18,
    1.2208911914e-02,
    -1.0812914587e-17,
    -1.7687832002e-02,
    1.3003615434e-17,
    2.5520789727e-02,
    -1.5077093014e-17,
    -3.7383473404e-02,
    1.6885753281e-17,
    5.7639465933e-02,
    -1.8292178972e-17,
    -1.0238751524e-01,
    1.9184749375e-17,
    3.1705153845e-01,
    4.9999578247e-01,
    3.1705153845e-01,
    1.9184749375e-17,
    -1.0238751524e-01,
    -1.8292178972e-17,
    5.7639465933e-02,
    1.6885753281e-17,
    -3.7383473404e-02,
    -1.5077093014e-17,
    2.5520789727e-02,
    1.3003615434e-17,
    -1.7687832002e-02,
    -1.0812914587e-17,
    1.2208911914e-02,
    8.6467504390e-18,
    -8.2875682607e-03,
    -6.6268362171e-18,
    5.4772919000e-03,
    4.8443045227e-18,
    -3.4903237743e-03,
    -3.3541486188e-18,
    2.1198655264e-03,
    7.9116920362e-18,
    -1.2067459471e-03,
    -1.2950905815e-18,
    6.2518079606e-04,
    -8.3724601096e-19,
    -2.7686815975e-04,
    -2.8512501234e-19,
    8.8329163001e-05,
    2.0116390918e-19,
    -8.9378515703e-06,
    2.7048772605e-34,
];
#[derive(Clone, Debug)]
struct Fir {
    data: [f32; 130],
    pos: usize,
}
impl Fir {
    const fn new() -> Self {
        Self {
            data: [0.0; 130],
            pos: 0,
        }
    }
    #[inline(always)]
    fn push(&mut self, x: f32) {
        let p = self.pos;
        self.data[p] = x;
        self.data[p + 65] = x;
        self.pos = if p == 64 { 0 } else { p + 1 };
    }
    #[inline(always)]
    fn step(&mut self, x: f32) -> f32 {
        let p = self.pos;
        self.data[p] = x;
        self.data[p + 65] = x;
        let base = p + 65;
        // Center tap (32) plus symmetric odd half-band pairs (k, 64 - k); even taps are ~0 and omitted.
        let mut y = TAPS[32] * self.data[base - 32];
        let mut k = 1usize;
        while k < 32 {
            y += TAPS[k] * (self.data[base - k] + self.data[base - (64 - k)]);
            k += 2;
        }
        self.pos = if p == 64 { 0 } else { p + 1 };
        y
    }
}
#[derive(Clone, Debug)]
pub struct HalfBandStage {
    up: Fir,
    down: Fir,
}
impl Default for HalfBandStage {
    fn default() -> Self {
        Self::new()
    }
}
impl HalfBandStage {
    pub const fn new() -> Self {
        Self {
            up: Fir::new(),
            down: Fir::new(),
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new();
    }
    pub fn upsample_sample(&mut self, x: f32) -> (f32, f32) {
        (2.0 * self.up.step(x), 2.0 * self.up.step(0.0))
    }
    #[inline(always)]
    pub fn downsample_sample(&mut self, a: f32, b: f32) -> f32 {
        let y = self.down.step(a);
        self.down.push(b);
        y
    }
}
#[derive(Clone, Debug)]
pub struct VoiceOversampler {
    pub factor: OversampleFactor,
    stage_2x: HalfBandStage,
    stage_4x: HalfBandStage,
}
impl Default for VoiceOversampler {
    fn default() -> Self {
        Self::new(OversampleFactor::X2)
    }
}
impl VoiceOversampler {
    pub const fn new(factor: OversampleFactor) -> Self {
        Self {
            factor,
            stage_2x: HalfBandStage::new(),
            stage_4x: HalfBandStage::new(),
        }
    }
    pub fn reset(&mut self) {
        self.stage_2x.reset();
        self.stage_4x.reset();
    }
    #[inline]
    pub fn process_voice_sample<F: FnMut() -> f32>(&mut self, mut f: F) -> f32 {
        match self.factor {
            OversampleFactor::X1 => f(),
            OversampleFactor::X2 => {
                let a = f();
                let b = f();
                self.stage_2x.downsample_sample(a, b)
            }
            OversampleFactor::X4 => {
                let a = f();
                let b = f();
                let c = f();
                let d = f();
                // One continuous state for both consecutive high-rate pairs.
                let x = self.stage_4x.downsample_sample(a, b);
                let y = self.stage_4x.downsample_sample(c, d);
                self.stage_2x.downsample_sample(x, y)
            }
        }
    }
}
