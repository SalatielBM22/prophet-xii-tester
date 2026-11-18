# PROPHET-XII Gate B1 — Android Audio, Offline WebView & USB MIDI Diagnostics (Issue #15)

## 1. Evidence & Qualification Status Summary

| Scope | Classification | Verification Evidence |
| :--- | :--- | :--- |
| Rust control plane, probe matrix, MIDI 1.0 stream decoder, EnginePort JSON parity | **HOST_VERIFIED** | `cargo test --locked -p prophet-android -p prophet-host` (`4/4` Android controller + `7/7` EnginePort tests passing on x86_64 Linux host) |
| ARM64 (`aarch64-linux-android`) Rust + AAudio NDK FFI compilation | **COMPILE_ONLY** | `cargo check --locked -p prophet-android --target aarch64-linux-android` passed (`libaaudio` link deferred without NDK linker/APK packaging) |
| Java Android sources (`MainActivity`, `SynthService`, `Native`) | **COMPILE_ONLY** | `javac -Xlint:all -source 8 -target 8` syntax/type/signature verification against API 26–35 stubs; real `android.jar` + `d8`/`aapt2` absent on VM |
| Physical APK build, Android device AAudio playback & USB MIDI live capture | **NOT_TESTED** | `ANDROID_SDK_ROOT`, `ANDROID_NDK_HOME`, `adb`, and authorized physical ARM64 Android phone are absent in the VM (`OPEN` physical root cause) |
| Live offline WebView UI rendering inside `MainActivity` | **NOT_TESTED** (Android Chromium), React UI bundled via `ui/dist` | Packaged in `scripts/build-android.sh` (`aapt2 link -A ui/dist`); browser UI visually tested on desktop Playwright in Gate B4 |

---

## 2. Root-Cause Audit of Baseline Android Failure (Issue #15)

Direct inspection of the baseline `android/` and `crates/prophet-android/` sources plus NDK/Oboe sources (`ndk-sys 0.5.0`, `oboe-sys 0.6.1`) identified concrete host/Java bugs (`PROVEN`) alongside physical HAL failure hypotheses (`INFERRED` / `OPEN`):

### 2.1. Confirmed Baseline Bugs (`PROVEN`)
1. **Unconditional `stopSelf()` on AudioFocus loss & non-granted start**: The baseline `SynthService.onCreate()` tore down the entire foreground service and HTTP/WS server whenever `requestAudioFocus` returned `AUDIOFOCUS_REQUEST_DELAYED` or `onAudioFocusChange(change < 0)` fired (including temporary notifications / `AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK`).
2. **Missing hardware rate/burst hints**: `Native.start()` opened AAudio blindly without querying `AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE` or `PROPERTY_OUTPUT_FRAMES_PER_BUFFER`, and without fallback negotiation across `PCM_FLOAT` vs `PCM_I16`, `LOW_LATENCY` vs `NONE`, or HAL-native rate `0`.
3. **Unsafe AAudio callback lifecycle & missing API-28 `dlsym` guards**: Direct linking to post-API-26 AAudio functions on Android 8.0/8.1 causes `UnsatisfiedLinkError`; teardown must call `AAudioStream_requestStop` and `AAudioStream_close` before freeing callback user pointers so AAudio threads cannot UAF.
4. **No integrated React WebView (`MainActivity`)**: Baseline `MainActivity` only displayed plain Android `TextView` + localhost instructions pointing at an unauthenticated/always-on `ServerSocket(8787)` with `android:usesCleartextTraffic="true"` and `?token=` passed in URL query strings.
5. **Broken protocol parity & missing USB MIDI**: Baseline `Native.command(int,int,int,float)` lacked Vector4, Reverb `LegacyRC1`/`DiffuseV1`, Sequencer/Arp, v2 presets, filter analysis, `p83 = Glide Enabled`, and Android `MidiManager` USB controller input.

### 2.2. Direct AAudio vs Oboe C++ Evidence (`SOURCE_INSPECTED`)
Inspected `.prophet-cache/rust-vendor/oboe-sys-0.6.1/oboe/src/aaudio/{AAudioLoader.cpp,AudioStreamAAudio.cpp}` and `.prophet-cache/rust-vendor/ndk-sys-0.5.0`:
- **API 26–28 compatibility (`PROVEN`)**: `AAudioStreamBuilder_setUsage` and `AAudioStreamBuilder_setContentType` require API 28+, and `AAudioStream_isMMapUsed` is an internal/hidden AAudio symbol. Resolved safely at runtime via `libc::dlsym(RTLD_DEFAULT, ...)` while statically linking only core API 26 `libaaudio` symbols (`start`, `stop`, `getSampleRate`, `getChannelCount`, `getFormat`, `getSharingMode`, `getPerformanceMode`, `getFramesPerBurst`, `getBufferCapacityInFrames`, `getBufferSizeInFrames`, `setBufferSizeInFrames`, `getXRunCount`).
- **Android R headset disconnect bug (`PROVEN` from Oboe source)**: Android R (`b/173928197`) surfaces `AAUDIO_ERROR_TIMEOUT` (`-885`) on output disconnect/reroute instead of `AAUDIO_ERROR_DISCONNECTED` (`-899`). Both (`-899`, `-885`, `-889` UNAVAILABLE) are classified as reconnectable in `is_reconnectable_aaudio_error`.
- **Why direct `dlsym`-guarded AAudio was retained instead of pulling `oboe-sys`**: `oboe-sys` requires CMake NDK C++ compilation and bundles `libc++_shared.so` APK packaging complexity on `minSdkVersion="26"`, while direct AAudio reproduces Oboe's verified API-28 `dlsym` guards, actual negotiated getter checks (`getSharingMode`/`getPerformanceMode`), mono/stereo `Float`/`I16` rendering, and disconnect handling purely in Rust + `prophet-host`. Note: Oboe also falls back to OpenSL ES on buggy API 26–27 vendor HALs; if an Android 8 device still rejects every step in `build_probe_plan`, switching to `oboe` remains an isolated crate swap in `crates/prophet-android`.

