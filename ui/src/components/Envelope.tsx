import React, { useMemo, useRef } from 'react';
import specs from '../param-specs.json';
import { envelopeStageNames } from '../protocol/types';
import type { VoiceEnvelope } from '../protocol/types';
import { paramId, normalize, denormalized, normalized } from '../controls/parameters';
import { programmedEnvelope } from './envelopeModel';
interface Props {
  values: readonly number[]; amp: boolean; sampleRate: number; voices?: VoiceEnvelope[]; levels: number[]; disabled: boolean; activeMask?: number;
  onChange: (id: number, value: number) => void; onBegin: (id: number) => void; onEnd: () => void;
}
export function Envelope({ values, amp, sampleRate, voices, levels, disabled, activeMask = 0, onChange, onBegin, onEnd }: Props) {
  const prefix = amp ? 'Amp' : 'Filt', kind = amp ? 'amp' : 'filter';
  const attack = values[paramId(prefix + 'Attack')], decay = values[paramId(prefix + 'Decay')], sustain = values[paramId(prefix + 'Sustain')], release = values[paramId(prefix + 'Release')];
  const model = useMemo(() => programmedEnvelope({ attack, decay, sustain, release }, sampleRate), [attack, decay, sustain, release, sampleRate]);
  const x = (time: number) => 8 + time / model.total * 274, y = (level: number) => 76 - level * 65;
  const handles = [
    { name: 'Attack', time: model.attack, level: 1 },
    { name: 'Decay', time: model.attack + model.decay, level: sustain },
    { name: 'Sustain', time: model.attack + model.decay + model.hold / 2, level: sustain },
    { name: 'Release', time: model.total, level: 0 },
  ];
  const drag = useRef<{ id: number; x: number; y: number; value: number; scale: number } | null>(null);
  const end = () => { drag.current = null; onEnd(); };
  return <div className="envelope-view"><small>PROGRAMMED {amp ? 'AMP' : 'FILTER'} ADSR · NOMINAL RC · {sampleRate} Hz</small>
    <svg className="envelope" aria-label={`Programmed ${kind} envelope`} data-testid={`envelope-${kind}`} viewBox="0 0 290 100"
      onPointerMove={e => {
        const d = drag.current; if (!d || disabled) return;
        const b = e.currentTarget.getBoundingClientRect();
        const value = specs[d.id].key.endsWith('Sustain') ? d.value - (e.clientY - d.y) / b.height * 100 / 65 : d.value + (e.clientX - d.x) / b.width * d.scale;
        onChange(d.id, normalize(specs[d.id], value));
      }} onPointerUp={end} onPointerCancel={end} onLostPointerCapture={end}>
      <path d={'M' + model.points.map(p => `${x(p.time)},${y(p.level)}`).join(' L')} fill="none" stroke="#e9b95e" strokeWidth="2" />
      <path d={`M${x(model.attack + model.decay)},84 L${x(model.attack + model.decay + model.hold)},84`} stroke="#80cdae" strokeDasharray="3 2" />
      <text x="8" y="97">0 s</text><text x="210" y="97">{model.total.toFixed(3)} s</text>
      {handles.map(h => { const id = paramId(prefix + h.name), s = specs[id]; return <circle key={h.name} cx={x(h.time)} cy={y(h.level)} r="4" fill="#e9b95e" tabIndex={disabled ? -1 : 0} role="slider" aria-label={`${prefix} envelope ${h.name}`} aria-valuenow={values[id]} aria-valuemin={s.min} aria-valuemax={s.max}
        onPointerDown={e => { if (disabled || e.button !== 0) return; e.preventDefault(); e.currentTarget.setPointerCapture(e.pointerId); drag.current = { id, x: e.clientX, y: e.clientY, value: values[id], scale: model.total }; onBegin(id); }}
        onKeyDown={e => { if (!disabled && ['ArrowUp', 'ArrowRight', 'ArrowDown', 'ArrowLeft'].includes(e.key)) { e.preventDefault(); onBegin(id); onChange(id, denormalized(s, normalized(s, values[id]) + (['ArrowUp', 'ArrowRight'].includes(e.key) ? .01 : -.01))); } }}
        onKeyUp={end} onBlur={end} />; })}
    </svg>
    <small>Seconds on x-axis; Sustain held 0.25 s for illustration. Per-voice RC dispersion and retriggers require live telemetry.</small>
    {voices ? <><small>Activity follows the kernel voice mask; inactive filter stages may be retained.</small><ol className="voice-envelopes" aria-label={`Running ${kind} envelopes`}>{voices.map((v, i) => <li key={i}>V{i + 1}: {envelopeStageNames[v.stage]} · {v.level.toFixed(3)} · {(v.elapsedSamples / sampleRate).toFixed(3)} s{v.timed ? ` · ${(v.progress * 100).toFixed(0)}%` : ' · indefinite'} · {activeMask & (1 << i) ? 'active' : 'inactive'}</li>)}</ol></>
      : <div className="envelope-unavailable"><small>Envelope stages unavailable on this legacy host. Levels below are instantaneous amplitudes, not stage/progress.</small><div className="voice-levels">{Array.from({ length: 12 }, (_, i) => <meter key={i} aria-label={`${prefix} voice ${i + 1} level`} value={levels[i] ?? 0} min="0" max="1" />)}</div></div>}
  </div>;
}
