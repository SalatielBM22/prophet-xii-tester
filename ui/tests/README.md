# Browser regression and performance runners

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
- `v2_consumer_fixture`: **simulated protocol-consumer test only** using the
  existing URLs. It tests confirmed factory references, reset/Undo/Redo,
  acknowledged MIDI lifecycle, supplied per-voice stage/time data and stale
  sequence rejection. It does not establish availability or correctness of an
  Agent B producer, DSP-side bypass, real continuous spectrum or native audio.

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
