# Native UI integration contract — additive v1

The authoritative producer contract is Agent B's [INTEGRATION_CONTRACT.md](https://github.com/SalatielBM22/prophet-xii-tester/blob/211c4f44dc735a89047dad32e725e39b9b8c9b68/docs/backend/INTEGRATION_CONTRACT.md).
Its [RT_OPTIMIZATION.md](https://github.com/SalatielBM22/prophet-xii-tester/blob/211c4f44dc735a89047dad32e725e39b9b8c9b68/docs/backend/RT_OPTIMIZATION.md) was read in full alongside actual source before integration.

This replaces the earlier Agent A v2 proposal. The implemented native wire protocol
remains `v:1`, IDs 0–82, with additive fields and `meta.extensions`. No v2
client/session fields, synthetic endpoints, per-channel MIDI mappings or guessed
envelope stages are sent. Capability names inside React are adapter details.

## Acceptance versus application

`POST /api/v1/command` returns HTTP 202 and `acceptedCommandId` for enqueue.
Only `parameterCommandIds[id] === acceptedCommandId` confirms application of a
direct parameter command. Snapshot/preset confirmation checks all 83 IDs. The
global `control.lastAppliedCommandId` is not a watermark for another client.

The store retains optimistic intent through stale telemetry and through a delayed
202 response, then reconciles the latest authoritative frame after the host ID
is known. It rejects regressed sequence/revision/frame/monotonic counters until
reconnect. Explicit dropped IDs, competing newer per-parameter stamps, send
errors and a four-second timeout produce visible errors/resynchronization rather
than false confirmation. Drag values coalesce in a bounded ordered HTTP lane.
Preset/panic commands are barriers. Reconnect invalidates in-flight generations
and factory references; two advancing snapshots establish connected state.
A three-second frame-progress watchdog rejects a stagnant ready:true snapshot.
This is a UI liveness check, not audio-device recovery or a latency measurement.

The original backend without extensions retains a labelled matching-value echo
heuristic; it has no causal ACK. No missing feature is inferred from version 1.

## Factory reference and controls

Metadata presets contain the loader's original 83 values. The UI remembers the
selected preset's immutable values only after observing all 83 applied IDs equal
to that preset request. Double-click, Alt+Enter and the explicit reset button use
that reference. Ordinary edits do not replace it; imports/slots/external factory
snapshots invalidate it where observable. No fabricated presetId/referenceRevision
fields are required. The backend has no selected-preset identity field, so a
freshly connected client must select/confirm a factory preset before reset.

Numeric editors validate finite/range/integer/enum domains. Pointer and keyboard
gestures produce one history entry; numeric edits/reset also support Undo/Redo.
Chorus I/II and Mode share one local atomic value. PW, Ensemble, Vowel/Shimmer,
modulation and BPM/Beats controls explain their actual conditions.

## MIDI Learn and bypass

Existing `learn{id}` starts capture. Native commands are `cancel_learn`,
`unlearn{id}`, `remap{cc,id}` and
`reverb_bypass{enabled,tail:"spill"|"kill"}`. No query command is invented:
`midiLearn:{active,target,ccMap[128]}` is authoritative state. Mappings are global
across channels; CC64/120/123 are reserved and MPE-channel CC74 takes precedence.
UI pending/assigned/idle comes from this state, not knob movement.

Reverb status is exactly `{enabled,tail,dormant,enableGain}`; the UI displays these
fields, independent wet/dry, and spill/kill selection. It does not rename dormant
as silence, infer an invented tail state, alias Decay to Size or claim calibrated
RT60. B owns the fade/tank behavior. Durable MIDI-map storage, full JNI parity and
audible-device qualification remain open in B's report.

## Envelopes

`ampEnvelopes[12]` and `filterEnvelopes[12]` contain numeric stage 0–4, level,
elapsedSamples, progress and timed. React names the actual stages and uses the
provided progress; Idle/Sustain remain indefinite. Seconds are elapsed/base
sample rate. Voice activity follows activeVoiceMask because an inactive filter
envelope can retain its previous stage. No generation or stage is inferred from
amplitude. Programmed ADSR remains a separate nominal visualization of the actual
RC law/thresholds, tested against the unchanged baseline f32 kernel at 48/96 kHz.
This is not a physical CEM3310 calibration claim.

## Continuous capture and Rust analysis

Only the advertised `continuous-capture` extension opens the existing authenticated
`/api/v1/capture` WebSocket. The Worker validates the real payload:
`{v:1,blocks:[{sequence,firstFrame,sampleRate,samples}]}`, max 64 samples/chunk
and a bounded message. Samples are actual post-FX/master mono-left output.

The same device-independent Rust STFT is compiled to WASM in a dedicated Worker.
Plans, input/output/scratch and the 64-sample bridge buffer are reused. Packet
sequence and absolute frame positions detect gaps; Rust resets overlap, averages
and peak history and waits for N fresh contiguous samples. Reconnect, invalid
payloads, sample-rate/window/N changes also clear history. No zero filling or
scope-snapshot concatenation disguises loss. Old/overlapping packets are rejected.
Drawing is capped at 30 Hz; scope snapshots and state remain separate.

The UI configures N=2048/4096/8192, Hann/BH/Kaiser, linear-power averaging and peak
hold/decay. It shows actual rate/bin spacing/window gain/ENBW and sample indices,
with a positive logarithmic Hz axis and single-sided amplitude dBFS. See
[analysis/WASM.md](../analysis/WASM.md) for bridge/build/runtime checks.

## Transfer response and remaining work

The actual `/api/v1/filter` is still B's fixed-48-kHz, 4096-sample DTFT worker.
The UI labels that rate explicitly even when audio runs at 96 kHz. It unwraps
the supplied phase samples, breaks below -100 dB, uses log-frequency coordinates
and keeps transfer magnitude dB separate from the live output dBFS spectrum.
A capacity-one pending/latest queue permits only one request in flight per chart;
obsolete responses are discarded without spawning a new HTTP measurement before
the previous request finishes.

The independent Rust FFT impulse/complex interpolation/dense unwrap/cancellation
implementation is tested and benchmarked, but the backend exposes neither an
impulse-response transport nor that analyzer. Replacement of the DTFT endpoint,
actual VCF tail-convergence integration and long-delay phase ambiguity on the
legacy sparse grid remain OPEN for joint work; no endpoint speedup is claimed.

Issues #1–#4 remain open where deadlines, RT60, physical fidelity, device/JNI/DAW
or ARM64 runtime validation are missing. UI integration does not qualify those
unmeasured properties.
