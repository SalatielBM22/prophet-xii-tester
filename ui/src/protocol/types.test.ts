import { describe, expect, it } from 'vitest';
import specs from '../param-specs.json';
import { decodeFrame, decodeMetadata, withFrameCapabilities } from './types';
import type { Frame } from './types';
function frame(): Frame {
  return { v: 1, ready: true, sampleRate: 48000, values: specs.map(s => s.min), scope: [0], activeVoiceMask: 0, voiceVcaLevels: Array(12).fill(0), voiceFiltEnvLevels: Array(12).fill(0), voiceMidiNotes: Array(12).fill(-1), dspLoadPercent: 0, underruns: 0, deadlineMisses: 0, frames: 1, trimpots: Array.from({ length: 12 }, () => [0, 0, 0, 0]) };
}
const envelope = { stage: 0, elapsedSamples: 0, level: 0, progress: 0, timed: false };
describe('actual additive native v1 protocol', () => {
  it('keeps original baseline v1 supported and rejects unknown major versions', () => {
    expect(decodeFrame(frame())).not.toBeNull();
    expect(decodeFrame({ ...frame(), v: 2 })).toBeNull();
    expect(() => decodeMetadata({ v: 2, presets: [] })).toThrow('Unsupported');
    expect(() => decodeMetadata({ protocolVersion: 2, presets: [] })).toThrow('Unsupported');
    expect(decodeMetadata({ presets: [] }).capabilities).toEqual([]);
  });
  it('enables capabilities from validated native structures, not invented version2 declarations', () => {
    const native = decodeFrame({ ...frame(), revision: 1, sequence: 2, monotonicNs: 42, parameterCommandIds: Array(83).fill(0), control: { lastAppliedCommandId: 0, appliedFrame: 0 }, ampEnvelopes: Array(12).fill(envelope), filterEnvelopes: Array(12).fill(envelope), midiLearn: { active: false, target: null, ccMap: Array(128).fill(null) }, reverb: { enabled: true, tail: 'spill', dormant: true, enableGain: 1 } });
    expect(native).not.toBeNull();
    const meta = decodeMetadata({ v: 1, extensions: ['continuous-capture'], presets: [{ id: 0, name: 'Factory', category: 'Test', values: specs.map(s => s.min) }] });
    expect(withFrameCapabilities(meta, native!).capabilities).toEqual(['factoryPresetValues', 'continuousSpectrum', 'parameterAck', 'midiLearnLifecycle', 'voiceEnvelopes', 'reverbBypass']);
    expect(withFrameCapabilities(meta, frame()).capabilities).toEqual(['factoryPresetValues', 'continuousSpectrum']);
  });
  it('rejects partial or malformed native stamps rather than silently applying legacy heuristics', () => {
    expect(decodeFrame({ ...frame(), revision: 1 })).toBeNull();
    expect(decodeFrame({ ...frame(), revision: 1, sequence: 1, monotonicNs: 1, parameterCommandIds: Array(82).fill(0), control: {} })).toBeNull();
    expect(decodeFrame({ ...frame(), revision: 1, sequence: '1', monotonicNs: 1, parameterCommandIds: Array(83).fill(0), control: { lastAppliedCommandId: 1, appliedFrame: 1 } })).toBeNull();
  });
  it('strictly validates real MIDI map, reverb and ADSR telemetry without inventing stages', () => {
    expect(decodeFrame({ ...frame(), midiLearn: { active: true, target: null, ccMap: Array(128).fill(null) } })).toBeNull();
    expect(decodeFrame({ ...frame(), midiLearn: { active: false, target: null, ccMap: Array(128).fill(83) } })).toBeNull();
    expect(decodeFrame({ ...frame(), reverb: { enabled: true, tail: 'spill', dormant: false, enableGain: 2 } })).toBeNull();
    expect(decodeFrame({ ...frame(), ampEnvelopes: Array(12).fill({ ...envelope, stage: 5 }), filterEnvelopes: Array(12).fill(envelope) })).toBeNull();
    expect(decodeFrame({ ...frame(), ampEnvelopes: Array(12).fill({ ...envelope, stage: 3, timed: true, progress: .5 }), filterEnvelopes: Array(12).fill(envelope) })).toBeNull();
    expect(decodeFrame({ ...frame(), ampEnvelopes: Array(12).fill(envelope) })).toBeNull();
  });
  it('rejects invalid factory values and accepts immutable producer snapshots', () => {
    const values = specs.map(s => s.min);
    const meta = decodeMetadata({ presets: [{ id: 0, name: 'Factory', category: 'Test', values }] });
    values[23] = 999;
    expect(meta.presets[0].values?.[23]).toBe(20);
    expect(() => decodeMetadata({ presets: [{ id: 0, name: 'Factory', category: 'Test', values: [0] }] })).toThrow('factory');
  });
});
