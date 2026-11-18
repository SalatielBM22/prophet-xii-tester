#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
out="${1:-binaries/linux-x86_64}"
mkdir -p "$out/PROPHET-XII.vst3/Contents/x86_64-linux"
for name in prophet-standalone prophet-render prophet-probes prophet-bench prophet-presets; do
  if [[ -f "target/release/$name" ]]; then
    cp "target/release/$name" "$out/$name"
  fi
done
if [[ -f target/release/libprophet_plugin.so ]]; then
  cp target/release/libprophet_plugin.so "$out/PROPHET-XII.clap"
  cp target/release/libprophet_plugin.so "$out/PROPHET-XII.vst3/Contents/x86_64-linux/PROPHET-XII.so"
fi
./scripts/bundle-lv2.sh "$out/PROPHET-XII.lv2"
