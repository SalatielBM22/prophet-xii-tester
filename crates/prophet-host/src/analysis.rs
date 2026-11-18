// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, cancellable off-thread VCF small-signal transfer response analysis
//! backed strictly by `prophet_analysis_core::validity::measure`.

use prophet_analysis_core::{
    log_frequencies,
    validity::{self, Validity},
    Error as AnalysisError, RequestToken, TransferPoint,
};
use prophet_dsp::vcf_moog::DualVcfStage;
use prophet_params::ParamId::*;
use prophet_params::NUM_PARAMS;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterValidity {
    Measurable,
    LongTail,
    SelfOscillating,
    NonlinearOperatingPoint,
    NumericalFailure,
}

impl From<Validity> for FilterValidity {
    fn from(v: Validity) -> Self {
        match v {
            Validity::Measurable => Self::Measurable,
            Validity::LongTail => Self::LongTail,
            Validity::SelfOscillating => Self::SelfOscillating,
            Validity::NonlinearOperatingPoint => Self::NonlinearOperatingPoint,
            Validity::NumericalFailure => Self::NumericalFailure,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterPoint {
    pub hz: f64,
    pub db: f64,
    pub phase: Option<f64>,
}

impl From<TransferPoint> for FilterPoint {
    fn from(p: TransferPoint) -> Self {
        Self {
            hz: p.hz,
            db: p.db,
            phase: p.phase,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterResponseReport {
    pub sample_rate: f64,
    pub window_samples: usize,
    pub validity: FilterValidity,
    pub points: Vec<FilterPoint>,
    pub tail_rms: Option<f64>,
    pub relative_scaling_error: Option<f64>,
    pub reason: String,
}

pub fn default_window_for_rate(_rate: f64) -> usize {
    // 65,536 is the upper bounded window supported by `validity::measure` and allows
    // the CEM3320's 5 Hz AC-coupled feedback capacitor to decay on normal damped patches
    // while still classifying long-tail low-cutoff/high-Q regimes honestly as `LongTail`.
    65_536
}

pub fn measure_filter_response(
    values: &[f32; NUM_PARAMS],
    sample_rate: f64,
    window_override: Option<usize>,
    token: Option<&RequestToken>,
) -> Result<FilterResponseReport, AnalysisError> {
    if !sample_rate.is_finite() || !(8000.0..=192_000.0).contains(&sample_rate) {
        return Err(AnalysisError::InvalidConfig);
    }
    let n = window_override
        .unwrap_or_else(|| default_window_for_rate(sample_rate))
        .clamp(1024, 65_536);
    let cutoff = values[FilterCutoff as usize];
    let res = values[FilterResonance as usize];
    let drive = values[FilterDrive as usize];
    let bass_comp = values[MoogBassComp as usize];
    let morph = values[FilterMorph as usize];

    let mut grid = [0.0f64; 256];
    let upper = (sample_rate * 0.49).min(20_000.0);
    log_frequencies(sample_rate, 20.0, upper, &mut grid)?;

    let measurement = validity::measure(
        sample_rate,
        &grid,
        n,
        || {
            let mut stage = DualVcfStage::new(sample_rate as f32, 1.0);
            stage.set_params(cutoff, res, drive, bass_comp, morph);
            move |x| stage.process_sample(x)
        },
        token,
    )?;

    let validity = FilterValidity::from(measurement.validity);
    // Strict validity gate: plot points are populated ONLY when `Measurable`.
    let points = if measurement.validity == Validity::Measurable {
        measurement.points.into_iter().map(FilterPoint::from).collect()
    } else {
        Vec::new()
    };
    let reason = match measurement.validity {
        Validity::Measurable => "Valid small-signal linear transfer response".to_owned(),
        Validity::LongTail => {
            "Filter impulse tail did not decay below threshold within bounded window".to_owned()
        }
        Validity::SelfOscillating => {
            "Filter is self-oscillating; small-signal transfer response undefined".to_owned()
        }
        Validity::NonlinearOperatingPoint => {
            "Filter operating point exhibits non-linear scaling error or DC bias".to_owned()
        }
        Validity::NumericalFailure => {
            "Non-finite or unbounded filter response prevented measurement".to_owned()
        }
    };

    Ok(FilterResponseReport {
        sample_rate,
        window_samples: n,
        validity,
        points,
        tail_rms: measurement.tail_rms.is_finite().then_some(measurement.tail_rms),
        relative_scaling_error: measurement
            .relative_scaling_error
            .is_finite()
            .then_some(measurement.relative_scaling_error),
        reason,
    })
}
