# PROPHET-XII Gate B2 — VST3 / CLAP & Embedded GUI Matrix

- **Work Order:** WO-XH-B v2 Gate B2 (Issue #14)
- **Branch:** `feat/dual-xhigh-host-ui-platform`
- **Verified Base:** `c5f1f873587fe553eaec67894231760b7f055ec7` (Agent A) / `b4d3f8c` (Gate B1)

## 1. Architecture Summary

`prophet-plugin` migrates VST3 and CLAP off the legacy direct `SynthEngine` / HTTP-port architecture onto `prophet-host` (`RtHostSlot`, `HostControlPlane`, and `prophet_host::embed`):

- **Lock-free Real-Time Audio Path ([`crates/prophet-plugin/src/lib.rs`](../../crates/prophet-plugin/src/lib.rs)):**
  - `process()` / `process_with_hook()` / `reset()` execute through `RtHostSlot` (`prophet-host`) and `EnginePort::process`.
  - No `Mutex` locks, JSON encoding/decoding, WebView calls, or `Box<PreparedEngine>` / `Box<PatternBank>` drops occur inside audio callbacks.
  - Off-thread engine preparation (`OversamplingMode` 1×/2×/4×, `SetReverb`, preset changes) uses `HostControlPlane` / `EngineWorker` via `BgTask::HostReconfig` and `BgTask::PollControlPlane` scheduled asynchronously via `ProcessContext::execute_background`.
  - Dynamic latency reporting calls `context.set_latency_samples(lat)` when a prepared engine swap changes `latency_samples()` (verified: `1x = 0`, `2x = 16`, `4x = 24` samples at 48 kHz).
  - Dynamic `ProcessStatus`: reports `ProcessStatus::KeepAlive` when voices or sequencer/arp are active, `ProcessStatus::Tail(samples)` while reverb decay rings out under Spill policy, and `ProcessStatus::Normal` when idle or Kill-tail bypassed.
  - Preserves all `86` NIH-plug host parameters (`reverb_enabled`, `reverb_kill_tail`, `p0`–`p82`, and `p83 = Glide Enabled`) without renumbering.
  - Compact `prophet_preset_v2` `PersistentField` stores CRC-32-validated v2 state (Vector4 patch, routing, reverb algorithm/drive makeup, optional non-default `PatternBank` & `Performance`) under 3 KB when using default pattern banks (measured `2561`–`2719` bytes in CLAP state streams).
  - `Prophet::filter_state()` migrates RC1 legacy states cleanly: infers `p83` (`Glide Enabled`) from `p42 > 0.0005` ONLY when `p83` is absent, and initializes empty `prophet_preset_v2` field for `PendingRestore::LegacyRc1`.

- **Offline Embedded UI Shell ([`crates/prophet-host/src/embed.rs`](../../crates/prophet-host/src/embed.rs)):**
  - Bakes `ui/dist` into `EMBEDDED_UI_ASSETS` at compile time via [`crates/prophet-host/build.rs`](../../crates/prophet-host/build.rs) — zero dependency on `127.0.0.1:8787` or any external browser/HTTP server.
  - **Linux X11 (`linux-gtk3-cairo` / `linux-webkit2gtk`):** Dynamically loads `libgtk-3.so.0`, `libgdk-3.so.0`, `libgobject-2.0.so.0`, `libglib-2.0.so.0`, `libcairo.so.2`, and `libX11.so.6`. Parents via `gtk_plug_new(xid)` inside DAW X11 windows or `gtk_window_new` for standalone. Runs a shared single-threaded GTK UI event reactor (`prophet-gtk-ui`) when the host lacks a native GTK main loop (`host_has_gtk_loop == false`).
    - **Tier 1 (`linux-webkit2gtk`):** Dynamically loads `libwebkit2gtk-4.1.so.0` / `4.0` when installed on target Linux hosts, registers custom offline `prophet://app/` URI scheme serving `EMBEDDED_UI_ASSETS`, and injects `window.__PROPHET_HOST__` + `window.Native` bootstrap before `<script type="module">`.
    - **Tier 2 (`linux-gtk3-cairo`):** Guaranteed interactive GTK3+Cairo faceplate fallback (reverb bypass toggle, panic button, 6 continuous faders including Cutoff/Resonance/FilterEnv/Glide/Detune/ReverbWet, live oscilloscope strip, and 2-octave performance keyboard) active when WebKit2GTK libraries are absent.
  - **Windows x64 (`windows_webview2`):** Compiles cleanly behind `#[cfg(target_os = "windows")]` with native child `HWND` (`WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN`), 30 Hz `WM_TIMER` snapshot repaint / GDI click interaction, and `WebView2Loader.dll` dynamic probing (`CreateCoreWebView2EnvironmentWithOptions`). Note: Full COM `ICoreWebView2Controller` navigation and `WebResourceRequested` interception are deferred for Windows host completion (see Section 4).

## 2. Qualification Matrix

| Target / Scenario | Status | Evidence Grade | Verification Method | Result Summary |
| :--- | :--- | :--- | :--- | :--- |
| `prophet-host` Unit & Embed Tests | PASS | PROVEN / HOST_VERIFIED | `cargo test --locked -p prophet-host` | `2/2` embed asset tests (`index.html`, script injection order) passed |
| `prophet-host` EnginePort Acceptance | PASS | PROVEN / HOST_VERIFIED | `cargo test --locked -p prophet-host` | `7/7` EnginePort v1 tests (MIDI arbitration, carried offsets, Vector4, v2 swap) passed |
| `prophet-plugin` Unit Tests | PASS | PROVEN / HOST_VERIFIED | `cargo test --locked -p prophet-plugin` | `5/5` tests (`86` params, RC1 `p83` migration, CRC v2 roundtrip, OS/latency/tail/poly-pressure, editor+zoom bridge) passed |
| `prophet-android` `aarch64` Cross-Check | PASS | COMPILE_ONLY | `cargo check --locked -p prophet-android --target aarch64-linux-android` | Zero errors after `prophet-host` / `Cargo.lock` updates |
| Linux CLAP Entry & Audio Smoke | PASS | PROVEN / HOST_VERIFIED | `/tmp/clap_smoke ./target/release/libprophet_plugin.so` | `86` params, `2561`-byte state save/load, sample-accurate MIDI at offset `17`, stereo energy `9.346701`, latency `16` samples |
| Linux CLAP Parameter/Bypass Isolation | PASS | PROVEN / HOST_VERIFIED | `/tmp/bypass_state ./target/release/libprophet_plugin.so` | Independent `reverb_enabled`, `reverb_kill_tail`, `p83` (`Glide Enabled`), state save/load (`2562` bytes) |
| Linux CLAP Dual GUI + XTest Input + State Reload | PASS | PROVEN / HOST_VERIFIED | `xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/clap_gui_multi ./target/release/libprophet_plugin.so` | Two simultaneous `GtkPlug` X11 editors (`0x400003` & `0x400007`), XTest pointer click raised p1 Filter Cutoff (`0.1324 -> 0.9519`, p2 stayed `0.1324`), clean destroy+reopen (`0x400082`), 4x/2x latency (`24`/`16`), state save (`2719`/`2561` bytes) & p3 restore (`lat=24`, `os='4'`, `rev_en=0`, `rev_kill=1`), `0` X11 errors |
| Linux VST3 COM Factory & X11 `IPlugView` Embed | PASS | PROVEN / HOST_VERIFIED | `xvfb-run -a -s "-screen 0 1600x900x24" /tmp/vst3_smoke ./target/release/libprophet_plugin.so` | `ModuleEntry`/`GetPluginFactory`/`ModuleExit`, `1` Audio Module Class (`PROPHET-XII`), `2166` exposed VST3 params, `IPlugView` `X11EmbedWindowID` `1180x720` embedded into child X11 `0x400003`, clean tear-down |
| Linux WebKit2GTK Full React DOM Inside Plugin | PARTIAL | NOT_TESTED | Inspected `embed.rs`; `libwebkit2gtk` absent in CI VM | Dynamic `libwebkit2gtk-4.1/4.0` + `prophet://app/` scheme code compiles; runtime fallback to `linux-gtk3-cairo` verified under Xvfb |
| Linux Commercial DAW Audio/GUI (Reaper / Ardour / Bitwig) | PARTIAL | NOT_TESTED | C ABIs verified; no GUI DAW installed in headless container | VST3/CLAP C ABIs and X11 parenting proven; live DAW session host testing pending on target desktop |
| Windows x64 VST3/CLAP + WebView2 | BLOCKED / PARTIAL | SOURCE_INSPECTED / NOT_TESTED | Cross-toolchain (`x86_64-pc-windows-*`) unavailable in VM; Win32 `HWND`+GDI fallback code present | Needs Windows x64 build & COM `ICoreWebView2Controller` completion + DAW verification on SalatielPC |
| macOS AudioUnit / VST3 / CLAP + WKWebView | NOT_TESTED | NOT_TESTED | Out of scope for Linux CI container | Safe stub on `target_os = "macos"` returns explicit error instead of crashing |

## 3. Reproducible Build & Verification Commands

```bash
cargo test --locked -p prophet-host -p prophet-plugin
cargo check --locked -p prophet-android --target aarch64-linux-android
cargo build --release --locked -p prophet-plugin

gcc -O2 tests/clap_smoke.c -I/home/antigravity/.prophet-cache/sysroot/usr/include -ldl -lm -o /tmp/clap_smoke
gcc -O2 crates/prophet-plugin/tests/bypass_state.c -I/home/antigravity/.prophet-cache/sysroot/usr/include -ldl -o /tmp/bypass_state
gcc -O2 -Wall -Wextra crates/prophet-plugin/tests/clap_gui_multi.c -I/home/antigravity/.prophet-cache/sysroot/usr/include -lX11 -lXtst -ldl -lm -o /tmp/clap_gui_multi
gcc -O2 -Wall -Wextra crates/prophet-plugin/tests/vst3_smoke.c -lX11 -ldl -o /tmp/vst3_smoke

/tmp/clap_smoke ./target/release/libprophet_plugin.so
/tmp/bypass_state ./target/release/libprophet_plugin.so
xvfb-run -a -s "-screen 0 2560x1080x24" /tmp/clap_gui_multi ./target/release/libprophet_plugin.so
xvfb-run -a -s "-screen 0 1600x900x24" /tmp/vst3_smoke ./target/release/libprophet_plugin.so
```

## 4. Open Items & Remaining Limitations

1. **CLAP Stepped `FloatParam` Representation (NIH-plug Detail):** `OversamplingMode` uses `FloatParam` (`1.0..=4.0`, `with_step_size(1.0)`), which `nih_plug` exposes to CLAP hosts as `[0.0, 1.0]` normalized (`1x = 0.0`, `2x ≈ 0.333`, `4x = 1.0`) with formatted text `'1'`, `'2'`, `'4'`.
2. **Windows x64 WebView2 COM Wiring:** While Win32 child `HWND` embedding, `WM_TIMER` snapshot polling, GDI interactive fallback, and dynamic `WebView2Loader.dll` probing are implemented, complete COM `ICoreWebView2Controller` navigation and custom `WebResourceRequested` scheme interception are not yet implemented or compiled on a Windows x64 toolchain.
3. **Linux WebKit2GTK Runtime:** Needs verification on a desktop distro with `libwebkit2gtk-4.1-0` (or `4.0`) installed to promote Tier 1 React DOM rendering inside plugins from `NOT_TESTED` to `HOST_VERIFIED`.
