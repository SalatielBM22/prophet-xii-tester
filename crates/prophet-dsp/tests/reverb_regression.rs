//! Reproductions for #1/#2. No source/UI artifacts are overwritten.
use prophet_dsp::fx::solaris_reverb::*;
fn isolated() -> SolarisReverbParams {
    SolarisReverbParams {
        wet_mix: 1.,
        pre_delay_ms: 0.,
        decay_sec: 0.3,
        ensemble_mode: EnsembleMode::Off,
        shimmer_amount: 0.,
        vowel_mix: 0.,
        mod_depth: 0.,
        ..Default::default()
    }
}
#[test]
fn zero_wet_must_not_suspend_a_stale_tail() {
    let mut r = SolarisCloudburstReverb::new(48000.);
    let mut p = isolated();
    for i in 0..4800 {
        r.process_stereo(if i == 0 { 0.5 } else { 0. }, 0., &p);
    }
    p.wet_mix = 0.;
    for _ in 0..144000 {
        assert_eq!(r.process_stereo(0., 0., &p), (0., 0.));
    }
    p.wet_mix = 1.;
    let mut energy = 0.;
    for _ in 0..9600 {
        let (l, rr) = r.process_stereo(0., 0., &p);
        energy += l * l + rr * rr;
    }
    assert!(energy < 1e-12, "old tail resumed: {energy}");
}
#[test]
fn bypass_kill_spill_and_reenable_are_explicit() {
    for sr in [48000., 96000.] {
        for policy in [ReverbTailPolicy::Kill, ReverbTailPolicy::Spill] {
            let mut r = SolarisCloudburstReverb::new(sr);
            let mut p = isolated();
            p.bypass_tail = policy;
            for i in 0..(sr * 0.1) as usize {
                r.process_stereo(if i == 0 { 0.5 } else { 0. }, 0., &p);
            }
            p.enabled = false;
            let mut tail = 0.;
            for i in 0..(sr * 0.1) as usize {
                let (l, rr) = r.process_stereo(0., 0., &p);
                if i > (sr * 0.006) as usize {
                    tail += l * l + rr * rr;
                }
            }
            if policy == ReverbTailPolicy::Kill {
                assert_eq!(tail, 0.);
                assert!(r.is_dormant());
            } else {
                assert!(tail > 1e-10);
            }
            if policy == ReverbTailPolicy::Kill {
                p.enabled = true;
                for _ in 0..(sr * 0.2) as usize {
                    assert_eq!(r.process_stereo(0., 0., &p), (0., 0.));
                }
            }
        }
    }
}
#[test]
fn kill_fades_dry_gain_and_accepts_new_audio_without_old_history() {
    let mut r = SolarisCloudburstReverb::new(48000.);
    let mut p = isolated();
    p.enabled = false;
    p.bypass_tail = ReverbTailPolicy::Kill;
    let mut last = 0.;
    for _ in 0..300 {
        let (l, _) = r.process_stereo(0.2, 0.2, &p);
        assert!((l - last).abs() < 0.003);
        last = l;
    }
    assert!((last - 0.2).abs() < 1e-6);
    p.enabled = true;
    let mut energy = 0.;
    for i in 0..9600 {
        let (l, rr) = r.process_stereo(if i < 100 { 0.2 } else { 0. }, 0., &p);
        energy += l * l + rr * rr;
    }
    assert!(energy > 0.001);
}
#[test]
fn reset_invalidates_every_delay_and_restores_rng() {
    let mut old = SolarisCloudburstReverb::new(48000.);
    let p = SolarisReverbParams {
        wet_mix: 1.,
        ..Default::default()
    };
    for i in 0..96000 {
        old.process_stereo(if i % 71 == 0 { 0.4 } else { 0. }, 0., &p);
    }
    old.reset();
    let mut fresh = SolarisCloudburstReverb::new(48000.);
    for i in 0..48000 {
        let x = if i % 101 == 0 { 0.2 } else { 0. };
        assert_eq!(
            old.process_stereo(x, 0., &p),
            fresh.process_stereo(x, 0., &p),
            "frame {i}"
        );
    }
}
#[test]
fn wet_dry_law_is_affine_for_the_same_tank() {
    let mut a = SolarisCloudburstReverb::new(48000.);
    let mut b = SolarisCloudburstReverb::new(48000.);
    let mut p = isolated();
    let mut q = p;
    q.wet_mix = 0.25;
    for i in 0..9600 {
        let x = if i % 73 == 0 { 0.1 } else { 0. };
        let (w, _) = a.process_stereo(x, 0., &p);
        let (y, _) = b.process_stereo(x, 0., &q);
        assert!((y - (x * 0.75 + w * 0.25)).abs() < 1e-7);
    }
    p.wet_mix = 0.;
    assert_eq!(a.process_stereo(0.25, -0.25, &p), (0.25, -0.25));
}
#[test]
fn ensemble_shimmer_sensitivity_and_freeze_are_finite() {
    for sr in [48000., 96000.] {
        let mut a = SolarisCloudburstReverb::new(sr);
        let mut b = SolarisCloudburstReverb::new(sr);
        let mut p = isolated();
        p.decay_sec = 1.;
        let mut q = p;
        q.ensemble_mode = EnsembleMode::Forte;
        q.shimmer_amount = 0.8;
        let mut difference = 0.;
        for i in 0..sr as usize {
            let x = if i % 97 == 0 { 0.1 } else { 0. };
            let (l, r) = a.process_stereo(x, x, &p);
            let (ll, rr) = b.process_stereo(x, x, &q);
            assert!([l, r, ll, rr].iter().all(|v| v.is_finite() && v.abs() < 8.));
            difference += (l - ll).powi(2) + (r - rr).powi(2);
        }
        assert!(difference > 0.001);
        q.freeze = true;
        for _ in 0..sr as usize {
            let (l, r) = b.process_stereo(1., 1., &q);
            assert!(l.is_finite() && r.is_finite() && l.abs() < 8. && r.abs() < 8.);
        }
    }
}
#[test]
fn freeze_does_not_record_new_excitation_in_predelay() {
    let mut a = SolarisCloudburstReverb::new(48000.);
    let mut b = SolarisCloudburstReverb::new(48000.);
    let mut p = isolated();
    p.pre_delay_ms = 100.;
    for i in 0..9600 {
        let x = if i == 0 { 0.2 } else { 0. };
        a.process_stereo(x, x, &p);
        b.process_stereo(x, x, &p);
    }
    p.freeze = true;
    for _ in 0..9600 {
        assert_eq!(a.process_stereo(0., 0., &p), b.process_stereo(1., 1., &p));
    }
    p.freeze = false;
    for _ in 0..9600 {
        assert_eq!(a.process_stereo(0., 0., &p), b.process_stereo(0., 0., &p));
    }
}
#[test]
fn fresh_zero_wet_stays_dormant_and_natural_tails_eventually_sleep() {
    let mut dry = SolarisCloudburstReverb::new(48000.);
    let mut p = isolated();
    p.wet_mix = 0.;
    for _ in 0..9600 {
        assert_eq!(dry.process_stereo(0.3, 0.3, &p), (0.3, 0.3));
    }
    assert!(dry.is_dormant());
    for sr in [48000., 96000.] {
        let mut r = SolarisCloudburstReverb::new(sr);
        p = isolated();
        for i in 0..(sr * 10.) as usize {
            let (l, rr) = r.process_stereo(if i == 0 { 0.2 } else { 0. }, 0., &p);
            assert!(l.is_finite() && rr.is_finite() && !l.is_subnormal() && !rr.is_subnormal());
        }
        assert!(r.is_dormant());
    }
}

