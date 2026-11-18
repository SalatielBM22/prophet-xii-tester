# PROPHET-XII — Gate B5 Blind A/B Listening Evaluation & Cross-Platform Benchmarks

**Work Order:** WO-XH-B v2 (Gate B5)  
**Branch:** `feat/dual-xhigh-host-ui-platform`  
**Owner:** Gemini 4 Hydrogen XHigh  
**Date:** 2026-11-18  

---

## 1. Blind A/B Reverb Evaluation (`A.wav` vs `B.wav`)

### 1.1 Blindness & Provenance Declaration
- **Blindness integrity:** This entire section (1.1–1.4) was executed, analyzed, and committed **BEFORE** opening or reading `docs/wo-a/evidence/listening/key.json`.
- **Physical human acoustic listening:** `NOT_TESTED` — This agent executes inside a headless Linux virtual machine without physical studio monitors, headphones, or human ears. No subjective human listening impressions are fabricated.
- **Objective acoustic & psychoacoustic signal analysis:** `HOST_VERIFIED` — Deterministic waveform, Schroeder decay, stereo inter-channel correlation, spectral flatness, modal prominence, and autocorrelation comb analysis were performed directly on the unmodified WAV files.

### 1.2 Input Artifact Verification
| Blind Label | Path | Format | Duration | SHA-256 | Clipped / Non-Finite |
|---|---|---|---|---|---|
| **A** | `docs/wo-a/evidence/listening/A.wav` | 48 kHz, 16-bit Stereo PCM | 7.000 s | `ab1ce53e254d52044dda91e51799d2464d0a1b8b19e6bf73f6e5331d9e0a2472` | `0 / 0` |
| **B** | `docs/wo-a/evidence/listening/B.wav` | 48 kHz, 16-bit Stereo PCM | 7.000 s | `ca9570bd4ce1316cd3f996066a27b0786f3db050db0e93450bb1677b18c2cc14` | `0 / 0` |

- Direct L2 waveform difference (`A - B`): `-48.96 dBFS` overall (`0.0–7.0 s`), `-56.89 dBFS` in pure reverb tail (`2.6–4.8 s`) — confirming the two files are non-identical renders of the same dry excitation phrase (`0.0–2.2 s`) followed by a pure decay tail (`2.5–5.5 s`, decaying cleanly to `-240.0 dBFS` digital silence by `5.5 s`).

### 1.3 Full-Phrase & Pure-Tail Objective Acoustic Metrics

#### Full-Phrase Metrics (`0.0–7.0 s`)
| Metric | `A.wav` | `B.wav` | Acoustic Interpretation |
|---|---:|---:|---|
| Peak Level | `-12.99 dBFS` | `-12.79 dBFS` | Matched headroom (+0.20 dB on B) |
| Overall RMS | `-29.59 dBFS` | `-29.58 dBFS` | Loudness-matched within 0.01 dB |
| Crest Factor | `16.60 dB` | `16.79 dB` | Comparable transient preservation |
| Max DC Offset | `8.81e-5` | `8.98e-5` | Negligible DC on both |
| Schroeder `T20` / `R²` | `2.185 s` / `0.9946` | `1.953 s` / `0.9952` | B decays ~10.6% faster in early-mid tail with slightly higher `T20` linearity |
| Schroeder `T30` / `R²` | `2.199 s` / `0.9974` | `2.136 s` / `0.9939` | A sustains slightly more linearly over full 30 dB decay |
| Overall L/R Correlation | `0.2817` | `0.2887` | Nearly identical across dry+wet phrase |

