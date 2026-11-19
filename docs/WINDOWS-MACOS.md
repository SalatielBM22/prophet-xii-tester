# Windows and macOS builds

Source paths use CPAL WASAPI (Windows) or CoreAudio (macOS), and midir's platform MIDI API. ASIO is not enabled.

## Windows

### Native window and embedded UI

The standalone opens a native window by default. On Windows that window hosts a Microsoft Edge WebView2 child through `wry` (`crates/prophet-host/src/embed.rs`, `mod windows_webview2`). The child renders the React build baked into the binary from `ui/dist`, so no HTTP listener or browser is involved.

- **Assets:** they are served from the offline `http://prophet.localhost/` scheme. Only that exact origin may navigate or reach the host bridge.
- **Bridge:** the UI talks to the engine through `window.chrome.webview.postMessage` (EnginePort v1, `embedded-host` transport). Telemetry is pushed at about 30 Hz.
- **Profile:** WebView2 keeps its user data in `%LOCALAPPDATA%\PROPHET-XII\WebView2`. Its default location next to the executable is read-only for DAWs installed under Program Files.
- **Message loop:** the standalone runs a Win32 message loop on the thread that created the window. Plugins rely on the DAW's editor thread loop instead.
- **Fallback:** without the WebView2 runtime the window falls back to a small GDI status view, whose backend kind is `windows-win32-gdi` instead of `windows-webview2`. Install the Evergreen WebView2 Runtime if needed; Windows 11 ships it.

`--http-dev` / `--bind` still serve the same UI to a browser for development.

### Toolchains and `WebView2Loader.dll`

| Toolchain | WebView2 loader | Runtime requirement |
|---|---|---|
| MSVC (`x86_64-pc-windows-msvc`), recommended for releases | linked statically (`WebView2LoaderStatic.lib`) | none beyond the WebView2 runtime |
| GNU / MinGW (`x86_64-pc-windows-gnu`), e.g. cross-built from WSL | imported from `WebView2Loader.dll` | the DLL must sit next to the executable, or the process fails with `STATUS_DLL_NOT_FOUND` (0xC0000135) |

For a GNU build, copy the Microsoft-signed `x64/WebView2Loader.dll` shipped inside the `webview2-com-sys` crate next to `prophet-standalone.exe`. A plugin DLL resolves its imports from the DAW's directory, not its own, so **ship VST3/CLAP builds from the MSVC toolchain**.

MSVC builds need the Visual Studio Build Tools with the "Desktop development with C++" workload, plus Rust stable MSVC and Node. Then:

1. Run `cargo build --release --locked -p prophet-standalone -p prophet-render -p prophet-plugin`.
2. `cd ui`, `npm ci`, `npm run build`. This rebuilds `ui/dist`, which is embedded at compile time.
3. Run `target\release\prophet-standalone.exe`.

Cross-building from WSL:

```sh
CARGO_TARGET_DIR=~/target-prophet cargo build --release --target x86_64-pc-windows-gnu -p prophet-standalone -p prophet-plugin
```

The MinGW linker is configured in `~/.cargo/config.toml`.

### Verification status

| Item | Status |
|---|---|
| Standalone native window with the React UI (WebView2), Windows 11, WASAPI 48 kHz | HOST_VERIFIED: the window stays responsive, the EnginePort v1 transport is active and capabilities are negotiated |
| `prophet-host` unit tests executed on Windows, including asset serving, bridge injection and the origin check | HOST_VERIFIED |
| VST3/CLAP editor inside a DAW, resize, multiple instances, reopen | NOT_TESTED |
| Audio timing on WASAPI (`--http-dev --timing-log FILE.csv`) | to be measured |

## macOS

Install Rust, Xcode Command Line Tools and Node, then the same Cargo/npm commands. Run `target/release/prophet-standalone --ui ui/dist`. The embedded editor returns an explicit "not available" error on macOS. CLAP/VST3 bundle layout, signing, architecture coverage and host loading must be completed and tested on macOS. No macOS binary, notarization, DAW validation or device performance is claimed.
