# Native controls checkpoint

The consumer uses Agent B's actual additive-v1 contract at
`211c4f44dc735a89047dad32e725e39b9b8c9b68` (research includes RT checkpoint
`430e4d8bf0fbf6d052f2822294b3fbef1c04e1ab`). The complete backend
`docs/backend/INTEGRATION_CONTRACT.md` and `RT_OPTIMIZATION.md`, plus native source,
were read before adapting controls. This supersedes the earlier speculative v2
consumer: no such API is required or sent.

- HTTP 202/acceptedCommandId means enqueue. Each parameter stays pending until
  its own applied ID is observed; global lastAppliedCommandId is not a client ACK.
- Factory reset uses the selected preset's original metadata values after all 83
  applied IDs confirm that request. Mouse, numeric editor, Alt+Enter, touch reset
  and gesture-level Undo/Redo share the same parameter state.
- MIDI uses learn/cancel_learn/unlearn/remap and authoritative active/target/ccMap.
  Mappings are global, CC64/120/123 reserved and MPE CC74 precedence is explained.
- Bypass uses enabled/tail spill-or-kill; status is dormant/enableGain, with no
  invented tail state or room-size parameter.
- Stage/level/base-rate elapsedSamples/progress/timed are actual per-voice data.
  Idle/Sustain are indefinite and activity comes from the kernel voice mask.
- Reconnect clears pending commands, history and factory reference. Invalid or
  regressed telemetry is ignored. Advancing frames establish liveness; a stalled
  ready:true snapshot causes the UI to disconnect/retry after three seconds.

The original v1 backend remains supported with explicitly limited matching-value
echo reconciliation and unavailable extensions. No backend files are modified by
these consumer commits, and no backend branch is merged into main.

The browser suite in the analysis/integration branch tests these controls against
an exact git-archive copy of B's checkpoint, including concurrent clients and
delayed acceptance/application. The independent spectral branch adds the Rust
WASM capture worker and transfer-display changes; the controls branch retains
the original meters until that branch is combined after review.

Current issues remain open for failing high-polyphony Moog deadlines, uncalibrated
RT60, physical fidelity, device/DAW/JNI qualification and native ARM64 execution.
Correct control mappings and UI tests do not establish those properties.
