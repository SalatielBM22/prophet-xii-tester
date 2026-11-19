# Windows RT baseline — SalatielPC (2026-11-19)

**Scope:** HOST_VERIFIED on one machine. Offline engine timing, no device callback or acoustic latency.
**Machine:** AMD Ryzen 3 PRO 2300U (4C/4T, 15 W mobile, 2018), 15 GB RAM, Windows 11 Pro 26300. Other applications were running, so the timing is not idle-system timing.
**Build:** release (`opt-level=3`, thin LTO, `codegen-units=1`), cross-compiled with `x86_64-pc-windows-gnu`, default `x86-64` target CPU. Base commit `3c9e51b`.

## 1. Scenario matrix (`prophet-host/examples/rt_benchmark.rs`)
Raw output: [`evidence/rt_benchmark_windows_2026-11-19.json`](evidence/rt_benchmark_windows_2026-11-19.json). Mean and p99 are per block; the budget is the block's audio duration.

| Case | Mean / budget | p99 | Over budget |
|---|---|---|---|
| Classic2 1v 48k 1× 256 | 0.54 / 5.33 ms (10%) | 4.42 ms | 2/300 |
| Classic2 12v 48k 2× 256 Moog+DiffuseV1 | 2.59 / 5.33 ms (49%) | 7.37 ms | 12/300 |
| Classic2 12v 48k 4× 256 | 2.68 / 5.33 ms (50%) | 6.20 ms | 14/300 |
| Vector4 12v 48k 2× 256 | 1.68 / 5.33 ms (31%) | 4.50 ms | 2/300 |
| Vector4 12v 96k 2× 480 | 4.45 / 5.00 ms (89%) | 12.87 ms | 54/250 |
| Vector4 12v 96k 4× 480 | 6.62 / 5.00 ms (132%) | 26.92 ms | 105/250 |
| 4 instances Classic2 6v 48k 2× 256 | 5.02 / 5.33 ms (94%) | 22.16 ms | 56/200 |

Every case reported 0 RT allocations and 0 non-finite samples. The max column reaches 120–170 ms in a few cases. That much jitter means scheduler preemption on a loaded desktop, not DSP work, so only the means and the medians below measure DSP cost.

## 2. Per-component cost (`prophet-control/examples/cost_breakdown.rs`)
Medians over 1500 blocks of 256 frames at 48 kHz (budget 5333 µs), preset 0, Curtis VCF.

| Component | Median cost | Share of budget |
|---|---|---|
| Idle kernel (no voices, FX off) | 48 µs | 0.9% |
| Chorus mode I | +48 µs | 0.9% |
| Reverb fed by 6 voices (LegacyRC1) | +169 µs | 3.2% |
| 12 voices @ 1× | +634 µs (53 µs/voice) | 11.9% |
| 12 voices @ 2× | +1148 µs (96 µs/voice) | 21.5% |
| 12 voices @ 4× | +2097 µs (175 µs/voice) | 39.3% |

**Conclusion:** voice processing dominates. FX are a few percent, and the reverb is dormant on silence.

## 3. What was ruled out
- **Envelope coefficient recalculation:** an in-process sampler (`pprof`) attributed 17.8% to `Cem3310Envelope::recalculate_coefficients`. A counter showed 0.003 recalculations per voice-sample, so this was an attribution artifact of inlined code. The guard in `set_adsr` works.
- **CEM3320 Newton iterations:** 1.93 iterations per call at 1× and 1.47 at 2×. The absolute tolerance is not forcing the 6-iteration cap.
- **Denormals:** enabling FTZ/DAZ did not change timings (WSL measurement).

## 4. Next step (open)
Per-function attribution needs a hardware-counter profiler on a non-inlined symbol view: `perf` (needs `linux-tools` with sudo in WSL), `samply` or WPR/WPA on Windows. Optimizations must keep the Classic2 RC1 renders bit-identical, or document and audition any deviation.

## 5. Qualification status (issue #4)
- **Not qualified on this machine:** Vector4 at 96 kHz (2× and 4×) and multi-instance 4×6 voices.
- **Candidate profile:** 48 kHz 2× with 12 voices (Vector4 mean 31%). It still needs WASAPI device timing (`--http-dev --timing-log`) and a long session before being called qualified.