---

## 3. Implemented Architecture (Gate B1 Deliverables)

- **Audio Probe Ladder (`crates/prophet-android/src/probe.rs`)**:
  Deduplicated fallback matrix starting with `(hintSr, PCM_FLOAT, LOW_LATENCY, SHARED)` -> `(HAL native 0, PCM_FLOAT, LOW_LATENCY, SHARED)` -> `(hintSr, PCM_I16, LOW_LATENCY, SHARED)` -> `(0, PCM_I16, LOW_LATENCY, SHARED)` -> `(0, PCM_FLOAT, NONE, SHARED)` -> `(0, PCM_I16, NONE, SHARED)` -> conservative `48000` / `44100` `PCM_I16` `NONE` `SHARED`. Records actual negotiated rate, channels, format, sharing mode, performance mode, burst, capacity, buffer size, and MMAP status per step in `probeHistory`.
- **Zero-Alloc RT Callback (`crates/prophet-android/src/lib.rs`)**:
  Executes `RtHostSlot::process_interleaved_f32` / `process_interleaved_i16` for stereo and downmixes cleanly for mono HALs without heap allocation, locks, JSON parsing, or JNI calls inside `aaudio_data_callback`. Applies click-free per-frame gain ramping for AudioFocus mute/duck (`0.25x` / `0.0x` / `1.0x`).
- **Persistent Controller & EnginePort Parity (`crates/prophet-android/src/controller.rs`)**:
  Stores `SoundState`, `PatternBank`, `Performance`, `CcLearnRouter`, and `CanonicalMidiArbiter` outside `ActiveStream`, preserving user patches, `p83` (`Glide Enabled`), sequencer/arp state, and USB MIDI across AAudio disconnect/reconnect and sample-rate renegotiation.
- **USB MIDI 1.0 & Audio Route (`SynthService.java` + `Midi1StreamDecoder`)**:
  Monitors `AudioDeviceCallback` and `android.media.midi.MidiManager`, opening external controller output ports into `Native.usbMidi(...)` decoded onto canonical `ORIGIN_HARDWARE_MIDI`.
- **Hardened Offline WebView (`MainActivity.java` + `AndroidManifest.xml`)**:
  - `android:usesCleartextTraffic="false"`
  - Serves packaged `assets/` (`ui/dist`) locally via framework `WebViewClient.shouldInterceptRequest` on `https://appassets.androidplatform.net/` (`setAllowFileAccess(false)`, `setAllowContentAccess(false)`).
  - Exposes minimal `@JavascriptInterface` object `ProphetAndroidBridge` (`dispatch`, `pollSnapshot`, `diagnostics`).
  - Developer LAN server (`ServerSocket(8787)`) is **OFF by default**, strictly opt-in via `HostControl("toggleDevBridge")`, rejects `?token=` URL parameters (`401`), and accepts only `Authorization: Bearer <token>` routing through the exact same `Native.dispatchJson` / `Native.pollSnapshotJson` contract.

---

## 4. Build, Install & Rollback Instructions (External SDK/Device Required)

On a workstation with Android SDK (API 35 + `build-tools/35.0.0`) and Android NDK (r27c+):

```bash
export ANDROID_SDK_ROOT=/path/to/android-sdk
export ANDROID_NDK_HOME=/path/to/android-ndk-r27c
./scripts/build-android.sh
```

Unsigned APK output: `binaries/android-arm64/PROPHET-XII-unsigned.apk` (or signed `binaries/android-arm64/PROPHET-XII.apk` when `PROPHET_KEYSTORE` and `PROPHET_KEYSTORE_PASSWORD_SOURCE` are exported).

To verify on a USB-debugging authorized device:
```bash
adb install -r binaries/android-arm64/PROPHET-XII-unsigned.apk # or signed APK
adb logcat -s PROPHET-XII AAudio AudioFlinger chromium
```

To collect audio startup telemetry without guessing:
1. Open the app -> tap **Diagnostics** in the error banner (if shown) or inspect `Native.diagnosticsJson()` via the UI System/Diagnostics drawer (`probeHistory`, `negotiatedSr`, `negotiatedFormat`, `framesPerBurst`, `mmapUsed`, `xruns`, `focusState`, `route`, `usbMidiDevices`).
2. To roll back cleanly: `adb uninstall org.prophetxii`.
