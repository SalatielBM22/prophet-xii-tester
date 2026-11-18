// SPDX-License-Identifier: GPL-3.0-only
/**
 * Unified `UiTransport` host-adapter abstraction across all PROPHET-XII surfaces:
 * - `embedded-host`: `window.__PROPHET_HOST__` (VST3, CLAP, LV2, Native Desktop Standalone)
 * - `android-bridge`: `window.ProphetAndroidBridge` (Android APK WebView + JNI/AAudio)
 * - `window-native`: `window.Native` (generic embedded native bridge fallback)
 * - `http-dev`: Opt-in developer HTTP/WebSocket bridge (`--http-dev` / `--bind` / RC1 compat)
 */
import {
  decodeUiCommandReply,
  decodeUiSnapshot,
  type AndroidAudioDiagnostics,
  type UiCommand,
  type UiCommandReply,
  type UiSnapshot,
} from './enginePort';

export type TransportKind = 'embedded-host' | 'android-bridge' | 'window-native' | 'http-dev';

export interface ProphetHostGlobal {
  backend: string;
  dispatchAsync: (cmd: UiCommand) => Promise<unknown>;
  requestSnapshot: () => string;
  setZoom: (zoom: number) => void;
  getZoom: () => number;
}

export interface ProphetAndroidBridgeGlobal {
  dispatch: (cmdJson: string) => string;
  pollSnapshot: () => string;
  diagnostics: () => string;
}

export interface WindowNativeGlobal {
  dispatch: (cmdJson: string) => string;
  poll: () => string;
}

interface HostWindow extends Window {
  __PROPHET_HOST__?: ProphetHostGlobal;
  ProphetAndroidBridge?: ProphetAndroidBridgeGlobal;
  Native?: WindowNativeGlobal;
}

export interface NativeUiTransport {
  readonly kind: Exclude<TransportKind, 'http-dev'>;
  readonly backendLabel: string;
  start(onSnapshot: (snapshot: UiSnapshot) => void, onError: (message: string) => void): () => void;
  dispatch(command: UiCommand): Promise<UiCommandReply>;
  setZoom(zoom: number): void;
  getZoom(): number;
  pollDiagnostics(): AndroidAudioDiagnostics | null;
}

export function detectTransportKind(win: Window = window): TransportKind {
  const w = win as HostWindow;
  if (w.__PROPHET_HOST__ && typeof w.__PROPHET_HOST__.dispatchAsync === 'function') {
    return 'embedded-host';
  }
  if (w.ProphetAndroidBridge && typeof w.ProphetAndroidBridge.dispatch === 'function') {
    return 'android-bridge';
  }
  if (w.Native && typeof w.Native.dispatch === 'function' && typeof w.Native.poll === 'function') {
    return 'window-native';
  }
  return 'http-dev';
}

class EmbeddedHostUiTransport implements NativeUiTransport {
  readonly kind = 'embedded-host' as const;
  private host: ProphetHostGlobal;
  private win: Window;

  constructor(host: ProphetHostGlobal, win: Window) {
    this.host = host;
    this.win = win;
  }

  get backendLabel(): string {
    return this.host.backend || 'embedded-host';
  }

  start(onSnapshot: (snapshot: UiSnapshot) => void, onError: (message: string) => void): () => void {
    const handleEvent = (ev: Event) => {
      const detail = (ev as CustomEvent<unknown>).detail;
      const decoded = decodeUiSnapshot(detail);
      if (decoded) {
        onSnapshot(decoded);
      } else if (detail !== undefined) {
        onError('Ignored malformed native host snapshot');
      }
    };
    this.win.addEventListener('prophet-snapshot', handleEvent);

    const requestInitial = () => {
      try {
        const cached = this.host.requestSnapshot();
        if (cached) {
          const parsed = decodeUiSnapshot(JSON.parse(cached));
          if (parsed) onSnapshot(parsed);
        }
      } catch {
        // Initial push arrives asynchronously from WebKit/WebView2 timer.
      }
    };
    requestInitial();
    const fallbackPoll = setInterval(() => {
      try {
        this.host.requestSnapshot();
      } catch {
        // Ignore transient host reload.
      }
    }, 500);

    return () => {
      clearInterval(fallbackPoll);
      this.win.removeEventListener('prophet-snapshot', handleEvent);
    };
  }

  async dispatch(command: UiCommand): Promise<UiCommandReply> {
    try {
      const raw = await this.host.dispatchAsync(command);
      return decodeUiCommandReply(raw);
    } catch (err) {
      return {
        ok: false,
        status: 'invalid',
        message: err instanceof Error ? err.message : String(err),
      };
    }
  }

