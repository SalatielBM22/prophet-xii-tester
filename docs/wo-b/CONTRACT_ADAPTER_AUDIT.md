# PROPHET-XII Gate B0 — EnginePort v1 Contract & Adapter Audit

**Branch:** `feat/dual-xhigh-host-ui-platform`
**Status:** PASS · PROVEN · HOST_VERIFIED (Gate B0 acceptance suite & workspace regression green)

## 1. Pinned Commit & Contract References

| Artifact / Milestone | SHA | Role / Authority |
|---|---|---|
| RC1 Baseline | `2978fdc02c85aa413dc76cc468cebb05f5cecc2a` | Original RC1 host/DSP baseline (`p0..p82`, `LegacyRC1`, 11 parity renders) |
| EnginePort v1 Contract | `995d772d7371480900fe74def6bb3c60674abddc` | Frozen wire semantics (`docs/wo-a/ENGINE_PORT_V1.md`, `control-v1.schema.json`, `control-v1.d.ts`, `fixtures/messages.json`) |
| Core Implementation Pin | `1f655c92f60a6a6124e7459014447f51f7d21517` | Completed `PreparedEngine`, `Vector4`, `Extended3/4`, `reverb` idle selection, `music`, `prophet_preset` v2 |
| Agent A Verified HEAD (Parent SHA) | `c5f1f873587fe553eaec67894231760b7f055ec7` | Exact starting parent of `feat/dual-xhigh-host-ui-platform` |

## 2. Inspected Agent A Surface (Read-Only Consumer Contract)

