# PROPHET-XII Gate B3 — LV2/Linux Plugin & Native Desktop Standalone Validation

- **Work Order:** WO-XH-B v2 Gate B3 (Issue #14, Front F7)
- **Branch:** `feat/dual-xhigh-host-ui-platform`
- **Base / Prior Gate:** `c5f1f873587fe553eaec67894231760b7f055ec7` (Agent A) / `1bebd11` (Gate B2)

---

## 1. LV2 Architecture Summary (`crates/prophet-lv2`)

[`crates/prophet-lv2`](../../crates/prophet-lv2/src/lib.rs) exposes a standalone C-ABI LV2 instrument plugin (`PROPHET-XII.lv2` / `prophet-xii.lv2`) backed by `prophet-host` (`RtHostSlot`, `HostControlPlane`, `prophet_preset` v2, and `prophet_host::embed`), with zero DSP duplication:

- **URIs & Discovery ([`lv2_ffi.rs`](../../crates/prophet-lv2/src/lv2_ffi.rs), [`ttl.rs`](../../crates/prophet-lv2/src/ttl.rs)):**
  - Plugin URI: `http://prophet-xii.org/plugins/prophet-xii` (`lv2:InstrumentPlugin`)
  - UI URI: `http://prophet-xii.org/plugins/prophet-xii#ui` (`ui:X11UI`)
  - State Property Key: `http://prophet-xii.org/plugins/prophet-xii#presetV2` (`atom:String` storing CRC-32-validated `prophet_preset` v2 JSON)
  - Entrypoints: `lv2_descriptor(0)` and `lv2ui_descriptor(0)` exported cleanly with deterministic Turtle generation (`gen_ttl` + [`scripts/bundle-lv2.sh`](../../scripts/bundle-lv2.sh)).
- **91-Port Deterministic Layout (`0..=90`):**
  - `0..=1`: `out_l`, `out_r` (`lv2:AudioPort`, `lv2:OutputPort`, `pg:StereoGroup`)
  - `2`: `control_in` (`atom:AtomPort` `lv2:InputPort`, `atom:Sequence` supporting `midi:MidiEvent`, `time:Position`, `patch:Message`)
  - `3`: `notify_out` (`atom:AtomPort` `lv2:OutputPort`, `atom:Sequence`, `lv2:connectionOptional`, `rsz:minimumSize 8192`)
  - `4`: `reverb_enabled` (`lv2:ControlPort` `lv2:InputPort`, `lv2:toggled`, `pprops:notOnGUI`)
  - `5`: `reverb_kill_tail` (`lv2:ControlPort` `lv2:InputPort`, `lv2:toggled`, `pprops:notOnGUI`)
  - `6..=88`: Engine parameters `p0`–`p82` (matching `PARAM_SPECS` units, `logarithmic`, `integer`, scale points for `ChorusMode`, `ReverbEnsembleMode`, `OversamplingMode` `{1, 2, 4}`, `NotePriority`, `GlideMode`)
  - `89`: `glide_enabled` (`p83` `GLIDE_ENABLED`, `lv2:toggled`)
  - `90`: `latency` (`lv2:ControlPort`, `lv2:OutputPort`, `lv2:reportsLatency`, dynamic `0` / `16` / `24` samples at 1x/2x/4x oversampling)
- **RT-Safe Worker, Host Sync & State Isolation:**
  - `run()` performs zero heap allocations, zero mutex acquisitions, and zero `Box<PreparedEngine>` / `Box<PatternBank>` drops. When `OversamplingMode` or `reverb_enabled` / `reverb_kill_tail` changes require re-preparation or idle swap, `run()` schedules a 1-byte `work:schedule` task onto the LV2 worker thread (`Lv2Worker`) and retires old instances cleanly in `work_response()` / background polling.
  - `time:Position` (`time:beatsPerMinute`) Atom events synchronize `MusicTransport` tempo without allocating.
  - `rt_override_*` + `last_host_rev_*` tracking prevents stale host control-port baselines from clobbering UI-driven changes or v2 state restores (`SetOversampling`, Vector4 topology, reverb bypass/tail), while UI <-> host automation uses `LV2UI_Write_Function` + `ui_port_event`.
  - `state:interface`: Missing `presetV2` key resets state safely to factory Init (`16`-sample 2x latency persisted across blocks); corrupted / CRC-failed JSON returns `LV2_STATE_ERR_UNKNOWN` without disturbing the running engine.
- **Embedded X11 UI (`ui:X11UI` in [`ui.rs`](../../crates/prophet-lv2/src/ui.rs)):**
  - Parents `GtkPlug` into host X11 `ui:parent` Window ID via `prophet_host::embed`, supporting `lv2:instance-access`, `ui:resize`, `ui:idleInterface`, and `ui:showInterface`.

---

## 2. Native Desktop Standalone Architecture (`crates/prophet-standalone`)

[`crates/prophet-standalone/src/main.rs`](../../crates/prophet-standalone/src/main.rs) has been migrated so default execution is a self-contained native desktop application backed by `prophet-host` (`create_host_session`, `RtHostSlot`, `HostControlPlane`, `spawn_embedded_editor(ParentWindow::StandaloneWindow, ...)`):

- **Default Mode (Zero HTTP / Zero TCP / Zero Browser):**
  - Launches CPAL audio callback (or `--null` software clock in test mode) directly via `RtHostSlot::process_planar_f32` (allocation-free stack chunking `<= 1024` frames across `f32`, `i16`, `u16` sample formats and mono/stereo/multi-channel devices).
  - Routes `midir` hardware MIDI through lock-free `rtrb::RingBuffer<[u8; 3]>` -> control worker thread -> `HostControlPlane::dispatch_hardware_midi(PhysicalMidiSource::Native, ...)` and drains retired `PreparedEngine` instances off the audio thread.
  - Opens a native `GtkWindow` (`ParentWindow::StandaloneWindow`, `1180x720`, resizable, zoomable) serving embedded React/Cairo assets without listening on any TCP port or opening external browser tabs.
  - `--headless` runs the native `prophet-host` + audio/MIDI session without opening an X11 window.
- **Opt-In Developer Mode (`--http-dev` / `--bind ADDR`):**
  - Retains the legacy Axum REST/WebSocket bridge (`/api/v1/{meta,command,state,filter,ws,capture}`) and static UI directory serving for developer automation and [`host_contract.py`](../../crates/prophet-standalone/tests/host_contract.py) parity.

---

## 3. Qualification Matrix

| Target / Check | Status | Evidence Grade | Command / Harness | Observed Result |
| :--- | :--- | :--- | :--- | :--- |
| `prophet-lv2` Rust Tests | PASS | PROVEN / HOST_VERIFIED | `cargo test --release --locked --offline -p prophet-lv2` | `3/3` passed (`lv2_abi_struct_sizes`, `ttl_defines_91_ports_and_required_extensions`, `lv2_descriptor_and_state_roundtrip_preserve_vector4_and_latency`), 0 warnings |
| LV2 Bundle Generation | PASS | PROVEN / HOST_VERIFIED | `./scripts/bundle-lv2.sh` | Generated `target/lv2/prophet-xii.lv2/{libprophet_lv2.so,manifest.ttl,prophet-xii.ttl}` cleanly |
| Real Lilv/Suil/Xvfb LV2 Host | PASS | PROVEN / HOST_VERIFIED | `xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/lv2_lilv_host` ([`lv2_lilv_host.c`](../../crates/prophet-lv2/tests/lv2_lilv_host.c)) | `lilv_world_load_all` + `lilv_plugin_verify` OK; `91` ports (latency port `90`); dual simultaneous `SuilInstance` `ui:X11UI` embeds (`0x400003` & `0x400007`, reopen `0x40007d`); `XTest` click drove `suil_write_cb` updating `s1` cutoff `2400.0 -> 17134.7 Hz` (`s2=2400.0`); Atom `time:Position` (`148.5 BPM`) + `midi:MidiEvent` produced `e1=1.9336`, `e2=0.0000`; `OS 2x->4x` worker swap set `latency=24` (`s2=16`); Lilv state string save/restore preserved `latency=24.0` + `cutoff` + `reverb_enabled=0.0`; missing key reset to Init (`latency=16.0` persisted); corrupt JSON returned `LV2_STATE_ERR_UNKNOWN`; `0` X11 errors |
| LV2 Schema CLI Linters (`sord_validate` / `lv2lint` / `lv2info`) | BLOCKED / NOT_TESTED | NOT_TESTED | Checked `.prophet-cache/sysroot/usr/bin` | Sysroot provides `liblilv-0` / `libsuil-0` / `libsord-0` and `lv2_validate` shell script, but lacks `sord_validate`, `lv2lint`, and `lv2info` binaries; Lilv/Serd/Sord Turtle validation via `lilv_world_load_all` + `lilv_plugin_verify` passed |
| Linux LV2 DAW Hosts (Ardour / Carla / Qtractor) | PARTIAL | NOT_TESTED | Binary hosts absent in CI container | Full C ABI, Turtle, Worker, State, Atom MIDI/Time, and Suil X11UI verified in custom Lilv/Suil host; interactive DAW runs pending desktop environment |
| Standalone Unit & RT Allocation Tests | PASS | PROVEN / HOST_VERIFIED | `cargo test --release --locked --offline -p prophet-standalone` | `16/16` passed (`3` main CLI/URL tests, `3` zero-allocation RT callback tests, `6` Engine/MIDI Learn contract tests, `4` VCF transfer tests) |
| Standalone Native Window + Zero TCP Check | PASS | PROVEN / HOST_VERIFIED | `xvfb-run -a -s "-screen 0 1920x1080x24" ./target/release/prophet-standalone --null --demo --duration 4` + `/proc/<pid>/fd` vs `/proc/net/tcp{,6}` | Spawned `embedded linux-gtk3-cairo` window (PID `2353`, `3` Unix/X11 sockets, `[]` TCP sockets — **0 TCP sockets**), exited `0` |
| Standalone `--headless` (No X11) | PASS | PROVEN / HOST_VERIFIED | `env -u DISPLAY ./target/release/prophet-standalone --null --headless --demo --duration 1` | Started native `prophet-host` + `--null` sink cleanly with no `DISPLAY`, exited `0` |
| Standalone Opt-In HTTP/WS Contract | PASS | PROVEN / HOST_VERIFIED | `python3 crates/prophet-standalone/tests/host_contract.py` | `PROVEN / HOST_VERIFIED`: `96k/128-frame` null host, `83` values, `10` factory snapshots, range checks, bypass ack, MIDI learn lifecycle, real ADSR stages, contiguous nonzero PCM (`22` blocks) |
| Linux Bundle Packaging Script | PASS | PROVEN / HOST_VERIFIED | `./scripts/bundle-linux.sh /tmp/test-linux-bundle` | Assembled `PROPHET-XII.clap`, `PROPHET-XII.vst3/Contents/x86_64-linux/PROPHET-XII.so`, `PROPHET-XII.lv2/{libprophet_lv2.so,manifest.ttl,prophet-xii.ttl}`, `prophet-standalone` |
| WebKit2GTK Tier 1 & Physical Audio Hardware | PARTIAL | NOT_TESTED | Inspected [`embed.rs`](../../crates/prophet-host/src/embed.rs) & [`main.rs`](../../crates/prophet-standalone/src/main.rs) | `libwebkit2gtk` and physical ALSA/WASAPI hardware absent in CI VM; runtime cleanly exercised Tier 2 `linux-gtk3-cairo` and `--null` clock |

---

## 4. Reproducible Verification Commands

```bash
export CARGO_HOME=/home/antigravity/.prophet-cache/cargo-home
export RUSTUP_HOME=/home/antigravity/.prophet-cache/rustup
export PKG_CONFIG_PATH=/home/antigravity/.prophet-cache/sysroot/usr/lib/x86_64-linux-gnu/pkgconfig:/home/antigravity/.prophet-cache/sysroot/usr/share/pkgconfig
export LD_LIBRARY_PATH=/home/antigravity/.prophet-cache/sysroot/usr/lib/x86_64-linux-gnu:${LD_LIBRARY_PATH:-}
export SUIL_MODULE_DIR=/home/antigravity/.prophet-cache/sysroot/usr/lib/x86_64-linux-gnu/suil-0

# 1. LV2 unit tests & bundle generation
cargo test --release --locked --offline -p prophet-lv2
./scripts/bundle-lv2.sh

# 2. C / Lilv / Suil / XTest multi-instance LV2 host verification
export LV2_PATH="$(pwd)/target/lv2:/home/antigravity/.prophet-cache/sysroot/usr/lib/lv2:/usr/lib/lv2"
gcc -O2 -Wall -Wextra -Werror \
  -I/home/antigravity/.prophet-cache/sysroot/usr/include \
  -I/home/antigravity/.prophet-cache/sysroot/usr/include/lilv-0 \
  -I/home/antigravity/.prophet-cache/sysroot/usr/include/suil-0 \
  -I/home/antigravity/.prophet-cache/sysroot/usr/include/sord-0 \
  -I/home/antigravity/.prophet-cache/sysroot/usr/include/serd-0 \
  crates/prophet-lv2/tests/lv2_lilv_host.c \
  -L/home/antigravity/.prophet-cache/sysroot/usr/lib/x86_64-linux-gnu \
  -Wl,-rpath,/home/antigravity/.prophet-cache/sysroot/usr/lib/x86_64-linux-gnu \
  -llilv-0 -lsuil-0 -lX11 -lXtst \
  -o /tmp/lv2_lilv_host
xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/lv2_lilv_host

# 3. Standalone native desktop GUI, headless, opt-in HTTP/WS contract, and bundling
cargo test --release --locked --offline -p prophet-standalone
cargo build --release --locked --offline -p prophet-standalone
xvfb-run -a -s "-screen 0 1920x1080x24" ./target/release/prophet-standalone --null --demo --duration 3
env -u DISPLAY ./target/release/prophet-standalone --null --headless --demo --duration 1
python3 crates/prophet-standalone/tests/host_contract.py
./scripts/bundle-linux.sh
```
