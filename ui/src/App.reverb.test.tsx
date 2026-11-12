import React from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import specs from './param-specs.json';

const defaultValues = specs.map(s => s.min);
const zeroEnvelope = { stage: 0, elapsedSamples: 0, level: 0, progress: 0, timed: false };

function makeFrame(overrides: Partial<{
  frames: number;
  sequence: number;
  revision: number;
  monotonicNs: number;
  lastAppliedCommandId: number;
  lastDroppedCommandId: number;
  enabled: boolean;
  tail: 'spill' | 'kill';
  dormant: boolean;
  enableGain: number;
}> = {}) {
  const {
    frames = 256,
    sequence = 1,
    revision = 1,
    monotonicNs = 1_000_000,
    lastAppliedCommandId = 0,
    lastDroppedCommandId,
    enabled = true,
    tail = 'spill',
    dormant = false,
    enableGain = enabled ? 1 : 0,
  } = overrides;
  return {
    v: 1,
    ready: true,
    sampleRate: 48000,
    values: [...defaultValues],
    scope: Array(128).fill(0),
    activeVoiceMask: 0,
    voiceVcaLevels: Array(12).fill(0),
    voiceFiltEnvLevels: Array(12).fill(0),
    voiceMidiNotes: Array(12).fill(0),
    dspLoadPercent: 1.5,
    underruns: 0,
    deadlineMisses: 0,
    frames,
    trimpots: Array.from({ length: 12 }, () => [0, 0, 0, 0]),
    revision,
    sequence,
    monotonicNs,
    parameterCommandIds: Array(83).fill(0),
    control: {
      lastAppliedCommandId,
      appliedFrame: frames,
      ...(lastDroppedCommandId !== undefined ? { lastDroppedCommandId } : {}),
    },
    ampEnvelopes: Array(12).fill(zeroEnvelope),
    filterEnvelopes: Array(12).fill(zeroEnvelope),
    reverb: { enabled, tail, dormant, enableGain },
  };
}

class MockWebSocket {
  static OPEN = 1;
  static CLOSED = 3;
  static instances: MockWebSocket[] = [];
  readyState = MockWebSocket.OPEN;
  onmessage: ((ev: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  url: string;
  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }
  emit(payload: unknown) {
    this.onmessage?.({ data: JSON.stringify(payload) });
  }
  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.onclose?.();
  }
}

class MockWorker {
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  postMessage() {}
  terminate() {}
}

type DeferredCommand = {
  body: Record<string, unknown>;
  resolve: (value: Response) => void;
};

