// SPDX-License-Identifier: GPL-3.0-only
//! Analysis for a non-real-time worker. This crate must not run in an audio callback.
//! Plans, windows, scratch, history and outputs are retained between calls.
//! Transport/device ownership remains with the host; v1 scope snapshots are not
//! continuous sample packets and must never be supplied to [`Stft`].
mod spectrum;
mod stream;
mod transfer;
pub use spectrum::{Config, Error, SpectrumAnalyzer, SpectrumView, Window, WindowMetrics};
pub use stream::{FrameMetadata, Stft};
pub use transfer::{
    log_frequencies, RequestClock, RequestToken, TransferAnalyzer, TransferMetadata, TransferPoint,
};
