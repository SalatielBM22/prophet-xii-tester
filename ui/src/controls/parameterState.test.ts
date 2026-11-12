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
  it('does not treat 202 enqueue or a global other-client stamp as parameter application', async () => {
    const transport = new ControlTransport('', vi.fn(() => Promise.resolve(new Response(JSON.stringify({ acceptedCommandId: 42 }), { status: 202 }))));
    transport.metadata = { protocolVersion: 1, capabilities: ['parameterAck'], presets: [] };
    const state = new ParameterState(transport);
    const first = frame({ sequence: 1, revision: 1, monotonicNs: 1000, parameterCommandIds: Array(83).fill(0), control: { lastAppliedCommandId: 0, appliedFrame: 0 } });
    state.receive(first); state.edit(23, 900); await vi.advanceTimersByTimeAsync(16);
    expect(state.getSnapshot().pending).toBe(1);
    state.receive({ ...first, frames: 2, sequence: 2, monotonicNs: 2000, control: { lastAppliedCommandId: 99, appliedFrame: 1 } });
    expect(state.getSnapshot().values[23]).toBe(900); expect(state.getSnapshot().pending).toBe(1);
    const values = [...first.values]; values[23] = 899.999;
    const stamps = [...first.parameterCommandIds!]; stamps[23] = 42;
    state.receive({ ...first, values, frames: 3, sequence: 3, revision: 2, monotonicNs: 3000, parameterCommandIds: stamps, control: { lastAppliedCommandId: 99, appliedFrame: 2 } });
    expect(state.getSnapshot().values[23]).toBe(899.999); expect(state.getSnapshot().pending).toBe(0);
    expect(state.receive({ ...first, frames: 4, sequence: 2, monotonicNs: 4000 })).toBe(false);
    expect(state.getSnapshot().values[23]).toBe(899.999); transport.dispose();
  });
  it('reports a competing direct write to the same parameter as superseded, not our ACK', async () => {
    const transport = new ControlTransport('', vi.fn(() => Promise.resolve(new Response(JSON.stringify({ acceptedCommandId: 42 }), { status: 202 }))));
    transport.metadata = { protocolVersion: 1, capabilities: ['parameterAck'], presets: [] };
    const state = new ParameterState(transport);
    const first = frame({ sequence: 1, revision: 1, monotonicNs: 1000, parameterCommandIds: Array(83).fill(0), control: { lastAppliedCommandId: 0, appliedFrame: 0 } });
    state.receive(first); state.edit(23, 900); await vi.advanceTimersByTimeAsync(16);
    const values = [...first.values]; values[23] = 1200;
    const stamps = [...first.parameterCommandIds!]; stamps[23] = 43;
    state.receive({ ...first, values, frames: 2, sequence: 2, revision: 2, monotonicNs: 2000, parameterCommandIds: stamps });
    expect(state.getSnapshot().values[23]).toBe(1200); expect(state.getSnapshot().pending).toBe(0); expect(state.getSnapshot().error).toMatch(/superseded/);
    transport.dispose();
  });
  it('rejects counter regressions until reconnect and then starts a fresh host epoch', () => {
    const transport = new ControlTransport('', vi.fn(accepted)), state = new ParameterState(transport);
    const first = frame({ frames: 1000, sequence: 10, revision: 5, monotonicNs: 10000, parameterCommandIds: Array(83).fill(0), control: { lastAppliedCommandId: 0, appliedFrame: 0 } });
    state.receive(first);
    expect(state.receive({ ...first, frames: 1, sequence: 0, revision: 0, monotonicNs: 1 })).toBe(false);
    state.disconnect(); transport.reset();
    expect(state.receive({ ...first, frames: 1, sequence: 0, revision: 0, monotonicNs: 1 })).toBe(true);
    expect(state.getSnapshot().canUndo).toBe(false); transport.dispose();
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
    expect(decodeFrame({ ...frame(), ampEnvelopes: [] })).toBeNull();
    expect(decodeFrame({ ...frame(), midiLearn: { active: false, target: null, ccMap: [255] } })).toBeNull();
    expect(decodeFrame({ ...frame(), reverb: { enabled: 'yes' } })).toBeNull();
    expect(decodeMetadata({ presets: [{ id: 0, name: 'Preset', category: 'Test' }] }).capabilities).toEqual([]);
  });
});
