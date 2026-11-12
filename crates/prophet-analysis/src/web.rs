//! Allocation-free packet ingestion for the browser's analysis Worker.
//!
//! JavaScript writes at most 64 floats through input_ptr(), then calls push().
//! No Rust reference into that input may remain active while JavaScript writes.
//! JS must rebuild typed-array views after WebAssembly.Memory.grow and must not
//! mutate output buffers. Pointers remain stable until the object is freed.
//! All FFT/STFT work still occurs on the Worker, never the UI/audio thread.
use crate::{Config, Error, FrameMetadata, Stft, Window};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const INPUT_CAPACITY: usize = 64;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct WebAnalyzer {
    stft: Stft,
    input: Vec<f32>,
    db: Vec<f32>,
    peak: Vec<f32>,
    latest: Option<FrameMetadata>,
    sample_rate: f32,
    fft_size: usize,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl WebAnalyzer {
    /// Window code: 0 Hann, 1 Blackman-Harris, 2 Kaiser. Hop is N/4 (75% overlap).
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new(
        fft_size: usize,
        sample_rate: f32,
        window_code: u8,
        average_alpha: f32,
        peak_hold_frames: u32,
        peak_decay_db_per_frame: f32,
        kaiser_beta: f32,
    ) -> Result<WebAnalyzer, Error> {
        let window = match window_code {
            0 => Window::Hann,
            1 => Window::BlackmanHarris,
            2 => Window::Kaiser { beta: kaiser_beta },
            _ => return Err(Error::InvalidConfig),
        };
        let config = Config {
            fft_size,
            sample_rate,
            window,
            average_alpha,
            peak_hold_frames,
            peak_decay_db_per_frame,
            ..Config::default()
        };
        let stft = Stft::new(config, fft_size / 4)?;
        Ok(Self {
            stft,
            input: vec![0.0; INPUT_CAPACITY],
            db: vec![config.floor_dbfs; fft_size / 2 + 1],
            peak: vec![config.floor_dbfs; fft_size / 2 + 1],
            latest: None,
            sample_rate,
            fft_size,
        })
    }
    pub fn input_ptr(&self) -> usize {
        self.input.as_ptr() as usize
    }
    pub fn db_ptr(&self) -> usize {
        self.db.as_ptr() as usize
    }
    pub fn peak_ptr(&self) -> usize {
        self.peak.as_ptr() as usize
    }
    pub fn input_capacity(&self) -> usize {
        INPUT_CAPACITY
    }
    pub fn bins(&self) -> usize {
        self.db.len()
    }
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }
    pub fn fft_size(&self) -> usize {
        self.fft_size
    }
    pub fn coherent_gain(&self) -> f32 {
        self.stft.window_metrics().coherent_gain
    }
    pub fn enbw_bins(&self) -> f32 {
        self.stft.window_metrics().enbw_bins
    }
    pub fn enbw_hz(&self) -> f32 {
        self.stft.window_metrics().enbw_hz
    }
    pub fn has_frame(&self) -> bool {
        self.latest.is_some()
    }
    pub fn first_frame(&self) -> f64 {
        self.latest.map_or(0.0, |m| m.first_sample as f64)
    }
    pub fn end_frame(&self) -> f64 {
        self.latest.map_or(0.0, |m| m.end_sample as f64)
    }
    pub fn sequence(&self) -> f64 {
        self.latest.map_or(0.0, |m| m.sequence as f64)
    }
    pub fn discontinuities(&self) -> f64 {
        self.stft.discontinuities() as f64
    }
    /// Returns completed frames (0 or 1 for the supported packet/hop sizes), or:
    /// -1 invalid packet length; -2 invalid sequence/frame safe integer;
    /// -3 nonfinite/out-of-bound samples; -4 duplicate/reordered/overlapping packet;
    /// -5 other analysis error. No string/error allocation occurs in this method.
    pub fn push(&mut self, sequence: f64, first_frame: f64, len: usize) -> i32 {
        if len == 0 || len > INPUT_CAPACITY {
            return -1;
        }
        let valid_integer =
            |x: f64| x.is_finite() && (0.0..=MAX_SAFE_INTEGER).contains(&x) && x.fract() == 0.0;
        if !valid_integer(sequence)
            || !valid_integer(first_frame)
            || first_frame + len as f64 > MAX_SAFE_INTEGER
        {
            return -2;
        }
        let before = self.stft.discontinuities();
        let WebAnalyzer {
            stft,
            input,
            db,
            peak,
            latest,
            ..
        } = self;
        let result = stft.push(
            sequence as u64,
            first_frame as u64,
            &input[..len],
            |metadata, view| {
                db.copy_from_slice(view.dbfs);
                peak.copy_from_slice(view.peak_dbfs);
                *latest = Some(metadata);
            },
        );
        match result {
            Ok(count) => {
                // Old results must not be rendered while waiting for a fresh
                // contiguous window after an acquisition gap.
                if count == 0 && stft.discontinuities() != before {
                    *latest = None;
                }
                count as i32
            }
            Err(Error::InvalidSamples) => -3,
            Err(Error::InvalidSequence) => -4,
            Err(_) => -5,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl From<Error> for JsValue {
    fn from(error: Error) -> Self {
        // Only construction can return a JS exception. Packet errors use numeric
        // status codes and never allocate a JS error or message per packet.
        JsValue::from_str(match error {
            Error::InvalidConfig => "Invalid spectral analysis configuration",
            _ => "Spectral analysis initialization failed",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_packets_match_native_spectrum_and_timestamps() {
        let mut analyzer = WebAnalyzer::new(2048, 48000.0, 0, 1.0, 25, 1.0, 8.6).unwrap();
        let pointers = (analyzer.input_ptr(), analyzer.db_ptr(), analyzer.peak_ptr());
        for packet in 0..32 {
            for (i, x) in analyzer.input.iter_mut().enumerate() {
                *x = 0.5 * (std::f32::consts::TAU * 37.0 * (packet * 64 + i) as f32 / 2048.0).sin();
            }
            assert_eq!(
                analyzer.push(packet as f64, (packet * 64) as f64, 64),
                i32::from(packet == 31)
            );
        }
        assert!(analyzer.has_frame());
        assert_eq!(analyzer.first_frame(), 0.0);
        assert_eq!(analyzer.end_frame(), 2048.0);
        assert!((analyzer.db[37] + 6.0206).abs() < 0.001);
        assert_eq!(
            pointers,
            (analyzer.input_ptr(), analyzer.db_ptr(), analyzer.peak_ptr())
        );
        assert_eq!(analyzer.push(33.0, 2112.0, 64), 0);
        assert!(!analyzer.has_frame());
        assert_eq!(analyzer.discontinuities(), 1.0);
    }
    #[test]
    fn invalid_browser_inputs_do_not_change_sequence_state() {
        let mut analyzer = WebAnalyzer::new(4096, 96000.0, 2, 0.3, 10, 1.0, 8.6).unwrap();
        assert_eq!(analyzer.push(0.0, 0.0, 0), -1);
        assert_eq!(analyzer.push(0.0, 0.0, 65), -1);
        for value in [f64::NAN, f64::INFINITY, -1.0, 0.5, MAX_SAFE_INTEGER + 1.0] {
            assert_eq!(analyzer.push(value, 0.0, 64), -2);
            assert_eq!(analyzer.push(0.0, value, 64), -2);
        }
        assert_eq!(analyzer.push(0.0, MAX_SAFE_INTEGER, 64), -2);
        analyzer.input[0] = f32::NAN;
        assert_eq!(analyzer.push(0.0, 0.0, 64), -3);
        analyzer.input[0] = 0.0;
        assert_eq!(analyzer.push(0.0, 0.0, 64), 0);
        assert_eq!(analyzer.push(0.0, 0.0, 64), -4);
        assert_eq!(analyzer.push(1.0, 64.0, 32), 0);
    }
}
