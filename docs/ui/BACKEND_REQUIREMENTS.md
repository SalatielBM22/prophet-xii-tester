# UI/backend contract — proposal 2.0.0

Status: Agent A consumer proposal, pending implementation and agreement by Agent B.
Baseline: `b47f2d60c7005cd6c81694c0e17c76f7a4f107dc`, issues [#1](https://github.com/AdhonisBR/prophet-xii/issues/1), [#2](https://github.com/AdhonisBR/prophet-xii/issues/2), [#3](https://github.com/AdhonisBR/prophet-xii/issues/3), [#4](https://github.com/AdhonisBR/prophet-xii/issues/4), including the three technical comments on #1. No producer or endpoint changes are included in Agent A's work.

## Negotiation and compatibility

Keep existing `/api/v1/meta`, `/api/v1/state`, `/api/v1/ws`, `/api/v1/command` and `/api/v1/filter` routes. Do not infer features from a version alone. Metadata may advertise `protocolVersion: 2` and an explicit string array `capabilities`: `parameterAck`, `factoryPresetValues`, `midiLearnLifecycle`, `voiceEnvelopes`, `reverbBypass`, `continuousSpectrum`, `filterResponseV2`. Absent capabilities mean unsupported. Commands and responses below are proposals on those negotiated routes, not claims that v1 implements them.

Keep parameter IDs 0–82 unchanged and append new IDs; preserve serialized preset version 1. Validate finite numbers, bounds and enum domains at both ends. Unknown optional fields must not break v1. Unknown required major versions must produce an unsupported state rather than simulated telemetry. Session changes invalidate revisions, pending acknowledgments and frame ordering. Authentication continues using the existing token mechanisms; never log tokens.

## Parameter changes, presets and ordering

V2 command envelopes add a random client session `clientId` and monotonically increasing integer `commandId`. For `parameterAck`, telemetry adds `parameterRevision`, `appliedCommandId`, `sequence` and `sampleTimestamp`; acknowledgment identifies this client. `parameterRevision` increases only when authoritative parameter state changes. Acknowledgments must mean the audio/control state has applied a command, not merely that HTTP accepted it. Rejected/expired commands need explicit errors. Preserve order per client; bounded coalescing can discard superseded parameter values but must preserve final gesture value and causal snapshot/preset/panic order.

Metadata capability `factoryPresetValues` supplies an immutable `values` array for each factory preset. A preset request must telemetrize `presetId` and `factoryPresetRevision` matching the metadata preset's `referenceRevision` before reset becomes enabled. Do not use a generic minimum or editable current snapshot as a factory default. A v1 host does not expose an authoritative factory reference; reset is unavailable unless a reference can be established without ambiguity. Imports and user snapshots do not become factory defaults.

The v1 adapter uses matching-value echoes as a bounded reconciliation heuristic; it cannot prove causal acknowledgment or distinguish coincidental values from external automation. UI must expose this limit, keep active gestures protected against stale snapshots, and report timeout/send errors instead of pinning values forever. Strong delayed/reordered-command guarantees require v2. Snapshot payloads must be validated in full before updating state.

## MIDI Learn

Capability `midiLearnLifecycle`: proposed command `type: "midi_learn"`, `action: "start" | "cancel" | "remove" | "replace" | "query"`, `parameterId` and, for replacement, explicit `channel` (0–15) and `cc` (0–127). Clarify global/channel mapping policy; conflict replacement must be atomic and reported. Telemetry `midiLearn` contains `state: "idle" | "pending" | "assigned"`, nullable `parameterId`, and `bindings: [{parameterId, channel, cc}]`. Persist binding changes in host/plugin state and provide the authoritative table after reconnect. Removal targets a parameter explicitly; cancellation changes pending capture without deleting an existing binding. V1's `learn` command can only start capture; cancel/remove/query/replace and a confirmed assigned indicator remain unsupported there. Local dismissal must not claim to cancel backend capture.

## Envelopes and clocks

Capability `voiceEnvelopes`: `envelopes: {amp: VoiceEnvelope[12], filter: VoiceEnvelope[12]}`. Each voice has `stage` (`Idle`, `Attack`, `Decay`, `Sustain`, `Release`), actual `level`, `elapsedSamples`, optional finite `stageDurationSamples`, and `noteGeneration`, a generation counter for retrigger/steal. Frames carry `sequence`, `sampleTimestamp` (absolute rendered sample index) and actual `sampleRate`. Stage/time comes from the producer; amplitude alone is insufficient. On sample-rate or generation change the consumer must rebase rather than extrapolate old progress. Idle/Sustain may have indeterminate duration. Dropped frames are visible via sequence discontinuity; retain bounded latest-frame state and never queue unbounded animation work.

Programmed ADSR is a separate nominal model, using the current kernel's attack target 1.2, exponent ln(6), decay/release exponent 4.8, undershoot epsilon 0.0015 and release threshold 0.0001. Durations follow threshold crossings, not arbitrary fixed widths; the time axis uses seconds and the configured sample rate. Per-voice RC dispersion, retrigger level and instantaneous edits require actual voice telemetry. This describes the current virtual analog kernel; it does not establish electrical fidelity to a CEM3310.

## Reverb

Capability `reverbBypass`: proposed command `type: "reverb_bypass"`, `enabled: boolean`, `tailPolicy: "spill" | "kill"`. Authoritative telemetry must expose `reverb: {enabled, tailPolicy, transition: "stable" | "crossfading", tailStatus: "active" | "decaying" | "cleared" | "frozen"}`. Agent B owns a bounded, click-free transition and defines state advancement under dry/wet and freeze. V1 `ReverbWet=0` is dry mix, **not** independent bypass. The UI must show bypass and tail feedback as unavailable on v1. Do not relabel Decay as Size; any future room-size parameter requires a new stable ID and measurable implementation.

Existing dependencies: Ensemble Blend requires Ensemble mp/forte; Vowel Morph requires Vowel Mix and is ineffective in freeze feedback; shimmer pitch/fifth requires Shimmer; modulation Speed requires Depth; unsynced Pre Delay is milliseconds, BPM/Beats are applicable only with Sync. UI disabled states explain each condition. These mappings do not prove audible sensitivity; Agent B must supply 48/96 kHz impulse/note sensitivity, tail/freeze and output stability evidence. No claim of proprietary Solaris/Cloudburst equivalence.

## Continuous spectrum and transfer response

Capability `continuousSpectrum`: transport either continuous sample chunks or precomputed spectrum frames **off the audio callback**. Samples need monotonically increasing `sequence`, absolute `firstSample`, `sampleRate`, `channel`, bounded data length, reset/session ID and explicit discontinuity/overrun count. STFT consumer resets history after a gap or rate change. V1 1024-sample scope snapshots have unknown gaps and cannot serve as a continuous STFT stream.

Spectrum frames need `sequence`, `firstSample`, `sampleRate`, `fftSize` (2048/4096/8192), `hopSize`, `window` (Hann/BH/Kaiser + beta), `coherentGain`, `enbwBins`, `normalization: "single-sided-amplitude-dbfs"`, `dbfs`, optional `peakDbfs` and averaging/peak configuration. DC/Nyquist are not doubled. Frequency is `k * sampleRate / fftSize`, with DC omitted from logarithmic plots. Average linear power before conversion to dB. The library API and numerical gates are documented under `docs/analysis/`.

Capability `filterResponseV2`: actual `sampleRate`, `sequence`/request generation, impulse length, FFT length, tail-convergence diagnostic, `hz`, `db`, unwrapped `phase` with undefined values represented as `null` below a declared magnitude floor. Interpolate complex FFT values; don't interpolate wrapped phase. Transfer magnitude is |H| in dB, not window-normalized audio dBFS. The current endpoint is a 4096-sample DTFT at fixed 48 kHz, regardless of live sample rate; legacy display must identify that rate explicitly. Latest request wins; abort obsolete fetches and drop late generations. Producer must coalesce bounded pending work; aborting browser fetch alone cannot stop already spawned backend computation. FFT zero padding refines interpolation, not physical resolving power. Agent B owns endpoint integration and impulse acquisition/tail convergence.

## Producer acceptance checklist

- Real v1 regression compatibility, 12 simultaneous voices, 44.1/48/96 kHz, bounded payloads and queues.
- Ack/revision/session tests with delayed, dropped and reordered transport; rejection and reconnect.
- Full MIDI lifecycle including conflicting CC and persistence.
- Envelope idle/attack/decay/sustain/release, retrigger/steal and sample-rate transitions; no guessed stages.
- Bypass kill/spill/freeze, audible parameter sensitivity and numerical stability.
- Continuous spectrum gap detection and independent-reference FFT/phase tests.
- No FFT, allocation, JSON, network work or UI callbacks on the audio thread. DSP performance/latency and hardware measurements belong to Agent B; WSLg buffering must be separated from callback cost.