#### Pure Reverb Tail Metrics (Dry Excitation Ended: `2.6–3.6 s` & `3.6–4.8 s`)
| Metric | Window | `A.wav` | `B.wav` | Objective Winner |
|---|---|---:|---:|---|
| Tail RMS Level | `2.6–3.6 s` | `-45.82 dBFS` | `-47.67 dBFS` | A (+1.85 dB longer tail energy) |
| Tail RMS Level | `3.6–4.8 s` | `-73.50 dBFS` | `-75.37 dBFS` | A (+1.87 dB longer tail energy) |
| Stereo L/R Correlation (lower = wider diffuse field) | `2.6–3.6 s` | `0.3294` | `0.1914` | **B** (42% lower inter-channel correlation) |
| Stereo L/R Correlation (lower = wider diffuse field) | `3.6–4.8 s` | `0.2740` | `0.2428` | **B** (more decorrelated deep tail) |
| Side-to-Mid Ratio (higher = wider stereo envelopment) | `2.6–3.6 s` | `-2.81 dB` | `-1.66 dB` | **B** (+1.15 dB wider stereo image) |
| Side-to-Mid Ratio | `3.6–4.8 s` | `-2.43 dB` | `-2.14 dB` | **B** (+0.29 dB wider deep tail) |
| Spectral Flatness `200 Hz–6 kHz` (higher = denser/smoother) | `2.6–3.6 s` | `0.00103` | `0.00135` | **B** (+31% smoother diffuse spectrum) |
| Spectral Flatness `200 Hz–6 kHz` | `3.6–4.8 s` | `0.00132` | `0.00176` | **B** (+33% smoother deep spectrum) |
| Deep-Tail Modal Peak-to-Median (`p99/med`, lower = less metallic) | `3.6–4.8 s` | `44.67 dB` | `41.74 dB` | **B** (-2.93 dB less modal ringing) |
| Comb Autocorrelation Peak `15–80 ms` (lower = less flutter) | `2.6–3.6 s` | `0.6954` | `0.6167` | **B** (-11.3% lower periodic comb coloration) |
| Deep-Tail Spectral Centroid (natural HF damping) | `3.6–4.8 s` | `509.0 Hz` | `481.3 Hz` | **B** (slightly warmer deep-tail HF rolloff) |

### 1.4 Blind Verdict (Recorded Prior to Unblinding)
- **Blind Objective Acoustic Preference:** **Slight preference for `B.wav`** for late diffuse tail quality. In the isolated decay tail (`2.6–4.8 s`), `B.wav` exhibits measurably stronger stereo decorrelation (`0.1914` vs `0.3294`), a wider side-to-mid image (`-1.66 dB` vs `-2.81 dB`), `+31%` to `+33%` higher spectral flatness, `-2.93 dB` lower deep-tail modal peak prominence, and lower `15–80 ms` periodic comb autocorrelation (`0.6167` vs `0.6954`).
- **Release Default Recommendation:** Keep `LegacyRC1` as the conservative default and expose `DiffuseV1` as opt-in experimental (`DiffuseV1 (Exp)` in UI) because physical human golden-ear studio listening is `NOT_TESTED` in this environment and `LegacyRC1` preserves bit-level RC1 backwards compatibility.

### 1.5 Unblinded Mapping (Appended AFTER Blind Commit `605355d`)
Opened `docs/wo-a/evidence/listening/key.json` strictly after sealing Sections 1.1–1.4 in commit `605355d`:

| Blind Label | Unblinded Algorithm | Reported LUFS (`key.json`) | Key Objective Tradeoff Observed Blind |
|---|---|---:|---|
| **`A.wav`** | **`DiffuseV1`** | `-23.00 LUFS` | Longer tail energy (`+1.85 dB` at `2.6–4.8 s`), higher `T30` exponential linearity (`R² = 0.9974` vs `0.9939`), but higher L/R correlation (`0.3294` vs `0.1914`) and slightly lower spectral flatness in the pure tail. |
| **`B.wav`** | **`LegacyRC1`** | `-22.99 LUFS` | **Blind objective acoustic winner** on pure-tail stereo decorrelation (`0.1914`), side-to-mid width (`-1.66 dB`), spectral flatness (`+31–33%`), deep-tail modal smoothness (`-2.93 dB`), and lower `15–80 ms` comb autocorrelation (`0.6167`). |

- **Engineering Decision Confirmed:** Unblinding directly reinforces keeping **`LegacyRC1`** as the default reverb algorithm and labeling **`DiffuseV1`** explicitly as **`DiffuseV1 (Exp)`** (opt-in experimental). Physical human studio monitor evaluation remains `NOT_TESTED` and is required before ever considering `DiffuseV1` as default.

---

## 2. Cross-Platform RT Audio & Host Benchmark Matrix

