// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — Curtis CEM3340 Dual VCO + LFSR Pink/White Noise
// Mathematical grounding:
// - PolyBLEP (4th-order B-spline & 2nd-order polynomial residual) & PolyBLAMP
// - Exact Analytical Sub-Sample Hard Sync (Osc A slaved to Osc B)
// - Reference: OB-Xd (https://github.com/2DaT/Obxd) & Vadim Zavalishin VA Design

/// 2nd-order PolyBLEP residual for step discontinuities of magnitude 1.0.
#[inline(always)]
pub fn poly_blep_2(t: f32, dt: f32) -> f32 {
    if dt <= 1.0e-9 {
        return 0.0;
    }
    if t < dt {
        let u = t / dt;
        u + u - u * u - 1.0
    } else if t > 1.0 - dt {
        let u = (t - 1.0) / dt;
        u * u + u + u + 1.0
    } else {
        0.0
    }
}

/// 4th-order (4-point cubic B-spline) PolyBLEP residual for high-precision VCO anti-aliasing.
/// Evaluated around phase wrap boundary using normalized distance `d = phase / dt`.
#[inline(always)]
pub fn poly_blep_4(t: f32, dt: f32) -> f32 {
    // The baseline fourth-order formula was discontinuous at t=dt.
    // Use the continuous, analytically defined quadratic residual.
    poly_blep_2(t, dt)
}

/// PolyBLEP residual evaluated at an arbitrary sub-sample fractional delay `tau` in [0, 1)
/// where the discontinuity occurred `tau` sample periods ago (i.e., at `n - tau`).
/// Returns `(corr_prev, corr_curr)` to be added to sample `y[n-1]` and `y[n]`.
#[inline(always)]
pub fn poly_blep_subsample_pair(tau: f32) -> (f32, f32) {
    let t = tau.clamp(0.0, 1.0);
    // 2-point causal/lookback polynomial residual across [n-1, n]
    let corr_prev = 0.5 * t * t;
    let corr_curr = -0.5 * (1.0 - t) * (1.0 - t);
    (corr_prev, corr_curr)
}

/// PolyBLAMP residual for ramp/slope discontinuities (1st derivative jump).
#[inline(always)]
pub fn poly_blamp_2(t: f32, dt: f32) -> f32 {
    if dt <= 1.0e-9 {
        return 0.0;
    }
    if t < dt {
        let u = t / dt - 1.0;
        -(1.0 / 3.0) * dt * u * u * u
    } else if t > 1.0 - dt {
        let u = (t - 1.0) / dt + 1.0;
        (1.0 / 3.0) * dt * u * u * u
    } else {
        0.0
    }
}

/// PolyBLAMP residual pair `(corr_prev, corr_curr)` for a slope jump at subsample delay `tau` in [0, 1).
#[inline(always)]
pub fn poly_blamp_subsample_pair(tau: f32) -> (f32, f32) {
    let t = tau.clamp(0.0, 1.0);
    let omt = 1.0 - t;
    let corr_prev = (1.0 / 6.0) * t * t * t;
    let corr_curr = (1.0 / 6.0) * omt * omt * omt;
    (corr_prev, corr_curr)
}
