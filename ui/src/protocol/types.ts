import specs from '../param-specs.json';

export type ParamSpec = typeof specs[number];
export type Capability = 'parameterAck' | 'factoryPresetValues' | 'midiLearnLifecycle' | 'voiceEnvelopes' | 'reverbBypass' | 'continuousSpectrum' | 'filterResponseV2';
const knownCapabilities: readonly Capability[] = ['parameterAck', 'factoryPresetValues', 'midiLearnLifecycle', 'voiceEnvelopes', 'reverbBypass', 'continuousSpectrum', 'filterResponseV2'];
export type EnvelopeStage = 'Idle' | 'Attack' | 'Decay' | 'Sustain' | 'Release';
export interface VoiceEnvelope {
  voice: number;
  stage: EnvelopeStage;
  level: number;
  elapsedSamples: number;
  stageDurationSamples: number | null;
  noteGeneration: number;
}
export interface MidiBinding { parameterId: number; channel: number; cc: number }
export interface MidiLearnState {
  state: 'idle' | 'pending' | 'assigned';
  parameterId: number | null;
  bindings: MidiBinding[];
}
export interface Frame {
  v: 1 | 2;
  ready: boolean;
  sampleRate: number;
  values: number[];
  scope: number[];
  activeVoiceMask: number;
  voiceVcaLevels: number[];
  voiceFiltEnvLevels: number[];
  voiceMidiNotes: number[];
  dspLoadPercent: number;
  underruns: number;
  deadlineMisses: number;
  frames: number;
  trimpots: number[][];
  sessionId?: string;
  sequence?: number;
  sampleTimestamp?: number;
  parameterRevision?: number;
  appliedCommandId?: number;
  clientId?: string;
  midiLearn?: MidiLearnState;
  envelopes?: { amp: VoiceEnvelope[]; filter: VoiceEnvelope[] };
  reverb?: { enabled: boolean; tailPolicy: 'kill' | 'spill'; transition: 'stable' | 'crossfading'; tailStatus: 'active' | 'decaying' | 'cleared' | 'frozen' };
  presetId?: number;
  factoryPresetRevision?: number;
}
export interface Preset { id: number; name: string; category: string; values?: number[]; referenceRevision?: number }
export interface Metadata {
  protocolVersion: 1 | 2;
  capabilities: Capability[];
  sessionId?: string;
  presets: Preset[];
}
export type V1Command =
  | { type: 'param'; id: number; value: number }
  | { type: 'snapshot'; values: number[] }
  | { type: 'preset'; index: number }
  | { type: 'midi'; status: number; data1: number; data2: number }
  | { type: 'panic' }
  | { type: 'learn'; id: number }
  | { type: 'trimpot'; voice: number; field: number; value: number };
export type Command = V1Command
  | { type: 'midi_learn'; action: 'start' | 'cancel' | 'remove' | 'replace' | 'query'; parameterId: number; channel?: number; cc?: number }
  | { type: 'reverb_bypass'; enabled: boolean; tailPolicy: 'kill' | 'spill' };

