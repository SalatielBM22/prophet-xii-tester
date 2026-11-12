use prophet_analysis::*;
use std::f32::consts::TAU;

fn tone(n: usize, bin: f32, amplitude: f32) -> Vec<f32> {
    (0..n)
        .map(|i| amplitude * (TAU * bin * i as f32 / n as f32).sin())
        .collect()
}
fn close(a: f32, b: f32, tol: f32) {
    assert!((a - b).abs() < tol, "{a} != {b} (tol={tol})");
}

#[test]
fn calibrated_sines_all_sizes_rates_and_windows() {
    for fft_size in [2048, 4096, 8192] {
        for sample_rate in [44100.0, 48000.0, 96000.0] {
            for window in [
                Window::Hann,
                Window::BlackmanHarris,
                Window::Kaiser { beta: 8.6 },
            ] {
                let mut a = SpectrumAnalyzer::new(Config {
                    fft_size,
                    sample_rate,
                    window,
                    ..Config::default()
                })
                .unwrap();
                let v = a.analyze(&tone(fft_size, 37.0, 0.5)).unwrap();
                close(v.dbfs[37], -6.0206, 0.003);
                close(v.bin_hz, sample_rate / fft_size as f32, 1e-5);
                assert_eq!(v.dbfs.len(), fft_size / 2 + 1);
                assert!(v.dbfs.iter().all(|v| v.is_finite()));
            }
        }
    }
}
#[test]
fn dc_and_nyquist_are_not_doubled() {
    let mut a = SpectrumAnalyzer::new(Config::default()).unwrap();
    close(
        a.analyze(&vec![0.5; 4096]).unwrap().dbfs[0],
        -6.0206,
        0.0001,
    );
    let x: Vec<_> = (0..4096)
        .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
        .collect();
    close(a.analyze(&x).unwrap().dbfs[2048], -6.0206, 0.0001);
}
#[test]
fn coherent_gain_enbw_and_psd_parseval() {
    let mut a = SpectrumAnalyzer::new(Config::default()).unwrap();
    let m = a.window_metrics();
    close(m.coherent_gain, 0.5, 1e-5);
    close(m.enbw_bins, 1.5, 1e-5);
    let mut seed = 1u32;
    let x: Vec<_> = (0..4096)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        })
        .collect();
    let weighted_energy = x
        .iter()
        .enumerate()
        .map(|(i, x)| {
            let w = 0.5 - 0.5 * (TAU * i as f32 / 4096.0).cos();
            x * x * w * w
        })
        .sum::<f32>()
        / (4096.0 * 0.375);
    let v = a.analyze(&x).unwrap();
    close(v.psd.iter().sum::<f32>() * v.bin_hz, weighted_energy, 1e-5);
}
#[test]
fn off_bin_scalloping_and_two_tone_separation() {
    let mut a = SpectrumAnalyzer::new(Config::default()).unwrap();
    let v = a.analyze(&tone(4096, 37.5, 0.5)).unwrap();
    assert!((v.dbfs[37] - (-7.444)).abs() < 0.02);
    let x: Vec<_> = tone(4096, 37.0, 0.5)
        .iter()
        .zip(tone(4096, 42.0, 0.25))
        .map(|(a, b)| a + b)
        .collect();
    let v = a.analyze(&x).unwrap();
    close(v.dbfs[37], -6.0206, 0.001);
    close(v.dbfs[42], -12.0412, 0.001);
    assert!(v.dbfs[39] < -90.0 && v.dbfs[40] < -90.0);
}
#[test]
fn impulse_silence_subnormal_and_invalid_samples() {
    let mut a = SpectrumAnalyzer::new(Config::default()).unwrap();
    let mut x = vec![0.0; 4096];
    x[2048] = 1.0;
    let v = a.analyze(&x).unwrap();
    close(v.dbfs[50], 20.0 * (2.0f32 / 2048.0).log10(), 0.0001);
    assert!(a
        .analyze(&vec![0.0; 4096])
        .unwrap()
        .dbfs
        .iter()
        .all(|v| *v == -180.0));
    assert!(a
        .analyze(&vec![f32::MIN_POSITIVE * 0.1; 4096])
        .unwrap()
        .dbfs
        .iter()
        .all(|v| v.is_finite()));
    x[1] = f32::NAN;
    assert!(matches!(a.analyze(&x), Err(Error::InvalidSamples)));
    assert!(matches!(a.analyze(&[0.0]), Err(Error::InvalidLength)));
}
#[test]
fn power_averaging_and_configurable_peak_hold() {
    let mut a = SpectrumAnalyzer::new(Config {
        average_alpha: 0.5,
        peak_hold_frames: 2,
        peak_decay_db_per_frame: 3.0,
        ..Config::default()
    })
    .unwrap();
    a.analyze(&tone(4096, 37.0, 0.5)).unwrap();
    let zero = vec![0.0; 4096];
    let v = a.analyze(&zero).unwrap();
    close(v.dbfs[37], -9.0309, 0.001);
    close(v.peak_dbfs[37], -6.0206, 0.001);
    a.analyze(&zero).unwrap();
    close(a.analyze(&zero).unwrap().peak_dbfs[37], -9.0206, 0.001);
    a.reset_history();
    assert_eq!(a.view().peak_dbfs[37], -180.0);
}

