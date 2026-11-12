use realfft::{num_complex::Complex32, RealFftPlanner, RealToComplex};
use std::{f32::consts::TAU, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Window {
    /// Periodic Hann (appropriate for DFT analysis, coherent gain exactly 0.5).
    Hann,
    BlackmanHarris,
    Kaiser {
        beta: f32,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub fft_size: usize,
    pub sample_rate: f32,
    pub window: Window,
    /// Weight of new *power*, 1 = no temporal smoothing. 0 < alpha <= 1.
    pub average_alpha: f32,
    pub peak_hold_frames: u32,
    pub peak_decay_db_per_frame: f32,
    pub floor_dbfs: f32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fft_size: 4096,
            sample_rate: 48000.0,
            window: Window::Hann,
            average_alpha: 1.0,
            peak_hold_frames: 25,
            peak_decay_db_per_frame: 1.0,
            floor_dbfs: -180.0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidConfig,
    InvalidLength,
    InvalidSamples,
    InvalidFrequency,
    Superseded,
    InvalidSequence,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "analysis error: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug)]
pub struct WindowMetrics {
    pub coherent_gain: f32,
    pub enbw_bins: f32,
    pub enbw_hz: f32,
}
pub struct SpectrumView<'a> {
    pub sample_rate: f32,
    pub fft_size: usize,
    pub bin_hz: f32,
    /// Smoothed one-sided amplitude squared; not PSD. See `psd` for power/Hz.
    pub power: &'a [f32],
    pub dbfs: &'a [f32],
    pub peak_dbfs: &'a [f32],
    /// Window-energy normalized one-sided power/Hz (DC/Nyquist not doubled).
    pub psd: &'a [f32],
}
pub struct SpectrumAnalyzer {
    config: Config,
    plan: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    window_sum: f32,
    window_energy: f32,
    input: Vec<f32>,
    output: Vec<Complex32>,
    scratch: Vec<Complex32>,
    power: Vec<f32>,
    dbfs: Vec<f32>,
    peak: Vec<f32>,
    peak_age: Vec<u32>,
    psd: Vec<f32>,
    initialized: bool,
}

fn i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..=64 {
        term *= x * x / (4.0 * f64::from(k * k));
        sum += term;
        if term < sum * 1e-15 {
            break;
        }
    }
    sum
}
impl SpectrumAnalyzer {
    pub fn new(config: Config) -> Result<Self, Error> {
        let c = config;
        if ![2048, 4096, 8192].contains(&c.fft_size)
            || !c.sample_rate.is_finite()
            || !(8000.0..=192000.0).contains(&c.sample_rate)
            || !c.average_alpha.is_finite()
            || !(0.0..=1.0).contains(&c.average_alpha)
            || c.average_alpha == 0.0
            || !c.peak_decay_db_per_frame.is_finite()
            || c.peak_decay_db_per_frame < 0.0
            || !c.floor_dbfs.is_finite()
            || !(-300.0..=-30.0).contains(&c.floor_dbfs)
            || matches!(c.window, Window::Kaiser { beta } if !beta.is_finite() || !(0.0..=20.0).contains(&beta))
        {
            return Err(Error::InvalidConfig);
        }
        let n = c.fft_size;
        let window: Vec<f32> = (0..n)
            .map(|i| {
                let angle = TAU * i as f32 / n as f32;
                match c.window {
                    Window::Hann => 0.5 - 0.5 * angle.cos(),
                    Window::BlackmanHarris => {
                        0.35875 - 0.48829 * angle.cos() + 0.14128 * (2.0 * angle).cos()
                            - 0.01168 * (3.0 * angle).cos()
                    }
                    Window::Kaiser { beta } => {
                        let x = 2.0 * i as f64 / (n - 1) as f64 - 1.0;
                        (i0(f64::from(beta) * (1.0 - x * x).max(0.0).sqrt()) / i0(f64::from(beta)))
                            as f32
                    }
                }
            })
            .collect();
        let window_sum = window.iter().sum();
        let window_energy = window.iter().map(|x| x * x).sum();
        let plan = RealFftPlanner::new().plan_fft_forward(n);
        let input = plan.make_input_vec();
        let output = plan.make_output_vec();
        let scratch = plan.make_scratch_vec();
        Ok(Self {
            config: c,
            plan,
            window,
            window_sum,
            window_energy,
            input,
            output,
            scratch,
            power: vec![0.0; n / 2 + 1],
            dbfs: vec![c.floor_dbfs; n / 2 + 1],
            peak: vec![c.floor_dbfs; n / 2 + 1],
            peak_age: vec![0; n / 2 + 1],
            psd: vec![0.0; n / 2 + 1],
            initialized: false,
        })
    }
    pub fn config(&self) -> Config {
        self.config
    }
    pub fn window_metrics(&self) -> WindowMetrics {
        let c = self.config;
        let bins = c.fft_size as f32 * self.window_energy / (self.window_sum * self.window_sum);
        WindowMetrics {
            coherent_gain: self.window_sum / c.fft_size as f32,
            enbw_bins: bins,
            enbw_hz: bins * c.sample_rate / c.fft_size as f32,
        }
    }
    pub fn reset_history(&mut self) {
        self.initialized = false;
        self.power.fill(0.0);
        self.dbfs.fill(self.config.floor_dbfs);
        self.peak.fill(self.config.floor_dbfs);
        self.peak_age.fill(0);
    }
    pub fn memory_bytes(&self) -> usize {
        4 * (self.window.len()
            + self.input.len()
            + self.power.len()
            + self.dbfs.len()
            + self.peak.len()
            + self.peak_age.len()
            + self.psd.len())
            + 8 * (self.output.len() + self.scratch.len())
    }
    /// Complete, contiguous mono frame. No allocation after construction.
    pub fn analyze(&mut self, samples: &[f32]) -> Result<SpectrumView<'_>, Error> {
        if samples.len() != self.config.fft_size {
            return Err(Error::InvalidLength);
        }
        // Bound arithmetic, while allowing ordinary audio overshoot above 0 dBFS.
        if samples.iter().any(|s| !s.is_finite() || s.abs() > 1e10) {
            return Err(Error::InvalidSamples);
        }
        for ((x, &s), &w) in self.input.iter_mut().zip(samples).zip(&self.window) {
            *x = s * w;
        }
        self.plan
            .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
            .map_err(|_| Error::InvalidSamples)?;
        let c = self.config;
        for (k, x) in self.output.iter().enumerate() {
            let edge = k == 0 || k == c.fft_size / 2;
            let multiplier = if edge { 1.0 } else { 2.0 };
            let raw = x.norm_sqr();
            let power = raw * multiplier * multiplier / (self.window_sum * self.window_sum);
            self.power[k] = if self.initialized {
                self.power[k] + c.average_alpha * (power - self.power[k])
            } else {
                power
            };
            self.psd[k] = raw * multiplier / (c.sample_rate * self.window_energy);
            self.dbfs[k] = (10.0 * self.power[k].max(1e-30).log10()).max(c.floor_dbfs);
            if self.dbfs[k] >= self.peak[k] {
                self.peak[k] = self.dbfs[k];
                self.peak_age[k] = 0;
            } else if self.peak_age[k] >= c.peak_hold_frames {
                self.peak[k] = (self.peak[k] - c.peak_decay_db_per_frame).max(self.dbfs[k]);
            } else {
                self.peak_age[k] += 1;
            }
        }
        self.initialized = true;
        Ok(self.view())
    }
    pub fn view(&self) -> SpectrumView<'_> {
        SpectrumView {
            sample_rate: self.config.sample_rate,
            fft_size: self.config.fft_size,
            bin_hz: self.config.sample_rate / self.config.fft_size as f32,
            power: &self.power,
            dbfs: &self.dbfs,
            peak_dbfs: &self.peak,
            psd: &self.psd,
        }
    }
}
