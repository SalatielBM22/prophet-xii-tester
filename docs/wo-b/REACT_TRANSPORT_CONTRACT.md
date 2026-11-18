# PROPHET-XII — Gate B4 React UI & Unified `UiTransport` Contract

**Work Order:** WO-XH-B v2 (Gate B4)  
**Branch:** `feat/dual-xhigh-host-ui-platform`  
**Owner:** Gemini 4 Hydrogen XHigh  
**Date:** 2026-11-18  

---

## 1. Unified `UiTransport` Architecture

All PROPHET-XII user interfaces (Standalone Native Window, VST3/CLAP/LV2 Plugin Editors, Android APK WebView, and opt-in `--http-dev` Browser Bridge) now run from a single React/TypeScript build (`ui/dist`, embedded into `prophet-host` via `EMBEDDED_UI_ASSETS` with offline-safe CSP and zero CDN dependencies).

The transport layer in `ui/src/protocol/uiTransport.ts` detects the active host adapter at boot without requiring any query parameters or external network sockets:

| Adapter | Detection Global | Surfaces | Inbound Snapshot Path | Outbound Command Path | Zoom Hook | Diagnostics Hook |
|---|---|---|---|---|---|---|
| `EmbeddedHostUiTransport` (`embedded-host`) | `window.__PROPHET_HOST__` | Linux GTK3/WebKit2GTK & Windows WebView2 (VST3, CLAP, LV2, Standalone Native) | `CustomEvent('prophet-snapshot')` push (~25 Hz) + initial `requestSnapshot()` pull | `dispatchAsync(cmd)` returning `UiCommandReply` | `setZoom(z)` / `getZoom()` | `null` |
| `AndroidBridgeUiTransport` (`android-bridge`) | `window.ProphetAndroidBridge` | Android APK WebView + JNI/AAudio | 33 ms polled `pollSnapshot()` with deduped JSON guard | Synchronous JNI `dispatch(JSON.stringify(cmd))` | `document.documentElement.style.zoom` | `pollDiagnostics()` (`AndroidAudioDiagnostics`) |
| `WindowNativeUiTransport` (`window-native`) | `window.Native` | Generic embedded native WebView hosts | 33 ms polled `poll()` with deduped JSON guard | Synchronous `dispatch(JSON.stringify(cmd))` | `document.documentElement.style.zoom` | `null` |
| Legacy `http-dev` (`http-dev`) | None (fallback) | Opt-in `prophet-standalone --http-dev` / `--bind` (RC1 compatibility) | WebSocket `Frame` stream + REST `/api/*` | REST + WS JSON commands | CSS zoom | `null` |

---

## 2. EnginePort v1 Wire Contract (`ui/src/protocol/enginePort.ts`)

### 2.1 Snapshot Schema (`UiSnapshot`, `version: 1`)
Validated strictly by `decodeUiSnapshot()` before reaching React state:
- `enginePort: WireSnapshot` — `version: 1`, `revision`, `renderedFrames`, `sampleRate`, `maxFrames`, `latencySamples`, `notes[]` (with `origin`, `channel`, `key`, `noteId`, `pressed`, `sustained`), `voices[]`, `fx` (`algorithm: LegacyRC1 | DiffuseV1`, `enabled`, `tail`, `frozen`, `dormant`), `capabilities[]`, `xruns`, `cpuPercent`.
- `parameters: number[]` — Full parameter array `0..83`, including `p63 OversamplingMode` (`1 | 2 | 4`) and `p83 GlideEnabled` (`0 | 1`).
- `patch: UiPatchState` — `topology` (`Classic2 | Extended3 | Extended4 | Vector4`), `routes: [B→C, C→D]` (`Sum | Frequency | Phase | Sync | Ring`), `ratios: [OscC, OscD]`, `position: { x, y }`, `weights: [A, B, C, D]`, `stages[5]`, `modulation[3]`, `loopHeld`, `idleForTopologyChange`.
- `music: UiMusicState | null` — Authoritative Rust `NoteScheduler` state: `playing`, `activePattern (0..63)`, `patternLen (1..32)`, `stepIndex`, `tempoMilli`, `swing`, `arpEnabled`, `arpOrder`, `arpOctaves`, `arpLatch`, `arpSeed`, `currentPattern.steps[32]`. **Zero JS step timers exist.**
- `filterAnalysis: FilterResponseReport | null` — `validity` (`Measurable | LongTail | SelfOscillating | NonlinearOperatingPoint | NumericalFailure`), `points[]` (magnitude dB + unwrapped phase), `tailRms`, `relativeScalingError`, `reason`.
- `presets: UiPresetSummary[]`, `midiLearn: UiMidiLearnState`, `kernelTelemetry: UiKernelTelemetry`, `driveMakeup: boolean`, `reverbBypassEffective`, `reverbTailEffective`, `lastNotice`, `diagnostics`.

