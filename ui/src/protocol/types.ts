import specs from '../param-specs.json';

export type ParamSpec = typeof specs[number];
/** UI capability names are internal; the native wire protocol remains additive v1. */
export type Capability = 'parameterAck' | 'factoryPresetValues' | 'midiLearnLifecycle' | 'voiceEnvelopes' | 'reverbBypass' | 'continuousSpectrum' | 'filterResponseV2';
export type EnvelopeStage = 0 | 1 | 2 | 3 | 4;
export interface VoiceEnvelope { stage: EnvelopeStage; elapsedSamples: number; level: number; progress: number; timed: boolean }
export const envelopeStageNames = ['Idle', 'Attack', 'Decay', 'Sustain', 'Release'] as const;
export interface MidiLearnState { active: boolean; target: number | null; ccMap: Array<number | null> }
export interface NativeControl {
  lastAppliedCommandId: number;
  appliedFrame: number;
  lastDroppedCommandId?: number;
  ingressDrops?: number;
  hardwareMidiDrops?: number;
}
export interface Frame {
  v: 1;
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
  revision?: number;
  sequence?: number;
  monotonicNs?: number;
  parameterCommandIds?: number[];
  control?: NativeControl;
  midiLearn?: MidiLearnState;
  ampEnvelopes?: VoiceEnvelope[];
  filterEnvelopes?: VoiceEnvelope[];
  reverb?: { enabled: boolean; tail: 'kill' | 'spill'; dormant: boolean; enableGain: number };
  captureDroppedChunks?: number;
}
export interface Preset { id: number; name: string; category: string; values?: number[] }
export interface Metadata { protocolVersion: 1; capabilities: Capability[]; extensions?: string[]; presets: Preset[] }
export type Command =
  | { type: 'param'; id: number; value: number }
  | { type: 'snapshot'; values: number[] }
  | { type: 'preset'; index: number }
  | { type: 'midi'; status: number; data1: number; data2: number }
  | { type: 'panic' }
  | { type: 'learn'; id: number }
  | { type: 'cancel_learn' }
  | { type: 'unlearn'; id: number }
  | { type: 'remap'; cc: number; id: number }
  | { type: 'reverb_bypass'; enabled: boolean; tail: 'kill' | 'spill' }
  | { type: 'trimpot'; voice: number; field: number; value: number };

