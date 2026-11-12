# Agent B: baseline and acceptance contract (#1, #4)

Baseline: `b47f2d60c7005cd6c81694c0e17c76f7a4f107dc`, reproduced on 2026-11-12.
Origin inspected: `https://github.com/SalatielBM22/prophet-xii-tester.git`.
Initial branch `work`, clean HEAD equal to baseline (therefore an ancestor).
Changes are local on `fix/rt-latency-issue-1`; no push, upstream write, worktree,
or replacement of delivered binaries. Fork privacy is user-declared, not
independently API-verified: api.github.com is denied by the environment proxy.
The four upstream issue bodies were read from github.com, not assumed from old reports.

## Contract

Order: numerical regression protection, bounded RT behavior, demonstrable circuit
fidelity, then optimization. Preserve IDs 0..82, default presets and reference
artifacts. No new physical model without an independent continuous/circuit oracle.
No automatic quality reduction, dropped polyphony, replacement Newton solver,
network, JSON, FFT, allocation or mutex in the callback. Reject invalid controls.
Owned scope is DSP/control/params/native backends and this documentation; UI,
analysis crate and root Cargo.toml belong to Agent A.

Acceptance: existing regressions pass, new counterexamples fail before fixes,
deterministic output and finite samples at 48/96 kHz and 1x/2x/4x, callback
allocations zero, measured p50/p95/p99/max and deadline fraction. Offline results
are not device real-time qualification. Actual WASAPI/ALSA buffering and loopback
latency must remain OPEN unless measured on a device.

## Reproduction

Linux x86_64 KVM, Intel Xeon Platinum 8573C, 5 virtual CPUs, approximately 33 GiB
RAM. Rust 1.100.0, release profile with thin LTO, Cargo.lock unchanged.
Source `/workspace/.prophet-tools/env.sh`. Run from the repository:

```
cargo test --release --locked --workspace
cargo run --release --locked -p prophet-standalone --example rt_matrix -- --iterations 100
cargo run --release --locked -p prophet-dsp --example hotspots
```

The matrix uses 256-frame stereo callbacks, 32 warmups and 100 measured callbacks
per case, 48/96 kHz, 1x/2x/4x, Curtis/Moog, 0/1/6/12 voices and FX 0=dry,
1=FDN/ensemble-off, 2=FDN/forte. Chorus is off to isolate reverb. Reverb wet .32,
other controls preset 0. Timings include AudioProcessor control/telemetry work;
hashing, Vec growth and sorting are outside the timed region. Queue is drained
after every callback; hashes are FNV-1a over f32 bits (regression identity, not a
cryptographic artifact digest). No CPU isolation; scheduling jitter is retained.
Use `--block 64`, `--block 512` for other buffer sizes. CPU percent is
processing time / (frames / sample rate), not audio transport latency.

Raw data: [baseline-rt-256.csv](evidence/baseline-rt-256.csv),
[baseline-hotspots.csv](evidence/baseline-hotspots.csv).
129 delivered files (including WAVs, benchmarks, binaries, presets and lockfile)
are protected by [baseline-artifacts.sha256](evidence/baseline-artifacts.sha256).
Verify with `sha256sum -c docs/backend/evidence/baseline-artifacts.sha256`.

## Reproduced observations

PROVEN / HOST_VERIFIED: 18 existing Rust tests pass; 144 benchmark cases complete,
all outputs finite, zero measured callback allocations. This short steady-state
matrix finds no subnormal output; it does not disprove long-tail denormals.
Kernel inline storage 105568 bytes; AudioProcessor 132448 bytes. Reverb heap
delay storage is `(131072 + 4*8192 + 8*32768)*4 = 1703936` bytes per engine,
excluding allocator metadata. Inline bytes do not include this heap allocation.

PROVEN / HOST_VERIFIED: at 48 kHz, 4x Moog, 12 voices, forte, p50 11983.837 us,
p99 18262.624 us, maximum 18414.118 us against 5333.333 us; 100/100 misses.
Dry version p50 11231.744 us and also 100/100 misses. This host reproduces DSP
overload independently of an audio device. No claim of reproducing 2 s latency.
Idle forte (zero voices), same rate/factor: p50 469.461 us, p99 1278.874 us.
Inactive voices already short-circuit; the FX path still processes zeros.

SOURCE_INSPECTED: Moog has up to 8 outer roots, each with four implicit stages
with up to 8 Newton iterations, plus final evaluation. Both coefficient setters
run per oversample. Ensemble repeats six distinct sinusoid values across 48
bands. Chorus recomputes rate-only exponentials each sample. Profiler: isolated
component wall-time harness; `perf` is not installed (no call-stack sampling).

OPEN / NOT_TESTED: actual Windows WASAPI/WSLg/ALSA device, MIDI hardware timestamp,
loopback latency, device queue depth, driver XRuns, scheduler starvation, fault
recovery, sustained thermal load and Snapdragon 665 performance.
