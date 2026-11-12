use crate::Error;
use realfft::{num_complex::Complex64, RealFftPlanner, RealToComplex};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

#[derive(Clone, Default)]
pub struct RequestClock(Arc<AtomicU64>);
pub struct RequestToken {
    clock: RequestClock,
    revision: u64,
}
impl RequestClock {
    /// Publishing a new token supersedes all prior jobs. One request group per chart.
    pub fn next(&self) -> RequestToken {
        let revision = self.0.fetch_add(1, Ordering::AcqRel) + 1;
        RequestToken {
            clock: self.clone(),
            revision,
        }
    }
}
impl RequestToken {
    pub fn is_current(&self) -> bool {
        self.clock.0.load(Ordering::Acquire) == self.revision
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct TransferPoint {
    pub hz: f64,
    pub db: f64,
    pub phase: Option<f64>,
}
#[derive(Clone, Copy, Debug)]
pub struct TransferMetadata {
    pub sample_rate: f64,
    pub impulse_samples: usize,
    pub fft_size: usize,
    pub bin_hz: f64,
    /// RMS of final 1/8 of impulse / RMS of entire impulse.
    pub tail_relative_rms: f64,
    /// Diagnostic only: backend must extend/recompute impulse if false.
    pub tail_converged: bool,
}
/// Unwindowed impulse-response FFT, independently normalized from the spectrum.
/// Padding increases interpolation density, never measured frequency resolution.
pub struct TransferAnalyzer {
    plan: Arc<dyn RealToComplex<f64>>,
    input: Vec<f64>,
    output: Vec<Complex64>,
    scratch: Vec<Complex64>,
    phase: Vec<Option<f64>>,
}
pub fn log_frequencies(
    sample_rate: f64,
    low: f64,
    high: f64,
    out: &mut [f64],
) -> Result<(), Error> {
    if !sample_rate.is_finite()
        || sample_rate <= 0.0
        || !low.is_finite()
        || !high.is_finite()
        || low <= 0.0
        || high < low
        || high > sample_rate * 0.5
        || out.len() < 2
    {
        return Err(Error::InvalidFrequency);
    }
    let count = out.len();
    for (i, f) in out.iter_mut().enumerate() {
        *f = low * (high / low).powf(i as f64 / (count - 1) as f64);
    }
    Ok(())
}
impl TransferAnalyzer {
    /// Retained data buffers only; excludes FFT plan tables and allocator metadata.
    pub fn memory_bytes(&self) -> usize {
        self.input.len() * std::mem::size_of::<f64>()
            + (self.output.len() + self.scratch.len()) * std::mem::size_of::<Complex64>()
            + self.phase.len() * std::mem::size_of::<Option<f64>>()
    }
    pub fn new(fft_size: usize) -> Result<Self, Error> {
        if !fft_size.is_power_of_two() || !(2048..=262144).contains(&fft_size) {
            return Err(Error::InvalidConfig);
        }
        let plan = RealFftPlanner::new().plan_fft_forward(fft_size);
        Ok(Self {
            input: plan.make_input_vec(),
            output: plan.make_output_vec(),
            scratch: plan.make_scratch_vec(),
            phase: vec![None; fft_size / 2 + 1],
            plan,
        })
    }
    /// Caller supplies sample rate, frequency grid and output storage. A request
    /// token enables coalescing/cancellation in the host's non-RT worker.
    pub fn analyze(
        &mut self,
        impulse: &[f64],
        sample_rate: f64,
        frequencies: &[f64],
        phase_floor_db: f64,
        out: &mut [TransferPoint],
        request: Option<&RequestToken>,
    ) -> Result<TransferMetadata, Error> {
        if impulse.is_empty() || impulse.len() > self.input.len() || frequencies.len() != out.len()
        {
            return Err(Error::InvalidLength);
        }
        if !sample_rate.is_finite()
            || sample_rate <= 0.0
            || !phase_floor_db.is_finite()
            || !(-300.0..=0.0).contains(&phase_floor_db)
        {
            return Err(Error::InvalidConfig);
        }
        if impulse.iter().any(|x| !x.is_finite() || x.abs() > 1e100) {
            return Err(Error::InvalidSamples);
        }
        if frequencies
            .iter()
            .any(|f| !f.is_finite() || *f < 0.0 || *f > sample_rate / 2.0)
            || frequencies.windows(2).any(|f| f[1] <= f[0])
        {
            return Err(Error::InvalidFrequency);
        }
        let current = || {
            if request.is_some_and(|r| !r.is_current()) {
                Err(Error::Superseded)
            } else {
                Ok(())
            }
        };
        current()?;
        self.input.fill(0.0);
        self.input[..impulse.len()].copy_from_slice(impulse);
        self.plan
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .map_err(|_| Error::InvalidSamples)?;
        current()?;
        // Unwrap the dense FFT grid before resampling. Unwrapping a sparse log
        // grid alone can miss entire turns for an otherwise valid delayed IR.
        let phase_floor_power = 10.0f64.powf(phase_floor_db / 10.0);
        let mut previous: Option<(f64, f64)> = None;
        for (i, x) in self.output.iter().enumerate() {
            if i % 256 == 0 {
                current()?;
            }
            if x.norm_sqr() < phase_floor_power {
                self.phase[i] = None;
                previous = None;
            } else {
                let wrapped = x.arg();
                let unwrapped = if let Some((last_wrapped, last_unwrapped)) = previous {
                    let delta = (wrapped - last_wrapped + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU)
                        - std::f64::consts::PI;
                    last_unwrapped + delta
                } else {
                    wrapped
                };
                self.phase[i] = Some(unwrapped);
                previous = Some((wrapped, unwrapped));
            }
        }
        for (i, (&hz, p)) in frequencies.iter().zip(out.iter_mut()).enumerate() {
            if i % 32 == 0 {
                current()?;
            }
            let bin = hz * self.input.len() as f64 / sample_rate;
            let k = (bin.floor() as usize).min(self.output.len() - 1);
            let next = (k + 1).min(self.output.len() - 1);
            let complex = self.output[k] + (self.output[next] - self.output[k]) * (bin - k as f64);
            let db = 20.0 * complex.norm().max(1e-150).log10();
            let phase = if db < phase_floor_db {
                None
            } else {
                let fraction = bin - k as f64;
                let anchor = if fraction.abs() < 1e-12 || k == next {
                    self.phase[k]
                } else {
                    self.phase[k]
                        .zip(self.phase[next])
                        .map(|(a, b)| a + (b - a) * fraction)
                };
                anchor.map(|anchor| {
                    let wrapped = complex.arg();
                    wrapped
                        + ((anchor - wrapped) / std::f64::consts::TAU).round()
                            * std::f64::consts::TAU
                })
            };
            *p = TransferPoint { hz, db, phase };
        }
        current()?;
        let energy = impulse.iter().map(|x| x * x).sum::<f64>();
        let tail = &impulse[impulse.len() * 7 / 8..];
        let tail_energy = tail.iter().map(|x| x * x).sum::<f64>();
        let relative = if energy == 0.0 {
            0.0
        } else {
            (tail_energy / tail.len() as f64 / (energy / impulse.len() as f64)).sqrt()
        };
        Ok(TransferMetadata {
            sample_rate,
            impulse_samples: impulse.len(),
            fft_size: self.input.len(),
            bin_hz: sample_rate / self.input.len() as f64,
            tail_relative_rms: relative,
            tail_converged: relative < 1e-4,
        })
    }
}
