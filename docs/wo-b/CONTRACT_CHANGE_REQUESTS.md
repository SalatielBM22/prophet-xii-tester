# PROPHET-XII Contract Change Requests (Agent B -> Agent A / Upstream)

Per `WO-XH-B v2` ownership rules, Agent B does **not** modify Agent A's read-only crates (`prophet-control`, `prophet-dsp`, `prophet-dsp-core`, `prophet-params`, `prophet-analysis-core`, `prophet-preset`). Mismatches found during Gate B0 inspection and acceptance testing are documented here with minimal reproductions and isolated host-side workarounds inside `crates/prophet-host/`.

---

## CCR-01 — Runtime Reverb Bypass & Tail Policy Setter on `PreparedEngine`

- **Severity:** High (prevents lock-free live toggling of RC1 `reverb_enabled` / `reverb_kill_tail` without resetting active voices)
- **Observed Gap:**
  - `PreparedEngine` exposes [`select_reverb(Algorithm)`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/engine.rs#L171) and [`reverb_state()`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/engine.rs), while [`SoundState::prepare`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/state.rs) sets `reverb_enabled` and `kill_tail` at prepare time.
  - However, `PreparedEngine::synth` is `pub(crate)`, and no `Action::Parameter` ID or public method mutates `reverb_enabled` or `ReverbTailPolicy` on a running `PreparedEngine`.
  - Legacy `crates/prophet-plugin/src/lib.rs`, `crates/prophet-standalone/src/lib.rs`, and `crates/prophet-android/src/lib.rs` relied on `synth.ipc.params.set_reverb_bypass(enabled, tail)` directly, which is inaccessible over `PreparedEngine`.
- **Minimal Reproduction:**
  1. Construct `let mut engine = PreparedEngine::prepare(...)?;`
  2. There is no `Action` variant or public `&mut PreparedEngine` method to flip `reverb_enabled = false` or `policy = ReverbTailPolicy::Kill` while retaining sounding voices.
- **Proposed Upstream Contract Addition:**
  - Add `pub fn set_reverb_bypass(&mut self, enabled: bool, policy: ReverbTailPolicy)` on `PreparedEngine` (or add a dedicated bypass/tail action in a future `EnginePort v2` revision) matching existing clickless `enable_gain` crossfade & kill-tail semantics.
- **Isolated Host-Side Workaround (`crates/prophet-host`):**
  - Track host shadow `reverb_enabled` and `reverb_kill_tail` flags in host adapters; when re-preparing via `SoundState::prepare` on transport/idle swap (or preset recall), apply exact `sound.reverb_enabled` / `sound.kill_tail`, and zero `ReverbWet` (`p62`) cleanly if bypass is toggled during active sound without interrupting voices, scheduling full flag alignment at idle boundary.

---

## CCR-02 — Host Accessors for MIDI Learn Map, Trimpots, and Voice Reset on `PreparedEngine`

- **Severity:** Medium (required for feature parity with legacy standalone/Android hosts and UI trimpots drawer)
- **Observed Gap:**
  - Inside `PreparedEngine`, `synth.ipc.params` holds per-voice trimpots (`voice_vco_a_cents`, `voice_vco_b_cents`, `voice_vcf_pct`, `voice_env_rc_pct`), `cc_map`, `begin_learn`, `cancel_learn`, `unlearn`, `remap`, and `kernel.reset()`, none of which are exposed on `PreparedEngine`.
  - Additionally, `PreparedEngine::handle_midi_message` is private and only runs `ProphetXiiPlugin::global_controller` defaults (`CC1 -> ModWheel`, `CC71 -> FilterResonance`, `CC74 -> FilterCutoff`) unless overridden.
- **Proposed Upstream Contract Addition:**
  - Expose read/write trimpot accessors on `PreparedEngine` (e.g. `set_voice_trimpots(voice: usize, vco_a_cents: f32, vco_b_cents: f32, vcf_pct: f32, env_rc_pct: f32)` and `trimpots() -> [[f32; 4]; 12]`), and optionally allow `EnginePort` panic/reset across all origins (`PanicAll`).
- **Isolated Host-Side Workaround (`crates/prophet-host`):**
  - [`CcLearnRouter`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/queue.rs#L169) intercepts learnable CCs cleanly off-DSP and emits `Action::Parameter` while forwarding raw MPE `ch>0 CC74`, `CC64` sustain, `CC66` sostenuto, and `CC120/123` as `Action::Midi`.
  - Host panic sends `Action::PanicOrigin` for active origins (`0..16`) or swaps a freshly prepared standby instance when hard DSP reset is requested.

---

## CCR-03 — Serial Consumption on Rejected Actions & Formal VCF Endpoint Schema

- **Severity:** Low (documentation / schema clarity)
- **Observed Behavior:**
  - In [`PreparedEngine::apply`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/engine.rs), `*previous = e.serial` is updated *before* validating `Action::Parameter(OversamplingMode)` or `Action::VectorXY` in non-`Vector4` topology. Thus an `Unsupported` or `Invalid` action still consumes that `(origin, serial)` watermark (replaying the same serial yields `Duplicate`).
  - `docs/wo-a/control-v1.schema.json` omits the worker VCF response schema noted as deferred in `docs/wo-a/CONTRACT_HANDOFF.md`.
- **Proposed Upstream Clarifications:**
  1. Document in `ENGINE_PORT_V1.md` that per-command ACK rejection (`Invalid` / `Unsupported` / `Capacity`) still advances the `(origin, serial)` watermark (only block-level `Err(PortError)` leaves watermarks untouched).
  2. Standardize `FilterResponseReport` (`validity`, populated `points` iff `Measurable`, `tailRms`, `relativeScalingError`, `windowSamples`, `sampleRate`, `reason`) for `v1` UI transports.
  3. Note that CEM3320 (`FilterMorph < 0.9999`) contains a 5 Hz AC-coupled feedback capacitor whose impulse tail requires `N >= 65_536` at 44.1/48 kHz to settle below `validity::measure`'s `1e-8 * energy` criterion even on damped low-Q patches (`cutoff = 1000 Hz, Q = 0.2`); hosts should default to `65_536` within `validity::measure`'s bounded `[1024, 65_536]` range rather than `16_384`.

---

## CCR-04 — Polyphonic Key Pressure (`0xA0`) Support in `Ownership::midi_owned`

- **Severity:** Low (MPE channel pressure `0xD0` works; poly-aftertouch `0xA0` rejected as `Unsupported`)
- **Observed Gap:**
  - In [`Ownership::midi_owned`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/ownership.rs#L260), status bytes `0x80`, `0x90`, `0xB0`, `0xD0`, and `0xE0` are handled, whereas `0xA0` (MIDI Polyphonic Key Pressure / CLAP `NoteEvent::PolyPressure`) falls into `_ => return Status::Unsupported`.
  - Legacy `crates/prophet-plugin/src/lib.rs` previously mutated `voice.mpe_pressure` directly across voices matching `midi_note == note`, which is no longer permitted under EnginePort v1 ownership.
- **Minimal Reproduction:**
  1. Send a `Command` with `Action::Midi { status: 0xA0, data1: 60, data2: 96, note_id: 60 }` to [`PreparedEngine::process`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/engine.rs#L288).
  2. Engine responds with `Ack { status: Status::Unsupported, .. }`.
- **Proposed Upstream Contract Addition:**
  - Handle `0xA0` in `Ownership::midi_owned` by updating `v.mpe_pressure` on active voices owned by `origin` matching `midi_note == d1` (or `note_id`).
- **Isolated Host-Side Workaround (`crates/prophet-plugin`):**
  - Map `NoteEvent::PolyPressure { timing, channel, pressure, .. }` to channel pressure `Action::Midi { status: 0xD0 | (channel & 0x0F), data1: pressure_u8, data2: 0, note_id }` on `ORIGIN_DAW_HOST` (which applies cleanly to channel/global aftertouch via `0xD0` without direct `kernel.allocator.voices` mutation).

