# Backend integration contract (#1, #2, #3)

Status: implemented native contract, HOST_VERIFIED on Linux null host. No React or
analysis-crate changes. Protocol `v:1`, parameter IDs/order 0..82 and existing
routes are retained. New fields are additive. Do not infer protocol parity across
native/JNI/plugin: differences are listed below.

## Native HTTP and acknowledgments

`POST /api/v1/command` still accepts `param`, `midi`, `preset`, `snapshot`, `learn`,
`panic`, `trimpot`. New commands:

```json
{"type":"cancel_learn"}
{"type":"unlearn","id":23}
{"type":"remap","cc":16,"id":23}
{"type":"reverb_bypass","enabled":false,"tail":"spill"}
```

Tail is `spill` or `kill`. Invalid IDs, reserved CC64/120/123 and out-of-range
scalar parameters return 400. Full ingress queue returns 429. Accepted replies
are 202 with `{"acceptedCommandId":N}`; this acknowledges enqueue, not playback.
IDs belong to this host session. If the later RT queue overflows, the relay
records `control.ingressDrops`, `lastDroppedCommandId`, and schedules a bounded
All Notes Off retry. Hardware-MIDI ingress loss is separately counted and also
requests All Notes Off. An accepted command can therefore be reported dropped.

`GET /api/v1/state` and `/api/v1/ws` retain all old fields. New:

| Field | Meaning |
| --- | --- |
| `revision` | Monotonic parameter-bank mutation counter, including learn/bypass operations; multiple increments per command are permitted |
| `parameterCommandIds[83]` | Most recently applied **direct** param/snapshot/preset command ID per parameter; zero means none. MIDI changes use revision/values, not this array |
| `control.lastAppliedCommandId` | Last stamped command applied, not a global watermark for concurrent clients; 0 can denote internal safety panic |
| `control.appliedFrame` | Generated frame index of that command's application |
| `sequence` | Telemetry publication sequence; gaps are legal |
| `monotonicNs` | Nanoseconds since AudioProcessor creation, sampled at publication, not wall time |
| `captureDroppedChunks` | RT capture transport drops, not device underruns |

The single audio owner publishes values after bounded command application. Agent A
should retain an in-flight edit until its per-parameter command acknowledgment,
then use the authoritative revision/value; coalesce obsolete drag updates. Do not
overwrite pending edits with an older snapshot or treat HTTP 202 as application.
Restart/reconnect resets session counters; flush client pending state on restart.
No durable user preset or MIDI-map storage is added. `ccMap` can be exported by
the non-RT client and restored by explicit remap commands; automatic persistence
and DAW MIDI-map state remain OPEN.

`GET /api/v1/meta` supplies all parameter specs (min/max, integer, logarithmic,
group) and all ten factory presets with their original **83 values** produced
through the same loader as audio. Numeric reset must use the selected preset's
value, not a universal default. Reverb bypass is an extension, not a renumbering
of the old parameter bank. Core setters reject nonfinite values and clamp/round
finite values for legacy in-process callers; HTTP scalar input is range-checked.

## ADSR telemetry and programmed curve

`ampEnvelopes[12]`, `filterEnvelopes[12]`: `{stage, elapsedSamples, level,
progress, timed}`. Stage codes are 0=Idle, 1=Attack, 2=Decay, 3=Sustain, 4=Release.
Counters advance at **base sample rate**, not oversampling rate; reset on stage
entry, note-on/retrigger and note-off into Release. Sustain/Idle have no finite
end (`timed:false`, `progress:0`); draw a stage indicator, not fake timeline motion.

With `c` the actual single-precision coefficient, `lambda=-ln(1-c)`, remaining
samples are `ln(distance/threshold)/lambda`, capped below at zero. Progress is
`elapsed/(elapsed+remaining)`. It is time progress, not level. Current parameters
may change mid-stage, so this is a current-law estimate, not a promise of future
duration. Snapshots are evaluated at at most ~25 Hz in the callback's bounded
diagnostic publication, never per oscillator/filter sample. No JSON is generated
there. Lower-level callers request `kernel.telemetry_snapshot()` explicitly;
`last_telemetry` alone contains only cheap block diagnostics.

Programmed shape must describe the actual normalized RC law:

- Attack: `level += (1.2-level)*cA`, threshold 1, `cA=1-exp(-ln(6)/(A*disp*fs))`.
- Decay: target `S-.0015`, coefficient `1-exp(-4.8/(D*disp*fs))`, stop at S.
- Release: target `-.0015`, same 4.8 constant over R, stop at .0001, then zero.
- Sustain: `level += (S-level)*.02`, indefinite duration.

Thus Release from level 1 lasts approximately `ln(1.0015/.0016)/4.8 * R`,
about 1.34 R, not exactly the knob value R. Decay time depends on starting level
and sustain. Voice RC dispersion applies independently; retrigger preserves
current level unless explicitly reset. These are legacy DSP semantics, not a
validated CEM3310 voltage calibration. Agent A owns curve rendering and editing.

