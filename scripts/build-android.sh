#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

: "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME to NDK r27c or newer}"
: "${ANDROID_SDK_ROOT:?Set ANDROID_SDK_ROOT to an Android SDK}"

# Override these for a different installed SDK/build-tools layout.
build_tools="${PROPHET_BUILD_TOOLS:-$ANDROID_SDK_ROOT/build-tools/35.0.0}"
android_jar="${PROPHET_ANDROID_JAR:-$ANDROID_SDK_ROOT/platforms/android-35/android.jar}"
ndk_clang="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android26-clang"

if [[ ! -x "$ndk_clang" ]]; then
  echo "error: NDK linker not found at $ndk_clang" >&2
  exit 1
fi
if [[ ! -f "$android_jar" ]]; then
  echo "error: android.jar not found at $android_jar" >&2
  exit 1
fi

export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$ndk_clang"
rustup target add aarch64-linux-android >/dev/null 2>&1 || true

# Build Rust ARM64 JNI shared library cleanly under locked dependencies.
cargo build --release --locked -p prophet-android --target aarch64-linux-android

# Build offline React UI bundle when requested or if ui/node_modules missing.
if [[ "${PROPHET_SKIP_UI_BUILD:-0}" != "1" ]]; then
  (
    cd ui
    if [[ ! -d node_modules ]]; then
      npm ci
    fi
    npm run build
  )
fi

if [[ ! -f ui/dist/index.html ]]; then
  echo "error: packaged offline WebView requires ui/dist/index.html" >&2
  exit 1
fi

rm -rf android/build
mkdir -p android/build/classes android/build/dex android/build/staging/lib/arm64-v8a binaries/android-arm64

if [[ -n "${PROPHET_ECJ_JAR:-}" ]]; then
  java -jar "$PROPHET_ECJ_JAR" -1.8 -classpath "$android_jar" -d android/build/classes \
    android/app/src/main/java/org/prophetxii/*.java
else
  javac -source 8 -target 8 -classpath "$android_jar" -d android/build/classes \
    android/app/src/main/java/org/prophetxii/*.java
fi

"$build_tools/d8" --lib "$android_jar" --min-api 26 --output android/build/dex \
  android/build/classes/org/prophetxii/*.class

"$build_tools/aapt2" link -I "$android_jar" \
  --manifest android/app/src/main/AndroidManifest.xml \
  -A ui/dist \
  -o android/build/unsigned.apk

cp target/aarch64-linux-android/release/libprophet_android.so android/build/staging/lib/arm64-v8a/
cp android/build/dex/classes.dex android/build/staging/
(cd android/build/staging && zip -q -r ../unsigned.apk classes.dex lib)

"$build_tools/zipalign" -f -p 4 android/build/unsigned.apk android/build/aligned.apk
cp target/aarch64-linux-android/release/libprophet_android.so binaries/android-arm64/

# The private signing key stays outside the delivery tree. Supply your own release keystore.
if [[ -n "${PROPHET_KEYSTORE:-}" ]]; then
  "$build_tools/apksigner" sign \
    --ks "$PROPHET_KEYSTORE" \
    --ks-pass "${PROPHET_KEYSTORE_PASSWORD_SOURCE:?Set an external keystore password source}" \
    --out binaries/android-arm64/PROPHET-XII.apk \
    android/build/aligned.apk
  "$build_tools/apksigner" verify --verbose binaries/android-arm64/PROPHET-XII.apk
else
  cp android/build/aligned.apk binaries/android-arm64/PROPHET-XII-unsigned.apk
fi
