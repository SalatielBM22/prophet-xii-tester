// SPDX-License-Identifier: GPL-3.0-only
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  clampToDiamond,
  computeVectorWeights,
  decodeUiCommandReply,
  decodeUiSnapshot,
  formatMidiNote,
  isPointInDiamond,
  type UiSnapshot,
} from './enginePort';
import { createNativeTransport, detectTransportKind } from './uiTransport';

function makeSnapshot(overrides: Partial<UiSnapshot> = {}): UiSnapshot {
  const parameters = Array.from({ length: 84 }, () => 0);
  parameters[18] = 8000;
  parameters[46] = 0.7;
  parameters[83] = 1;
  return {
    version: 1,
    backend: 'embedded-webkit',
    ready: true,
    enginePort: {
      version: 1,
      revision: '1',
      renderedFrames: '480',
      sampleRate: 48000,
      maxFrames: 256,
      latencySamples: 0,
      notes: [],
      voices: [],
      fx: {
        algorithm: 'LegacyRC1',
        version: 1,
        enabled: true,
        tail: 'spill',
        frozen: false,
        dormant: true,
      },
      capabilities: ['oversampling', 'vector4', 'sequencer'],
      xruns: 0,
      cpuPercent: 12.5,
    },
    parameters,
    scope: [0, 0.25, -0.25, 0],
    patch: {
      topology: 'Classic2',
      routes: ['Sum', 'Sum'],
      ratios: [1, 1],
      position: { x: 0, y: 0 },
      weights: [0.25, 0.25, 0.25, 0.25],
      stages: [
        { target: { x: 0, y: 1 }, millis: 0 },
        { target: { x: 1, y: 0 }, millis: 200 },
        { target: { x: 0, y: -1 }, millis: 200 },
        { target: { x: -1, y: 0 }, millis: 200 },
        { target: { x: 0, y: 0 }, millis: 200 },
      ],
      modulation: [
        { x: 0, y: 0 },
        { x: 0, y: 0 },
        { x: 0, y: 0 },
      ],
      loopHeld: false,
      idleForTopologyChange: true,
    },
    kernelTelemetry: {
      activeVoiceMask: 0,
      voiceVcaLevels: Array(12).fill(0),
      voiceFiltEnvLevels: Array(12).fill(0),
      voiceMidiNotes: Array(12).fill(0),
      ampEnvelopes: [],
      filterEnvelopes: [],
      vectorWeights: Array(12).fill(null),
    },
    music: null,
    midiLearn: {
      learningTarget: null,
      ccMap: Array(84).fill(null),
      physicalSource: 'native',
    },
    driveMakeup: true,
    reverbBypassEffective: false,
    reverbTailEffective: 'spill',
    presets: [{ id: 0, name: 'Warm Brass', category: 'Brass', glide: false }],
    filterAnalysis: null,
    lastNotice: null,
    diagnostics: null,
    ...overrides,
  };
}

describe('enginePort diamond math & decoders', () => {
  it('enforces |x| + |y| <= 1 diamond constraint and preserves normalized weights', () => {
    expect(isPointInDiamond(0, 0)).toBe(true);
    expect(isPointInDiamond(0.5, 0.5)).toBe(true);
    expect(isPointInDiamond(0.9, 0.9)).toBe(false);

    const clamped = clampToDiamond(1, 1);
    expect(Math.abs(clamped.x) + Math.abs(clamped.y)).toBeCloseTo(1, 6);
    expect(clamped.x).toBeCloseTo(0.5, 6);
    expect(clamped.y).toBeCloseTo(0.5, 6);

    const weightsCenter = computeVectorWeights(0, 0);
    expect(weightsCenter.reduce((a, b) => a + b, 0)).toBeCloseTo(1, 6);
    weightsCenter.forEach(w => expect(w).toBeCloseTo(0.25, 6));

    const weightsNorth = computeVectorWeights(0, 1);
    expect(weightsNorth[0]).toBeCloseTo(0, 6); // A = West
    expect(weightsNorth[1]).toBeCloseTo(1, 6); // B = North
    expect(weightsNorth[2]).toBeCloseTo(0, 6); // C = East
    expect(weightsNorth[3]).toBeCloseTo(0, 6); // D = South

    const weightsWest = computeVectorWeights(-1, 0);
    expect(weightsWest[0]).toBeCloseTo(1, 6); // A = West
    expect(weightsWest[1]).toBeCloseTo(0, 6);
  });

  it('decodes valid UiSnapshot and rejects malformed payloads', () => {
    const snap = makeSnapshot();
    expect(decodeUiSnapshot(snap)?.backend).toBe('embedded-webkit');
    expect(decodeUiSnapshot(null)).toBeNull();
    expect(decodeUiSnapshot({ version: 2 })).toBeNull();
    expect(decodeUiCommandReply(JSON.stringify({ ok: true, status: 'applied', message: 'ok' }))).toEqual({
      ok: true,
      status: 'applied',
      message: 'ok',
    });
    expect(decodeUiCommandReply('not-json').ok).toBe(false);
    expect(formatMidiNote(60)).toBe('C4');
  });
});