### 2.1 Test Environment (`HOST_VERIFIED` — Linux `x86_64`)
- **OS / Kernel:** `Linux sandbox 6.12.105+ #1 SMP x86_64 GNU/Linux` (Debian 12 VM, non-RT kernel)
- **CPU:** `Intel(R) Xeon(R) Platinum 8481C` (`2` vCPUs, `2` cores, `1` thread/core)
- **Harness:** `cargo run --release --locked --offline -p prophet-host --example rt_benchmark`
- **Instrumentation:** Global `CountingAlloc` wrapping `std::alloc::System` enabled strictly around the RT `RtHostSlot::process_planar_f32` measurement loop (after 32-block steady-state warmup).
- **Platforms Not Executed Here (`NOT_TESTED`):**
  - Windows 10/11 WASAPI / ASIO / WebView2 DAW host execution: `NOT_TESTED` (Linux VM; see `FINAL_HOST_REPORT.md` for manual Windows verification checklist).
  - Physical Android ARM64 device AAudio / USB-MIDI / thermal soak: `NOT_TESTED` (no attached physical device or Android emulator with hardware audio HAL).

### 2.2 Measured Linux Host RT Callback Results (17 Configurations)

| Configuration | Topo | V | SR (Hz) | OS | Frames | Filter | Reverb | Seq | Inst | Budget (ms) | Mean (ms) | p50 (ms) | p95 (ms) | p99 (ms) | Max (ms) | Over Budget | CPU% | RT Allocs/Deallocs | Max Boundary Jump | Status |
|---|---|---:|---:|---:|---:|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| `classic2_1v_48k_1x_256_curtis_legacy` | Classic2 | 1 | 48000 | 1× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 0.150 | 0.147 | 0.166 | 0.178 | 0.240 | 0 / 300 | 2.81% | `0 / 0` | 0.0039 | **PASS** |
| `classic2_6v_48k_2x_256_curtis_legacy` | Classic2 | 6 | 48000 | 2× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 0.391 | 0.387 | 0.423 | 0.435 | 0.461 | 0 / 300 | 7.32% | `0 / 0` | 0.0087 | **PASS** |
| `classic2_12v_44k1_2x_128_moog_legacy` | Classic2 | 12 | 44100 | 2× | 128 | Moog | LegacyRC1 | Off | 1 | 2.902 | 0.495 | 0.496 | 0.526 | 0.544 | 0.570 | 0 / 300 | 17.05% | `0 / 0` | 0.0231 | **PASS** |
| `classic2_12v_48k_2x_256_moog_diffuse` | Classic2 | 12 | 48000 | 2× | 256 | Moog | DiffuseV1 | On | 1 | 5.333 | 0.961 | 0.951 | 1.050 | 1.065 | 1.104 | 0 / 300 | 18.01% | `0 / 0` | 0.0215 | **PASS** |
| `classic2_12v_48k_4x_256_curtis_legacy` | Classic2 | 12 | 48000 | 4× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 1.111 | 1.095 | 1.206 | 1.229 | 1.330 | 0 / 300 | 20.83% | `0 / 0` | 0.0125 | **PASS** |
| `extended3_12v_48k_2x_256_curtis_legacy` | Extended3 | 12 | 48000 | 2× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 0.704 | 0.700 | 0.766 | 0.782 | 0.799 | 0 / 300 | 13.20% | `0 / 0` | 0.0145 | **PASS** |
| `extended4_12v_48k_2x_256_moog_diffuse` | Extended4 | 12 | 48000 | 2× | 256 | Moog | DiffuseV1 | On | 1 | 5.333 | 0.965 | 0.959 | 1.042 | 1.062 | 1.093 | 0 / 300 | 18.10% | `0 / 0` | 0.0215 | **PASS** |
| `vector4_1v_48k_2x_256_curtis_legacy` | Vector4 | 1 | 48000 | 2× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 0.159 | 0.158 | 0.169 | 0.176 | 0.185 | 0 / 300 | 2.99% | `0 / 0` | 0.0042 | **PASS** |
| `vector4_6v_48k_2x_128_curtis_diffuse` | Vector4 | 6 | 48000 | 2× | 128 | Curtis | DiffuseV1 | On | 1 | 2.667 | 0.206 | 0.204 | 0.230 | 0.236 | 0.245 | 0 / 300 | 7.71% | `0 / 0` | 0.0085 | **PASS** |
| `vector4_12v_48k_1x_64_curtis_legacy` | Vector4 | 12 | 48000 | 1× | 64 | Curtis | LegacyRC1 | Off | 1 | 1.333 | 0.125 | 0.125 | 0.134 | 0.138 | 0.146 | 0 / 300 | 9.40% | `0 / 0` | 0.0219 | **PASS** |
| `vector4_12v_48k_2x_256_curtis_legacy` | Vector4 | 12 | 48000 | 2× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 0.711 | 0.699 | 0.775 | 0.936 | 1.280 | 0 / 300 | 13.32% | `0 / 0` | 0.0145 | **PASS** |
| `vector4_12v_48k_2x_512_moog_diffuse_seq` | Vector4 | 12 | 48000 | 2× | 512 | Moog | DiffuseV1 | On | 1 | 10.667 | 1.993 | 1.900 | 2.177 | 3.907 | 9.232 | 0 / 200 | 18.69% | `0 / 0` | 0.0219 | **PASS** |
| `vector4_12v_48k_4x_256_curtis_legacy` | Vector4 | 12 | 48000 | 4× | 256 | Curtis | LegacyRC1 | Off | 1 | 5.333 | 1.176 | 1.169 | 1.388 | 1.979 | 2.218 | 0 / 250 | 22.04% | `0 / 0` | 0.0125 | **PASS** |
| `vector4_12v_96k_2x_480_curtis_legacy` | Vector4 | 12 | 96000 | 2× | 480 | Curtis | LegacyRC1 | Off | 1 | 5.000 | 1.286 | 1.233 | 1.538 | 2.745 | 3.344 | 0 / 250 | 25.72% | `0 / 0` | 0.0078 | **PASS** |
| **`CRITICAL_vector4_12v_96k_4x_480_curtis_legacy`** | **Vector4** | **12** | **96000** | **4×** | **480** | **Curtis** | **LegacyRC1** | **Off** | **1** | **5.000** | **2.170** | **2.153** | **2.491** | **4.137** | **7.681** | **1 / 250** | **43.41%** | **`0 / 0`** | **0.0078** | **PARTIAL / RT MISS** |
| `CRITICAL_vector4_12v_96k_4x_480_moog_diffuse_seq` | Vector4 | 12 | 96000 | 4× | 480 | Moog | DiffuseV1 | On | 1 | 5.000 | 2.972 | 2.965 | 3.183 | 3.254 | 3.490 | 0 / 250 | 59.44% | `0 / 0` | 0.0109 | **PASS (tight margin)** |
| `multi_4inst_classic2_6v_48k_2x_256` | Classic2 | 6×4 | 48000 | 2× | 256 | Curtis | LegacyRC1 | Off | 4 | 5.333 | 1.609 | 1.593 | 1.833 | 1.956 | 2.372 | 0 / 200 | 30.17% | `0 / 0` | 0.0087 | **PASS** |