### 2.2 Command Schema (`UiCommand` -> `UiCommandReply`)
Rust `crates/prophet-host/src/ui.rs` supports both canonical `snake_case` and `camelCase` field aliases via `#[serde(alias = "...")]`:
- `param`, `midi`, `vectorXY`, `panic`, `factoryPreset`, `presetLoadJson`, `presetSaveJson`, `setOversampling`, `setPatch`, `setReverb`, `setDriveMakeup`, `midiLearn`, `setPhysicalMidiSource`, `musicControl`, `measureFilter`, `hostControl` (including `getFactoryPresets` for off-audio confirmed factory preset bank hydration and `setUiZoom`).
- `UiCommandReply` returns `{ ok, status, message, swapId?, presetJson?, filterAnalysis?, data? }`.

---

## 3. UI Surfaces & Gate B4 Engineering Decisions

1. **Clean Rotary Controls (`ui/src/controls/Knob.tsx`):**
   - Removed intrusive per-knob `RESET` buttons and repetitive `'Factory reset unavailable'` banners.
   - Double-click, `Ctrl+Click`, or `Alt+Enter` resets cleanly to the confirmed Rust factory preset value fetched once off-audio via `hostControl: getFactoryPresets` (and updated on every `factoryPreset` load).
   - Preserves Shift fine-drag, inline numeric entry, right-click MIDI Learn with CC badges, and keyboard navigation.
2. **Vector4 & Multi-Oscillator Panel (`ui/src/components/VectorPanel.tsx`):**
   - Exposes `Classic2`, `Extended3`, `Extended4`, `Vector4` with explicit `!idleForTopologyChange` lock guard when voices are active.
   - Interactive diamond XY joystick strictly enforces `|x| + |y| <= 1` (`clampToDiamond`, matching `prophet_dsp_core::vector::Point::weights`: `A = West`, `B = North`, `C = East`, `D = South`), throttled at 30 Hz and smoothed 5 ms inside Rust DSP.
   - Exposes 5-stage trajectory (`S1..S5`), `loopHeld`, and `Velocity / Pressure / Bipolar LFO` XY modulation depths.
3. **Adjacent VCF Small-Signal Transfer Plot (`ui/src/components/VcfResponsePlot.tsx`):**
   - Embedded directly inside the `FILTER` panel next to Cutoff/Resonance/Drive/Morph/Moog Bass Comp.
   - Renders magnitude `|H(f)|` (amber) + unwrapped phase (teal) when `validity === 'Measurable'`.
   - When `SelfOscillating`, `LongTail`, `NonlinearOperatingPoint`, or `NumericalFailure`, honestly withholds the Bode curve and reports `validity`, `reason`, `tailRms`, and `relativeScalingError` (`data-testid="vcf-invalid-fallback"`).
4. **Reverb Algorithm & Active-Tail Guard:**
   - Exposes `LegacyRC1` vs `DiffuseV1 (Exp)` alongside `Bypass` and `Tail Policy` (`Spill` / `Kill`).
   - Algorithm switching is disabled while `enginePort.fx.dormant === false` (active reverb tail), preventing mid-tail buffer glitches.