describe('uiTransport detection & adapters', () => {
  const win = window as unknown as Record<string, unknown>;

  afterEach(() => {
    delete win.__PROPHET_HOST__;
    delete win.ProphetAndroidBridge;
    delete win.Native;
    vi.restoreAllMocks();
  });

  it('defaults to http-dev when no native bridge global is injected', () => {
    expect(detectTransportKind(window)).toBe('http-dev');
    expect(createNativeTransport(window)).toBeNull();
  });

  it('detects and drives EmbeddedHostUiTransport (window.__PROPHET_HOST__)', async () => {
    const snap = makeSnapshot();
    const dispatchAsync = vi.fn().mockResolvedValue({ ok: true, status: 'applied', message: 'ok' });
    const setZoom = vi.fn();
    win.__PROPHET_HOST__ = {
      backend: 'webkit2gtk',
      dispatchAsync,
      requestSnapshot: () => JSON.stringify(snap),
      setZoom,
      getZoom: () => 1.25,
    };

    expect(detectTransportKind(window)).toBe('embedded-host');
    const transport = createNativeTransport(window);
    expect(transport?.kind).toBe('embedded-host');
    expect(transport?.backendLabel).toBe('webkit2gtk');
    expect(transport?.getZoom()).toBe(1.25);

    const received: UiSnapshot[] = [];
    const stop = transport!.start(s => received.push(s), () => {});
    expect(received.length).toBe(1);

    const updated = makeSnapshot({ backend: 'webkit2gtk-push' });
    window.dispatchEvent(new CustomEvent('prophet-snapshot', { detail: updated }));
    expect(received.length).toBe(2);
    expect(received[1].backend).toBe('webkit2gtk-push');

    const reply = await transport!.dispatch({ type: 'setOversampling', factor: 2 });
    expect(reply.ok).toBe(true);
    expect(dispatchAsync).toHaveBeenCalledWith({ type: 'setOversampling', factor: 2 });

    transport!.setZoom(1.5);
    expect(setZoom).toHaveBeenCalledWith(1.5);
    stop();
  });

  it('detects and drives AndroidBridgeUiTransport (window.ProphetAndroidBridge)', async () => {
    const snap = makeSnapshot({ backend: 'android-aaudio' });
    const dispatch = vi.fn().mockReturnValue(JSON.stringify({ ok: true, status: 'applied', message: 'ok' }));
    win.ProphetAndroidBridge = {
      dispatch,
      pollSnapshot: () => JSON.stringify(snap),
      diagnostics: () => JSON.stringify({ lifecycle: 'Running', sampleRate: 48000, framesPerBurst: 192, xruns: 0 }),
    };

    expect(detectTransportKind(window)).toBe('android-bridge');
    const transport = createNativeTransport(window)!;
    expect(transport.kind).toBe('android-bridge');

    const received: UiSnapshot[] = [];
    const stop = transport.start(s => received.push(s), () => {});
    expect(received.length).toBe(1);
    expect(transport.pollDiagnostics()?.framesPerBurst).toBe(192);

    const reply = await transport.dispatch({ type: 'param', id: 83, value: 1 });
    expect(reply.ok).toBe(true);
    expect(dispatch).toHaveBeenCalledWith(JSON.stringify({ type: 'param', id: 83, value: 1 }));
    stop();
  });

  it('detects and drives WindowNativeUiTransport (window.Native)', async () => {
    const snap = makeSnapshot({ backend: 'window-native' });
    const dispatch = vi.fn().mockReturnValue(JSON.stringify({ ok: true, status: 'applied', message: 'ok' }));
    win.Native = {
      dispatch,
      poll: () => JSON.stringify(snap),
    };

    expect(detectTransportKind(window)).toBe('window-native');
    const transport = createNativeTransport(window)!;
    expect(transport.kind).toBe('window-native');

    const received: UiSnapshot[] = [];
    const stop = transport.start(s => received.push(s), () => {});
    expect(received.length).toBe(1);

    const reply = await transport.dispatch({ type: 'panic', all_origins: true });
    expect(reply.ok).toBe(true);
    stop();
  });
});