describe('reverb optimistic bypass and tail guard', () => {
  let commandQueue: DeferredCommand[] = [];
  let postedBodies: Array<Record<string, unknown>> = [];

  beforeEach(() => {
    vi.useFakeTimers();
    MockWebSocket.instances = [];
    commandQueue = [];
    postedBodies = [];
    vi.stubGlobal('WebSocket', MockWebSocket);
    vi.stubGlobal('Worker', MockWorker);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      fillRect() {},
      beginPath() {},
      moveTo() {},
      lineTo() {},
      stroke() {},
      fillText() {},
    } as unknown as CanvasRenderingContext2D);

    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.startsWith('/api/v1/meta')) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            v: 1,
            extensions: ['reverb-bypass', 'voice-envelopes', 'continuous-capture'],
            presets: [{ id: 0, name: 'Init', category: 'Lead', values: [...defaultValues] }],
          }),
        } as Response);
      }
      if (url.startsWith('/api/v1/filter')) {
        return Promise.resolve({
          ok: true,
          json: async () => ({
            sampleRate: 48000,
            hz: [20, 20000],
            db: [0, -24],
            phase: [0, -1],
          }),
        } as Response);
      }
      if (url.startsWith('/api/v1/command')) {
        const body = JSON.parse(String(init?.body ?? '{}')) as Record<string, unknown>;
        postedBodies.push(body);
        return new Promise<Response>(resolve => {
          commandQueue.push({ body, resolve });
        });
      }
      return Promise.reject(new Error(`Unexpected fetch ${url}`));
    }));
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  async function mountConnected() {
    render(<App />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    const ws = MockWebSocket.instances.find(s => s.url.includes('/api/v1/ws'))!;
    expect(ws).toBeDefined();
    await act(async () => {
      ws.emit(makeFrame({ frames: 256, sequence: 1 }));
      await vi.advanceTimersByTimeAsync(20);
      ws.emit(makeFrame({ frames: 512, sequence: 2 }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb ON' })).toBeEnabled();
    return ws;
  }

  function resolveNextCommand(acceptedCommandId: number) {
    const req = commandQueue.shift()!;
    req.resolve({
      ok: true,
      text: async () => JSON.stringify({ acceptedCommandId }),
    } as Response);
  }

  it('retains optimistic state across stale telemetry until host confirms applied command id', async () => {
    const ws = await mountConnected();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Reverb ON' }));
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    expect(screen.getByText(/updating/)).toBeInTheDocument();

    // Pre-202 stale WS telemetry must not revert button
    await act(async () => {
      ws.emit(makeFrame({ frames: 768, sequence: 3, lastAppliedCommandId: 0, enabled: true, tail: 'spill' }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();

    // Resolve HTTP 202 with acceptedCommandId = 10
    await act(async () => {
      resolveNextCommand(10);
      await Promise.resolve();
      await Promise.resolve();
    });
    // Still pending while lastAppliedCommandId < 10
    await act(async () => {
      ws.emit(makeFrame({ frames: 1024, sequence: 4, lastAppliedCommandId: 9, enabled: true, tail: 'spill' }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    expect(screen.getByText(/updating/)).toBeInTheDocument();

    // Authoritative telemetry arrives confirming commandId = 10
    await act(async () => {
      ws.emit(makeFrame({ frames: 1280, sequence: 5, lastAppliedCommandId: 10, enabled: false, tail: 'spill', dormant: false, enableGain: 0 }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    expect(screen.queryByText(/updating/)).not.toBeInTheDocument();
    expect(screen.getByText(/Tank processing · spill · enable gain 0.000/)).toBeInTheDocument();
  });

  it('preserves sequential bypass toggle plus tail policy selection across intermediate frames', async () => {
    const ws = await mountConnected();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Reverb ON' }));
      fireEvent.change(screen.getByLabelText('Reverb tail policy'), { target: { value: 'kill' } });
    });

    // ControlTransport serializes HTTP requests: command #1 is in flight, command #2 is queued
    expect(postedBodies).toEqual([
      { type: 'reverb_bypass', enabled: false, tail: 'spill' },
    ]);
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    expect((screen.getByLabelText('Reverb tail policy') as HTMLSelectElement).value).toBe('kill');

    // First command resolves (id 1); ControlTransport immediately dispatches queued command #2 with preserved enabled=false, tail='kill'
    await act(async () => {
      resolveNextCommand(1);
      await Promise.resolve();
      await Promise.resolve();
      ws.emit(makeFrame({ frames: 768, sequence: 3, lastAppliedCommandId: 1, enabled: false, tail: 'spill' }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(postedBodies).toEqual([
      { type: 'reverb_bypass', enabled: false, tail: 'spill' },
      { type: 'reverb_bypass', enabled: false, tail: 'kill' },
    ]);
    expect((screen.getByLabelText('Reverb tail policy') as HTMLSelectElement).value).toBe('kill');

    // Resolve second command (id 2) and confirm id 2 with dormant kill
    await act(async () => {
      resolveNextCommand(2);
      await Promise.resolve();
      await Promise.resolve();
      ws.emit(makeFrame({ frames: 1024, sequence: 4, lastAppliedCommandId: 2, enabled: false, tail: 'kill', dormant: true, enableGain: 0 }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    expect((screen.getByLabelText('Reverb tail policy') as HTMLSelectElement).value).toBe('kill');
    expect(screen.getByText(/Tank dormant · kill · enable gain 0.000/)).toBeInTheDocument();
  });

  it('clears pending reverb guard cleanly on dropped command, timeout, and websocket close', async () => {
    const ws = await mountConnected();

    // 1) Dropped command notice
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Reverb ON' }));
    });
    await act(async () => {
      resolveNextCommand(42);
      await Promise.resolve();
      await Promise.resolve();
      ws.emit(makeFrame({ frames: 768, sequence: 3, lastAppliedCommandId: 0, lastDroppedCommandId: 42, enabled: true, tail: 'spill' }));
      await vi.advanceTimersByTimeAsync(20);
    });
    expect(screen.getByRole('button', { name: 'Reverb ON' })).toBeInTheDocument();
    expect(screen.getByText('Host dropped the reverb bypass request')).toBeInTheDocument();

    // 2) 4s timeout fallback while audio telemetry continues advancing normally
    await act(async () => {
      fireEvent.change(screen.getByLabelText('Reverb tail policy'), { target: { value: 'kill' } });
    });
    await act(async () => {
      resolveNextCommand(43);
      await Promise.resolve();
      await Promise.resolve();
    });
    expect((screen.getByLabelText('Reverb tail policy') as HTMLSelectElement).value).toBe('kill');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2200);
      ws.emit(makeFrame({ frames: 1024, sequence: 4, lastAppliedCommandId: 0, enabled: true, tail: 'spill' }));
      await vi.advanceTimersByTimeAsync(20);
      await vi.advanceTimersByTimeAsync(2000);
    });
    expect((screen.getByLabelText('Reverb tail policy') as HTMLSelectElement).value).toBe('spill');
    expect(screen.getByText('Host did not confirm reverb state change within 4 seconds')).toBeInTheDocument();

    // 3) Disconnect resets pending state
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Reverb ON' }));
    });
    expect(screen.getByRole('button', { name: 'Reverb BYPASSED' })).toBeInTheDocument();
    await act(async () => {
      ws.close();
      await Promise.resolve();
    });
    expect(screen.getByRole('button', { name: 'Reverb bypass unavailable' })).toBeDisabled();
  });
});
