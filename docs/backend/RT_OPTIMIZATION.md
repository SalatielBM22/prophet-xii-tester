# RT fixes and qualification checkpoint (#1, #2, #4)

This is a **checkpoint**, not issue closure or device RT qualification.
Baseline, environment, commands and protected artifact hashes are in
[RT_BASELINE.md](RT_BASELINE.md); the exact API is in
[INTEGRATION_CONTRACT.md](INTEGRATION_CONTRACT.md).

## Implemented changes

1. Share the three slow/fast Ensemble LFO phase pairs across 48 bands (96 sine
   evaluations become 6); cache chorus sample-rate-only exponentials. PROVEN /
   HOST_VERIFIED: all 144 baseline audio hashes remain bitwise identical at this
   intermediate step (`cached-rt-256.csv`). No solver or fidelity reduction.
2. Reverb enable with 5 ms fade and explicit spill/kill. Zero wet advances an
   existing tail but no longer feeds hidden new excitation. A fresh dry/empty
   tank remains dormant. Reset/kill invalidate delay history with read-age guards,
   avoiding a full 1.7 MB memset. RNG resets deterministically. Freeze stops new
   predelay recording. Recursive FX stores/output discard values below 1e-20
   (-400 dBFS); natural tails sleep after a conservative complete storage horizon.
3. Native queue timestamps separate ingress/relay/RT waits. An event-driven relay
   replaces the fixed 10 ms command poll; telemetry/capture drains at 5 ms cadence.
   Buffers are validated/requested against device ranges. Callback-frame/timing,
   backend timestamp estimate and complete-callback quantiles are observable.
4. Bounded continuous PCM transport, full MIDI Learn lifecycle, parameter revisions
   and direct-command acknowledgments, factory snapshots and actual envelope stage/
   time telemetry. No FFT/JSON/network/lock was introduced in the callback.
5. Inactive filter branches reset on morph reentry, avoiding stale states; the
   existing smoother handles the crossfade. Blend remains linear, with no phase
   or equal-power claim. Newton equations/counts and oversampling quality unchanged.

## Measured benefit and limits

Single-run component wall profile (ns per sample):

| Component | Baseline | Shared phases/cache |
| --- | ---: | ---: |
| Curtis | 87.07 | 82.57 |
| Moog | 624.89 | 639.24 |
| Reverb ensemble off | 386.88 | 363.84 |
| Reverb mp | 2004.05 | 1172.07 |
| Reverb forte | 1919.80 | 1142.41 |

PROVEN / HOST_VERIFIED: recorded Ensemble component improvement is about 40–42%.
These are single wall-time runs with scheduling noise, not confidence intervals.
The unrelated Curtis/Moog variation is not claimed as an optimization. `perf`
was absent; profiling here is isolated component timing plus configuration
contrasts and source cost inspection, not a sampled call-stack trace.

256-frame callback comparison, 48 kHz / 4x (microseconds):

| Case | Run | p50 | p95 | p99 | max | misses /100 |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| zero voices, forte | baseline | 469.461 | 689.813 | 1278.874 | 1337.978 | 0 |
| zero voices, forte | candidate | 9.099 | 9.538 | 18.816 | 28.431 | 0 |
| 12 voices Moog, forte | baseline | 11983.837 | 14194.138 | 18262.624 | 18414.118 | 100 |
| 12 voices Moog, forte | candidate | 12187.941 | 14336.140 | 17036.618 | 22391.269 | 100 |
| 12 voices Moog, dry | baseline | 11231.744 | 12368.904 | 12894.424 | 13662.789 | 100 |
| 12 voices Moog, dry | candidate | 12132.823 | 18063.267 | 37507.663 | 60953.684 | 100 |

Budget: 5333.333 us. FAILED / HOST_VERIFIED: high-polyphony 4x Moog still cannot
meet this host's budget. Do not call the overall kernel optimized or RT-qualified.
The candidate has worse active-load maxima in some cases. Baseline had 39/144
cases with at least one miss, candidate 45/144; totals 2440 vs 2453/14400 callbacks.
These shared-VM sequential runs do not isolate the causal contribution of code
vs scheduler variation. There is no universal no-regression timing claim.
Next performance work must isolate the Moog solver, cache/coefficient updates,
subsample-invariant work and publication cost, retaining residual/error checks.

An intermediate negative result (`candidate-wet0-rt-256.csv`) is retained: advancing
wet=0 **while continuously exciting** the hidden tank needlessly increased dry
cost. The final version stops hidden excitation while advancing existing tails;
the stale-tail regression still passes. Existing tails can cost CPU until they
decay, as required by spill semantics. Explicit kill can terminate that cost.

## Matrix and memory

`candidate-rt-256.csv`: 144 cases, 100 measurements +32 warmups each, 48/96 kHz,
1x/2x/4x, Curtis/Moog, 0/1/6/12 voices, FX dry/ensemble-off/forte. Active voice
count is asserted after warmup. All 48 **dry** output hashes match baseline.
Active FX output is intentionally not claimed bitwise unchanged after state/
silence handling; no physical model was replaced.

`candidate-capture-rt-64.csv` and `candidate-capture-rt-512.csv`: 144 cases each,
same rates/quality/polyphony, with PCM capture enabled and its consumer drained
outside timing. These are callback function measurements, not actual negotiated
device buffers. All 432 final cases have finite outputs, zero measured allocations
and zero emitted subnormal samples. Selected longer chorus/reverb tests exercise
silence after excitation, rather than equating a fresh empty tank with a tail.
Allocation instrumentation counts alloc, alloc_zeroed and realloc; a positive
control demonstrates zeroed allocations are detected. Deallocation/locks/syscalls
are not certified absent by an allocation counter alone; source paths were read.

