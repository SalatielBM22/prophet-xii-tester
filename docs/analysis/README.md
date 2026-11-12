# Non-RT spectral analysis, contract 1

`crates/prophet-analysis` is an independent Rust workspace/library. It does not
import the synth, an audio backend, React, network code or devices. The root
workspace/lockfile and the baseline DSP are untouched. The host must call it in
an analysis worker, never the audio callback. It is implemented and tested in
isolation; the running v1 host does **not** yet produce continuous analysis data.

## Build and validate

The cloud environment's original Cargo home replaces all crates with the baseline
vendor archive, which intentionally does not contain these new FFT dependencies.
Use a separate home; retain the installed Rust toolchain/proxy/CA settings:

```sh
cd /workspace/prophet-xii
. /workspace/.prophet-cloud/env.sh
export CARGO_HOME=/workspace/.prophet-cloud/analysis-cargo
export CARGO_NET_OFFLINE=false
cargo test --manifest-path crates/prophet-analysis/Cargo.toml --release --locked
cargo fmt --manifest-path crates/prophet-analysis/Cargo.toml -- --check
cargo bench --manifest-path crates/prophet-analysis/Cargo.toml --bench fft --locked
rustup target add aarch64-unknown-linux-gnu
cargo check --manifest-path crates/prophet-analysis/Cargo.toml --target aarch64-unknown-linux-gnu --release --locked
```

The new crate has its own tracked lockfile and ignores local `target/`. No
downloaded PFFFT source, native library, compiler output or private material is
part of the change. The usual Rust registry integrity checks remain enabled.

## Spectrum API and conventions

`SpectrumAnalyzer::new(Config)` plans the real R2C FFT and creates window, input,
complex output/scratch, power, dBFS, peak, peak-age and PSD buffers once. Supported
sizes are 2048/4096/8192; default N=4096, fs=48000 Hz, periodic Hann. Periodic
four-term Blackman–Harris and symmetric Kaiser (beta 0–20) are selectable.
`analyze(&[f32])` requires one complete contiguous mono frame and returns a
borrowed `SpectrumView`; consume/copy it before the next analysis call. Plan and
twiddle reuse is provided by realfft/rustfft. No custom SIMD code is introduced.

For unnormalized FFT X, window w, coherent sum S=sum(w), and edge multiplier
m=1 at DC/Nyquist and m=2 at positive interior bins:

- one-sided amplitude is m*abs(X)/S;
- `power` is the square of that amplitude, exponentially averaged in **linear
  power**, with `average_alpha` as the weight of the new frame;
- `dbfs` is 10*log10(power), floored to the configured finite dBFS floor;
- a bin-centered peak-amplitude 0.5 sine reads -6.0206 dBFS;
- `psd` is instantaneous m*abs(X)^2/(fs*sum(w^2)), in FS^2/Hz. Integrating PSD
  over bin width recovers window-energy normalized mean square. This is distinct
  from the peak-amplitude spectrum and its smoothing;
- `window_metrics()` returns coherent gain, ENBW in bins and Hz; Hann gives 0.5
  and 1.5 bins respectively;
- `peak_hold_frames` and `peak_decay_db_per_frame` operate on the displayed
  averaged dBFS. Their time duration depends on analysis cadence, not chart FPS.

DC and Nyquist are never doubled. Frequency of bin k is k*fs/N. A logarithmic
chart must exclude DC, start at a declared positive Hz and stop at Nyquist.
Finite audio overshoot above 0 dBFS is supported; nonfinite values and amplitudes
above 1e10 are rejected before state mutation. Silence/subnormals yield finite
floor values. `reset_history()` resets average/peak after a stream discontinuity.
`memory_bytes()` reports retained data buffers, excluding the FFT plan's internal
tables, allocator overhead and enclosing object's metadata.

## Continuous STFT and backend integration boundary

