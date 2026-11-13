// SPDX-License-Identifier: GPL-3.0-only
//! Non-real-time R2C spectrum, STFT, and transfer analysis shared from
//! `crates/prophet-analysis` as a pure `rlib` for host/workspace targets.
#[path = "../../prophet-analysis/src/spectrum.rs"]
mod spectrum;
#[path = "../../prophet-analysis/src/stream.rs"]
mod stream;
#[path = "../../prophet-analysis/src/transfer.rs"]
mod transfer;

pub use spectrum::{Config, Error, SpectrumAnalyzer, SpectrumView, Window, WindowMetrics};
pub use stream::{FrameMetadata, Stft};
pub use transfer::{
    log_frequencies, RequestClock, RequestToken, TransferAnalyzer, TransferMetadata, TransferPoint,
};
