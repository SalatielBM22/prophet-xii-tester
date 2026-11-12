//! Native worker compute microbenchmarks. Not an audio callback deadline claim.
use prophet_analysis::*;
use realfft::RealFftPlanner;
use rustfft::{num_complex::Complex32, FftPlanner};
use std::{hint::black_box, time::Instant};

fn measure(mut f: impl FnMut(), warmup: usize, count: usize) -> [u128; 4] {
    for _ in 0..warmup {
        f();
    }
    let mut ns = Vec::with_capacity(count);
    for _ in 0..count {
        let t = Instant::now();
        f();
        ns.push(t.elapsed().as_nanos());
    }
    ns.sort_unstable();
    [
        ns[count / 2],
        ns[(count * 95 / 100).min(count - 1)],
        ns[(count * 99 / 100).min(count - 1)],
        ns[count - 1],
    ]
}
fn row(name: &str, n: usize, stats: [u128; 4], bytes: usize, warmup: usize, count: usize) {
    println!(
        "{name},{n},{},{},{},{},{bytes},{warmup},{count}",
        stats[0], stats[1], stats[2], stats[3]
    );
}
fn main() {
    println!("method,n,p50_ns,p95_ns,p99_ns,max_ns,retained_buffer_bytes,warmup,iterations");
    for n in [2048, 4096, 8192] {
        let x: Vec<_> = (0..n)
            .map(|i| (std::f32::consts::TAU * 37.3 * i as f32 / n as f32).sin())
            .collect();
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let mut input = plan.make_input_vec();
        let mut output = plan.make_output_vec();
        let mut scratch = plan.make_scratch_vec();
        let stats = measure(
            || {
                input.copy_from_slice(&x);
                plan.process_with_scratch(&mut input, &mut output, &mut scratch)
                    .unwrap();
                black_box(&output);
            },
            100,
            2000,
        );
        row(
            "realfft_copy_r2c",
            n,
            stats,
            input.len() * 4 + (output.len() + scratch.len()) * 8,
            100,
            2000,
        );
        let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
        let mut complex = vec![Complex32::default(); n];
        let mut c_scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        let stats = measure(
            || {
                for (v, &x) in complex.iter_mut().zip(&x) {
                    *v = Complex32::new(x, 0.0);
                }
                fft.process_with_scratch(&mut complex, &mut c_scratch);
                black_box(&complex);
            },
            100,
            2000,
        );
        row(
            "rustfft_copy_c2c",
            n,
            stats,
            (complex.len() + c_scratch.len()) * 8,
            100,
            2000,
        );
        let window: Vec<_> = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
            .collect();
        let stats = measure(
            || {
                for ((out, x), w) in input.iter_mut().zip(&x).zip(&window) {
                    *out = x * w;
                }
                black_box(&input);
            },
            100,
            2000,
        );
        row(
            "cached_hann_apply_only",
            n,
            stats,
            (window.len() + input.len()) * 4,
            100,
            2000,
        );
        let mut analyzer = SpectrumAnalyzer::new(Config {
            fft_size: n,
            ..Config::default()
        })
        .unwrap();
        let stats = measure(
            || {
                black_box(analyzer.analyze(&x).unwrap());
            },
            100,
            1000,
        );
        row(
            "hann_r2c_power_db_peak_psd",
            n,
            stats,
            analyzer.memory_bytes(),
            100,
            1000,
        );
        let hop = n / 4;
        let mut stft = Stft::new(
            Config {
                fft_size: n,
                ..Config::default()
            },
            hop,
        )
        .unwrap();
        let mut sequence = 0u64;
        let stats = measure(
            || {
                stft.push(sequence, sequence * hop as u64, &x[..hop], |meta, view| {
                    black_box((meta, view));
                })
                .unwrap();
                sequence += 1;
            },
            100,
            1000,
        );
        row(
            "stft_packet_hop_n4_with_analysis",
            n,
            stats,
            stft.memory_bytes(),
            100,
            1000,
        );
    }
    let impulse: Vec<_> = (0..4096).map(|i| 0.02 * 0.98f64.powi(i)).collect();
    let mut f = vec![0.0; 256];
    log_frequencies(48000.0, 20.0, 20000.0, &mut f).unwrap();
    let mut analyzer = TransferAnalyzer::new(65536).unwrap();
    let mut out = vec![TransferPoint::default(); 256];
    let stats = measure(
        || {
            black_box(
                analyzer
                    .analyze(&impulse, 48000.0, &f, -100.0, &mut out, None)
                    .unwrap(),
            );
        },
        20,
        200,
    );
    row(
        "transfer_fft65536_log256",
        4096,
        stats,
        analyzer.memory_bytes(),
        20,
        200,
    );
    let stats = measure(
        || {
            for &freq in &f {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, &x) in impulse.iter().enumerate() {
                    let a = -std::f64::consts::TAU * freq * i as f64 / 48000.0;
                    re += x * a.cos();
                    im += x * a.sin();
                }
                black_box((re, im));
            }
        },
        2,
        32,
    );
    row("baseline_dtft4096_log256", 4096, stats, 0, 2, 32);
}