const finiteNumbers = (x: unknown, length?: number): x is number[] => Array.isArray(x) && (length === undefined || x.length === length) && x.every(v => typeof v === 'number' && Number.isFinite(v));
const object = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null;
export function decodeFrame(input: unknown): Frame | null {
  if (!object(input) || input.ready !== true || (input.v !== 1 && input.v !== 2)) return null;
  if (!finiteNumbers(input.values, specs.length) || !finiteNumbers(input.scope) || input.scope.length > 8192) return null;
  for (const key of ['voiceVcaLevels', 'voiceFiltEnvLevels', 'voiceMidiNotes']) if (!finiteNumbers(input[key], 12)) return null;
  for (const key of ['sampleRate', 'frames', 'activeVoiceMask', 'dspLoadPercent', 'underruns', 'deadlineMisses']) if (typeof input[key] !== 'number' || !Number.isFinite(input[key])) return null;
  if (Number(input.sampleRate) < 8000 || Number(input.sampleRate) > 384000 || Number(input.frames) < 0) return null;
  if (!Array.isArray(input.trimpots) || input.trimpots.length !== 12 || !input.trimpots.every(t => finiteNumbers(t, 4))) return null;
  if (input.v === 2 && (typeof input.sessionId !== 'string' || !input.sessionId || !Number.isSafeInteger(input.sequence) || !Number.isSafeInteger(input.parameterRevision))) return null;
  if (input.envelopes !== undefined) {
    const e = input.envelopes;
    const valid = (a: unknown) => Array.isArray(a) && a.length === 12 && a.every((v: unknown, i: number) => object(v) && v.voice === i && ['Idle', 'Attack', 'Decay', 'Sustain', 'Release'].includes(String(v.stage)) && typeof v.level === 'number' && Number.isFinite(v.level) && v.level >= 0 && v.level <= 1 && Number.isSafeInteger(v.elapsedSamples) && Number(v.elapsedSamples) >= 0 && (v.stageDurationSamples === null || (Number.isSafeInteger(v.stageDurationSamples) && Number(v.stageDurationSamples) > 0)) && Number.isSafeInteger(v.noteGeneration));
    if (!object(e) || !valid(e.amp) || !valid(e.filter)) return null;
  }
  if (input.midiLearn !== undefined) {
    const m = input.midiLearn;
    if (!object(m) || !['idle', 'pending', 'assigned'].includes(String(m.state)) || !(m.parameterId === null || Number.isInteger(m.parameterId) && Number(m.parameterId) >= 0 && Number(m.parameterId) < specs.length) || !Array.isArray(m.bindings) || m.bindings.length > 2048 || !m.bindings.every(b => object(b) && Number.isInteger(b.parameterId) && Number(b.parameterId) >= 0 && Number(b.parameterId) < specs.length && Number.isInteger(b.channel) && Number(b.channel) >= 0 && Number(b.channel) < 16 && Number.isInteger(b.cc) && Number(b.cc) >= 0 && Number(b.cc) < 128)) return null;
  }
  if (input.reverb !== undefined) {
    const r = input.reverb;
    if (!object(r) || typeof r.enabled !== 'boolean' || !['kill', 'spill'].includes(String(r.tailPolicy)) || !['stable', 'crossfading'].includes(String(r.transition)) || !['active', 'decaying', 'cleared', 'frozen'].includes(String(r.tailStatus))) return null;
  }
  for (const key of ['appliedCommandId', 'parameterRevision', 'sequence', 'sampleTimestamp', 'factoryPresetRevision', 'presetId']) if (input[key] !== undefined && (!Number.isSafeInteger(input[key]) || Number(input[key]) < 0)) return null;
  return input as unknown as Frame;
}
export function decodeMetadata(input: unknown): Metadata {
  if (!object(input) || !Array.isArray(input.presets)) throw Error('Invalid host metadata');
  if (input.protocolVersion !== undefined && input.protocolVersion !== 1 && input.protocolVersion !== 2) throw Error('Unsupported host protocol');
  if (input.protocolVersion === 2 && (typeof input.sessionId !== 'string' || !input.sessionId)) throw Error('Missing host session');
  const presets = input.presets.map((p: unknown) => {
    if (!object(p) || !Number.isInteger(p.id) || typeof p.name !== 'string' || typeof p.category !== 'string') throw Error('Invalid preset metadata');
    return { id: Number(p.id), name: p.name, category: p.category, values: finiteNumbers(p.values, specs.length) ? [...p.values] : undefined, referenceRevision: Number.isSafeInteger(p.referenceRevision) && Number(p.referenceRevision) >= 0 ? Number(p.referenceRevision) : undefined };
  });
  return { protocolVersion: input.protocolVersion === 2 ? 2 : 1, capabilities: input.protocolVersion === 2 && Array.isArray(input.capabilities) ? input.capabilities.filter((x): x is Capability => typeof x === 'string' && knownCapabilities.includes(x as Capability)) : [], sessionId: typeof input.sessionId === 'string' ? input.sessionId : undefined, presets };
}