`Stft::new(config, hop)` preallocates a ring and frame buffer in the **consumer**.
Use hop=N/4 for 75% overlap (default recommended integration) or N/2 for 50%.
`push(sequence, first_sample, samples, callback)` accepts bounded contiguous
packets. It emits only after N contiguous samples, and then once per hop. Input
sample indices and output intervals are absolute within one stream generation;
`end_sample` is exclusive. Time is sample_index/fs, not a browser wall clock.

Missing sequence/sample ranges reset overlap **and** average/peak history and
increment `discontinuities`. Old, overlapping, duplicate or reordered packets are
rejected without changing state. A new device/sample rate/stream generation must
construct a new `Stft`; do not reset timestamps inside the same stream object.
Output frame sequence remains monotonic across gaps. The callback receives a
borrowed spectrum and `FrameMetadata`, preventing an implicit growing result
queue. Packet validation scans before mutation; an invalid packet produces no
partial spectrum.

Agent B must implement the producer and transport. Suggested versioned producer
payload, **not an endpoint implemented by this change**:

```text
streamId: string             changes on restart/rate/config change
sequence: u64                increases once per packet
firstSample: u64             absolute sample index in streamId
sampleRate: number           exact producer sample rate
channel: "mono-left"         explicit channel/downmix convention
samples: contiguous f32[]    packet maximum negotiated and bounded
droppedPackets: u64          producer overflow counter
```

The callback producer may copy a bounded sample block into an existing bounded
SPSC ring and increment counters, dropping a complete packet on overflow. Window
generation, FFT, JSON/network I/O and UI drawing belong to workers. Consumer
results should use a capacity-one latest-frame slot for UI publication. A slow
chart drops old completed analysis frames rather than increasing audio latency.
No claim of RT deadline impact is made until B integrates/measures that path.

The v1 `scope[1024]` snapshots at ~25 Hz contain unknown inter-snapshot gaps.
They remain suitable for a labelled waveform/single captured frame, **not** for
continuous STFT, overlapping windows or manufactured N=4096/8192 resolution.
The production UI must show continuous spectrum unavailable for that transport.

## Transfer analysis and phase

`TransferAnalyzer::new(fft_size)` reuses an f64 real FFT, scratch, dense phase and
output buffers. Powers of two from 2048 through 262144 are accepted. Caller
supplies an impulse, actual sample rate, strictly increasing frequency grid,
phase floor and output storage. The impulse is **unwindowed**, is zero padded,
and receives no audio-spectrum/coherent-gain normalization: abs(H)=1 means 0 dB
transfer gain. Complex bins are linearly interpolated onto log Hz; phase itself
is never interpolated across its wrapped branch cut.

The dense FFT grid is unwrapped before log resampling. Interpolated complex phase
is assigned the turn consistent with the dense phase anchor, so sparse log
points do not silently lose whole turns. Phase below the caller's magnitude
floor, or next to masked interpolation bins, is `None` (serialize as null/break
the path). Unwrapping resets after undefined regions. Padding must be dense
enough that adjacent FFT bins resolve phase changes below pi; extremely long
pure delays require a larger FFT. Interpolation density is not measured spectral
resolution. Increasing impulse duration is what captures a long/high-Q tail.

`TransferMetadata` reports actual sample rate, impulse length, FFT size, bin
spacing and final-eighth RMS relative to whole-impulse RMS. `tail_converged` uses
the declared 1e-4 relative-RMS diagnostic threshold; it is a warning for the
backend to extend/recompute an impulse, not proof of an arbitrary filter's
physical fidelity. This crate cannot generate the synth's impulse response.

`RequestClock::next()` creates a token that supersedes previous jobs. `analyze`
checks it before/after FFT and during phase/resampling work, returning
`Error::Superseded`; the in-progress FFT itself is not interruptible. Agent B
should keep at most one pending request per chart and discard superseded output
even on cancellation races. The caller-owned output may be partially changed
when cancelled, so publish it **only** on successful current completion.

## Numerical evidence

