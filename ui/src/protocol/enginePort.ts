// SPDX-License-Identifier: GPL-3.0-only
/**
 * Exact TypeScript wire contract matching `crates/prophet-host/src/{ui.rs, wire.rs, analysis.rs}`
 * and `docs/wo-a/control-v1.schema.json` (EnginePort v1).
 */
import type { ParamSpec } from './types';

export const GLIDE_ENABLED_PARAM_ID = 83;
export const GLIDE_ENABLED_SPEC: ParamSpec = {
  id: 83,
  key: 'GlideEnabled',
  name: 'Glide Enabled',
  min: 0,
  max: 1,
  integer: true,
  logarithmic: false,
  group: 'PERFORMANCE',
};

export const ORIGIN_DAW_HOST = 0;
export const ORIGIN_HARDWARE_MIDI = 1;
export const ORIGIN_LOCAL_UI = 2;
export const ORIGIN_REMOTE_WS = 3;
export const ORIGIN_WEB_MIDI = 4;
export const ORIGIN_USB_MIDI = 5;
export const ORIGIN_TEST_HARNESS = 6;
export const ORIGIN_MUSIC_SCHEDULER = 15;

export type UiTopology = 'Classic2' | 'Extended3' | 'Extended4' | 'Vector4';
export type UiRoute = 'Sum' | 'Frequency' | 'Phase' | 'Sync' | 'Ring';
export type UiArpOrder = 'Up' | 'Down' | 'UpDown' | 'AsPlayed' | 'Random' | 'Chord';
export type UiStepKind = 'rest' | 'note' | 'tie';
export type WireAlgorithm = 'LegacyRC1' | 'DiffuseV1';
export type WireTailPolicy = 'spill' | 'kill';
export type WireStatus = 'applied' | 'duplicate' | 'capacity' | 'unsupported' | 'invalid';
export type PhysicalMidiSource = 'native' | 'browser';
export type FilterValidity =
  | 'Measurable'
  | 'LongTail'
  | 'SelfOscillating'
  | 'NonlinearOperatingPoint'
  | 'NumericalFailure';

export const TOPOLOGIES: readonly UiTopology[] = ['Classic2', 'Extended3', 'Extended4', 'Vector4'];
export const ROUTES: readonly UiRoute[] = ['Sum', 'Frequency', 'Phase', 'Sync', 'Ring'];
export const ARP_ORDERS: readonly UiArpOrder[] = ['Up', 'Down', 'UpDown', 'AsPlayed', 'Random', 'Chord'];
export const STEP_KINDS: readonly UiStepKind[] = ['rest', 'note', 'tie'];

export interface UiPoint {
  x: number;
  y: number;
}

export interface UiStage {
  target: UiPoint;
  millis: number;
}

export interface UiPatchState {
  topology: UiTopology;
  routes: [UiRoute, UiRoute];
  ratios: [number, number];
  position: UiPoint;
  weights: [number, number, number, number];
  stages: [UiStage, UiStage, UiStage, UiStage, UiStage];
  modulation: [UiPoint, UiPoint, UiPoint];
  loopHeld: boolean;
  idleForTopologyChange: boolean;
}

export interface UiStep {
  kind: UiStepKind;
  key: number;
  velocity: number;
  accent: boolean;
  glide: boolean;
  gate: number;
  ratchet: number;
}

export interface UiPattern {
  index: number;
  len: number;
  steps: UiStep[];
}

export interface UiMusicState {
  playing: boolean;
  activePattern: number;
  patternLen: number;
  stepIndex: number;
  tempoMilli: number;
  swing: number;
  arpEnabled: boolean;
  arpOrder: UiArpOrder;
  arpOctaves: number;
  arpLatch: boolean;
  arpSeed: number;
  currentPattern: UiPattern;
}

export interface UiMidiLearnState {
  learningTarget: number | null;
  ccMap: Array<number | null>;
  physicalSource: PhysicalMidiSource;
}

export interface UiEnvelopeTelemetry {
  stage: number;
  elapsedSamples: number;
  progress: number;
  level: number;
  timed: boolean;
}

export interface UiKernelTelemetry {
  activeVoiceMask: number;
  voiceVcaLevels: number[];
  voiceFiltEnvLevels: number[];
  voiceMidiNotes: number[];
  ampEnvelopes: UiEnvelopeTelemetry[];
  filterEnvelopes: UiEnvelopeTelemetry[];
  vectorWeights: Array<[number, number, number, number] | null>;
}

export interface UiPresetSummary {
  id: number;
  name: string;
  category: string;
  glide: boolean;
}

export interface UiNotice {
  seq: number;
  ok: boolean;
  code: string;
  message: string;
}

export interface WireNote {
  origin: number;
  channel: number;
  key: number;
  noteId: number;
  pressed: boolean;
  sustained: boolean;
}

