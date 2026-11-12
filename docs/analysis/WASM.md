# Continuous-capture WASM bridge

The optional `web` feature exports the existing Rust STFT to a browser Worker.
It does not introduce another FFT algorithm, backend endpoint, audio device or
synthetic production data. The actual producer is Agent B's checkpoint
`211c4f44` (`tester/research/physical-models-issue-4`), whose complete
`docs/backend/INTEGRATION_CONTRACT.md`, `RT_OPTIMIZATION.md` and native
capture/relay implementation were read before integrating this bridge.

The original baseline backend remains compatible for controls/waveform but has
no continuous spectrum. Merely having this library does not establish that the
browser has connected to an appropriate producer.

## Build

The crate remains an independent workspace. `wasm-bindgen` is an optional,
**wasm32-only** dependency pinned to 0.2.130; native/default builds do not compile
it. The CLI version must match the crate version. Installation and downloads
retain normal registry checks/TLS validation:

```sh
cd /workspace/prophet-xii
. /workspace/.prophet-cloud/env.sh
export CARGO_HOME=/workspace/.prophet-cloud/analysis-cargo
export CARGO_NET_OFFLINE=false
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.130 --locked \
  --root /workspace/.prophet-cloud/wasm-bindgen
cargo build --manifest-path crates/prophet-analysis/Cargo.toml \
  --release --features web --target wasm32-unknown-unknown --locked
/workspace/.prophet-cloud/wasm-bindgen/bin/wasm-bindgen \
  target/wasm32-unknown-unknown/release/prophet_analysis.wasm \
  --target web --out-dir /tmp/prophet-analysis-web --out-name prophet_analysis
node crates/prophet-analysis/tests/web_runtime.mjs /tmp/prophet-analysis-web
```

The resulting JS, `.d.ts` and WASM files are generated outputs, not vendored
source. The UI build generates/copies them into its ignored
`ui/generated/analysis` directory. This bridge change does not commit a native
binary, tool installation or generated WASM to Git.

Native integration validation, including the existing allocation counter:

```sh
cargo test --manifest-path crates/prophet-analysis/Cargo.toml \
  --release --features web --locked
cargo fmt --manifest-path crates/prophet-analysis/Cargo.toml -- --check
```

## Exported ABI

All getters below are methods. Constructor errors throw; packet processing uses
numeric status codes so invalid packets do not allocate JS error strings.

```ts
new WebAnalyzer(
  fftSize, sampleRate, windowCode,
  averageAlpha, peakHoldFrames, peakDecayDbPerFrame, kaiserBeta
)
```

Supported sizes: 2048/4096/8192. Window codes: 0 periodic Hann, 1 Blackman–Harris,
2 Kaiser. Hop is N/4 (75% overlap). The remaining configuration validation and
amplitude conventions are the same as the native analyzer. The bridge caches no
separate JS interpretation of the window math.

| Method | Meaning |
|---|---|
| `input_ptr()`, `input_capacity()` | byte offset and capacity of 64 writable f32 samples |
| `db_ptr()`, `peak_ptr()`, `bins()` | byte offsets of read-only-by-convention latest dBFS/peak arrays, N/2+1 floats |
| `fft_size()`, `sample_rate()` | actual constructor configuration |
| `coherent_gain()`, `enbw_bins()`, `enbw_hz()` | metrics from the actual Rust window coefficients |
| `push(sequence, firstFrame, len)` | consume 1–64 samples already written into input storage; returns 0/1 completed frames |
| `has_frame()` | whether a complete current-generation/gap-free frame is available |
| `first_frame()`, `end_frame()` | latest window's absolute first/exclusive-end sample indices |
| `sequence()` | latest completed analysis frame sequence |
| `discontinuities()` | number of acquisition gaps seen by this analyzer |
| `free()` | dispose the analyzer; all its buffer pointers then become invalid |

`push` negative status values:

- -1: packet length zero or above 64;
- -2: negative/fractional/nonfinite or unsafe sequence/first-frame integer, or
  packet end beyond `Number.MAX_SAFE_INTEGER`;
- -3: nonfinite/out-of-bound sample;
- -4: duplicate/reordered/overlapping old packet;
- -5: other analysis failure.

Input validation happens before STFT state mutation. Sequence/frame values are
passed as f64, validated as JavaScript safe integers, then converted to u64.
The producer's u64 counters must not be silently rounded by the consumer.

## Worker ownership and memory

Instantiate the module and `WebAnalyzer` **inside a dedicated Worker**. Parse and
validate the actual capture messages there. Write the next packet into
`new Float32Array(wasm.memory.buffer, analyzer.input_ptr(), 64)` while no Rust
call/borrow is active, then call `push`. No Rust `unsafe` code is required for
this bridge. Pointer storage uses retained vectors and is stable until `free`.

JavaScript must reacquire typed-array views whenever `wasm.memory.buffer`
changes: any WASM memory growth detaches old views. Construction of another
analyzer or a caller-initiated `memory.grow` can cause that. Never reuse views
after `free` or write into the output buffers. The live packet path itself
performs no Rust heap allocation and does not grow memory in the runtime tests.

Do not send every 64-sample packet through React. Only publish completed spectra
at a bounded chart cadence; replace the last pending output rather than growing
a message/result backlog. The producer uses real contiguous post-FX/master left
PCM. Neither the old 1024-sample scope snapshots nor interpolated/zero-filled
gaps may substitute for those packets.

On a future sequence/sample gap the STFT clears overlap and averaging/peak
history; `has_frame()` becomes false until N fresh contiguous samples arrive.
Duplicate/old packets return -4. On disconnect/reconnect or sample-rate/config
change, free/recreate the analyzer and discard pending old Worker results using
a client generation token. A new valid window takes N/fs seconds to acquire
(85.33 ms at 4096/48 kHz), independent of UI animation rate. `firstFrame` denotes
generated stream time, not device playback/acoustic time.

## Evidence and limits

The native `web` suite passes 18 tests: 15 existing accuracy/stream tests, two
bridge packet/validation tests and the allocation test. That counter includes
128 bridge packet calls and invalid-argument paths and records zero allocations
after construction. Native tests exercise the exact bridge implementation,
without a target-only fake analyzer.

The actual generated WASM executes in Node, with 27 combinations of N,
44.1/48/96 kHz and Hann/BH/Kaiser. It verifies -6.0206 dBFS sine amplitude,
Rust-derived window metrics, timestamps, stable pointers/no packet memory growth,
short chunks, gaps/stale packets, safe integer/sample rejection and refreshing
typed-array views after forced `memory.grow`. This is real WASM execution, not a
mocked browser module. Raw outcomes are saved in `web-native-tests.txt`,
`web-wasm-build.txt` and `web-runtime.json`, covered by `SHA256SUMS`.

Live capture/socket/browser Worker validation belongs to the UI integration and
is recorded separately; standalone module PASS does not establish that endpoint
or device behavior. The actual `/api/v1/filter` remains the fixed-48-kHz DTFT
worker in B's checkpoint. This bridge does not replace it or claim a live VCF FFT
endpoint. Earlier independent native benchmarks are reused; no unrelated
benchmark repetition or ARM64 runtime/performance claim is introduced here.
