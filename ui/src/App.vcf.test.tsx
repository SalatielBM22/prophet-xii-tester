import React from 'react';
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import specs from './param-specs.json';

// The HTTP-dev host keeps one filter request clock and answers 409 to every
// superseded request. Two components measuring the same filter therefore make
// one of them fail on every change, so exactly one owner may post.
const defaultValues = specs.map(s => s.min);
const cutoffId = specs.find(s => s.key === 'FilterCutoff')!.id;
const zeroEnvelope = { stage: 0, elapsedSamples: 0, level: 0, progress: 0, timed: false };

function frame(sequence: number, values: number[]) {
  return {
    v: 1, ready: true, sampleRate: 48000, values, scope: Array(128).fill(0),
    activeVoiceMask: 0, voiceVcaLevels: Array(12).fill(0), voiceFiltEnvLevels: Array(12).fill(0),
    voiceMidiNotes: Array(12).fill(0), dspLoadPercent: 1, underruns: 0, deadlineMisses: 0,
    frames: 256 * sequence, trimpots: Array.from({ length: 12 }, () => [0, 0, 0, 0]),
    revision: sequence, sequence, monotonicNs: 1_000_000 * sequence,
    parameterCommandIds: Array(specs.length).fill(0),
    control: { lastAppliedCommandId: 0, appliedFrame: 256 * sequence },
    ampEnvelopes: Array(12).fill(zeroEnvelope), filterEnvelopes: Array(12).fill(zeroEnvelope),
    reverb: { enabled: true, tail: 'spill', dormant: false, enableGain: 1 },
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
  constructor(public url: string) { MockWebSocket.instances.push(this); }
  emit(payload: unknown) { this.onmessage?.({ data: JSON.stringify(payload) }); }
  close() { this.readyState = MockWebSocket.CLOSED; this.onclose?.(); }
}
class MockWorker { onmessage = null; onerror = null; postMessage() {} terminate() {} }

describe('HTTP-dev VCF response plot', () => {
  let filterPosts = 0;
  beforeEach(() => {
    vi.useFakeTimers();
    filterPosts = 0;
    MockWebSocket.instances = [];
    vi.stubGlobal('WebSocket', MockWebSocket);
    vi.stubGlobal('Worker', MockWorker);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
      fillRect() {}, beginPath() {}, moveTo() {}, lineTo() {}, stroke() {}, fillText() {},
    } as unknown as CanvasRenderingContext2D);
    vi.stubGlobal('fetch', vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.startsWith('/api/v1/meta')) {
        return Promise.resolve({ ok: true, json: async () => ({ v: 1, extensions: ['continuous-capture'], presets: [] }) } as Response);
      }
      if (url.startsWith('/api/v1/filter')) {
        filterPosts++;
        return Promise.resolve({ ok: true, status: 200, json: async () => ({ sampleRate: 48000, hz: [20, 20000], db: [0, -24], phase: [0, -1] }) } as Response);
      }
      if (url.startsWith('/api/v1/command')) return Promise.resolve({ ok: true, text: async () => '' } as Response);
      return Promise.reject(new Error(`Unexpected fetch ${url}`));
    }));
  });
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

  it('has a single filter-measurement owner while the analysis drawer is open', async () => {
    render(<App />);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    const ws = MockWebSocket.instances.find(s => s.url.includes('/api/v1/ws'))!;
    const values = [...defaultValues];
    // The legacy transport reports connected after two frames with advancing audio.
    await act(async () => {
      ws.emit(frame(1, values));
      await vi.advanceTimersByTimeAsync(20);
      ws.emit(frame(2, values));
      await vi.advanceTimersByTimeAsync(20);
    });
    // Effects scheduled by the connection run after that act; let their timers fire.
    await act(async () => { await vi.advanceTimersByTimeAsync(400); });
    expect(document.querySelector('.analysis')).not.toBeNull();
    expect(filterPosts).toBe(1);

    // A cutoff sweep arriving as telemetry at 50 Hz keeps the curve moving.
    filterPosts = 0;
    for (let i = 0; i < 25; i++) {
      values[cutoffId] = 200 + i * 100;
      await act(async () => { ws.emit(frame(3 + i, [...values])); await vi.advanceTimersByTimeAsync(20); });
    }
    await act(async () => { await vi.advanceTimersByTimeAsync(400); });
    expect(filterPosts).toBeGreaterThanOrEqual(3);
    expect(document.querySelectorAll('.filterchart')).toHaveLength(0);
  });
});
