use crate::{Config, Error, SpectrumAnalyzer, SpectrumView};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameMetadata {
    pub sequence: u64,
    pub first_sample: u64,
    /// Exclusive sample index. Time in seconds = sample index / sample rate.
    pub end_sample: u64,
    pub discontinuities: u64,
}
/// Bounded consumer-side overlapping STFT. Not an audio-callback producer.
/// A backend-owned bounded SPSC queue supplies contiguous sample packets here.
pub struct Stft {
    analyzer: SpectrumAnalyzer,
    ring: Vec<f32>,
    frame: Vec<f32>,
    cursor: usize,
    filled: usize,
    until_frame: usize,
    hop: usize,
    expected_sequence: Option<u64>,
    expected_sample: Option<u64>,
    frame_sequence: u64,
    discontinuities: u64,
}
impl Stft {
    pub fn memory_bytes(&self) -> usize {
        self.analyzer.memory_bytes()
            + (self.ring.len() + self.frame.len()) * std::mem::size_of::<f32>()
    }
    pub fn new(config: Config, hop: usize) -> Result<Self, Error> {
        if hop == 0 || hop > config.fft_size {
            return Err(Error::InvalidConfig);
        }
        Ok(Self {
            analyzer: SpectrumAnalyzer::new(config)?,
            ring: vec![0.0; config.fft_size],
            frame: vec![0.0; config.fft_size],
            cursor: 0,
            filled: 0,
            until_frame: config.fft_size,
            hop,
            expected_sequence: None,
            expected_sample: None,
            frame_sequence: 0,
            discontinuities: 0,
        })
    }
    pub fn discontinuities(&self) -> u64 {
        self.discontinuities
    }
    /// Validation occurs before mutation. Dropped packets reset all overlap and
    /// averaging history; output resumes after N contiguous samples. Duplicate,
    /// reordered or overlapping old packets are rejected without changing state.
    /// callback must consume/copy the borrowed view before return.
    pub fn push(
        &mut self,
        sequence: u64,
        first_sample: u64,
        samples: &[f32],
        mut emit: impl FnMut(FrameMetadata, SpectrumView<'_>),
    ) -> Result<usize, Error> {
        if samples.is_empty() {
            return Err(Error::InvalidLength);
        }
        if samples.iter().any(|s| !s.is_finite() || s.abs() > 1e10) {
            return Err(Error::InvalidSamples);
        }
        let next_sequence = sequence.checked_add(1).ok_or(Error::InvalidSequence)?;
        let next_sample = first_sample
            .checked_add(samples.len() as u64)
            .ok_or(Error::InvalidSequence)?;
        if self.expected_sequence.is_some_and(|s| sequence < s)
            || self.expected_sample.is_some_and(|s| first_sample < s)
        {
            return Err(Error::InvalidSequence);
        }
        if self.expected_sequence.is_some_and(|s| s != sequence)
            || self.expected_sample.is_some_and(|s| s != first_sample)
        {
            self.cursor = 0;
            self.filled = 0;
            self.until_frame = self.ring.len();
            self.discontinuities += 1;
            self.analyzer.reset_history();
        }
        self.expected_sequence = Some(next_sequence);
        self.expected_sample = Some(next_sample);
        let mut count = 0;
        for (i, &x) in samples.iter().enumerate() {
            self.ring[self.cursor] = x;
            self.cursor = (self.cursor + 1) % self.ring.len();
            self.filled = (self.filled + 1).min(self.ring.len());
            self.until_frame -= 1;
            if self.until_frame == 0 {
                for (j, y) in self.frame.iter_mut().enumerate() {
                    *y = self.ring[(self.cursor + j) % self.ring.len()];
                }
                let end_sample = first_sample + i as u64 + 1;
                let meta = FrameMetadata {
                    sequence: self.frame_sequence,
                    first_sample: end_sample - self.ring.len() as u64,
                    end_sample,
                    discontinuities: self.discontinuities,
                };
                let spectrum = self.analyzer.analyze(&self.frame)?;
                emit(meta, spectrum);
                self.frame_sequence += 1;
                count += 1;
                self.until_frame = self.hop;
            }
        }
        Ok(count)
    }
}
