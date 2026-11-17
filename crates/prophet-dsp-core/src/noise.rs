// SPDX-License-Identifier: GPL-3.0-only
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
