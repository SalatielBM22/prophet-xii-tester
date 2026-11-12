import { describe, expect, it } from 'vitest';
import { decodeFrame, decodeMetadata } from './types';

const frame = () => ({ v: 1, ready: true, sampleRate: 48000, values: Array(83).fill(0), scope: Array(1024).fill(0), activeVoiceMask: 0, voiceVcaLevels: Array(12).fill(0), voiceFiltEnvLevels: Array(12).fill(0), voiceMidiNotes: Array(12).fill(0), dspLoadPercent: 0, underruns: 0, deadlineMisses: 0, frames: 1024, trimpots: Array.from({ length: 12 }, () => [0, 0, 0, 0]) });
describe('protocol boundary', () => {
  it('retains legacy metadata without granting unadvertised capabilities', () => {
    expect(decodeMetadata({ presets: [], capabilities: ['parameterAck'] })).toMatchObject({ protocolVersion: 1, capabilities: [] });
    expect(decodeFrame(frame())).not.toBeNull();
  });
  it('does not coerce malformed protocol fields into trusted telemetry', () => {
    expect(decodeFrame({ ...frame(), v: '2' })).toBeNull();
    expect(decodeFrame({ ...frame(), ready: 'true' })).toBeNull();
    expect(decodeFrame({ ...frame(), appliedCommandId: '42' })).toBeNull();
    expect(decodeFrame({ ...frame(), values: Array(83).fill(NaN) })).toBeNull();
  });
  it('rejects unknown required protocol versions and missing v2 session identity', () => {
    expect(() => decodeMetadata({ protocolVersion: 3, presets: [] })).toThrow('Unsupported');
    expect(() => decodeMetadata({ protocolVersion: 2, presets: [] })).toThrow('session');
    expect(decodeFrame({ ...frame(), v: 2, sessionId: '', sequence: 1, parameterRevision: 1 })).toBeNull();
  });
  it('negotiates only recognized advertised capabilities and rejects invalid optional state', () => {
    expect(decodeMetadata({ protocolVersion: 2, sessionId: 'host', capabilities: ['parameterAck', 'futureFeature'], presets: [] }).capabilities).toEqual(['parameterAck']);
    expect(decodeFrame({ ...frame(), reverb: { enabled: true } })).toBeNull();
    expect(decodeFrame({ ...frame(), envelopes: { amp: [], filter: [] } })).toBeNull();
    expect(decodeFrame({ ...frame(), midiLearn: { state: 'assigned', parameterId: 23, bindings: [{ parameterId: 23, channel: 16, cc: 200 }] } })).toBeNull();
  });
});