Inspection confirmed that the actual Rust implementation in [`crates/prophet-control/src/`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-control/src/engine.rs) and [`crates/prophet-dsp-core/src/port.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-dsp-core/src/port.rs) governs all runtime behavior:

1. **Core Trait & Entrypoint:** `prophet_control::port::EnginePort` implemented by `prophet_control::PreparedEngine`.
   - `prepare(Prepare { sample_rate: 8000..=192000, max_frames: 1..=1024, oversampling: 1 | 2 | 4 })` runs off the audio thread.
   - Reported dry path latency is base-rate oversampling lookahead only: `0` (1×), `16` (2×), `24` (4×) samples.
   - `process` and `process_music` accept stereo planar `f32` buffers (`<= max_frames`), ordered `Command` slices (`<= 256` total combined events per block), and matching ACK/status output slices.
2. **Event & Serial Semantics:**
   - Up to `16` origins (`0..=15`) with strictly positive, monotonic `u32` serials per origin; replays at/below watermark yield `Status::Duplicate`.
   - Note identity is `(origin, channel, key, note_id)`. Velocity-0 `NoteOn` normalizes to `NoteOff`. `PanicOrigin` releases only its own origin.
   - `PreparedEngine::notes()` returns held (`pressed: true`) and pedal-retained (`pressed: false`, `sostenuto` or `sustain`) notes; releasing voices without key/pedal retention appear in `voices()` (`active && !gate && releasing`).
3. **Parameters & Revisions:**
   - Legacy IDs `p0..p82` remain intact without renumbering.
   - `p83` (`GLIDE_ENABLED`) is registered independently alongside `GlideTime` (`p48`) and `GlideLaw`.
   - `SoundState::from_rc1` infers `glide = values[GlideTime] > 0.0005`.
   - `OversamplingMode` (`p63`) returns `Status::Unsupported` in `process()` and must be applied via off-thread `SoundState::prepare` + lock-free swap.
   - Parameter bank `revision` is a `u64` parameter counter only; `Action::Midi`, `Action::VectorXY`, and `NoteScheduler` progression do **not** increment `revision`.
4. **Vector4 & Topology:**
   - Diamond constraint `|x| + |y| <= 1.0` (`Point::valid`); corners `(A:-1,0)`, `(B:0,+1)`, `(C:+1,0)`, `(D:0,-1)` mix 4 sources (`0.25` each at center), smoothed with 5 ms joystick slew.
   - `Action::VectorXY` returns `Applied` only when topology is `Topology::Vector4` and coordinates are valid; otherwise returns `Unsupported` or `Invalid`.
   - `PreparedEngine::set_patch(Patch)` succeeds only with idle voices (or matching active topology when reconfiguring routes cleanly) and returns `Status::Unsupported` while voices are active.
5. **Reverb (`LegacyRC1` default, `DiffuseV1` experimental):**
   - `PreparedEngine::select_reverb(Algorithm)` applies only when the reverb tank is dormant/idle (or matches active algorithm), returning `Status::Unsupported` when an active tail exists.
6. **Music Clock & Sequencer/Arpeggiator (`prophet_control::music`):**
   - `NoteScheduler` (`Copy`), `Arpeggiator` (Up/Down/UpDown/AsPlayed/Random/Chord, 1–4 octaves, latch, deterministic `seed`), `PatternBank` (64 patterns × 32 steps).
   - Speculative `trial = scheduler; trial.block(...)` commits only after `engine.process_music(...)` succeeds; `MusicError::Capacity` or combined `> 256` events require segment subdivision; `stop()` emits `Trigger::Release` that must be flushed before audio suspend.
7. **Presets & Bounded VCF Analysis:**
   - `prophet_preset::Preset` decodes v1 (including immutable `presets/00..09-factory.json`) and v2 JSON envelopes (512 KiB max, `reject unknown fields`, ISO-HDLC CRC-32).
   - `prophet_analysis_core::validity::measure` runs strictly off the audio thread and populates frequency response points **only** when returning `Validity::Measurable`.

## 3. Shared Host Runtime Architecture (`crates/prophet-host`)

To ensure uniform behavior without duplicating scheduling, carry, ownership, or worker logic across backends, [`crates/prophet-host/`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/lib.rs) provides the shared EnginePort v1 host adapter crate:

| Module | Responsibilities & Key Symbols |
|---|---|
| [`wire.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/wire.rs) | `EngineMessage`, `WireBlock`, `WireAck`, `WireSnapshot`, strict validator matching `control-v1.schema.json` (`deny_unknown_fields`, decimal-string `u64`, offset ordering, strict diamond `validate_strict_diamond`), zero-alloc RT `SnapshotPod`, capability bitmask & host intersection |
| [`queue.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/queue.rs) | `SerialIssuer`, `CanonicalMidiArbiter` (single physical MIDI origin + `PanicOrigin` on source handoff), `CcLearnRouter` (preserves MPE `ch>0 CC74`, `CC64/66/120/123`), `MusicTransport` (`flush_stop_before_suspend`), zero-alloc `BlockScheduler` (atomic offset/origin validation -> zero buffers on `Err`, `offset == frames` right-endpoint carry, stable causal `(offset, arrival_seq)` sort, sub-block splitting `<= max_frames` & `<= 256` combined events, speculative scheduler commit) |
| [`runtime.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/runtime.rs) | `WorkerSwapCoordinator` & `AudioEngineSlot` paired over lock-free `rtrb` swap & retire queues so `Box<PreparedEngine>` and `Box<PatternBank>` are retired back to the worker and **never dropped on the audio thread** |
| [`preset.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/preset.rs) | Includes `crates/prophet-preset/src/lib.rs` via `#[path]` (avoiding multiple-workspace Cargo errors while keeping identical nominal `SoundState`/`PatternBank`/`Performance` types), exposes `factory_presets()`, `decode_and_prepare`, `encode_from_engine`, explicit user diagnostics (`HostPresetError::describe`) |
| [`analysis.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/analysis.rs) | Cancellable worker VCF analyzer (`measure_filter_response`) backed by `validity::measure`, returning points iff `FilterValidity::Measurable`; default `65_536`-sample bounded window ensures damped CEM3320 patches (with 5 Hz feedback AC-coupling capacitor) converge while preserving `LongTail` / `SelfOscillating` classification |

### Cargo Workspace Reconciliation Note

Adding a path dependency directly to `crates/prophet-preset` from the root workspace fails with `error: multiple workspace roots found in the same workspace`. Preserving Agent A's read-only files intact, [`crates/prophet-host/src/preset.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/src/preset.rs) uses the same `#[path = "../../prophet-preset/src/lib.rs"]` pattern Agent A uses for `prophet-dsp-core` and `prophet-analysis-core`, and root [`Cargo.toml`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/Cargo.toml) adds only `"crates/prophet-host"` (updating root [`Cargo.lock`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/Cargo.lock); `crates/prophet-preset/Cargo.lock` remains untouched).

## 4. Wire Fixture & Acceptance Test Matrix

Verified by [`crates/prophet-host/tests/engine_port_acceptance.rs`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/crates/prophet-host/tests/engine_port_acceptance.rs) (`6/6` tests passing under `--locked`):

| Acceptance Criterion | Verification Result | Evidence / Test |
|---|---|---|
| `docs/wo-a/fixtures/messages.json` decode, schema validate, and execute | PASS | `b0_wire_fixtures_validate_and_execute_against_prepared_engine` |
| Strict unknown-field rejection, decimal `u64`, `offset < frames`, diamond geometry | PASS | `b0_wire_fixtures_validate_and_execute_against_prepared_engine` |
| Atomic rejected block & buffer zeroing (never expose partial output) | PASS | `b0_rejected_blocks_carry_offsets_and_splitting_preserve_causality` |
| Right-endpoint carry (`offset == frames`) to offset `0` of next block | PASS | `b0_rejected_blocks_carry_offsets_and_splitting_preserve_causality` |
| Sub-block splitting for host `frames > max_frames` (`600 > 256`) and `> 256` cmds (`300 > 256`) | PASS | `b0_rejected_blocks_carry_offsets_and_splitting_preserve_causality` |
| Canonical physical MIDI ownership + `PanicOrigin` handoff without dual triggers | PASS | `b0_canonical_midi_arbitration_and_cc_learn_router` |
| CC Learn router excluding MPE ch>0 CC74, CC64/66/120/123, OversamplingMode | PASS | `b0_canonical_midi_arbitration_and_cc_learn_router` |
| `OversamplingMode` RT `Unsupported` -> off-thread swap without RT drops (`latency 16 -> 24`) | PASS | `b0_vector4_reverb_oversampling_swap_and_rc1_glide_migration` |
| `VectorXY` diamond validation & unchanged parameter `revision` | PASS | `b0_vector4_reverb_oversampling_swap_and_rc1_glide_migration` |
| Idle-only `select_reverb` & `set_patch` preserving active voices/tail on refusal | PASS | `b0_vector4_reverb_oversampling_swap_and_rc1_glide_migration` |
| RC1 `SoundState::from_rc1` glide inference (`GlideTime > 0.0005` -> `p83`) | PASS | `b0_vector4_reverb_oversampling_swap_and_rc1_glide_migration` |
| `MusicTransport` chord/ratchet splitting, stop-release flush, v2 preset CRC tamper reject & factory recall | PASS | `b0_music_transport_stop_flush_and_v2_preset_worker_swap` |
| VCF `validity::measure` gating (`Measurable` points vs empty `LongTail` + `Superseded` cancel) | PASS | `b0_vcf_validity_gating_and_cancellation` |

### Baseline Regression Suite (Post-B0)

- `cargo test --locked -p prophet-host`: **6/6 passed**
- `cargo test --locked --manifest-path crates/prophet-preset/Cargo.toml`: **2/2 passed**
- `cargo test --locked --workspace`: **all workspace crates passed** (54 tests across `prophet-analysis-core`, `prophet-control`, `prophet-dsp`, `prophet-host`, `prophet-render`, `prophet-standalone`, zero errors)

## 5. Adapter Compatibility & Capability Intersection Matrix

| Capability String | Engine Support | Standalone (B3) | VST3/CLAP (B2) | LV2 (B3) | Android (B1) | React UI (B4) |
|---|---|---|---|---|---|---|
| `enginePortV1` | Default on | Target | Target | Target | Target | Target |
| `glideEnabled` (`p83`) | Default on | Target | Target | Target | Target | Target |
| `reverbState` | Default on | Target | Target | Target | Target | Target |
| `selectIdleReverb` | Opt-in / Idle-only | Target | Target | Target | Target | Target (explicit experimental warning) |
| `oscillatorPatch` | Idle / Match-only | Target | Target | Target | Target | Target |
| `noteScheduler` | Opt-in clock | Target | Target (host-synced) | Target (Atom/Time) | Target | Target (32-step / 64-slot) |
| `vectorXY` | Only in `Topology::Vector4` | Target | Target | Target | Target | Target (diamond constraint) |

### Unsupported / Restricted Items Explicitly Documented

- **In-callback oversampling changes (`p63`):** Always return `Status::Unsupported`; hosts trigger worker re-prepare (`WorkerSwapCoordinator`).
- **Mid-tail reverb algorithm switching:** Always returns `Status::Unsupported`; UI displays explicit idle-required message and keeps existing state.
- **Per-voice topology re-allocation (`set_patch`) with active voices:** Returns `Status::Unsupported`; never silently cuts held/releasing voices.
- **`reverb_enabled` / `ReverbTailPolicy` live toggle gap:** See [`CONTRACT_CHANGE_REQUESTS.md`](file:///home/antigravity/SalatielBM22/prophet-xii-wo-b/docs/wo-b/CONTRACT_CHANGE_REQUESTS.md) (`CCR-01`).
- **Direct MIDI Learn & trimpot access on `PreparedEngine`:** See `CCR-02`; handled safely in `prophet-host::queue::CcLearnRouter`.
- **Pre-existing RT deadline failure (`34/13,824` blocks):** Unchanged from Agent A baseline; remains tracked for Gate B5 qualification.