fn goertzel_power(samples: &[f32], sr: f64, freq_hz: f64) -> f64 {
    let w = 2.0 * core::f64::consts::PI * freq_hz / sr;
    let coeff = 2.0 * w.cos();
    let mut s_prev = 0.0f64;
    let mut s_prev2 = 0.0f64;
    for &x in samples {
        let s = (x as f64) + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }
    let power = s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2;
    power.max(0.0) / (samples.len().max(1) as f64)
}

fn schroeder_t20_t30_r2(energy: &[f64], sr: f64, db_low: f64, db_high: f64) -> (f64, f64) {
    let mut cum = vec![0.0f64; energy.len()];
    let mut acc = 0.0f64;
    for i in (0..energy.len()).rev() {
        acc += energy[i];
        cum[i] = acc;
    }
    let norm = cum[0].max(1e-30);
    let (mut n, mut x, mut y, mut xx, mut yy, mut xy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, &c) in cum.iter().enumerate() {
        let db = 10.0 * (c / norm).max(1e-30).log10();
        if db >= db_low && db <= db_high {
            let t = i as f64 / sr;
            n += 1.0;
            x += t;
            y += db;
            xx += t * t;
            yy += db * db;
            xy += t * db;
        }
    }
    assert!(n > 20.0, "insufficient regression samples");
    let dx = n * xx - x * x;
    let dy = n * yy - y * y;
    let slope = (n * xy - x * y) / dx;
    let r2 = (n * xy - x * y).powi(2) / (dx * dy.max(1e-30));
    (-60.0 / slope, r2)
}

