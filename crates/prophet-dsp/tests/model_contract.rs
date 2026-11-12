//! Independent digital linear-response oracle; does not assert hardware fidelity.
use prophet_dsp::{env_cem3310::*, vcf_cem3320::*, vcf_moog::*};
#[test]
fn four_pole_small_signal_matches_bilinear_analytic_transfer() {
    for sr in [48000.0f32, 96000.] {
        for fc in [80., 2000., 18000.] {
            for bin in [8, 128, 512] {
                let mut c = Cem3320Filter::new(sr, 1.);
                c.set_params(fc, 0., 1.);
                let mut m = MoogLadderFilter::new(sr, 1.);
                m.set_params(fc, 0., 1., 0.5);
                let w = std::f64::consts::TAU * bin as f64 / 4096.;
                let mut z = [[0f64; 2]; 2];
                let amp = 0.001;
                for n in 0..8192 {
                    let x = (w * n as f64).cos() * amp;
                    let ys = [c.process_sample(x as f32), m.process_sample(x as f32)];
                    if n >= 4096 {
                        for j in 0..2 {
                            z[j][0] += ys[j] as f64 * (w * n as f64).cos();
                            z[j][1] -= ys[j] as f64 * (w * n as f64).sin();
                        }
                    }
                }
                let g = (std::f64::consts::PI * fc as f64 / sr as f64).tan();
                let ratio = (w * 0.5).tan() / g;
                let mag = (1. + ratio * ratio).powi(-2);
                let phase = -4. * ratio.atan();
                let expected = [mag * phase.cos(), mag * phase.sin()];
                for (name, actual) in ["curtis", "moog"].iter().zip(z) {
                    for i in 0..2 {
                        let measured = actual[i] * 2. / 4096. / amp;
                        assert!(
                            (measured - expected[i]).abs() < 2e-5 + mag * 0.005,
                            "{name} sr={sr} fc={fc} bin={bin} component={i} {measured} vs {}",
                            expected[i]
                        );
                    }
                }
            }
        }
    }
}
#[test]
fn rational_saturator_derivative_matches_independent_finite_difference() {
    for i in -299..300 {
        let x = i as f32 * 0.01;
        let h = 0.001;
        let numerical = (fast_tanh(x + h) - fast_tanh(x - h)) / (2. * h);
        assert!((numerical - fast_sech2(x)).abs() < 0.0002);
    }
}
#[test]
fn envelope_progress_uses_time_and_reports_indefinite_sustain() {
    for sr in [48000.0f32, 96000.] {
        for attack in [0.001, 1.] {
            for sustain in [0., 0.5, 1.] {
                let mut e = Cem3310Envelope::new(sr, 1.);
                e.set_adsr(attack, 0.01, sustain, 0.01);
                e.note_on(true);
                let half = (attack * sr * 0.5) as usize;
                for _ in 0..half {
                    e.process_sample();
                }
                let t = e.telemetry();
                assert_eq!(t.stage, EnvStage::Attack as u8);
                assert_eq!(t.elapsed_samples, half as u64);
                assert!((t.progress - 0.5).abs() < 0.03, "{}", t.progress);
                for _ in 0..(sr * (attack + 0.1)) as usize {
                    e.process_sample();
                }
                assert_eq!(e.stage, EnvStage::Sustain);
                assert!(!e.telemetry().timed);
                e.note_off();
                assert_eq!(e.stage_elapsed_samples, 0);
                for _ in 0..(sr * 0.1) as usize {
                    e.process_sample();
                }
                assert_eq!(e.stage, EnvStage::Idle);
                assert_eq!(e.telemetry().progress, 0.);
            }
        }
    }
}
#[test]
fn morph_does_not_reintroduce_frozen_filter_state() {
    let mut f = DualVcfStage::new(48000., 1.);
    f.set_params(2000., 0.8, 1.8, 0.5, 1.);
    for _ in 0..500 {
        f.process_sample(0.3);
    }
    assert!(f.moog_ladder.s.iter().any(|s| s.abs() > 0.001));
    f.set_params(2000., 0.8, 1.8, 0.5, 0.);
    for _ in 0..500 {
        f.process_sample(0.);
    }
    f.set_params(2000., 0.8, 1.8, 0.5, 0.001);
    assert_eq!(f.moog_ladder.s, [0.; 4]);
}
#[test]
fn linear_morph_ramp_has_bounded_dc_transition_in_both_directions() {
    let mut f = DualVcfStage::new(48000., 1.);
    f.set_params(2000., 0., 1., 0.5, 0.);
    let mut last = 0.;
    for _ in 0..4096 {
        last = f.process_sample(0.2);
    }
    for direction in [true, false] {
        for i in 1..=240 {
            let t = i as f32 / 240.;
            f.set_params(2000., 0., 1., 0.5, if direction { t } else { 1. - t });
            let next = f.process_sample(0.2);
            assert!((next - last).abs() < 0.01, "jump {}", next - last);
            last = next;
        }
    }
}
#[test]
fn oversampling_reported_latency_matches_generation_impulse() {
    use prophet_dsp::oversample::*;
    for factor in [
        OversampleFactor::X1,
        OversampleFactor::X2,
        OversampleFactor::X4,
    ] {
        let mut os = VoiceOversampler::new(factor);
        let mut high_frame = 0;
        let mut ys = [0f32; 128];
        for y in &mut ys {
            *y = os.process_voice_sample(|| {
                let x = if high_frame == 0 { 1. } else { 0. };
                high_frame += 1;
                x
            });
        }
        let peak = ys
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert_eq!(peak, factor.latency_samples() as usize);
    }
}
#[test]
fn chorus_tail_reaches_exact_silence_without_output_subnormals() {
    use prophet_dsp::fx::juno_chorus::*;
    for sr in [48000., 96000.] {
        for mode in [
            JunoChorusMode::ModeI,
            JunoChorusMode::ModeII,
            JunoChorusMode::ModeIPlusII,
        ] {
            let mut c = Juno106BbdChorus::new(sr);
            c.mode = mode;
            c.wet_mix = 1.;
            c.bbd_noise_level = 0.;
            let mut energy = 0.;
            for i in 0..(sr * 3.) as usize {
                let x = if i < 480 { 0.1 } else { 0. };
                let (l, r) = c.process_stereo(x, -x);
                assert!(l.is_finite() && r.is_finite() && !l.is_subnormal() && !r.is_subnormal());
                energy += l * l + r * r;
                if i > (sr * 2.5) as usize {
                    assert_eq!((l, r), (0., 0.));
                }
            }
            assert!(energy > 0.01);
        }
    }
}
