# Building the integrated UI

Use the existing checkout; do not discard local changes or replace backend files.
The native backend must implement the contract at Agent B research checkpoint
`211c4f44dc735a89047dad32e725e39b9b8c9b68`. This frontend remains compatible with
the earlier host but reports missing native extensions explicitly.

Prerequisites: Rust stable (validated 1.100.0), Node >=18, wasm32-unknown-unknown
target, and wasm-bindgen-cli **0.2.130**, matching the pinned bridge dependency.
Rust registry/artifact verification and inherited proxy/CA trust stay enabled.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.130
cd ui
npm ci
npm run build
```

Build runs Rust/WASM generation, TypeScript and Vite. Generated JS/types/WASM are
ignored under `ui/generated/analysis`; production artifacts are generated in the
chosen Vite output directory, not committed as new binaries. A fresh checkout
needs the generation step before a standalone typecheck; `npm run build:analysis`
runs just that step. `WASM_BINDGEN` can select an existing matching CLI executable.

The managed cloud's baseline Cargo home intentionally replaces all crates with
its older vendor snapshot. Use the prepared separate FFT dependency cache:

```sh
cd /workspace/prophet-xii
. /workspace/.prophet-cloud/env.sh
cd ui
PROPHET_ANALYSIS_CARGO_HOME=/workspace/.prophet-cloud/analysis-cargo npm run build -- --outDir /workspace/.prophet-cloud/task-a/integrated-ui-dist
npm run lint
npm run typecheck
npm test
```

The script also detects the scoped CLI installed at
`/workspace/.prophet-cloud/wasm-bindgen/bin/wasm-bindgen`; normal machines use PATH.
The separate Cargo home changes source selection, not checksum/TLS verification.

## Native integration without merging branches

For this verification B was built from an exact Git archive in a separate
directory. No owned backend file, root Cargo manifest/lockfile, original binary
or original evidence was edited. The UI was served by that real Rust process:

```sh
# From the backend checkout/archive, with its native prerequisites active:
cargo build --release --locked -p prophet-standalone
target/release/prophet-standalone --null --sample-rate 48000 --buffer-frames 256 \
  --bind 127.0.0.1:8792 --ui /workspace/.prophet-cloud/task-a/integrated-ui-dist
```

Use 96000 and a separate port for the second-rate checks. Null sink processes
actual audio/DSP/capture without audible playback; it does not qualify a device.

For frontend hot reload, `npm run dev` builds the WASM first. Vite proxies the
existing HTTP and WebSocket API routes to `PROPHET_BACKEND_URL` (default native
host on loopback port 8787). This includes continuous PCM; no endpoint is added:

```sh
PROPHET_BACKEND_URL=http://127.0.0.1:8792 npm run dev
```

Reproduction of spectral tests, WASM execution and retained native benchmarks is
in [../analysis/WASM.md](../analysis/WASM.md) and
[../analysis/BENCHMARKS.md](../analysis/BENCHMARKS.md). The browser test commands
and fault-injection scope are in [../../ui/tests/README.md](../../ui/tests/README.md).
