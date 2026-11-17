//! Schroeder energy-decay measurement (T20, T30, R^2, multiband <400 / 400-3k / >3k Hz), outside any callback. Issues #1/#4.
use core::f64::consts::{PI, SQRT_2};
use prophet_dsp::fx::solaris_reverb::*;

#[derive(Clone, Copy)]
struct Biquad64 {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad64 {
    fn lowpass(sr: f64, fc: f64) -> Self {
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / SQRT_2;
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 - cos_w0) * 0.5) / a0,
            b1: (1.0 - cos_w0) / a0,
            b2: ((1.0 - cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn highpass(sr: f64, fc: f64) -> Self {
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = w0.cos();
        let alpha = w0.sin() / SQRT_2;
        let a0 = 1.0 + alpha;
        Self {
            b0: ((1.0 + cos_w0) * 0.5) / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: ((1.0 + cos_w0) * 0.5) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline(always)]
    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

fn schroeder_fit(energy: &mut [f64], sr: f64, db_low: f64, db_high: f64) -> (f64, f64) {
    let mut sum = 0.0f64;
    for e in energy.iter_mut().rev() {
        sum += *e;
        *e = sum;
    }
    let norm = energy[0].max(1e-30);
    let (mut count, mut x, mut y, mut xx, mut yy, mut xy) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    for (i, e) in energy.iter().enumerate() {
        let db = 10.0 * (*e / norm).max(1e-30).log10();
        if db >= db_low && db <= db_high {
            let t = i as f64 / sr;
            count += 1.0;
            x += t;
            y += db;
            xx += t * t;
            yy += db * db;
            xy += t * db;
        }
    }
    if count <= 10.0 {
        return (f64::NAN, 0.0);
    }
    let denom_x = count * xx - x * x;
    let denom_y = count * yy - y * y;
    let slope = (count * xy - x * y) / denom_x;
    let r2 = (count * xy - x * y).powi(2) / (denom_x * denom_y.max(1e-30));
    (-60.0 / slope, r2)
}

fn schroeder_t20_only(energy: &[f64], sr: f64, norm: f64) -> f64 {
    let (mut count, mut x, mut y, mut xx, mut xy) = (0f64, 0f64, 0f64, 0f64, 0f64);
    // Cumulative integral already done in energy if reused, or compute backwards on fly:
    let mut cum = 0.0;
    let mut total = 0.0;
    for &e in energy {
        total += e;
    }
    let norm = total.max(norm).max(1e-30);
    // Single reverse pass
    for (i, &e) in energy.iter().enumerate().rev() {
        cum += e;
        let db = 10.0 * (cum / norm).max(1e-30).log10();
        if (-25.0..=-5.0).contains(&db) {
            let t = i as f64 / sr;
            count += 1.0;
            x += t;
            y += db;
            xx += t * t;
            xy += t * db;
        }
    }
    if count <= 10.0 {
        return f64::NAN;
    }
    let slope = (count * xy - x * y) / (count * xx - x * x);
    -60.0 / slope
}

fn main() {
    println!(
        "sample_rate,profile,requested_seconds,t20_rt60_seconds,t20_r2,t30_rt60_seconds,t30_r2,low_lt400_t20_s,mid_400_3k_t20_s,high_gt3k_t20_s,peak,stereo_correlation"
    );
    for sr in [48000., 96000.] {
        for decay in [0.5, 1.5, 4.8, 10.0] {
            for neutral in [true, false] {
                let mut p = SolarisReverbParams {
                    wet_mix: 1.,
                    decay_sec: decay,
                    pre_delay_ms: 0.,
                    ..Default::default()
                };
                if neutral {
                    p.ensemble_mode = EnsembleMode::Off;
                    p.shimmer_amount = 0.;
                    p.vowel_mix = 0.;
                    p.mod_depth = 0.;
                    p.high_cut_hz = 18000.;
                    p.low_cut_hz = 20.;
                }
                let mut r = SolarisCloudburstReverb::new(sr);
                let frames = ((decay * 4.2 + 1.0) * sr) as usize;
                let mut energy = Vec::with_capacity(frames);
                let mut energy_low = Vec::with_capacity(frames);
                let mut energy_mid = Vec::with_capacity(frames);
                let mut energy_high = Vec::with_capacity(frames);

                let sr64 = sr as f64;
                let mut lp400_a = [Biquad64::lowpass(sr64, 400.0); 2];
                let mut lp400_b = [Biquad64::lowpass(sr64, 400.0); 2];
                let mut hp400_a = [Biquad64::highpass(sr64, 400.0); 2];
                let mut lp3k_a = [Biquad64::lowpass(sr64, 3000.0); 2];
                let mut hp3k_a = [Biquad64::highpass(sr64, 3000.0); 2];
                let mut hp3k_b = [Biquad64::highpass(sr64, 3000.0); 2];

                let (mut peak, mut lr, mut ll, mut rr) = (0f64, 0f64, 0f64, 0f64);
                for i in 0..frames {
                    let (l, right) = r.process_stereo(if i == 0 { 0.5 } else { 0. }, 0., &p);
                    assert!(l.is_finite() && right.is_finite());
                    let (l, right) = (l as f64, right as f64);
                    peak = peak.max(l.abs()).max(right.abs());
                    lr += l * right;
                    ll += l * l;
                    rr += right * right;
                    energy.push(l * l + right * right);

                    let low_l = lp400_b[0].process(lp400_a[0].process(l));
                    let low_r = lp400_b[1].process(lp400_a[1].process(right));
                    energy_low.push(low_l * low_l + low_r * low_r);

                    let mid_l = lp3k_a[0].process(hp400_a[0].process(l));
                    let mid_r = lp3k_a[1].process(hp400_a[1].process(right));
                    energy_mid.push(mid_l * mid_l + mid_r * mid_r);

                    let high_l = hp3k_b[0].process(hp3k_a[0].process(l));
                    let high_r = hp3k_b[1].process(hp3k_a[1].process(right));
                    energy_high.push(high_l * high_l + high_r * high_r);
                }
                let mut energy_t30 = energy.clone();
                let (t20, r2_20) = schroeder_fit(&mut energy, sr64, -25.0, -5.0);
                let (t30, r2_30) = schroeder_fit(&mut energy_t30, sr64, -35.0, -5.0);
                let low_t20 = schroeder_t20_only(&energy_low, sr64, 0.0);
                let mid_t20 = schroeder_t20_only(&energy_mid, sr64, 0.0);
                let high_t20 = schroeder_t20_only(&energy_high, sr64, 0.0);

                println!(
                    "{sr},{},{decay},{t20:.6},{r2_20:.6},{t30:.6},{r2_30:.6},{low_t20:.6},{mid_t20:.6},{high_t20:.6},{peak:.6},{:.6}",
                    if neutral { "neutral" } else { "artistic-default" },
                    lr / (ll * rr).sqrt()
                );
            }
        }
    }
}