## MIDI Learn

`midiLearn = {active, target: id|null, ccMap: [id|null;128]}`. Maps are global
across channels, except existing MPE CC74 handling on channels 1..15 takes
precedence and cannot complete Learn there. CC64 is sustain; CC120/123 are panic.
Cancel stops capture; Unlearn removes all CCs mapped to the parameter and cancels
that pending target. Remap unassigns the parameter's previous CC and replaces the
selected CC's previous binding; last explicit remap wins. Capture completes by
calling the same remap operation. `active:false` plus ccMap reveals the result;
do not infer assignment from knob motion. Invalid operations preserve bindings.

## Reverb

`reverb = {enabled, tail, dormant, enableGain}`. Input enable and dry coefficient
ramp linearly over 5 ms. Enabled: `(1-wet)*dry + wet*tank`. Bypass/spill: ramp
to dry unity, stop new excitation, retain `wet*tank` until the tail dies. Bypass/
kill: also ramp wet contribution to zero, logically invalidate every delay and
enter dormancy. Kill requires no heap allocation or full delay-buffer memset.
Wet=0 mutes output wet but **continues advancing** any tank state; it is not an
enable control. Freeze blocks excitation, including new predelay recording.
Dormant means no tank processing is needed; it does not imply device silence
when the dry signal is present. No Size parameter was invented or aliased to Decay.

## Continuous PCM for Agent A

`/api/v1/capture` is an authenticated WebSocket with the same loopback/token rule
as state. Payload `{"v":1,"blocks":[{"sequence":N,"firstFrame":F,"sampleRate":FS,
"samples":[...]}]}`. Samples are mono left-channel **actual post-FX/master output**,
at most 64 per chunk, with final short chunks permitted. Frame is the first
sample's generated-stream timestamp, not device playback time. Consecutive
chunks obey `next.firstFrame = firstFrame + samples.length`. Sequence advances
even on drops. The callback copies into a preallocated 256-chunk SPSC (<=16384
samples, ~64 KiB PCM); the non-RT relay drains at 5 ms cadence into an 8-message
broadcast. Slow clients lose messages; reset STFT overlap on gaps, reconnect or
sample-rate change. No interpolation or zero filling may disguise a gap.
The older `scope[1024]` is a visualization snapshot, **not** continuous STFT input.
No FFT, windows, transport, serialization or lock is in the callback.

## Timing/backend interpretation

`callback`: observed frames, processingUs, inter-callback intervalUs, mean/p50/
p95/p99/max over the previous <=128 completed callbacks. Timing includes command
drain, DSP, capture and telemetry push; a publication includes timings through
the preceding completed callback. Percent is wall processing / frame budget.
Sorting/JSON occur outside RT. Intervals are observational, not certified scheduler
jitter or starvation diagnosis. Control wait is host ingress-to-application;
relayWait and rtQueueWait split it, excluding browser/MIDI-device/acoustic delay.

`backendBufferedUs` is CPAL playback timestamp minus callback timestamp, or null
for null/unsupported/invalid timestamps. It is an estimate supplied by backend,
not a loopback measurement or a complete count of ALSA/Pulse/WSLg queues.
`deviceErrors` counts callback error notifications; legacy `underruns` aliases it.
`driverXruns:null` is deliberate on CPAL: no comprehensive XRun counter exists
here. Deadline misses are CPU budget exceedances, a different metric.

`--sample-rate HZ`, `--buffer-frames N` allow explicit requests. Known ranges
default toward 256 frames; out-of-range requests fail. Unknown ranges use backend
default with a diagnostic, and reject explicit requests that cannot be validated.
Native Windows uses CPAL WASAPI without ASIO; Linux ALSA is retained. Device
recovery/reconnection and Windows/WSLg loopback remain NOT_TESTED; an error count
does not establish successful recovery.

## JNI and plugins

Android JNI retains command kinds 0..5, adds 6=Cancel, 7=Unlearn(a), 8=Remap(cc=a,
id=b), 9=ReverbBypass(enabled=a, kill=b), booleans 0/1. JSON state adds envelopes,
revision, MIDI map and reverb status. AArch64 Android code type-checks; no emulator
or device was run. Continuous capture/command timing parity and durable JNI state
remain OPEN. No Java/UI files were changed.

CLAP/VST3 retain p0..p82 and add stable BoolParam IDs `reverb_enabled` and
`reverb_kill_tail`. Defaults enabled/spill. Generic host controls and NIH-plug
parameter state own these settings. DAW automation/restore and audio-device
recovery are NOT_TESTED. Kernel changes are shared by all backends.

Reproduce: `cargo test --release --locked -p prophet-standalone --tests` and
`python crates/prophet-standalone/tests/host_contract.py` (build host first; Node
>=22 provides the WebSocket test client). Test ports 18878; no production process
or original binary is replaced.
