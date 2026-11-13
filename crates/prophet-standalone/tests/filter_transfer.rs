// SPDX-License-Identifier: GPL-3.0-only
use prophet_analysis::{Error, RequestClock};
use prophet_params::ParamId::*;
use prophet_params::{NUM_PARAMS, PARAM_SPECS};
use prophet_standalone::transfer::compute_filter_transfer;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::Semaphore;

fn baseline_values() -> [f32; NUM_PARAMS] {
    core::array::from_fn(|i| PARAM_SPECS[i].min)
}

#[test]
fn transfer_respects_sample_rate_cutoff_rolloff_and_phase_mask() {
    for sr in [48000.0, 96000.0] {
        for morph in [0.0f32, 1.0f32] {
            let mut v = baseline_values();
            v[FilterCutoff as usize] = 1000.0;
            v[FilterResonance as usize] = 0.2;
            v[FilterDrive as usize] = 1.0;
            v[MoogBassComp as usize] = 0.0;
            v[FilterMorph as usize] = morph;

            let res = compute_filter_transfer(&v, sr, None).unwrap();
            assert_eq!(res.sample_rate, sr);
            assert_eq!(res.hz.len(), 256);
            assert_eq!(res.db.len(), 256);
            assert_eq!(res.phase.len(), 256);
            assert!(res.tail_converged, "sr={sr} morph={morph} should converge");
            assert_eq!(res.fft_size, res.impulse_samples * 2);
            assert!((res.bin_hz - sr / res.fft_size as f64).abs() < 1e-9);
            assert!(res.hz.first().copied().unwrap() >= 20.0);
            assert!(res.hz.last().copied().unwrap() <= (sr * 0.49).min(20000.0) + 1e-6);

            let passband = *res
                .hz
                .iter()
                .zip(res.db.iter())
                .find(|(f, _)| **f >= 100.0)
                .unwrap()
                .1;
            let at_cutoff = *res
                .hz
                .iter()
                .zip(res.db.iter())
                .min_by(|a, b| (a.0 - 1000.0).abs().total_cmp(&(b.0 - 1000.0).abs()))
                .unwrap()
                .1;
            let stopband = *res
                .hz
                .iter()
                .zip(res.db.iter())
                .find(|(f, _)| **f >= 10000.0)
                .unwrap()
                .1;

            assert!(
                at_cutoff < passband - 1.0 && stopband < at_cutoff - 40.0,
                "sr={sr} morph={morph} passband={passband} at_cutoff={at_cutoff} stopband={stopband}"
            );

            for (db, phase) in res.db.iter().zip(res.phase.iter()) {
                assert!(db.is_finite());
                if *db < -100.0 {
                    assert!(
                        phase.is_none(),
                        "phase should be masked below -100 dB ({db})"
                    );
                }
            }
        }
    }
}

#[test]
fn transfer_adaptively_extends_high_q_tail_and_handles_self_oscillation() {
    let mut high_q = baseline_values();
    high_q[FilterCutoff as usize] = 800.0;
    high_q[FilterResonance as usize] = 0.85;
    high_q[FilterDrive as usize] = 1.0;
    high_q[MoogBassComp as usize] = 0.0;
    high_q[FilterMorph as usize] = 0.0;

    let res = compute_filter_transfer(&high_q, 96000.0, None).unwrap();
    assert!(
        res.impulse_samples > 4096,
        "expected adaptive tail extension beyond 4096, got {}",
        res.impulse_samples
    );
    assert_eq!(res.fft_size, res.impulse_samples * 2);
    assert!(
        res.tail_converged,
        "high-Q below self-oscillation must converge (samples={}, rel_rms={:.3e})",
        res.impulse_samples, res.tail_relative_rms
    );
    assert!(res.tail_relative_rms < 1e-4);

    // Extreme self-oscillating Q must terminate cleanly without panicking.
    let mut self_osc = high_q;
    self_osc[FilterResonance as usize] = 0.98;
    let unstable = compute_filter_transfer(&self_osc, 96000.0, None).unwrap();
    assert_eq!(unstable.impulse_samples, 131072);
    assert_eq!(unstable.fft_size, 262144);
    assert!(!unstable.tail_converged);
    assert!(unstable.db.iter().all(|d| d.is_finite()));
}

#[test]
fn transfer_respects_superseded_request_clock() {
    let clock = RequestClock::default();
    let stale = clock.next();
    let _newer = clock.next();

    let mut v = baseline_values();
    v[FilterCutoff as usize] = 800.0;
    v[FilterResonance as usize] = 0.5;
    assert!(matches!(
        compute_filter_transfer(&v, 96000.0, Some(&stale)),
        Err(Error::Superseded)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transfer_semaphore_bounds_concurrency_and_supersedes_burst() {
    let clock = RequestClock::default();
    let permits = Arc::new(Semaphore::new(2));
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for i in 0..8 {
        let token = clock.next();
        let permits = Arc::clone(&permits);
        let active = Arc::clone(&active);
        let peak = Arc::clone(&peak);
        let mut v = baseline_values();
        v[FilterCutoff as usize] = 400.0 + i as f32 * 300.0;
        v[FilterResonance as usize] = 0.85;
        handles.push(tokio::spawn(async move {
            let _permit = permits.acquire().await.unwrap();
            if !token.is_current() {
                return Err(Error::Superseded);
            }
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            let res = tokio::task::spawn_blocking(move || {
                compute_filter_transfer(&v, 96000.0, Some(&token))
            })
            .await
            .unwrap();
            active.fetch_sub(1, Ordering::SeqCst);
            res
        }));
    }

    let mut ok_count = 0usize;
    let mut superseded_count = 0usize;
    let last_idx = handles.len() - 1;
    for (idx, h) in handles.into_iter().enumerate() {
        match h.await.unwrap() {
            Ok(data) => {
                assert_eq!(data.sample_rate, 96000.0);
                ok_count += 1;
            }
            Err(Error::Superseded) => {
                assert_ne!(
                    idx, last_idx,
                    "latest transfer job must never be superseded"
                );
                superseded_count += 1;
            }
            Err(other) => panic!("unexpected transfer error: {other:?}"),
        }
    }

    assert!(peak.load(Ordering::SeqCst) <= 2);
    assert!(ok_count >= 1 && ok_count + superseded_count == 8);
}