#[test]
fn calibrated_rt60_tracks_targets_across_48k_and_96k() {
    for sr in [48000.0f32, 96000.0f32] {
        let mut prev_artistic = 0.0f64;
        for &target in &[0.5f32, 1.5, 4.8, 10.0] {
            let frames = ((target as f64 * 4.0 + 1.0) * sr as f64) as usize;
            // Neutral broadband profile: T20 and T30 within 3% of requested RT60 with R^2 > 0.999
            let mut r_neu = SolarisCloudburstReverb::new(sr);
            let p_neu = SolarisReverbParams {
                wet_mix: 1.0,
                pre_delay_ms: 0.0,
                decay_sec: target,
                ensemble_mode: EnsembleMode::Off,
                shimmer_amount: 0.0,
                vowel_mix: 0.0,
                mod_depth: 0.0,
                high_cut_hz: 18000.0,
                low_cut_hz: 20.0,
                ..Default::default()
            };
            let mut energy_neu = Vec::with_capacity(frames);
            let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
            for i in 0..frames {
                let x = if i == 0 { 0.5 } else { 0.0 };
                let (l, r) = r_neu.process_stereo(x, 0.0, &p_neu);
                let (lf, rf) = (l as f64, r as f64);
                lr += lf * rf;
                ll += lf * lf;
                rr += rf * rf;
                energy_neu.push(lf * lf + rf * rf);
            }
            let (t20, r2_20) = schroeder_t20_t30_r2(&energy_neu, sr as f64, -25.0, -5.0);
            let (t30, r2_30) = schroeder_t20_t30_r2(&energy_neu, sr as f64, -35.0, -5.0);
            let rel_err_20 = ((t20 - target as f64) / target as f64).abs();
            let rel_err_30 = ((t30 - target as f64) / target as f64).abs();
            let corr = lr / (ll * rr).sqrt().max(1e-30);
            assert!(rel_err_20 < 0.03, "sr={sr} target={target} t20={t20}");
            assert!(rel_err_30 < 0.03, "sr={sr} target={target} t30={t30}");
            assert!(r2_20 > 0.999 && r2_30 > 0.999, "sr={sr} target={target} r2=({r2_20},{r2_30})");
            assert!(corr.abs() < 0.10, "sr={sr} target={target} stereo_corr={corr}");

            // Artistic default profile: smooth linear log decay and strictly monotonic vs target
            let mut r_art = SolarisCloudburstReverb::new(sr);
            let p_art = SolarisReverbParams {
                wet_mix: 1.0,
                pre_delay_ms: 0.0,
                decay_sec: target,
                ..Default::default()
            };
            let mut energy_art = Vec::with_capacity(frames);
            for i in 0..frames {
                let x = if i == 0 { 0.5 } else { 0.0 };
                let (l, r) = r_art.process_stereo(x, 0.0, &p_art);
                energy_art.push((l as f64) * (l as f64) + (r as f64) * (r as f64));
            }
            let (t20_art, r2_art) = schroeder_t20_t30_r2(&energy_art, sr as f64, -25.0, -5.0);
            assert!(r2_art > 0.998, "artistic R^2 too low: {r2_art}");
            assert!(t20_art > prev_artistic, "artistic decay not monotonic: {t20_art} <= {prev_artistic}");
            prev_artistic = t20_art;
        }
    }
}