5. **32-Step / 64-Pattern Sequencer & Arpeggiator (`ui/src/components/SequencerPanel.tsx`):**
   - 4 pages × 8 steps (`1–8`, `9–16`, `17–24`, `25–32`), 64 pattern slots, tempo `20..400 BPM`, swing `0..75%`, Arp (`Up|Down|UpDown|AsPlayed|Random|Chord`, `1..4` octaves, latch, seed), per-step `Kind/Key/Vel/Gate/Accent/Glide/Ratchet`.
6. **61-Key Piano Keyboard (`ui/src/components/Keyboard.tsx`):**
   - Proper `C2–C7` (36 white, 25 black) layout with black keys positioned cleanly over white-key seams, Pitch Bend wheel, Physical MIDI Owner (`Native OS` / `Web MIDI`) toggle, and distinct per-origin highlights (`HW`, `DAW`, `SEQ`, `UI`, `Sustained`, `Releasing`).
7. **Multi-Zoom & Preset v2 CRC-32 Codec (`ui/src/App.tsx`):**
   - `75% / 100% / 125% / 150%` zoom bar wired directly to host `setZoom`.
   - `EXPORT V2` and `PASTE V2 JSON` modal wired to Rust `presetSaveJson` / `presetLoadJson`, surfacing CRC-32 validation errors inline.
   - Collapsible Android AAudio Diagnostics drawer when `ProphetAndroidBridge` is active.

---

## 4. Verification Evidence (`HOST_VERIFIED`)

### 4.1 Typecheck, Lint, Unit & Component Tests
- `npm run typecheck` (`tsc --noEmit`): **PASS** (0 errors)
- `npm run lint` (`eslint src`): **PASS** (0 errors, 0 warnings)
- `npm test -- --run` (Vitest): **PASS — 14 test files, 83/83 tests passed**
  - Includes all 12 pre-existing RC1 / `--http-dev` suites (`App.reverb.test.tsx`, `Knob.test.tsx`, `AnalysisPanel.test.tsx`, `Envelope.native.test.tsx`, etc.)
  - Plus new Gate B4 suites: `ui/src/protocol/uiTransport.test.ts` (6 tests) and `ui/src/App.enginePort.test.tsx` (5 end-to-end integration tests)
- `npm run build` (`build:analysis` WASM + Vite production bundle): **PASS** (`ui/dist/assets/prophet_analysis_bg-*.wasm` + JS/CSS built offline)

### 4.2 Headless Chromium / Playwright Multi-Zoom & Multi-Viewport Verification
Executed against production `ui/dist` with injected `window.__PROPHET_HOST__` and `window.ProphetAndroidBridge`:

| Surface | Viewport | Zoom | Console Errors | Page Errors | Horizontal Overflow | Keys (W/B) | Intrusive `RESET` Buttons |
|---|---|---|---|---|---|---|---|
| Desktop Embedded | `1440×900` | `75%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1440×900` | `100%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1440×900` | `125%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1440×900` | `150%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1180×720` | `75%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1180×720` | `100%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1180×720` | `125%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Desktop Embedded | `1180×720` | `150%` | 0 | 0 | `0 px` | 36 / 25 | 0 |
| Android WebView | `412×915` | `100%` | 0 | 0 | `0 px` | responsive | 0 |

Interactive Playwright assertions verified:
- Vector4 diamond corner drag clamped accurately to `{ x: 0.5, y: 0.5 }` (`|x| + |y| = 1.0`).
- Sequencer page 4 (`25–32`) navigation revealed `Step 32 kind`.
- Preset v2 CRC-32 `EXPORT V2` modal rendered serialized Rust payload.
- Dynamic `SelfOscillating` VCF snapshot cleanly replaced SVG Bode curve with withheld fallback displaying `tailRms: 4.12e-1` and `Scaling err: 35.00%`.

Visual evidence screenshots captured in `docs/wo-b/screenshots/`:
- `desktop-100-vector4.png` (`1440×900` @ `100%` zoom)
- `desktop-125-zoom.png` (`1180×720` @ `125%` zoom)
- `android-webview-412x915.png` (`412×915` Android WebView with AAudio diagnostics drawer open)
