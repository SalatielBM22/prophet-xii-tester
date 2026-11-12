import React, { memo, useRef, useState } from 'react';
import type { ParamSpec } from '../protocol/types';
import { denormalized, formatValue, normalized, parseValue, unit } from './parameters';
interface Props {
  spec: ParamSpec; value: number; disabled: boolean; reason: string; resetValue?: number;
  onChange: (id: number, value: number) => void; onBegin: (id: number) => void; onEnd: () => void; onLearn: (id: number) => void;
}
export const Knob = memo(function Knob({ spec: s, value, disabled, reason, resetValue, onChange, onBegin, onEnd, onLearn }: Props) {
  const drag = useRef<{ y: number; n: number } | null>(null);
  const [editing, setEditing] = useState(false), [text, setText] = useState(''), [error, setError] = useState('');
  const n = normalized(s, value), toggle = s.integer && s.min === 0 && s.max === 1;
  const resetReason = resetValue === undefined ? 'Reset unavailable: host does not provide a confirmed factory preset reference' : 'Double-click or Alt+Enter to reset to loaded factory preset';
  const reset = () => { if (!disabled && resetValue !== undefined) onChange(s.id, resetValue); };
  const end = () => { drag.current = null; onEnd(); };
  const commit = () => {
    try { onChange(s.id, parseValue(s, text)); setEditing(false); setError(''); }
    catch (cause) { setError(String(cause instanceof Error ? cause.message : cause)); }
  };
  return <div className={`control ${reason ? 'conditional' : ''}`} data-param-key={s.key} title={`${s.name} · ${reason || resetReason}`} onContextMenu={e => { e.preventDefault(); onLearn(s.id); }}>
    <button disabled={disabled} aria-label={s.name} aria-describedby={`hint-${s.id}`} role={toggle ? 'switch' : 'slider'} aria-checked={toggle ? value >= .5 : undefined}
      aria-valuenow={toggle ? undefined : value} aria-valuetext={formatValue(s, value)} aria-valuemin={toggle ? undefined : s.min} aria-valuemax={toggle ? undefined : s.max}
      className={toggle ? `switch ${value >= .5 ? 'lit' : ''}` : 'knob'}
      onClick={e => { if (e.altKey) { reset(); return; } if (toggle) onChange(s.id, value >= .5 ? 0 : 1); }}
      onDoubleClick={reset}
      onKeyDown={e => {
        if (e.altKey && e.key === 'Enter') { e.preventDefault(); reset(); return; }
        if (e.key === 'Home' || e.key === 'End') { e.preventDefault(); onBegin(s.id); onChange(s.id, e.key === 'Home' ? s.min : s.max); }
        if (['ArrowUp', 'ArrowRight', 'ArrowDown', 'ArrowLeft'].includes(e.key)) {
          e.preventDefault(); onBegin(s.id);
          const direction = ['ArrowUp', 'ArrowRight'].includes(e.key) ? 1 : -1;
          if (s.key === 'OversamplingMode') { const choices = [1, 2, 4], index = choices.indexOf(value); onChange(s.id, choices[Math.min(2, Math.max(0, index + direction))]); }
          else onChange(s.id, s.integer ? value + direction : denormalized(s, n + direction * (e.shiftKey ? .001 : .01)));
        }
      }}
      onKeyUp={e => { if (['Home', 'End', 'ArrowUp', 'ArrowRight', 'ArrowDown', 'ArrowLeft'].includes(e.key)) onEnd(); }}
      onBlur={end}
      onPointerDown={e => {
        if (toggle || e.button !== 0 || e.altKey) return;
        e.currentTarget.setPointerCapture(e.pointerId); drag.current = { y: e.clientY, n }; onBegin(s.id);
      }}
      onPointerMove={e => { if (drag.current) onChange(s.id, denormalized(s, drag.current.n + (drag.current.y - e.clientY) / 160)); }}
      onPointerUp={end} onPointerCancel={end} onLostPointerCapture={end}>
      {toggle ? (value >= .5 ? 'ON' : 'OFF') : <span style={{ transform: `rotate(${-135 + 270 * n}deg)` }}><i /></span>}
    </button>
    <label>{s.name.replace(/^(Osc A|Osc B|Reverb|Chorus|Filter|Filt|Amp|Lfo) /, '')}</label>
    {editing ? <div className="numeric-editor"><input aria-label={`${s.name} value`} aria-invalid={Boolean(error)} type="text" inputMode="decimal" value={text} autoFocus
      onFocus={e => e.currentTarget.select()} onChange={e => { setText(e.target.value); setError(''); }}
      onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); commit(); } if (e.key === 'Escape') { e.preventDefault(); setEditing(false); setError(''); } }}
      onBlur={() => { setEditing(false); setError(''); }} /><small>{unit(s)}</small>{error && <span role="alert">{error}</span>}</div>
      : <button className="readout" aria-label={`Edit ${s.name}`} disabled={disabled} onClick={() => { setText(String(value)); setEditing(true); setError(''); }}>{formatValue(s, value)}</button>}
    {resetValue !== undefined && <button className="reset-control" aria-label={`Reset ${s.name} to factory preset`} disabled={disabled} onClick={reset}>RESET</button>}
    <small id={`hint-${s.id}`} className="control-hint">{reason || (resetValue === undefined ? 'Factory reset unavailable' : '')}</small>
  </div>;
});
