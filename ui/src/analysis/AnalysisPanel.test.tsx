import React from 'react';
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AnalysisPanel } from './AnalysisPanel';
import type { Frame, Metadata } from '../protocol/types';

const values = Array(83).fill(0);
const frame = (sequence: number): Frame => ({ v: 1, ready: true, sampleRate: 48000, values, scope: [], activeVoiceMask: 0, voiceVcaLevels: [], voiceFiltEnvLevels: [], voiceMidiNotes: [], dspLoadPercent: 0, underruns: 0, deadlineMisses: 0, frames: sequence, trimpots: [], spectrum: { sequence, firstSample: sequence * 1024, sampleRate: 48000, fftSize: 2048, hopSize: 512, window: 'hann', coherentGain: .5, enbwBins: 1.5, normalization: 'single-sided-amplitude-dbfs', dbfs: Array(1025).fill(-60) } } as Frame);
const metadata: Metadata = { protocolVersion: 1, capabilities: [], presets: [] };
class TestWorker {
  static instances: TestWorker[] = [];
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  messages: unknown[] = [];
  terminated = false;
  constructor() { TestWorker.instances.push(this); }
  postMessage(message: unknown) { this.messages.push(message); }
  terminate() { this.terminated = true; }
}

beforeEach(() => {
  vi.useFakeTimers();
  TestWorker.instances = [];
  vi.stubGlobal('Worker', TestWorker);
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ fillRect() {}, beginPath() {}, moveTo() {}, lineTo() {}, stroke() {}, fillText() {} } as unknown as CanvasRenderingContext2D);
  vi.stubGlobal('fetch', vi.fn(() => new Promise(() => {})));
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('analysis pipeline', () => {
  it('does not create a continuous FFT pipeline from legacy scope snapshots', () => {
    const view = render(<AnalysisPanel frame={frame(1)} values={values} connected token="" metadata={metadata} />);
    expect(view.getByText(/Continuous spectrum unavailable/)).toBeInTheDocument();
    expect(TestWorker.instances).toHaveLength(0);
    expect(view.queryByLabelText('Continuous output spectrum dBFS')).not.toBeInTheDocument();
  });
  it('uses one capture worker independent of scope telemetry frames', () => {
    const native: Metadata = { ...metadata, extensions: ['continuous-capture'] };
    const view = render(<AnalysisPanel frame={frame(1)} values={values} connected token="" metadata={native} />);
    const worker = TestWorker.instances[0];
    view.rerender(<AnalysisPanel frame={frame(2)} values={values} connected token="" metadata={native} />);
    view.rerender(<AnalysisPanel frame={frame(3)} values={values} connected token="" metadata={native} />);
    expect(TestWorker.instances).toHaveLength(1);
    expect(worker.messages.filter(m => (m as {kind:string}).kind === 'start')).toHaveLength(1);
    expect(worker.messages[0]).toMatchObject({ kind: 'start', url: expect.stringContaining('/api/v1/capture') });
    view.unmount();
    expect(worker.terminated).toBe(true);
  });
  it('ignores superseded transfer responses even if fetch ignores abort', async () => {
    const complete: ((response: unknown) => void)[] = [];
    const request = vi.fn(() => new Promise(resolve => complete.push(resolve)));
    vi.stubGlobal('fetch', request);
    const view = render(<AnalysisPanel frame={frame(1)} values={values} connected token="" metadata={metadata} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(180); });
    const changed = [...values]; changed[23] = 1000;
    view.rerender(<AnalysisPanel frame={frame(1)} values={changed} connected token="" metadata={metadata} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(180); });
    expect(request).toHaveBeenCalledTimes(1);
    const response = (db: number[]) => ({ ok: true, json: async () => ({ hz: [20, 20000], db, phase: [0, -1] }) });
    await act(async () => { complete[0](response([0, -90])); });
    expect(request).toHaveBeenCalledTimes(2);
    expect(view.container.querySelector('path')!.getAttribute('d')).toBe('');
    await act(async () => { complete[1](response([0, -20])); });
    expect(view.container.querySelector('path')!.getAttribute('d')).toContain('55.000');
    expect(view.getByText(/48000 Hz fixed legacy/)).toBeInTheDocument();
  });
  it('displays native FFT transfer sample rate, resolution, and tail convergence without legacy re-unwrapping', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        sampleRate: 96000,
        hz: [20, 2000, 20000],
        db: [0, -30, -120],
        phase: [-0.1, -6.9, null],
        fftSize: 16384,
        impulseSamples: 8192,
        tailConverged: true,
        tailRelativeRms: 2e-5,
        binHz: 96000 / 16384,
      }),
    })));
    const view = render(<AnalysisPanel frame={frame(1)} values={values} connected token="" metadata={metadata} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(180); });
    expect(view.getByText(/96000 Hz · 16384-point FFT \(8192 samples\) · Δf 5\.859 Hz · tail converged/)).toBeInTheDocument();
    const phasePath = view.container.querySelectorAll('path')[1].getAttribute('d')!;
    expect(phasePath).toContain((75 - (-6.9) * 12).toFixed(3));
  });
  it('handles HTTP 409 superseded transfer responses without reporting a transport failure', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false, status: 409 })));
    const view = render(<AnalysisPanel frame={frame(1)} values={values} connected token="" metadata={metadata} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(180); });
    expect(view.getByText('Measurement superseded by newer request')).toBeInTheDocument();
  });
});
