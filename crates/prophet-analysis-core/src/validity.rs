//! Bounded, worker-only small-signal transfer measurement.
use crate::{Error, RequestToken, TransferAnalyzer, TransferPoint};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Validity {
    Measurable,
    LongTail,
    SelfOscillating,
    NonlinearOperatingPoint,
    NumericalFailure,
}
pub struct Measurement {
    pub validity: Validity,
    pub points: Vec<TransferPoint>,
    pub tail_rms: f64,
    pub relative_scaling_error: f64,
}
/// Three freshly initialized filters receive zero, epsilon and 4*epsilon respectively.
/// Only a decayed signal with small amplitude-scaling error yields a transfer curve.
pub fn measure<F: FnMut(f32) -> f32>(
    rate: f64,
    grid: &[f64],
    n: usize,
    make: impl Fn() -> F,
    token: Option<&RequestToken>,
) -> Result<Measurement, Error> {
    if !(1024..=65536).contains(&n)
        || !n.is_power_of_two()
        || grid.is_empty()
        || grid.len() > 1024
        || !rate.is_finite()
        || !(8000.0..=192000.0).contains(&rate)
        || grid
            .iter()
            .any(|f| !f.is_finite() || *f < 0.0 || *f > rate * 0.5)
        || grid.windows(2).any(|v| v[0] >= v[1])
    {
        return Err(Error::InvalidConfig);
    }
    let current = || {
        if token.is_some_and(|t| !t.is_current()) {
            Err(Error::Superseded)
        } else {
            Ok(())
        }
    };
    current()?;
    let (mut silence, mut small, mut large) = (make(), make(), make());
    let mut impulse = Vec::with_capacity(n);
    let (mut energy, mut error, mut tail, mut prior, mut large_tail, mut autonomous) =
        (0., 0., 0., 0., 0., 0.);
    let mut tail_sum = 0.;
    const EPS: f32 = 0.001;
    for i in 0..n {
        if i % 64 == 0 {
            current()?
        }
        let x = if i == 0 { EPS } else { 0.0 };
        let a = f64::from(small(x));
        let b = f64::from(large(4.0 * x));
        let z = f64::from(silence(0.0));
        if [a, b, z].iter().any(|v| !v.is_finite() || v.abs() > 1e12) {
            return Ok(Measurement {
                validity: Validity::NumericalFailure,
                points: Vec::new(),
                tail_rms: f64::NAN,
                relative_scaling_error: f64::NAN,
            });
        }
        energy += a * a;
        error += (b / 4.0 - a).powi(2);
        if i >= n * 7 / 8 {
            tail += a * a;
            tail_sum += a;
            large_tail += b * b;
            autonomous += z * z;
        } else if i >= n * 3 / 4 {
            prior += a * a;
        }
        impulse.push(a / f64::from(EPS));
    }
    let scaling = (error / energy.max(1e-60)).sqrt();
    let tail_rms = (tail / (n / 8) as f64).sqrt();
    let ac_energy = (tail - tail_sum * tail_sum / (n / 8) as f64).max(0.);
    let oscillating = ac_energy > 0.01 * tail
        && tail_rms > 1e-6
        && tail >= 0.8 * prior
        && (autonomous > 1e-10 || (0.8..1.25).contains(&(large_tail / tail)));
    let validity = if oscillating {
        Validity::SelfOscillating
    } else if scaling > 0.02 || autonomous > 1e-10 {
        Validity::NonlinearOperatingPoint
    } else if tail * 8.0 > 1e-8 * energy {
        Validity::LongTail
    } else {
        Validity::Measurable
    };
    let mut points = Vec::new();
    if validity == Validity::Measurable {
        current()?;
        points.resize(
            grid.len(),
            TransferPoint {
                hz: 0.,
                db: 0.,
                phase: None,
            },
        );
        TransferAnalyzer::new(n)?.analyze(&impulse, rate, grid, -100., &mut points, token)?;
    }
    Ok(Measurement {
        validity,
        points,
        tail_rms,
        relative_scaling_error: scaling,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_gain_and_invalid_regimes() {
        let run = |f: fn(f32) -> f32| measure(48000., &[1000.], 4096, || f, None).unwrap();
        let gain = run(|x| x * 0.5);
        assert_eq!(gain.validity, Validity::Measurable);
        assert!((gain.points[0].db + 6.0206).abs() < 1e-4);
        assert_eq!(run(|x| x * x).validity, Validity::NonlinearOperatingPoint);
        assert_eq!(run(|_| f32::NAN).validity, Validity::NumericalFailure);
        assert_eq!(run(|_| 1.0).validity, Validity::NonlinearOperatingPoint);
        let alternating = measure(
            48000.,
            &[100.],
            4096,
            || {
                let mut sample = 0.1;
                move |_| {
                    sample = -sample;
                    sample
                }
            },
            None,
        )
        .unwrap();
        assert_eq!(alternating.validity, Validity::SelfOscillating);
        assert!(alternating.points.is_empty());
        let locked = measure(
            48000.,
            &[100.],
            4096,
            || {
                let mut y = 0.;
                move |x| {
                    y = 0.9999 * y + x;
                    y
                }
            },
            None,
        )
        .unwrap();
        assert_eq!(locked.validity, Validity::LongTail);
        assert!(locked.points.is_empty());
    }
}
