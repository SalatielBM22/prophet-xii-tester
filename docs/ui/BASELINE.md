# Gate A0: observed baseline

Baseline: `b47f2d60c7005cd6c81694c0e17c76f7a4f107dc`, tag
`v1.0.0-experimental`. The tag and supplied desktop binaries remain unchanged.
Issues #1–#4, including the three technical comments on #1, were read before
implementation. The scope/limitations in #4 apply: working VA DSP and finite
outputs do not establish electrical equivalence with physical instruments.

The live React entry is `ui/src/main.tsx` → `ui/src/App.tsx`. Its original
`Knob`, `Envelope` and `Scope` implement the controls and diagnostics used by the
browser. `ui/src/components/SynthPanels.tsx` and `ui/src/state/synthState.ts` are
unimported historical prototypes, as already stated in `ui/src/LEGACY.md`.
They were not used as a source of production defaults or telemetry.

## Existing checks before corrections

| Check | Outcome |
| --- | --- |
| `npm run lint` | UNAVAILABLE: baseline has no lint script |
| `npx tsc --noEmit` | PASS |
| `npm run build -- --outDir <external-directory>` | PASS |
| `npm test` | UNAVAILABLE: baseline has no unit/component test script |
| Existing `tests/api_smoke.py` | PASS against the real Rust v1 host |
| Existing `tests/ui_smoke.py` | PASS in Chromium against the real Rust v1 host |

The original smoke checks demonstrate a functional baseline but do not cover
the regressions below. No missing or zero-test runner is counted as a pass.

## Reproducible defects

`ui/tests/baseline_probe.py` was run against the original compiled frontend and
the real Rust host in null-sink mode. It emits observations, not an exit-code
claim that the desired behaviors pass. Its delayed HTTP routes are explicit
test-only fault injection; no production API or telemetry was replaced.

| Desired behavior | Baseline observed result |
| --- | --- |
| Attack duration changes the programmed curve | FAIL: 0.001 s and 1 s produce identical SVG paths |
| Click numeric readout to edit | FAIL: no numeric input appears |
| Double-click restores loaded factory value | FAIL: cutoff remains 1000 Hz after editing factory 680 Hz |
| One drag creates one undo entry | FAIL: 680 → 3011.268 Hz, one Undo returns 2660.086 Hz |
| Stale WebSocket frame preserves pending local edit | FAIL: with 450 ms HTTP delay, optimistic 727.866 Hz reverts to 680 Hz until backend applies it |
| Chorus I then II preserves both intents | FAIL: with the same delay, mode ends at II (2), expected I+II (3) |

Chromium 156.0.8078.79 reported no browser runtime errors during this probe.
The original exact 1024-point JavaScript FFT, extracted from the baseline commit,
was benchmarked in this browser: 100 warm-up transforms, then 150 batches of 20.
Per-transform elapsed milliseconds: p50 **0.135**, p95 **0.165**, p99 **0.300**,
max **0.320**. This includes the original window/allocations/magnitude conversion
and is a shared-cloud-machine observation, not a real-time audio deadline or
native ARM64 measurement. FFT size and language/runtime differ from the new Rust
benchmarks; an unqualified speedup ratio would be misleading.

`ui/tests/browser_performance.py --label baseline` also exercised one six-second
continuous knob gesture while the real v1 host continued streaming telemetry.
It observed 361 animation frames, p95 frame interval 16.8 ms, maximum 33.3 ms,
zero long tasks, 0.969 s browser script CPU and 2.138 s total browser task CPU.
This passes the explicitly configured 50 ms p95 responsiveness budget. It does
not isolate React commits from Canvas/transport work, establish RT safety, or
reproduce the user's overloaded Windows/WSLg session.

## Artifacts and hashes

Generated artifacts live outside tracked delivery evidence at
`/workspace/.prophet-cloud/task-a/baseline/`. The probe's output hashes are:

```text
08660f730d42f0cf8422a34b405535fd1df82a3869874ab4d09ef440f7bec7a1  probe/baseline-desktop.png
08dd899529bb6f24b6a67b6444cb3de76077cce57b65e39c5b7380cfca5aff35  probe/baseline-probe.json
e47255ca0f8c884307edb8976cd687575d57c1c752e4a00616cd78ecdd123622  ui-dist/assets/index-BQbNFYp2.css
e70738d24e891a7fc0dcd89ad9c4b4562b2615834ee5bb4ddbb5e3f7291ebfce  ui-dist/assets/index-Cku28J_7.js
```

To repeat, serve the baseline UI build from a Rust test host (never overwrite
tracked binaries/evidence), then run:

```sh
. /workspace/.prophet-cloud/env.sh
python3 ui/tests/baseline_probe.py --base http://127.0.0.1:8787 \
  --output /workspace/.prophet-cloud/task-a/baseline-repeat
```

This records browser interactions and control values. It does not measure
acoustic latency, WSLg buffering, physical MIDI input, DAW operation, or hardware
filter/reverb equivalence. Those remain backend/device validation work.