#[test]
fn shimmer_octave_and_fifth_generate_expected_pitch_shifted_partials() {
    for sr in [48000.0f32, 96000.0f32] {
        let len = (sr * 1.2) as usize;
        let skip = (sr * 0.25) as usize;
        let drive_hz = 440.0f64;

        let render = |shimmer: f32, fifth: f32| -> Vec<f32> {
            let mut r = SolarisCloudburstReverb::new(sr);
            let p = SolarisReverbParams {
                wet_mix: 1.0,
                pre_delay_ms: 0.0,
                decay_sec: 2.5,
                ensemble_mode: EnsembleMode::Off,
                shimmer_amount: shimmer,
                shimmer_fifth_blend: fifth,
                shimmer_semitones: 12.0,
                vowel_mix: 0.0,
                mod_depth: 0.0,
                high_cut_hz: 16000.0,
                low_cut_hz: 40.0,
                ..Default::default()
            };
            let mut out = Vec::with_capacity(len - skip);
            for i in 0..len {
                let phase = 2.0 * core::f64::consts::PI * drive_hz * (i as f64) / (sr as f64);
                let x = if i < (sr * 0.4) as usize {
                    0.25 * (phase.sin() as f32)
                } else {
                    0.0
                };
                let (l, right) = r.process_stereo(x, x, &p);
                if i >= skip {
                    // Use linear stereo difference (L - R)/2 so anti-phase shimmer tap is doubled without introducing rectification harmonics
                    out.push(0.5 * (l - right));
                }
            }
            out
        };

        let dry_shimmer = render(0.0, 0.0);
        let oct = render(0.75, 0.0);
        let fifth = render(0.75, 1.0);

        let find_peak = |buf: &[f32], f_lo: f64, f_hi: f64| -> (f64, f64) {
            let mut best = (0.0f64, -1.0f64);
            let mut f = f_lo;
            while f <= f_hi {
                let p = goertzel_power(buf, sr as f64, f);
                if p > best.1 {
                    best = (f, p);
                }
                f += 0.5;
            }
            best
        };

        let oct_base_880 = find_peak(&dry_shimmer, 840.0, 920.0).1;
        let oct_880 = find_peak(&oct, 840.0, 920.0).1;
        let fifth_base_659 = find_peak(&dry_shimmer, 620.0, 700.0).1;
        let fifth_659 = find_peak(&fifth, 620.0, 700.0).1;
        let fifth_at_880 = find_peak(&fifth, 840.0, 920.0).1;

        assert!(oct_880 > oct_base_880 * 100.0 + 1e-6, "sr={sr} +12st failed: {oct_880} vs {oct_base_880}");
        assert!(fifth_659 > fifth_base_659 * 100.0 + 1e-6, "sr={sr} +7st failed: {fifth_659} vs {fifth_base_659}");
        assert!(oct_880 > fifth_at_880 * 4.0, "sr={sr} +12st vs +7st separation: {oct_880} vs {fifth_at_880}");
    }
}