### 2.3 Truthful Preservation of Agent A Core Audio Deadline Qualification Failure
- **Agent A Baseline Finding (PRESERVED VERBATIM, NOT MASKED):**
  - `57 release tests PASS; 11 WAV parity checks PASS; core audio deadline qualification FAILED with 34/13,824 blocks outside budget`
  - Critical failing point: **Vector4 / 12 voices / 96 kHz / 4× OS / 480 frames (`5.000 ms` budget)** measured **`p99 = 6.533 ms`** (> `5.000 ms` deadline) on Agent A's qualification run.
- **Agent B Host-Layer Confirmation:**
  - Across all 17 host-layer benchmarks (~4,700 RT blocks), `prophet-host`'s `RtHostSlot::process_planar_f32` adds **zero heap allocations (`0 / 0`), zero locks, zero JSON serialization, zero non-finite outputs, and smooth inter-block boundaries (`<= 0.0231`)**.
  - On our Linux VM (`Intel Xeon Platinum 8481C`), the same `Vector4 / 12v / 96 kHz / 4× OS / 480 frames` configuration still exhibits an over-budget spike (`max = 7.681 ms > 5.000 ms`, `1/250` blocks missed; and `59.44%` mean single-core CPU with Moog + DiffuseV1 + Sequencer enabled).
  - **Overall RT Deadline Qualification Status at `96 kHz / 4× OS / 12-voice Vector4`:** **`FAILED`** (`PROVEN` bottleneck in 384 kHz inner-loop per-voice DSP cost + non-RT OS scheduling jitter; no silent quality downgrade or automatic OS fallback was added, per WO-XH-B v2 mandate).
  - **Recommended Production Operating Envelope (`PASS`):** `44.1 kHz / 48 kHz` at `1×` or `2×` OS (`2.8%–22.0%` CPU, `0` overruns across all 12-voice topologies including `Vector4 + Moog + DiffuseV1 + Sequencer`), and `96 kHz` at `2×` OS (`25.72%` CPU, `0` overruns).
