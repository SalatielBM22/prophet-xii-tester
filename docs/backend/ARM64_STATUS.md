# ARM64 and platform status (#1, #4)

PROVEN / HOST_VERIFIED (cross-compilation on x86_64, not execution):

- Rust 1.100.0 `aarch64-unknown-linux-gnu` builds DSP/control/params release rlibs.
- A separate actual relocatable AArch64 ELF object was emitted from the same
  dependency-free kernel source. [arm64-object.txt](evidence/arm64-object.txt)
  records its ELF machine/header; SHA256 is
  `512d9d5fdec5b29f9738a1d222a38343f1b7d89f95034babaa1e357ffb0693cb`.
- `cargo check --target aarch64-linux-android -p prophet-android --locked` checks
  the Android-gated AAudio/JNI code, including new MIDI/bypass/ADSR telemetry.
  A host-only Cargo test would not exercise this cfg branch.
- `cargo check --target x86_64-pc-windows-gnu -p prophet-standalone --locked`
  checks the native Windows CPAL/WASAPI build path. It does not link an .exe or
  execute a Windows driver. ASIO was not added or made mandatory.

Reproduce from repository after sourcing `/workspace/.prophet-tools/env.sh`:

```sh
rustup target add aarch64-unknown-linux-gnu aarch64-linux-android x86_64-pc-windows-gnu
cargo build --release --locked --target aarch64-unknown-linux-gnu -p prophet-dsp -p prophet-control -p prophet-params
rustc --edition 2021 --target aarch64-unknown-linux-gnu --crate-type lib --crate-name prophet_dsp -C opt-level=3 -C lto=off -C embed-bitcode=no --emit=obj crates/prophet-dsp/src/lib.rs -o /workspace/.prophet-build/agent-b/prophet-dsp-aarch64.o
readelf -h /workspace/.prophet-build/agent-b/prophet-dsp-aarch64.o
cargo check --locked --target aarch64-linux-android -p prophet-android
cargo check --locked --target x86_64-pc-windows-gnu -p prophet-standalone
```

The standard Cargo release uses thin LTO; its intermediate objects may be LLVM
bitcode, so readelf rejecting those does not establish a bad target. The explicit
non-LTO object above independently demonstrates AArch64 machine code generation.

PROVEN / SOURCE_INSPECTED: no x86 intrinsics were introduced. The existing `simd`
feature is empty, not a measured vector implementation. `rustc --print cfg
--target aarch64-linux-android` reports NEON and 64-bit atomics available; this
is a compiler target capability, not runtime feature detection or a SIMD speedup.
Portable scalar underflow guards were used instead of changing x86 MXCSR/ARM FPCR.

OPEN / NOT_TESTED:

- No Android SDK, NDK, `ANDROID_NDK_HOME` or `ANDROID_SDK_ROOT` was available.
  No APK/JNI shared object was linked, signed, installed or executed. Type-checking
  does not validate the AAudio ABI, stream timing, XRun polling or recovery.
- No AArch64 runner/emulator/qemu, Windows machine, physical audio device test,
  or Moto G8/Snapdragon 665 was used. No DEVICE_VERIFIED or
  ANDROID_EMULATOR_VERIFIED result exists.
- CPU/thermal/memory limits on Snapdragon 665, NEON scheduling, sustained
  deadlines, device buffer negotiation and disconnect/reconnect remain open.
- JNI does not yet implement native capture/control-timestamp parity. New command
  kinds require Agent A's Java/UI binding work. CLAP/VST3 host behavior is separate.
- Bitwise determinism is verified only for repeated runs/partitions on this
  x86_64 host; different libm, compiler/FMA and architecture can change f32 bits.

Next device gate: build with a reviewed NDK, query actual AAudio format/rate/burst/
buffer and XRun counts off RT, then run the same 0/1/6/12-voice 48/96 kHz and
1x/2x/4x matrix at several supported buffers with sustained thermal load and
disconnect recovery. Preserve all failures and output hashes. No extrapolation
from Xeon timings is sufficient to qualify the phone. Firmware, partitions,
bootloader and root were not touched.