Kernel inline bytes: baseline 105568, candidate 106208. AudioProcessor: 132448 →
144352 bytes. Reverb boxed delay buffers remain 1703936 bytes/engine. The optional
256x64 capture ring has 64 KiB PCM plus per-chunk headers and allocator metadata.
OS resident footprint, allocator metadata, Tokio/network/UI memory and peak RSS
are not measured by `size_of`; do not report these inline numbers as total RSS.

Reproduce:

```sh
cargo run --release --locked -p prophet-standalone --example rt_matrix -- --iterations 100
target/release/examples/rt_matrix --iterations 100 --block 64 --capture
target/release/examples/rt_matrix --iterations 100 --block 512 --capture
cargo run --release --locked -p prophet-dsp --example hotspots
cargo run --release --locked -p prophet-dsp --example reverb_decay
cargo run --release --locked -p prophet-dsp --example filter_metrics
```

## Evidence ledger / remaining gates

Final recorded command outcomes: **41 Rust tests passed, 0 failed, 0 ignored**
across 26 runners (including zero-test targets, which are not counted as tests).
Clippy exited 0 with 107 warning lines. The native HTTP/WebSocket contract,
original CLAP smoke test and added reverb state test passed. Repeated 12-voice
WAVs match byte-for-byte at 48/96 kHz; MIDI-file renders also completed. See
[qualification.json](evidence/qualification.json),
[renders.json](evidence/renders.json),
[tested-source.sha256](evidence/tested-source.sha256) and
[candidate-artifacts.sha256](evidence/candidate-artifacts.sha256).
The qualification's FAILED status refers to RT/RT60 acceptance, not a hidden
software test failure. Raw timings retain failures and worse maxima.

CLAP ABI tests use official SDK tag 1.2.6, Git tag ref
`69a69252fdd6ac1d06e246d9a04c0a89d9607a17`, downloaded tar SHA256
`6548b5831ad9e92551c752753e535b18fead4001165ef1df8e643efefa7133b7`.
The headers are retained at `/workspace/.prophet-tools/clap-sdk/include`, outside
the repository. Example reproduction after building prophet-plugin:

```sh
cc -O2 -I/workspace/.prophet-tools/clap-sdk/include tests/clap_smoke.c -o /tmp/prophet-clap-smoke -ldl -lm
/tmp/prophet-clap-smoke target/release/libprophet_plugin.so
cc -O2 -I/workspace/.prophet-tools/clap-sdk/include crates/prophet-plugin/tests/bypass_state.c -o /tmp/prophet-bypass-state -ldl -lm
/tmp/prophet-bypass-state target/release/libprophet_plugin.so
python crates/prophet-standalone/tests/host_contract.py
```

Standalone/renderer/plugin builds use `target`, never the original `binaries`.
Rendered WAVs remain in `/workspace/.prophet-build/agent-b/renders`; hashes and
reproduction parameters are saved, not silently substituted for delivered WAVs.
Verify original artifacts with the baseline SHA256 manifest. The tree is confined
to owned crates and docs/backend; root Cargo.toml, Cargo.lock, ui and the analysis
crate are unchanged.

| Gate | Status / provenance | Evidence / limit |
| --- | --- | --- |
| B0 baseline | PROVEN / HOST_VERIFIED | Existing suite reproduced, 144 cases, reference SHA256 manifest; scope is Linux VM |
| B1 performance/latency | FAILED / HOST_VERIFIED; device OPEN | Idle/Ensemble fixes measured, high-polyphony deadlines still fail; no 2 s loopback diagnosis |
| B2 reverb | PROVEN functional subset; RT60 FAILED | Fade/spill/kill/reset/freeze/wet-zero/finite tests; calibrated multiband RT60 and full sensitivity matrix OPEN |
| B3 CEM3320 | OPEN physical; selected digital PROVEN | Independent analytic response, derivative/residual/stress checks; no circuit/SPICE oracle, original model retained |
| B4 Moog/morph | OPEN physical; selected transition PROVEN | Solver unchanged, stale-state counterexample fixed; full high-Q transition/performance study OPEN |
| B5 VCO/ADSR/BBD | OPEN physical; digital regressions PROVEN | No new circuit model or high-fidelity mode promoted |
| B6 Agent A contract | PROVEN native / HOST_VERIFIED | Real PCM WebSocket + HTTP test, 83 IDs retained; JNI capture/timing and durable MIDI-map parity OPEN |
| B7 qualification | FAILED RT; selected software checks PROVEN | Rust tests, Clippy, MIDI/render, CLAP, alias/residual/finite tests; real device/deadline/recovery qualification OPEN |
| B8 ARM64 | PROVEN compilation / HOST_VERIFIED; device OPEN | Actual AArch64 ELF, ARM crates, Android and Windows type checks; no phone/emulator execution |

Clippy completes with warnings (including historical excessive-precision,
needless-loop and dependency warnings); it is not warning-clean. The two default
approx_constant errors were corrected with the identical f32 standard constant.
NIH-plug VST3 macro future-compatibility warnings remain; no dependency/lockfile
was changed to hide them. Numerical tests do not validate hardware fidelity.

Actual Windows WASAPI vs native ALSA vs WSLg, device XRun/queue depth, acoustic/
electrical loopback, control-flood timing, full RT60 octave bands, full per-control
musical sensitivity, shimmer pitch accuracy, freeze energy drift, live device
fault/recovery and sustained Snapdragon load are **OPEN / NOT_TESTED**. No ASIO,
firmware, device installation or original bundle replacement occurred. No push,
upstream merge or public publication was performed.
