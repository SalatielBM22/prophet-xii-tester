# Controls delivery and integration status

Branch: `fix/ui-controls-issue-2`, based on preserved
`v1.0.0-experimental` / `b47f2d60c7005cd6c81694c0e17c76f7a4f107dc`.
Destination: `SalatielBM22/prophet-xii-tester`. No merge or main-branch change.

## Implemented

- Active App uses typed protocol adapters, a bounded ordered/coalescing HTTP
  control lane and a parameter store that protects local intent until a matching
  v1 value echo or negotiated v2 causal acknowledgment. One pointer/key gesture
  is one Undo; numeric edits, snapshots and reset participate in history.
- Click-to-edit finite/range/enum validation; Enter commits, Escape cancels.
  Confirmed factory reset supports double-click, Alt+Enter and a touch-accessible
  reset button. V1 has no authoritative factory values; reset is explicitly
  unavailable there, without using historical prototype defaults.
- Chorus buttons and Mode share one fresh atomic local mode value. Conditional
  PW, Ensemble, Vowel, Shimmer, modulation and BPM/pre-delay states explain why
  a control is inactive. Reverb wet/dry is distinguished from independent bypass.
- Programmed ADSR uses the baseline kernel's RC exponent/threshold semantics and
  a seconds axis. Running voice stages are supplied only by negotiated telemetry;
  v1 amplitude meters are not presented as stage/progress. The Rust fixture
  generator reads the real unchanged kernel at 48/96 kHz across 24 ADSR cases;
  nominal f64 timing agrees within one sample or 0.3% of the f32 recurrence.
- Reconnect refreshes metadata and host session; stale frames/HTTP generations
  are rejected. Keyboard notes, pointer/touch cancellation, focus loss, panic and
  Web MIDI handler cleanup are covered. Invalid telemetry does not replace state.
- Full MIDI lifecycle, preset references, envelope stages and reverb bypass/tail
  controls are capability-gated consumers. No new endpoint/producer is invented.

Changed production areas: `ui/src/App.tsx`, `controls/`, `protocol/`, the active
Envelope/model, isolated LegacyMeters and styles. Historical SynthPanels and
synthState remain unimported and unchanged. No DSP/audio/Android/plugin files,
root Cargo manifests/lock, delivered binaries or original evidence were changed.

## Checks and evidence

| Check | Outcome |
| --- | --- |
| TypeScript, production Vite build, ESLint | PASS |
| Controls unit/component/protocol/kernel-oracle tests | 46 PASS; 0 failed/skipped |
| Real Rust v1 E2E: numeric, gesture history, touch/keyboard, delayed snapshots/Chorus, dependencies, envelopes, unsupported features, reconnect | 8 PASS |
| Simulated v2 consumer contract: causal ack, confirmed reset by mouse/keyboard/touch, MIDI lifecycle/query, 12 voice stages, rate/retrigger/ordering | 1 PASS; **not backend integration** |
| Chromium frontend stress: 6-second knob gesture with real telemetry | PASS: p95 frame interval 16.8 ms, max 33.3 ms, zero long tasks |

Baseline stress also measured p95 16.8 ms, max 33.3 ms, zero long tasks. Script
CPU was 0.969 s baseline / 0.793 s controls, Task CPU 2.138 / 2.071 s. These are
shared-cloud browser observations, not isolated React commit timings or an audio
deadline/latency claim. See `ui/tests/README.md` for exact methods.

Reproduce with an externally built UI served by the real Rust null test host:

```sh
cd /workspace/prophet-xii
. /workspace/.prophet-cloud/env.sh
(cd ui && npm ci && npm run lint && npm run typecheck && npm test && npm run build -- --outDir /workspace/.prophet-cloud/task-a/ui-dist)
target/release/prophet-standalone --null --bind 127.0.0.1:8788 --ui /workspace/.prophet-cloud/task-a/ui-dist
# In a separate shell, with env.sh active:
python3 ui/tests/e2e_controls.py --base http://127.0.0.1:8788 --output /workspace/.prophet-cloud/task-a/e2e
python3 ui/tests/browser_performance.py --base http://127.0.0.1:8788 --label controls --output /workspace/.prophet-cloud/task-a/performance
cargo run --release --locked --manifest-path ui/tests/envelope-oracle/Cargo.toml
```

The last command prints regenerated kernel-reference JSON; compare it with
`ui/tests/fixtures/envelope-kernel.json` before intentionally refreshing that
fixture for a new kernel version. Do not overwrite the original delivery
evidence. Current reports/screenshots/hashes are outside the checkout under
`/workspace/.prophet-cloud/task-a/e2e-controls-final/`, `performance/` and
`baseline/`. An initial failing E2E report is retained separately; the real
browser uncovered a native-fetch binding error and the corrected build was
rerun successfully. No missing/zero-test runner is reported as PASS.

## Pending Agent B/device validation

See [BACKEND_REQUIREMENTS.md](BACKEND_REQUIREMENTS.md), proposal 2.0.0. Actual
v2 producers, causal acknowledgments, persisted MIDI lifecycle, per-voice stage
clocks, continuous analysis and click-free reverb bypass/tail handling are not
available in the baseline backend. Audible PW/reverb sensitivity, WSLg/native
audio buffering, acoustic latency and performance under integrated audio load
remain Agent B/device tests. No electrical/hardware or proprietary effect
equivalence is claimed. Native Windows/WASAPI is needed for a useful native
Windows latency comparison; WSLg buffering cannot be diagnosed from UI timing.
