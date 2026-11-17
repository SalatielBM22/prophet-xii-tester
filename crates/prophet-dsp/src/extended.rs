use crate::vector::{Patch, Route, Topology};
include!(concat!(env!("OUT_DIR"), "/waves.rs"));

fn wave(family: usize, phase: f32, dt: f32) -> f32 {
    let index = phase * 512.;
    let i = (index as usize).min(511);
    let fraction = index - i as f32;
    let read = |level: usize| {
        let t = &BANK[family][level];
        t[i] + fraction * (t[i + 1] - t[i])
    };
    // Fade each octave in only once *all* its partials are below Nyquist.
    let harmonics = 0.49 / dt;
    let level = (31 - (harmonics as u32).max(1).leading_zeros()).min(5) as usize;
    if level == 0 {
        return read(0);
    }
    let blend = (harmonics / (1 << level) as f32 - 1.).min(1.);
    let lower = read(level - 1);
    lower + (read(level) - lower) * blend
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ExtraOscillators {
    phases: [f32; 2],
}
impl ExtraOscillators {
    pub fn mix(
        &mut self,
        patch: &Patch,
        ab: [f32; 2],
        gains: [f32; 2],
        mut wrap: bool,
        base_dt: f32,
        weights: [f32; 4],
    ) -> f32 {
        let mut signals = [ab[0], ab[1], 0., 0.];
        let count = if patch.topology == Topology::Extended3 {
            1
        } else {
            2
        };
        for i in 0..count {
            let route = patch.routes[i];
            let source = signals[i + 1];
            let mut dt = base_dt * patch.ratios[i];
            if route == Route::Frequency {
                dt *= 1. + 0.5 * source.clamp(-1., 1.);
            }
            dt = dt.clamp(1e-8, 0.49);
            if route == Route::Sync && wrap {
                self.phases[i] = 0.;
            }
            let next = self.phases[i] + dt;
            wrap = next >= 1.;
            self.phases[i] = if wrap { next - 1. } else { next };
            let phase = if route == Route::Phase {
                (self.phases[i] + 0.125 * source).rem_euclid(1.)
            } else {
                self.phases[i]
            };
            let signal = wave(i, phase, dt);
            signals[i + 2] = if route == Route::Ring {
                signal * source
            } else {
                signal
            };
        }
        if patch.topology == Topology::Vector4 {
            signals.iter().zip(weights).map(|(v, w)| v * w).sum()
        } else {
            (signals[0] * gains[0] + signals[1] * gains[1] + signals[2] + signals[3])
                / (count + 2) as f32
        }
    }
}