  setZoom(zoom: number): void {
    if (typeof this.host.setZoom === 'function') {
      this.host.setZoom(zoom);
    }
  }

  getZoom(): number {
    return typeof this.host.getZoom === 'function' ? this.host.getZoom() : 1.0;
  }

  pollDiagnostics(): AndroidAudioDiagnostics | null {
    return null;
  }
}

class AndroidBridgeUiTransport implements NativeUiTransport {
  readonly kind = 'android-bridge' as const;
  readonly backendLabel = 'android-aaudio';
  private bridge: ProphetAndroidBridgeGlobal;
  private zoom = 1.0;

  constructor(bridge: ProphetAndroidBridgeGlobal) {
    this.bridge = bridge;
  }

  start(onSnapshot: (snapshot: UiSnapshot) => void, onError: (message: string) => void): () => void {
    let lastJson = '';
    const poll = () => {
      try {
        const raw = this.bridge.pollSnapshot();
        if (!raw || raw === lastJson) return;
        lastJson = raw;
        const decoded = decodeUiSnapshot(JSON.parse(raw));
        if (decoded) {
          onSnapshot(decoded);
        }
      } catch (err) {
        onError(`Android bridge snapshot error: ${err instanceof Error ? err.message : String(err)}`);
      }
    };
    poll();
    const timer = setInterval(poll, 33);
    return () => clearInterval(timer);
  }

  async dispatch(command: UiCommand): Promise<UiCommandReply> {
    try {
      const raw = this.bridge.dispatch(JSON.stringify(command));
      return decodeUiCommandReply(raw);
    } catch (err) {
      return {
        ok: false,
        status: 'invalid',
        message: err instanceof Error ? err.message : String(err),
      };
    }
  }

  setZoom(zoom: number): void {
    this.zoom = zoom;
    document.documentElement.style.zoom = String(zoom);
  }

  getZoom(): number {
    return this.zoom;
  }

  pollDiagnostics(): AndroidAudioDiagnostics | null {
    try {
      const raw = this.bridge.diagnostics();
      if (!raw) return null;
      return JSON.parse(raw) as AndroidAudioDiagnostics;
    } catch {
      return null;
    }
  }
}

class WindowNativeUiTransport implements NativeUiTransport {
  readonly kind = 'window-native' as const;
  readonly backendLabel = 'window-native';
  private native: WindowNativeGlobal;
  private zoom = 1.0;

  constructor(native: WindowNativeGlobal) {
    this.native = native;
  }

  start(onSnapshot: (snapshot: UiSnapshot) => void, onError: (message: string) => void): () => void {
    let lastJson = '';
    const poll = () => {
      try {
        const raw = this.native.poll();
        if (!raw || raw === lastJson) return;
        lastJson = raw;
        const decoded = decodeUiSnapshot(JSON.parse(raw));
        if (decoded) onSnapshot(decoded);
      } catch (err) {
        onError(`Native bridge poll error: ${err instanceof Error ? err.message : String(err)}`);
      }
    };
    poll();
    const timer = setInterval(poll, 33);
    return () => clearInterval(timer);
  }

  async dispatch(command: UiCommand): Promise<UiCommandReply> {
    try {
      const raw = this.native.dispatch(JSON.stringify(command));
      return decodeUiCommandReply(raw);
    } catch (err) {
      return {
        ok: false,
        status: 'invalid',
        message: err instanceof Error ? err.message : String(err),
      };
    }
  }

  setZoom(zoom: number): void {
    this.zoom = zoom;
    document.documentElement.style.zoom = String(zoom);
  }

  getZoom(): number {
    return this.zoom;
  }

  pollDiagnostics(): AndroidAudioDiagnostics | null {
    return null;
  }
}

/** Creates the authoritative native `UiTransport` when running inside VST3/CLAP/LV2/Standalone/Android. */
export function createNativeTransport(win: Window = window): NativeUiTransport | null {
  const w = win as HostWindow;
  const kind = detectTransportKind(win);
  if (kind === 'embedded-host' && w.__PROPHET_HOST__) {
    return new EmbeddedHostUiTransport(w.__PROPHET_HOST__, win);
  }
  if (kind === 'android-bridge' && w.ProphetAndroidBridge) {
    return new AndroidBridgeUiTransport(w.ProphetAndroidBridge);
  }
  if (kind === 'window-native' && w.Native) {
    return new WindowNativeUiTransport(w.Native);
  }
  return null;
}