fn dtft(x: &[f64], hz: f64, sr: f64) -> (f64, f64) {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, x) in x.iter().enumerate() {
        let p = -std::f64::consts::TAU * hz * i as f64 / sr;
        re += x * p.cos();
        im += x * p.sin();
    }
    (20.0 * re.hypot(im).max(1e-150).log10(), im.atan2(re))
}
#[test]
fn spectrum_agrees_with_independent_direct_dft() {
    let n = 2048;
    let mut a = SpectrumAnalyzer::new(Config {
        fft_size: n,
        ..Config::default()
    })
    .unwrap();
    let x = tone(n, 37.3, 0.4);
    let y: Vec<_> = x
        .iter()
        .enumerate()
        .map(|(i, x)| {
            f64::from(*x) * (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos())
        })
        .collect();
    let v = a.analyze(&x).unwrap();
    for bin in [0, 1, 35, 36, 37, 38, 39, 500, 1024] {
        let (db, _) = dtft(&y, bin as f64 * 48000.0 / n as f64, 48000.0);
        let expected = db - 20.0 * (n as f64 * 0.5).log10()
            + if bin == 0 || bin == n / 2 {
                0.0
            } else {
                20.0 * 2.0f64.log10()
            };
        if expected > -90.0 {
            close(v.dbfs[bin], expected as f32, 0.01);
        }
    }
}
#[test]
fn blackman_harris_reduces_far_sidelobes() {
    let x = tone(4096, 37.4, 0.5);
    let mut hann = SpectrumAnalyzer::new(Config::default()).unwrap();
    let hann_far = hann.analyze(&x).unwrap().dbfs[50];
    let mut bh = SpectrumAnalyzer::new(Config {
        window: Window::BlackmanHarris,
        ..Config::default()
    })
    .unwrap();
    assert!(bh.analyze(&x).unwrap().dbfs[50] < hann_far - 20.0);
}
#[test]
fn noise_power_and_pink_spectral_tilt() {
    let n = 4096;
    let mut a = SpectrumAnalyzer::new(Config {
        average_alpha: 0.05,
        ..Config::default()
    })
    .unwrap();
    let mut seed = 7u32;
    let mut white = vec![0.0; n];
    let mut pink = vec![0.0; n];
    let mut rows = [0.0f32; 12];
    let mut accumulator = 0.0f32;
    let mut white_low = 0.0;
    let mut white_high = 0.0;
    let mut pink_low = 0.0;
    let mut pink_high = 0.0;
    for frame in 0..64 {
        for i in 0..n {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            white[i] = x;
            let index = ((frame * n + i + 1) as u32).trailing_zeros().min(11) as usize;
            accumulator -= rows[index];
            rows[index] = x;
            accumulator += x;
            pink[i] = accumulator / 12.0;
        }
        let v = a.analyze(&white).unwrap();
        white_low += v.psd[32..64].iter().sum::<f32>() / 32.0;
        white_high += v.psd[512..1024].iter().sum::<f32>() / 512.0;
    }
    a.reset_history();
    for frame in 0..64 {
        for i in 0..n {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let x = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            let index = ((frame * n + i + 1) as u32).trailing_zeros().min(11) as usize;
            accumulator -= rows[index];
            rows[index] = x;
            accumulator += x;
            pink[i] = accumulator / 12.0;
        }
        let v = a.analyze(&pink).unwrap();
        pink_low += v.psd[32..64].iter().sum::<f32>() / 32.0;
        pink_high += v.psd[512..1024].iter().sum::<f32>() / 512.0;
    }
    assert!((0.7..1.4).contains(&(white_low / white_high)));
    assert!(
        (8.0..40.0).contains(&(pink_low / pink_high)),
        "pink ratio {}",
        pink_low / pink_high
    );
}
#[test]
fn stft_contiguous_packet_partition_and_gap_reset() {
    let mut s = Stft::new(
        Config {
            fft_size: 2048,
            ..Config::default()
        },
        512,
    )
    .unwrap();
    let x = tone(4096, 37.0, 0.5);
    let mut metadata = Vec::new();
    for (seq, chunk) in x.chunks(256).enumerate() {
        s.push(seq as u64, seq as u64 * 256, chunk, |m, v| {
            assert!(v.dbfs.iter().all(|v| v.is_finite()));
            metadata.push(m);
        })
        .unwrap();
    }
    assert_eq!(
        metadata.iter().map(|m| m.first_sample).collect::<Vec<_>>(),
        vec![0, 512, 1024, 1536, 2048]
    );
    assert_eq!(
        s.push(17, 4352, &x[..1024], |_, _| panic!(
            "must wait for a full contiguous frame"
        ))
        .unwrap(),
        0
    );
    assert_eq!(s.discontinuities(), 1);
    assert!(matches!(
        s.push(17, 4352, &x[..1024], |_, _| panic!("stale packet")),
        Err(Error::InvalidSequence)
    ));
    assert_eq!(
        s.push(18, 5376, &x[1024..2048], |m, _| assert_eq!(
            m.first_sample,
            4352
        ))
        .unwrap(),
        1
    );
    assert!(matches!(
        s.push(u64::MAX, 0, &x[..1], |_, _| {}),
        Err(Error::InvalidSequence)
    ));
}
#[test]
fn transfer_complex_interpolation_matches_dtft_at_48_and_96k() {
    let mut a = TransferAnalyzer::new(65536).unwrap();
    let impulse: Vec<_> = (0..4096).map(|i| 0.02 * 0.98f64.powi(i)).collect();
    for sr in [48000.0, 96000.0] {
        let mut f = vec![0.0; 256];
        log_frequencies(sr, 20.0, 20000.0, &mut f).unwrap();
        let mut points = vec![TransferPoint::default(); 256];
        let meta = a
            .analyze(&impulse, sr, &f, -100.0, &mut points, None)
            .unwrap();
        assert!(meta.tail_converged);
        assert_eq!(meta.sample_rate, sr);
        for p in &points {
            let (db, phase) = dtft(&impulse, p.hz, sr);
            assert!((db - p.db).abs() < 0.002, "{}: {}", p.hz, db - p.db);
            assert!((phase - p.phase.unwrap()).abs() < 0.001);
        }
    }
}
#[test]
fn transfer_unwrap_mask_cancellation_and_tail_diagnostic() {
    let mut a = TransferAnalyzer::new(8192).unwrap();
    let mut impulse = vec![0.0; 2048];
    impulse[16] = 1.0;
    let f: Vec<_> = (0..513).map(|i| i as f64 * 24000.0 / 512.0).collect();
    let mut out = vec![TransferPoint::default(); f.len()];
    a.analyze(&impulse, 48000.0, &f, -100.0, &mut out, None)
        .unwrap();
    for p in &out {
        assert!((p.phase.unwrap() + std::f64::consts::TAU * p.hz * 16.0 / 48000.0).abs() < 1e-10);
    }
    let sparse = [20.0, 1000.0, 5000.0, 20000.0];
    a.analyze(&impulse, 48000.0, &sparse, -100.0, &mut out[..4], None)
        .unwrap();
    for p in &out[..4] {
        assert!((p.phase.unwrap() + std::f64::consts::TAU * p.hz * 16.0 / 48000.0).abs() < 1e-5);
    }
    a.analyze(&vec![0.0; 2048], 48000.0, &f, -100.0, &mut out, None)
        .unwrap();
    assert!(out.iter().all(|p| p.phase.is_none() && p.db.is_finite()));
    let slow: Vec<_> = (0..2048).map(|i| 0.9999f64.powi(i)).collect();
    assert!(
        !a.analyze(&slow, 48000.0, &f, -100.0, &mut out, None)
            .unwrap()
            .tail_converged
    );
    let clock = RequestClock::default();
    let stale = clock.next();
    let _new = clock.next();
    assert!(matches!(
        a.analyze(&impulse, 48000.0, &f, -100.0, &mut out, Some(&stale)),
        Err(Error::Superseded)
    ));
    assert!(matches!(
        a.analyze(
            &impulse,
            48000.0,
            &[100.0, 50.0],
            -100.0,
            &mut out[..2],
            None
        ),
        Err(Error::InvalidFrequency)
    ));
}
#[test]
fn resonant_transfer_dtft_agreement_and_tail() {
    let r = 0.999f64;
    let sr = 48000.0;
    let w = std::f64::consts::TAU * 2000.0 / sr;
    let impulse: Vec<_> = (0..32768)
        .map(|i| r.powi(i) * (w * i as f64).sin())
        .collect();
    let f: Vec<_> = (0..129)
        .map(|i| 1900.0 + i as f64 * 200.0 / 128.0)
        .collect();
    let mut out = vec![TransferPoint::default(); f.len()];
    let mut a = TransferAnalyzer::new(262144).unwrap();
    let meta = a.analyze(&impulse, sr, &f, -100.0, &mut out, None).unwrap();
    assert!(meta.tail_converged);
    for p in &out {
        let (db, phase) = dtft(&impulse, p.hz, sr);
        assert!((db - p.db).abs() < 0.002);
        assert!((phase - p.phase.unwrap()).abs() < 0.0005);
    }
}
#[test]
fn invalid_configuration_is_rejected() {
    for fft_size in [0, 1024, 3000, 16384] {
        assert!(SpectrumAnalyzer::new(Config {
            fft_size,
            ..Config::default()
        })
        .is_err());
    }
    assert!(Stft::new(Config::default(), 0).is_err());
    assert!(SpectrumAnalyzer::new(Config {
        average_alpha: 0.0,
        ..Config::default()
    })
    .is_err());
    assert!(SpectrumAnalyzer::new(Config {
        window: Window::Kaiser { beta: f32::NAN },
        ..Config::default()
    })
    .is_err());
    assert!(TransferAnalyzer::new(4095).is_err());
}

#[test]
fn realfft_forward_inverse_roundtrip_at_all_supported_sizes() {
    let mut planner = realfft::RealFftPlanner::<f32>::new();
    for n in [2048, 4096, 8192] {
        let original = tone(n, 37.3, 0.5);
        let forward = planner.plan_fft_forward(n);
        let inverse = planner.plan_fft_inverse(n);
        let mut input = original.clone();
        let mut bins = forward.make_output_vec();
        let mut output = inverse.make_output_vec();
        forward.process(&mut input, &mut bins).unwrap();
        inverse.process(&mut bins, &mut output).unwrap();
        for (actual, expected) in output.iter().zip(original) {
            close(*actual / n as f32, expected, 1e-6);
        }
    }
}