const finiteNumbers = (x: unknown, length?: number): x is number[] => Array.isArray(x) && (length === undefined || x.length === length) && x.every(v => typeof v === 'number' && Number.isFinite(v));
const object = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null;
const count = (x: unknown) => typeof x === 'number' && Number.isSafeInteger(x) && x >= 0;
const parameterId = (x: unknown) => count(x) && Number(x) < specs.length;
const level = (x: unknown) => typeof x === 'number' && Number.isFinite(x) && x >= 0 && x <= 1;
const validPreset = (x: unknown): x is number[] => finiteNumbers(x, specs.length) && x.every((v, i) => v >= specs[i].min && v <= specs[i].max && (!specs[i].integer || Number.isInteger(v)) && (specs[i].key !== 'OversamplingMode' || [1, 2, 4].includes(v)));
export function decodeFrame(input: unknown): Frame | null {
  if (!object(input) || input.ready !== true || input.v !== 1) return null;
  if (!finiteNumbers(input.values, specs.length) || !finiteNumbers(input.scope) || input.scope.length > 8192) return null;
  for (const key of ['voiceVcaLevels', 'voiceFiltEnvLevels', 'voiceMidiNotes']) if (!finiteNumbers(input[key], 12)) return null;
  for (const key of ['sampleRate', 'dspLoadPercent']) if (typeof input[key] !== 'number' || !Number.isFinite(input[key])) return null;
  for (const key of ['frames', 'activeVoiceMask', 'underruns', 'deadlineMisses']) if (!count(input[key])) return null;
  if (Number(input.sampleRate) < 8000 || Number(input.sampleRate) > 384000 || Number(input.activeVoiceMask) > 4095) return null;
  if (!Array.isArray(input.trimpots) || input.trimpots.length !== 12 || !input.trimpots.every(t => finiteNumbers(t, 4))) return null;
  const nativeKeys = ['revision', 'parameterCommandIds', 'sequence', 'monotonicNs', 'control'];
  if (nativeKeys.some(k => input[k] !== undefined)) {
    if (!['revision', 'sequence', 'monotonicNs'].every(k => count(input[k])) || !finiteNumbers(input.parameterCommandIds, specs.length) || !input.parameterCommandIds.every(count)) return null;
    const control = input.control;
    if (!object(control) || !count(control.lastAppliedCommandId) || !count(control.appliedFrame)) return null;
    for (const key of ['lastDroppedCommandId', 'ingressDrops', 'hardwareMidiDrops']) if (control[key] !== undefined && !count(control[key])) return null;
  }
  for (const key of ['ampEnvelopes', 'filterEnvelopes']) if (input[key] !== undefined) {
    const array = input[key];
    if (!Array.isArray(array) || array.length !== 12 || !array.every(v => object(v) && count(v.stage) && Number(v.stage) <= 4 && count(v.elapsedSamples) && level(v.level) && level(v.progress) && typeof v.timed === 'boolean' && ((v.stage === 0 || v.stage === 3) ? v.timed === false && v.progress === 0 : true))) return null;
  }
  if ((input.ampEnvelopes === undefined) !== (input.filterEnvelopes === undefined)) return null;
  if (input.midiLearn !== undefined) {
    const m = input.midiLearn;
    if (!object(m) || typeof m.active !== 'boolean' || !(m.target === null || parameterId(m.target)) || !Array.isArray(m.ccMap) || m.ccMap.length !== 128 || !m.ccMap.every(id => id === null || parameterId(id)) || (m.active && m.target === null)) return null;
  }
  if (input.reverb !== undefined) {
    const r = input.reverb;
    if (!object(r) || typeof r.enabled !== 'boolean' || !['kill', 'spill'].includes(String(r.tail)) || typeof r.dormant !== 'boolean' || !level(r.enableGain)) return null;
  }
  if (input.captureDroppedChunks !== undefined && !count(input.captureDroppedChunks)) return null;
  return input as unknown as Frame;
}
export function decodeMetadata(input: unknown): Metadata {
  if (!object(input) || !Array.isArray(input.presets)) throw Error('Invalid host metadata');
  if (input.v !== undefined && input.v !== 1 || input.protocolVersion !== undefined && input.protocolVersion !== 1) throw Error('Unsupported host protocol');
  if (input.extensions !== undefined && (!Array.isArray(input.extensions) || !input.extensions.every(e => typeof e === 'string'))) throw Error('Invalid native extensions');
  const presets = input.presets.map((p: unknown) => {
    if (!object(p) || !parameterId(p.id) || typeof p.name !== 'string' || typeof p.category !== 'string') throw Error('Invalid preset metadata');
    if (p.values !== undefined && !validPreset(p.values)) throw Error('Invalid factory preset values');
    return { id: Number(p.id), name: p.name, category: p.category, values: validPreset(p.values) ? [...p.values] : undefined };
  });
  const extensions = Array.isArray(input.extensions) ? input.extensions as string[] : [];
  const capabilities: Capability[] = [];
  if (presets.length && presets.every(p => p.values !== undefined)) capabilities.push('factoryPresetValues');
  if (extensions.includes('continuous-capture')) capabilities.push('continuousSpectrum');
  return { protocolVersion: 1, capabilities, extensions: [...extensions], presets };
}
/** Enable native controls only after validating their real telemetry structures. */
export function withFrameCapabilities(metadata: Metadata, frame: Frame): Metadata {
  const capabilities: Capability[] = metadata.capabilities.filter(c => c === 'factoryPresetValues' || c === 'continuousSpectrum');
  if (frame.parameterCommandIds && frame.revision !== undefined && frame.sequence !== undefined && frame.monotonicNs !== undefined && frame.control) capabilities.push('parameterAck');
  if (frame.midiLearn) capabilities.push('midiLearnLifecycle');
  if (frame.ampEnvelopes && frame.filterEnvelopes) capabilities.push('voiceEnvelopes');
  if (frame.reverb) capabilities.push('reverbBypass');
  return { ...metadata, capabilities };
}
