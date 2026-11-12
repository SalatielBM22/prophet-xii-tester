# Browser regression and performance runners

## Actual additive-v1 backend integration

`e2e_native.py` targets the implemented Agent B contract at backend commit
`211c4f4`. It reads genuine metadata, accepted command IDs, per-parameter applied
IDs, MIDI mappings, envelope stages/progress, reverb state and continuous PCM.
It does not simulate a protocol-v2 producer or invent lifecycle endpoints.

```sh
. /workspace/.prophet-cloud/env.sh
python3 ui/tests/e2e_native.py --base http://127.0.0.1:8792 --sample-rate 48000
```

Use a dedicated backend instance serving the integrated UI. The default sixteen
cases include the basic numeric/mouse/touch/conditional-control regressions,
factory resets from the actual ten-preset metadata bank, real native MIDI
start/cancel/remap/unlearn, conflicting/reserved CCs, global mapping behavior,
the MPE member-channel CC74 Learn exception, twelve actual envelope stages,
base-rate elapsed counters, retrigger, release/idle, reverb spill/kill and raw
post-FX capture with a deliberately suppressed real message.

Acknowledgment cases separately test an unrelated client's command, an external
same-parameter edit, an HTTP response delivered after application, and accepted
HTTP 202 while applied telemetry has deliberately been withheld. HTTP 202 is
never asserted to mean audible playback or application. The capture case checks
transport samples, chunk bounds, sequence/frame continuity and a detectable gap;
it explicitly does not substitute a raw-stream check for UI/WASM STFT reset
verification.

`native_spectrum` verifies real 220/440 Hz output through Rust WASM STFT and
browser rendering across FFT sizes/windows/averaging settings. Its test-only
loader imports the untouched production worker and hooks only message delivery:
it drops a genuine PCM packet and closes a genuine capture socket, then requires
a fresh contiguous window and visible reset count. The loader is generated
outside the checkout beside the build and removed after the case. No PCM,
frequencies or producer metadata are synthesized. `native_transfer` checks one
in-flight legacy DTFT request with one coalesced latest intent, fixed 48 kHz
label, and phase unwrap/masking against the real endpoint. `native_watchdog`
withholds genuine state delivery while the producer advances, requires disabled
controls, and verifies a fresh connection.

For a second native null host started with `--sample-rate 96000`, reuse selected
cases without overwriting 48 kHz evidence:

```sh
python3 ui/tests/e2e_native.py --base http://127.0.0.1:8793 --sample-rate 96000 \
  --case native_envelopes --case native_capture \
  --output /workspace/.prophet-cloud/task-a/e2e-native-96k
```

Reports, screenshots and SHA-256 sums go outside the checkout. These are native
software-integration checks on a null sink, not physical audio/MIDI-device,
Windows/WSLg latency, calibrated RT60, ARM-device timing or DAW qualification.

## Original-v1 regression

These Python runners use Playwright with the installed Chromium and write all
artifacts outside the checkout. They do not overwrite historical delivery
evidence. They require a running Rust test host serving the intended UI build.
Use a dedicated null-sink host/port and run suites sequentially: preset and MIDI
commands intentionally change that host's state.

```sh
cd /workspace/prophet-xii
. /workspace/.prophet-cloud/env.sh
python3 ui/tests/e2e_controls.py --base http://127.0.0.1:8788
python3 ui/tests/browser_performance.py --base http://127.0.0.1:8788 --label controls
```

The E2E runner returns a nonzero exit code on any failed assertion/browser
exception. It writes one JSON result per case, screenshots, an aggregate JSON
report, and SHA-256 sums. Cases can be selected with repeated `--case NAME`;
use a separate `--output DIRECTORY` for scoped reruns so the full-run evidence is
preserved.

- `numeric`: numeric editing, validation, Escape, Undo/Redo reaching real Rust.
- `drag_undo`: mouse pointer capture and one drag per Undo/Redo.
- `touch_keyboard`: trusted CDP touch gestures, touch cancellation, keyboard knob
  nudges, and Space NoteOn/NoteOff verified against real Rust voice/sample output.
- `delayed_controls`: explicit 450 ms test-only HTTP delays while genuine
  WebSocket telemetry continues; pending edits and Chorus I/II retain intent.
- `dependencies`: PWM, pre-delay/BPM, vowel, shimmer and modulation gating.
- `envelopes`: programmed Attack/Release geometry changes; v1 levels do not
  masquerade as stage/time markers.
- `unavailable_v1`: genuine v1 MIDI Learn assignment and explicit disabled
  unsupported cancellation/remapping/query controls.
- `reconnect`: test-only malformed frame and disconnect injection; controls
  recover when a fresh genuine WebSocket connection succeeds.

The speculative protocol-v2 fixture has been retired. Historical point-in-time
reports are retained; actual additive-v1 native results supersede them.

`browser_performance.py` reports animation-frame p50/p95/p99/max, long tasks,
browser script/layout/style/task CPU durations and heap sizes during a six-second
knob gesture with real telemetry. The explicit default responsiveness budget is
50 ms p95; it is configurable. This measurement includes interaction handlers,
React, drawing and transport work, and is **not an isolated React commit cost or
audio-thread benchmark**. Keep settings/browser/backend conditions identical for
before/after comparisons.

`baseline_probe.py` must be run against the **original b47f2d6 UI**, not the fixed
UI. It records six desired-behavior failures and benchmarks the exact legacy
JavaScript FFT extracted from Git. Its exit code means the observation probe
completed; the JSON's individual PASS/FAIL results describe the tested behavior.
See `docs/ui/BASELINE.md` for the baseline record and original artifact hashes.
