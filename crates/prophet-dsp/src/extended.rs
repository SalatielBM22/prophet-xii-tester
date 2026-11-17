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

#[cfg(test)]
mod tests {
    use super::*;
    /// With a coherent increment of 151/512, only the fundamental is fully
    /// enabled by the table crossover. Check the FFT-bin projections for C/D.
    #[test]
    fn near_nyquist_tables_do_not_fold_upper_partials() {
        const N: usize = 512;
        let bin = 151;
        for family in 0..2 {
            let dt = bin as f32 / N as f32;
            let samples: [f32; N] =
                std::array::from_fn(|i| wave(family, ((i * bin) % N) as f32 / N as f32, dt));
            let level = |k: usize| {
                let mut re = 0.;
                let mut im = 0.;
                for (i, &x) in samples.iter().enumerate() {
                    let (s, c) = (std::f64::consts::TAU * (k * i) as f64 / N as f64).sin_cos();
                    re += x as f64 * c;
                    im += x as f64 * s;
                }
                2. * re.hypot(im) / N as f64
            };
            assert!((level(bin) - 0.55).abs() < 1e-6);
            assert!((0..=N / 2).filter(|&k| k != bin).all(|k| level(k) < 1e-6));
        }
    }
    #[test]
    fn pitch_band_changes_are_continuous_at_all_table_boundaries() {
        for family in 0..2 {
            for level in 1..=5 {
                for i in 0..512 {
                    let dt = 0.49 / (1 << level) as f32;
                    let phase = i as f32 / 512.;
                    assert!(
                        (wave(family, phase, dt * 0.99999) - wave(family, phase, dt * 1.00001))
                            .abs()
                            < 2e-5
                    );
                }
            }
        }
    }
}
