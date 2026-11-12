import specs from '../param-specs.json';
import type { ParamSpec } from '../protocol/types';
export const paramId = (key: string) => {
  const spec = specs.find(s => s.key === key);
  if (!spec) throw Error(`Unknown parameter ${key}`);
  return spec.id;
};
export function normalize(spec: ParamSpec, value: number) {
  if (!Number.isFinite(value)) throw Error('Enter a finite number');
  value = Math.min(spec.max, Math.max(spec.min, value));
  if (spec.key === 'OversamplingMode') return value < 1.5 ? 1 : value < 3 ? 2 : 4;
  return spec.integer ? Math.round(value) : value;
}
export function parseValue(spec: ParamSpec, text: string) {
  if (!text.trim()) throw Error('Enter a number');
  const value = Number(text);
  if (!Number.isFinite(value) || value < spec.min || value > spec.max) throw Error(`Enter a value from ${spec.min} to ${spec.max}`);
  if (spec.integer && !Number.isInteger(value)) throw Error('Enter a whole number');
  if (spec.key === 'OversamplingMode' && ![1, 2, 4].includes(value)) throw Error('Choose 1, 2 or 4');
  return value;
}
export function normalized(spec: ParamSpec, value: number) {
  const fraction = spec.logarithmic ? Math.log(Math.max(spec.min, value) / spec.min) / Math.log(spec.max / spec.min) : (value - spec.min) / (spec.max - spec.min);
  return Math.min(1, Math.max(0, fraction));
}
export function denormalized(spec: ParamSpec, value: number) {
  value = Math.min(1, Math.max(0, value));
  return normalize(spec, spec.logarithmic ? spec.min * Math.pow(spec.max / spec.min, value) : spec.min + value * (spec.max - spec.min));
}
export function unit(spec: ParamSpec) {
  if (['FilterCutoff', 'ReverbHighCut', 'ReverbLowCut', 'LfoFreq', 'ReverbModSpeed'].includes(spec.key)) return 'Hz';
  if (spec.key === 'ReverbPreDelay') return 'ms';
  if (/Attack|Decay|Release|GlideTime/.test(spec.key)) return 's';
  if (spec.key === 'ReverbTempoBpm') return 'BPM';
  if (spec.key === 'ReverbPreDelayBeats') return 'beats';
  if (/Fine|UnisonDetune/.test(spec.key)) return 'cents';
  if (/Semi|PitchBend/.test(spec.key)) return 'semitones';
  return '';
}
export function formatValue(spec: ParamSpec, value: number) {
  const enums: Record<string, string[]> = { ChorusMode: ['OFF', 'I', 'II', 'I+II'], ReverbEnsembleMode: ['Off', 'mp', 'forte'], NotePriority: ['Last', 'Low', 'High'] };
  if (enums[spec.key]) return enums[spec.key][Math.round(value)];
  if (spec.key === 'OversamplingMode') return `${value}×`;
  return `${Number(value.toPrecision(6))}${unit(spec) ? ` ${unit(spec)}` : ''}`;
}
export function inactiveReason(spec: ParamSpec, values: readonly number[]) {
  const v = (key: string) => values[paramId(key)];
  if (spec.key === 'OscAPw' && !v('OscAPulse')) return 'Enable OscA Pulse to use Pulse Width';
  if (spec.key === 'OscBPw' && !v('OscBPulse')) return 'Enable OscB Pulse to use Pulse Width';
  if (spec.key === 'ReverbPreDelay' && v('ReverbPreDelaySync')) return 'Pre-delay uses BPM and Beats while Sync is on';
  if (['ReverbTempoBpm', 'ReverbPreDelayBeats'].includes(spec.key) && !v('ReverbPreDelaySync')) return 'Enable Reverb Pre Delay Sync';
  if (spec.key === 'ReverbEnsembleBlend' && !v('ReverbEnsembleMode')) return 'Enable Ensemble mp or forte; Off disables Ensemble only';
  if (spec.key === 'ReverbVowelMorph' && (v('ReverbVowelMix') === 0 || v('ReverbFreeze'))) return 'Vowel Morph requires Vowel Mix above zero and Freeze off';
  if (['ShimmerFifth', 'ShimmerSemitones'].includes(spec.key) && !v('ReverbShimmer')) return 'Increase Reverb Shimmer to use this control';
  if (spec.key === 'ReverbModSpeed' && !v('ReverbModDepth')) return 'Increase Reverb Mod Depth to hear modulation speed';
  return '';
}