export interface WireVoice {
  index: number;
  key: number;
  gate: boolean;
  releasing: boolean;
  detuneSemis: number;
  pan: number;
}

export interface WireFx {
  algorithm: WireAlgorithm;
  version: number;
  enabled: boolean;
  tail: WireTailPolicy;
  frozen: boolean;
  dormant: boolean;
}

export interface WireSnapshot {
  version: number;
  revision: string;
  renderedFrames: string;
  sampleRate: number;
  maxFrames: number;
  latencySamples: number;
  notes: WireNote[];
  voices: WireVoice[];
  fx: WireFx;
  capabilities: string[];
  xruns: number | null;
  cpuPercent: number | null;
}

export interface FilterPoint {
  hz: number;
  db: number;
  phase: number | null;
}

export interface FilterResponseReport {
  sampleRate: number;
  windowSamples: number;
  validity: FilterValidity;
  points: FilterPoint[];
  tailRms: number | null;
  relativeScalingError: number | null;
  reason: string;
}

export interface AndroidAudioDiagnostics {
  lifecycle?: string;
  sampleRate?: number | null;
  channelCount?: number | null;
  format?: string | null;
  performanceMode?: string | null;
  sharingMode?: string | null;
  framesPerBurst?: number | null;
  bufferCapacityFrames?: number | null;
  bufferSizeFrames?: number | null;
  mmapUsed?: boolean | null;
  xruns?: number | null;
  callbackInvocations?: number;
  framesRendered?: number;
  focusState?: string;
  focusGain?: number;
  route?: string;
  usbMidiDevices?: string[];
  devBridgeEnabled?: boolean;
  lastErrorCode?: number | null;
  lastErrorText?: string | null;
}

export interface UiSnapshot {
  version: number;
  backend: string;
  ready: boolean;
  enginePort: WireSnapshot;
  parameters: number[];
  scope: number[];
  patch: UiPatchState;
  kernelTelemetry: UiKernelTelemetry;
  music: UiMusicState | null;
  midiLearn: UiMidiLearnState;
  driveMakeup: boolean;
  reverbBypassEffective: boolean;
  reverbTailEffective: WireTailPolicy;
  presets: UiPresetSummary[];
  filterAnalysis: FilterResponseReport | null;
  lastNotice: UiNotice | null;
  diagnostics: Record<string, unknown> | null;
}

export type UiCommand =
  | { type: 'param'; id: number; value: number }
  | { type: 'midi'; status: number; data1: number; data2: number; note_id?: number }
  | { type: 'vectorXY'; x: number; y: number }
  | { type: 'panic'; all_origins?: boolean }
  | { type: 'factoryPreset'; index: number }
  | { type: 'presetLoadJson'; json: string }
  | { type: 'presetSaveJson'; name: string; category: string }
  | { type: 'setOversampling'; factor: 1 | 2 | 4 }
  | {
      type: 'setPatch';
      topology: UiTopology;
      routes: [UiRoute, UiRoute];
      ratios: [number, number];
      stages: [UiStage, UiStage, UiStage, UiStage, UiStage];
      modulation: [UiPoint, UiPoint, UiPoint];
      loop_held: boolean;
    }
  | {
      type: 'setReverb';
      algorithm?: WireAlgorithm;
      enabled?: boolean;
      tail?: WireTailPolicy;
    }
  | { type: 'setDriveMakeup'; enabled: boolean }
  | { type: 'midiLearn'; action: 'begin' | 'cancel' | 'unlearn' | 'remap'; param_id?: number; cc?: number }
  | { type: 'setPhysicalMidiSource'; source: PhysicalMidiSource }
  | {
      type: 'musicControl';
      op:
        | 'start'
        | 'startGrid'
        | 'seek'
        | 'stop'
        | 'setPattern'
        | 'setPatternLen'
        | 'setStep'
        | 'setTempo'
        | 'setSwing'
        | 'setArp'
        | 'arpKey';
      pattern?: number;
      pattern_len?: number;
      step_index?: number;
      step?: UiStep;
      tempo_milli?: number;
      swing?: number;
      arp_enabled?: boolean;
      arp_order?: UiArpOrder;
      arp_octaves?: number;
      arp_latch?: boolean;
      arp_seed?: number;
      grid_step?: number;
      frame?: number;
    }
  | { type: 'analyzeFilter'; sample_rate?: number }
  | {
      type: 'hostControl';
      op: string;
      enabled?: boolean;
      device?: string;
      sample_rate?: number;
      buffer_frames?: number;
      midi_port?: string;
    };

export interface UiCommandReply {
  ok: boolean;
  status?: WireStatus | null;
  message: string;
  swapId?: number;
  presetJson?: string;
  filterAnalysis?: FilterResponseReport;
  data?: Record<string, unknown>;
}