15 accuracy/stream tests plus one dedicated allocation test pass in release mode.
Coverage includes every N at 44.1/48/96 kHz, all windows, known sine amplitude,
off-bin Hann scalloping, two-tone separation, DC/Nyquist, PSD Parseval, white and
Voss pink noise tilt, BH sidelobes, impulse, silence/subnormal/no-NaN behavior,
linear-power averaging, peak hold, STFT packet partitioning/gaps/stale packets,
forward/inverse roundtrip, phase masking/unwrapping, stale request cancellation,
tail convergence, lowpass and high-Q resonator impulses.

The independent numerical oracle is an explicit f64 direct DTFT summation, not
the FFT implementation under test. Spectrum bins above -90 dBFS agree within
0.01 dB. 4096-sample lowpass impulses sampled with a 65536 FFT at 48/96 kHz agree
with 256 direct log-frequency DTFT values within 0.002 dB and 0.001 rad. A
32768-sample, r=0.999 resonator with a 262144 FFT agrees over its resonant band
within 0.002 dB and 0.0005 rad. These are **algorithmic fixture** tolerances, not
hardware/device equivalence thresholds. Agent B still needs to test actual VCF
impulses, its endpoint, sample-rate metadata and superseded-job scheduling.

The separate allocation test enables a global allocator counter only after
planning and verifies zero allocations over repeated spectrum/STFT/transfer work.
Output JSON serialization and transport are intentionally outside this count.

## Choice of FFT and optional PFFFT reference

realfft 3.5.0 uses rustfft 6.4.1, selected here because it exposes planned R2C,
reusable scratch and pure-Rust deployment without a new C/FFI boundary. rustfft's
default features include AVX/SSE/NEON with platform dispatch. AArch64 release
`cargo check` passed; no ARM64 executable was run and no Moto G8/native NEON
timing is claimed.

PFFFT was evaluated as an independent optional C reference using
[marton78/pffft](https://github.com/marton78/pffft), commit
`efc1dd2de6c3562fc464f905d37d9281f08a20dd`. Its source/header retains the
UCAR/NCAR FFTPACK and author notices and BSD-like license conditions. No PFFFT
source is copied or translated into this crate. The original harness is
`crates/prophet-analysis/benches/pffft_reference.c`; it validates selected real
bins against a direct DFT and verifies forward/inverse reconstruction before
timing. To reproduce with separately fetched pinned sources:

```sh
git clone https://github.com/marton78/pffft.git /tmp/prophet-pffft
git -C /tmp/prophet-pffft checkout efc1dd2de6c3562fc464f905d37d9281f08a20dd
cc -O3 -std=c99 -I/tmp/prophet-pffft/include/pffft -I/tmp/prophet-pffft/src \
  crates/prophet-analysis/benches/pffft_reference.c \
  /tmp/prophet-pffft/src/pffft.c /tmp/prophet-pffft/src/pffft_common.c \
  -lm -o /tmp/prophet-pffft-reference
/tmp/prophet-pffft-reference
```

See [BENCHMARKS.md](BENCHMARKS.md) and the raw CSVs. PFFFT is competitive; the
measurements do not justify a custom SIMD implementation or a claim that Rust is
intrinsically faster. Adding an FFI production dependency would require aligned
buffers, lifecycle/error safety, build tooling, license notices and separate
platform validation. Retaining the smaller pure-Rust integration is a deliberate
cost/portability choice, not a speed claim.

## Remaining integration work

- Agent B: bounded continuous producer/analysis worker, versioned capabilities,
  result metadata, telemetry endpoint and sample-rate/generation semantics.
- Agent B: actual VCF impulse generation, convergence extension and endpoint
  replacement; the current v1 DTFT endpoint remains unchanged.
- Native ARM64 execution and timing, integrated audio-load/deadline tests, and
  1/2/12-voice end-to-end spectrum validation need that producer and hardware.
- This library's independent PASS does not mean the running application has an
  integrated continuous spectrum or a verified analog circuit model.