#[test]
fn vowel_formant_morph_selects_ae_iou_spectral_peaks() {
    let sr = 48000.0f32;
    let f_centers = [730.0, 1840.0, 2290.0, 840.0, 300.0]; // distinctive F1/F2 probes for A, E, I, O, U
    let morphs = [0.0f32, 0.25, 0.5, 0.75, 1.0];
    let mut responses = Vec::new();
    for &morph in &morphs {
        let mut vf = SolarisVowelFilter::default();
        let mut state = 0x9E37_79B9u32;
        let len = 48000usize;
        let mut out = Vec::with_capacity(len - 4800);
        for i in 0..len {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let noise = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            let y = vf.process(noise * 0.25, morph, sr);
            assert!(y.is_finite());
            if i >= 4800 {
                out.push(y);
            }
        }
        responses.push(out);
    }
    // Vowel A (morph=0.0, F1=730) > Vowel I at 730 Hz; Vowel I (morph=0.5, F2=2290) > Vowel U at 2290 Hz
    let a_730 = goertzel_power(&responses[0], sr as f64, 730.0);
    let i_730 = goertzel_power(&responses[2], sr as f64, 730.0);
    let i_2290 = goertzel_power(&responses[2], sr as f64, 2290.0);
    let u_2290 = goertzel_power(&responses[4], sr as f64, 2290.0);
    let e_1840 = goertzel_power(&responses[1], sr as f64, 1840.0);
    let u_1840 = goertzel_power(&responses[4], sr as f64, 1840.0);
    assert!(a_730 > i_730 * 3.0, "Vowel A F1(730) should exceed I: {a_730} vs {i_730}");
    assert!(e_1840 > u_1840 * 3.0, "Vowel E F2(1840) should exceed U: {e_1840} vs {u_1840}");
    assert!(i_2290 > u_2290 * 3.0, "Vowel I F2(2290) should exceed U: {i_2290} vs {u_2290}");

    // Ensure every adjacent vowel pair A/E/I/O/U produces a distinct spectrum across formant probes
    for v_idx in 0..4 {
        let mut diff_sum = 0.0;
        for &probe_f in &f_centers {
            let p0 = goertzel_power(&responses[v_idx], sr as f64, probe_f);
            let p1 = goertzel_power(&responses[v_idx + 1], sr as f64, probe_f);
            diff_sum += (p0 - p1).abs() / (p0 + p1 + 1e-12);
        }
        assert!(diff_sum > 0.5, "Vowels {v_idx} and {} insufficiently distinct", v_idx + 1);
    }
}

#[test]
fn ensemble_modes_differentiate_harmonics_and_stereo_spread() {
    let sr = 48000.0f32;
    let run_mode = |mode: EnsembleMode, broadband: bool| -> (Vec<f32>, f64, f64) {
        let mut r = SolarisCloudburstReverb::new(sr);
        let p = SolarisReverbParams {
            wet_mix: 1.0,
            pre_delay_ms: 0.0,
            decay_sec: 1.2,
            ensemble_mode: mode,
            ensemble_blend: 0.8,
            shimmer_amount: 0.0,
            vowel_mix: 0.0,
            mod_depth: 0.25,
            ..Default::default()
        };
        let n = 48000usize;
        let skip = 12000usize;
        let mut diff = Vec::with_capacity(n - skip);
        let (mut lr, mut ll, mut rr, mut rms) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let mut rng = 0x5A17_9B31u32;
        for i in 0..n {
            let x = if broadband {
                rng ^= rng << 13;
                rng ^= rng >> 17;
                rng ^= rng << 5;
                0.25 * ((rng as f32 / u32::MAX as f32) * 2.0 - 1.0)
            } else {
                0.25 * (2.0 * core::f32::consts::PI * 220.0 * (i as f32) / sr).sin()
            };
            let (l, right) = r.process_stereo(x, x, &p);
            if i >= skip {
                let (lf, rf) = (l as f64, right as f64);
                lr += lf * rf;
                ll += lf * lf;
                rr += rf * rf;
                rms += 0.5 * (lf * lf + rf * rf);
                diff.push(l - right);
            }
        }
        let corr = lr / (ll * rr).sqrt().max(1e-30);
        let mean_rms = rms / diff.len() as f64;
        (diff, mean_rms, corr)
    };

    let (off_m, off_e, off_corr) = run_mode(EnsembleMode::Off, false);
    let (mp_m, mp_e, mp_corr) = run_mode(EnsembleMode::MezzoPiano, false);
    let (forte_m, forte_e, forte_corr) = run_mode(EnsembleMode::Forte, false);

    let off_h = goertzel_power(&off_m, sr as f64, 440.0) + goertzel_power(&off_m, sr as f64, 660.0);
    let mp_h = goertzel_power(&mp_m, sr as f64, 440.0) + goertzel_power(&mp_m, sr as f64, 660.0);
    let forte_h = goertzel_power(&forte_m, sr as f64, 440.0) + goertzel_power(&forte_m, sr as f64, 660.0);

    assert!(mp_h > off_h + 1e-6 && forte_h > mp_h * 1.15, "harmonic progression failed: off={off_h} mp={mp_h} forte={forte_h}");
    assert!(forte_e > mp_e && mp_e > off_e && off_e > 0.0, "energy progression failed: off={off_e} mp={mp_e} forte={forte_e}");
    assert!(
        mp_corr.abs() < off_corr.abs() && forte_corr.abs() < off_corr.abs() && forte_corr.abs() < 0.85,
        "sine ensemble decorrelation failed: off={off_corr} mp={mp_corr} forte={forte_corr}"
    );

    let (_, off_bb_e, off_bb_corr) = run_mode(EnsembleMode::Off, true);
    let (_, mp_bb_e, mp_bb_corr) = run_mode(EnsembleMode::MezzoPiano, true);
    let (_, forte_bb_e, forte_bb_corr) = run_mode(EnsembleMode::Forte, true);
    assert!(
        off_bb_corr.abs() < 0.35 && mp_bb_corr.abs() < 0.35 && forte_bb_corr.abs() < 0.35,
        "broadband stereo decorrelation failed: off={off_bb_corr} mp={mp_bb_corr} forte={forte_bb_corr}"
    );
    assert!(
        forte_bb_e > mp_bb_e && mp_bb_e > off_bb_e,
        "broadband energy progression failed: off={off_bb_e} mp={mp_bb_e} forte={forte_bb_e}"
    );
}