export function isPointInDiamond(x: number, y: number): boolean {
  return Number.isFinite(x) && Number.isFinite(y) && Math.abs(x) + Math.abs(y) <= 1.00001;
}

/** Projects any Cartesian `(x, y)` cleanly onto the Vector4 diamond domain `|x| + |y| <= 1`. */
export function clampToDiamond(x: number, y: number): UiPoint {
  const fx = Number.isFinite(x) ? x : 0;
  const fy = Number.isFinite(y) ? y : 0;
  const sum = Math.abs(fx) + Math.abs(fy);
  if (sum <= 1) {
    return { x: Number(fx.toFixed(4)), y: Number(fy.toFixed(4)) };
  }
  const scale = 1 / sum;
  return {
    x: Number((fx * scale).toFixed(4)),
    y: Number((fy * scale).toFixed(4)),
  };
}

/** Mirrors `prophet_dsp_core::vector::Point::weights` for instant pointer feedback. */
export function computeVectorWeights(x: number, y: number): [number, number, number, number] {
  const p = clampToDiamond(x, y);
  const c = Math.max(0, (1 - Math.abs(p.x) - Math.abs(p.y)) * 0.25);
  return [
    c + Math.max(0, -p.x), // A (West)
    c + Math.max(0, p.y),  // B (North)
    c + Math.max(0, p.x),  // C (East)
    c + Math.max(0, -p.y), // D (South)
  ];
}

const NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'] as const;

export function formatMidiNote(key: number): string {
  if (!Number.isInteger(key) || key < 0 || key > 127) return '—';
  const octave = Math.floor(key / 12) - 1;
  return `${NOTE_NAMES[key % 12]}${octave}`;
}

export function originShortLabel(origin: number): string {
  switch (origin) {
    case ORIGIN_DAW_HOST:
      return 'DAW';
    case ORIGIN_HARDWARE_MIDI:
      return 'HW';
    case ORIGIN_LOCAL_UI:
      return 'UI';
    case ORIGIN_REMOTE_WS:
      return 'WS';
    case ORIGIN_WEB_MIDI:
      return 'WM';
    case ORIGIN_USB_MIDI:
      return 'USB';
    case ORIGIN_MUSIC_SCHEDULER:
      return 'SEQ';
    default:
      return `O${origin}`;
  }
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null;
const isFiniteNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);

/** Strict structural validator for EnginePort v1 `UiSnapshot` JSON payloads. */
export function decodeUiSnapshot(input: unknown): UiSnapshot | null {
  if (!isObj(input) || input.version !== 1 || typeof input.backend !== 'string' || typeof input.ready !== 'boolean') {
    return null;
  }
  if (!Array.isArray(input.parameters) || input.parameters.length < 83 || !input.parameters.every(isFiniteNum)) {
    return null;
  }
  if (!Array.isArray(input.scope) || !input.scope.every(isFiniteNum)) {
    return null;
  }
  if (!isObj(input.enginePort) || input.enginePort.version !== 1) {
    return null;
  }
  const ep = input.enginePort;
  if (!isFiniteNum(ep.sampleRate) || ep.sampleRate < 8000 || ep.sampleRate > 384000) {
    return null;
  }
  if (!isObj(ep.fx) || !['LegacyRC1', 'DiffuseV1'].includes(String(ep.fx.algorithm))) {
    return null;
  }
  if (!isObj(input.patch) || !TOPOLOGIES.includes(input.patch.topology as UiTopology)) {
    return null;
  }
  if (!isObj(input.kernelTelemetry) || !isFiniteNum(input.kernelTelemetry.activeVoiceMask)) {
    return null;
  }
  if (!isObj(input.midiLearn) || !Array.isArray(input.midiLearn.ccMap)) {
    return null;
  }
  if (!Array.isArray(input.presets)) {
    return null;
  }
  return input as unknown as UiSnapshot;
}

export function decodeUiCommandReply(input: unknown): UiCommandReply {
  if (typeof input === 'string') {
    try {
      return decodeUiCommandReply(JSON.parse(input));
    } catch {
      return { ok: false, status: 'invalid', message: 'Malformed bridge reply JSON' };
    }
  }
  if (!isObj(input) || typeof input.ok !== 'boolean') {
    return { ok: false, status: 'invalid', message: 'Invalid host command reply' };
  }
  return {
    ok: input.ok,
    status: (input.status as WireStatus | null | undefined) ?? null,
    message: typeof input.message === 'string' ? input.message : input.ok ? 'OK' : 'Command failed',
    swapId: typeof input.swapId === 'number' ? input.swapId : undefined,
    presetJson: typeof input.presetJson === 'string' ? input.presetJson : undefined,
    filterAnalysis: isObj(input.filterAnalysis) ? (input.filterAnalysis as unknown as FilterResponseReport) : undefined,
    data: isObj(input.data) ? input.data : undefined,
  };
}
