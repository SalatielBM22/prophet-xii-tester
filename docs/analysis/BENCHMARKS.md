# Measured worker costs

Measured 2026-11-12 on x86_64, AMD EPYC 9V74, Rust 1.100.0 / LLVM 23.1.5,
Linux cloud container. Release opt-level 3; default rustfft AVX/SSE dispatch,
no custom target flags. PFFFT C was compiled with `cc -O3 -std=c99`, default
platform SIMD. Runs share a scheduled host with development tasks; CPU pinning,
governor and RT scheduling were not controlled. Outliers and distributions matter.

The CSVs retain p50/p95/p99/max, warmup and sample counts. Each timed Rust sample
includes `Instant` overhead; the C harness uses `CLOCK_MONOTONIC`. For FFT kernel
rows a real-input copy/conversion is timed as well. No result should be read as
an audio callback deadline or acoustic latency measurement.

| Method | N | p50 µs | p95 µs | p99 µs | Max µs |
|---|---:|---:|---:|---:|---:|
| realfft R2C + copy | 2048 | 1.842 | 3.134 | 3.556 | 75.023 |
| rustfft C2C + real-to-complex copy | 2048 | 2.253 | 2.293 | 3.585 | 79.360 |
| PFFFT ordered R2C + copy | 2048 | 1.732 | 1.752 | 3.075 | 115.134 |
| realfft R2C + copy | 4096 | 7.462 | 8.152 | 30.886 | 714.236 |
| rustfft C2C + real-to-complex copy | 4096 | 5.458 | 7.962 | 27.171 | 287.012 |
| PFFFT ordered R2C + copy | 4096 | 3.906 | 3.946 | 8.523 | 158.098 |
| realfft R2C + copy | 8192 | 9.204 | 15.433 | 32.459 | 162.535 |
| rustfft C2C + real-to-complex copy | 8192 | 11.547 | 17.597 | 58.718 | 386.012 |
| PFFFT ordered R2C + copy | 8192 | 9.344 | 9.434 | 37.557 | 247.393 |

These single-host measurements do not show a universal FFT winner: notably the
4096 realfft run was slower than C2C under host scheduling load. realfft remains
the production library choice for its planned R2C API, pure Rust portability and
reusable scratch; no intrinsic language speed claim or custom SIMD rewrite is
justified by these results. PFFFT is an independent optional reference, with no
source copied into production.

| Full worker operation | N | p50 µs | p95 µs | p99 µs | Max µs |
|---|---:|---:|---:|---:|---:|
| Cached Hann multiplication only | 4096 | 0.460 | 0.461 | 0.470 | 39.520 |
| Hann + R2C + power average + dBFS + peak + PSD | 4096 | 24.327 | 38.909 | 86.640 | 1282.212 |
| STFT hop=N/4 packet + ring copy + full analysis | 4096 | 35.033 | 54.252 | 107.642 | 220.732 |
| Reusable f64 65536 FFT + dense unwrap + 256 complex log samples | 4096 impulse | 767.106 | 1157.444 | 1485.257 | 3819.395 |
| Baseline-form direct 4096-sample DTFT at 256 log frequencies | 4096 impulse | 15572.062 | 17774.520 | 19865.721 | 19865.721 |

The transfer fixture uses the same 4096-sample normalized one-pole impulse and
20–20000 Hz grid in both implementations. The FFT uses zero padding to reduce
complex interpolation error and performs extra dense phase unwrapping/masking.
Median worker compute in this run is about 20.3× lower than the direct DTFT.
The independent accuracy tests constrain dB/radian errors; this is not just a
replacement timing test that accepts incorrect output. These timings exclude
the synth's impulse generation, JSON, HTTP, UI drawing and actual VCF integration.
The existing backend endpoint is unchanged, so no endpoint speedup is claimed.

The isolated allocation test passed with **0 allocations** after planning across
repeated spectrum, STFT and transfer calls. CSV buffer sizes exclude FFT plan
tables, allocator metadata and caller inputs/outputs. For N=4096 the spectrum
retains 90140 bytes; the STFT plus spectrum retains 122908 bytes. The larger
transfer plan retains 1572896 bytes, trading bounded worker memory for denser
phase/interpolation precision.

15 accuracy/stream tests and 1 allocation test passed. PFFFT's separate harness
passed direct-DFT bin checks and inverse reconstruction before printing its CSV.
AArch64 release `cargo check` passed with rustfft's default NEON feature enabled;
neither native execution nor timings are available on ARM64/Moto G8. No CPU
performance claims are extrapolated from x86_64 to ARM64.

Reproduction commands and PFFFT pinned revision/license notes are in
[README.md](README.md). Raw results:

- [benchmarks-x86_64.csv](benchmarks-x86_64.csv)
- [pffft-x86_64.csv](pffft-x86_64.csv)
- [validation-release.txt](validation-release.txt)
- [aarch64-check.txt](aarch64-check.txt)
- [SHA256SUMS](SHA256SUMS)
