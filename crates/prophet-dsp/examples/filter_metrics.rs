//! Coherent harmonic projections (independent of Agent A's FFT implementation).
//! Digital-model diagnostics only; no physical circuit comparison is claimed.
use prophet_dsp::{vcf_cem3320::Cem3320Filter, vcf_moog::MoogLadderFilter};
const N: usize = 16384;
fn measure(
    mut process: impl FnMut(f32) -> f32,
    sr: f64,
    amplitude: f64,
    two_tone: bool,
) -> (f64, f64, f64, f64, f64) {
    let mut re = [0f64; 10];
    let mut im = [0f64; 10];
    let mut peak = 0f64;
    let frequencies = [
        375., 750., 1125., 1500., 1875., 2250., 2625., 3000., 140.625, 843.75,
    ];
    for n in 0..N * 2 {
        let t = n as f64 / sr;
        let input = amplitude
            * ((std::f64::consts::TAU * 375. * t).sin()
                + if two_tone {
                    (std::f64::consts::TAU * 609.375 * t).sin()
                } else {
                    0.
                });
        let y = process(input as f32) as f64;
        assert!(y.is_finite());
        peak = peak.max(y.abs());
        if n >= N {
            for j in 0..10 {
                let angle = std::f64::consts::TAU * frequencies[j] * t;
                re[j] += y * angle.cos();
                im[j] -= y * angle.sin();
            }
        }
    }
    let a: [f64; 10] = std::array::from_fn(|j| 2. * re[j].hypot(im[j]) / N as f64);
    let db = |value: f64| 20. * (value / a[0].max(1e-30)).max(1e-20).log10();
    (
        db(a[1]),
        db(a[2]),
        db(a[1..8].iter().map(|x| x * x).sum::<f64>().sqrt()),
        db((a[8] * a[8] + a[9] * a[9]).sqrt()),
        peak,
    )
}
fn main() {
    println!("sample_rate,filter,cutoff,resonance,drive,input_peak_per_tone,test,h2_dbc,h3_dbc,thd_2_to_8_dbc,selected_imd3_dbc,peak");
    for sr in [48000.0f32, 96000.] {
        for fc in [80., 2000., 10000.] {
            for resonance in [0.2, 0.8] {
                for amplitude in [0.01, 0.1, 0.5] {
                    for two in [false, true] {
                        for moog in [false, true] {
                            let mut c = Cem3320Filter::new(sr, 1.);
                            c.set_params(fc, resonance, 1.8);
                            let mut m = MoogLadderFilter::new(sr, 1.);
                            m.set_params(fc, resonance, 1.8, 0.5);
                            let (h2, h3, thd, imd, peak) = measure(
                                |x| {
                                    if moog {
                                        m.process_sample(x)
                                    } else {
                                        c.process_sample(x)
                                    }
                                },
                                sr as f64,
                                amplitude,
                                two,
                            );
                            println!("{sr},{},{fc},{resonance},1.8,{amplitude},{},{h2:.4},{h3:.4},{thd:.4},{imd:.4},{peak:.6}",if moog{"moog"}else{"curtis"},if two{"two-tone"}else{"sine"});
                        }
                    }
                }
            }
        }
    }
}
