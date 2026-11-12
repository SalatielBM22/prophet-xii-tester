import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import specs from '../param-specs.json';
import { ParameterState } from './parameterState';
import { ControlTransport } from '../protocol/transport';
import { decodeFrame, decodeMetadata } from '../protocol/types';
import type { Frame } from '../protocol/types';

function frame(overrides: Partial<Frame> = {}): Frame {
  const values = specs.map(s => s.min); values[23] = 680;
  return { v: 1, ready: true, values, sampleRate: 48000, frames: 1, scope: [0], voiceVcaLevels: Array(12).fill(0), voiceFiltEnvLevels: Array(12).fill(0), voiceMidiNotes: Array(12).fill(-1), activeVoiceMask: 0, dspLoadPercent: 0, underruns: 0, deadlineMisses: 0, trimpots: Array.from({ length: 12 }, () => [0, 0, 0, 0]), ...overrides };
}
const accepted = () => Promise.resolve(new Response(null, { status: 202 }));
describe('ParameterState', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it('groups an entire gesture, protects stale echoes and preserves unrelated remote parameters on undo', async () => {
    const transport = new ControlTransport('', vi.fn(accepted)), state = new ParameterState(transport);
    state.receive(frame()); state.begin(23); state.edit(23, 900); state.edit(23, 1234); state.end();
    await vi.advanceTimersByTimeAsync(16);
    const stale = frame({ frames: 2 }); stale.values[24] = .5;
    state.receive(stale); expect(state.getSnapshot().values[23]).toBe(1234);
    const echoed = frame({ frames: 3 }); echoed.values[23] = 1234; echoed.values[24] = .5;
    state.receive(echoed); expect(state.getSnapshot().pending).toBe(0);
    state.undo(); expect(state.getSnapshot().values[23]).toBe(680); expect(state.getSnapshot().values[24]).toBe(.5);
    expect(state.getSnapshot().canUndo).toBe(false);
    state.redo(); expect(state.getSnapshot().values[23]).toBe(1234);
    transport.dispose();
  });
  it('requires matching-client v2 ACKs and rejects reordered sequence/revision', async () => {
    const transport = new ControlTransport('', vi.fn(accepted));
    transport.metadata = { protocolVersion: 2, capabilities: ['parameterAck'], sessionId: 'session-a', presets: [] };
    const state = new ParameterState(transport);
    const first = frame({ v: 2, sessionId: 'session-a', sequence: 1, parameterRevision: 1, appliedCommandId: 0, clientId: transport.clientId });
    state.receive(first); state.edit(23, 900); await vi.advanceTimersByTimeAsync(16);
    state.receive({ ...first, frames: 2, sequence: 2, clientId: 'another-client', appliedCommandId: 999 });
    expect(state.getSnapshot().values[23]).toBe(900); expect(state.getSnapshot().pending).toBe(1);
    const values = [...first.values]; values[23] = 899.999;
    state.receive({ ...first, values, frames: 3, sequence: 3, parameterRevision: 2, appliedCommandId: 1 });
    expect(state.getSnapshot().values[23]).toBe(899.999); expect(state.getSnapshot().pending).toBe(0);
    expect(state.receive({ ...first, frames: 4, sequence: 2 })).toBe(false);
    expect(state.getSnapshot().values[23]).toBe(899.999); transport.dispose();
  });
  it('reports timeout and ignores old-session HTTP completion', async () => {
    let release!: (response: Response) => void;
    const transport = new ControlTransport('', () => new Promise(resolve => { release = resolve; })), state = new ParameterState(transport);
    state.receive(frame()); state.edit(23, 900); await vi.advanceTimersByTimeAsync(16);
    await vi.advanceTimersByTimeAsync(4001); state.expire();
    expect(state.getSnapshot().pending).toBe(0); expect(state.getSnapshot().values[23]).toBe(680); expect(state.getSnapshot().error).toMatch(/4 seconds/);
    state.disconnect(); transport.reset();
    release(new Response(null, { status: 202 })); await Promise.resolve(); await Promise.resolve();
    expect(state.getSnapshot().error).toBe(''); transport.dispose();
  });
  it('invalidates pending edits/history before a confirmed preset barrier without deleting later intent', async () => {
    const transport = new ControlTransport('', vi.fn(accepted)), state = new ParameterState(transport);
    state.receive(frame()); state.edit(23, 900);
    const barrier = transport.nextId();
    state.edit(24, .5);
    state.acceptPresetBarrier(barrier);
    expect(state.getSnapshot().pending).toBe(1); expect(state.getSnapshot().values[23]).toBe(680); expect(state.getSnapshot().values[24]).toBe(.5);
    transport.dispose();
  });
  it('rejects invalid imported snapshot atomically', () => {
    const transport = new ControlTransport('', vi.fn(accepted)), state = new ParameterState(transport);
    state.receive(frame()); const values = [...state.getSnapshot().values]; values[0] = .5;
    expect(() => state.applySnapshot(values)).toThrow('Invalid preset'); expect(state.getSnapshot().values[0]).toBe(specs[0].min); transport.dispose();
  });
});
describe('typed wire boundary', () => {
  it('rejects malformed optional telemetry and unknown protocol majors', () => {
    expect(decodeFrame(frame())).not.toBeNull();
    expect(decodeFrame({ ...frame(), v: 3 })).toBeNull();
    expect(decodeFrame({ ...frame(), scope: [NaN] })).toBeNull();
    expect(decodeFrame({ ...frame(), envelopes: { amp: [] } })).toBeNull();
    expect(decodeFrame({ ...frame(), midiLearn: { state: 'assigned', bindings: [{ cc: 255 }] } })).toBeNull();
    expect(decodeFrame({ ...frame(), reverb: { enabled: 'yes' } })).toBeNull();
    expect(decodeMetadata({ presets: [{ id: 0, name: 'Preset', category: 'Test' }] }).capabilities).toEqual([]);
  });
});