#[test]
fn ten_second_freeze_stays_flat_finite_and_decorrelated() {
    for sr in [48000.0f32, 96000.0f32] {
        let mut r = SolarisCloudburstReverb::new(sr);
        let mut p = SolarisReverbParams {
            wet_mix: 1.0,
            pre_delay_ms: 10.0,
            decay_sec: 4.8,
            ensemble_mode: EnsembleMode::Off,
            shimmer_amount: 0.0,
            vowel_mix: 0.0,
            ..Default::default()
        };
        let excite = (sr * 0.25) as usize;
        let mut state = 0x1234_5678u32;
        for _ in 0..excite {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let n = (state as f32 / u32::MAX as f32) * 2.0 - 1.0;
            r.process_stereo(n * 0.25, n * 0.25, &p);
        }
        p.freeze = true;
        // Let predelay/diffuser settle into tank
        for _ in 0..(sr * 0.2) as usize {
            r.process_stereo(0.0, 0.0, &p);
        }
        let sec = sr as usize;
        let mut first_rms = 0.0f64;
        let mut last_rms = 0.0f64;
        let (mut lr, mut ll, mut rr) = (0.0f64, 0.0f64, 0.0f64);
        for s in 0..10 {
            let mut slice = 0.0f64;
            for _ in 0..sec {
                let (l, right) = r.process_stereo(0.0, 0.0, &p);
                assert!(l.is_finite() && right.is_finite() && !l.is_subnormal() && !right.is_subnormal());
                let (lf, rf) = (l as f64, right as f64);
                slice += lf * lf + rf * rf;
                lr += lf * rf;
                ll += lf * lf;
                rr += rf * rf;
            }
            if s == 0 {
                first_rms = (slice / (2.0 * sec as f64)).sqrt();
            }
            if s == 9 {
                last_rms = (slice / (2.0 * sec as f64)).sqrt();
            }
        }
        assert!(first_rms > 1e-4, "sr={sr} freeze too quiet: {first_rms}");
        let drift_db = (20.0 * (last_rms / first_rms).log10()).abs();
        let corr = (lr / (ll * rr).sqrt().max(1e-30)).abs();
        assert!(drift_db < 0.05, "sr={sr} 10s freeze drift_db={drift_db}");
        assert!(corr < 0.25, "sr={sr} 10s freeze stereo_corr={corr}");
    }
}

