// SPDX-License-Identifier: GPL-3.0-only
import React, { memo, useRef, useState } from 'react';
import type { ParamSpec } from '../protocol/types';
import { denormalized, formatValue, normalized, parseValue, unit } from './parameters';

interface Props {
  spec: ParamSpec;
  value: number;
  disabled: boolean;
  reason: string;
  resetValue?: number;
  learnedCc?: number | null;
  onChange: (id: number, value: number) => void;
  onBegin: (id: number) => void;
  onEnd: () => void;
  onLearn: (id: number) => void;
}

export const Knob = memo(function Knob({
  spec: s,
  value,
  disabled,
  reason,
  resetValue,
  learnedCc,
  onChange,
  onBegin,
  onEnd,
  onLearn,
}: Props) {
  const drag = useRef<{ y: number; n: number } | null>(null);
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState('');
  const [error, setError] = useState('');

  const n = normalized(s, value);
  const toggle = s.integer && s.min === 0 && s.max === 1;
  const resetHint =
    resetValue === undefined
      ? 'Factory default reset awaits a confirmed factory preset reference'
      : 'Double-click, Ctrl+click, or Alt+Enter to reset to factory preset value';
  const reset = () => {
    if (!disabled && resetValue !== undefined) onChange(s.id, resetValue);
  };
  const end = () => {
    drag.current = null;
    onEnd();
  };
  const commit = () => {
    try {
      onChange(s.id, parseValue(s, text));
      setEditing(false);
      setError('');
    } catch (cause) {
      setError(String(cause instanceof Error ? cause.message : cause));
    }
  };

  const shortLabel = s.name.replace(/^(OscA|OscB|Osc A|Osc B|Reverb|Chorus|Filter|Filt|Amp|Lfo|Poly Mod|Unison) /, '');

  return (
    <div
      className={`control ${reason ? 'conditional' : ''}`}
      data-param-key={s.key}
      title={`${s.name} · ${reason || resetHint}${learnedCc != null ? ` · CC ${learnedCc}` : ''}`}
      onContextMenu={e => {
        e.preventDefault();
        onLearn(s.id);
      }}
    >
      <div className="knob-wrap">
        <button
          type="button"
          disabled={disabled}
          aria-label={s.name}
          aria-describedby={reason ? `hint-${s.id}` : undefined}
          role={toggle ? 'switch' : 'slider'}
          aria-checked={toggle ? value >= 0.5 : undefined}
          aria-valuenow={toggle ? undefined : value}
          aria-valuetext={formatValue(s, value)}
          aria-valuemin={toggle ? undefined : s.min}
          aria-valuemax={toggle ? undefined : s.max}
          className={toggle ? `switch ${value >= 0.5 ? 'lit' : ''}` : 'knob'}
          onClick={e => {
            if (e.altKey || e.ctrlKey || e.metaKey) {
              e.preventDefault();
              reset();
              return;
            }
            if (toggle) onChange(s.id, value >= 0.5 ? 0 : 1);
          }}
          onDoubleClick={reset}
          onKeyDown={e => {
            if (e.altKey && e.key === 'Enter') {
              e.preventDefault();
              reset();
              return;
            }
            if (e.key === 'Home' || e.key === 'End') {
              e.preventDefault();
              onBegin(s.id);
              onChange(s.id, e.key === 'Home' ? s.min : s.max);
            }
            if (['ArrowUp', 'ArrowRight', 'ArrowDown', 'ArrowLeft'].includes(e.key)) {
              e.preventDefault();
              onBegin(s.id);
              const direction = ['ArrowUp', 'ArrowRight'].includes(e.key) ? 1 : -1;
              if (s.key === 'OversamplingMode') {
                const choices = [1, 2, 4];
                const index = choices.indexOf(value);
                onChange(s.id, choices[Math.min(2, Math.max(0, index + direction))]);
              } else {
                onChange(s.id, s.integer ? value + direction : denormalized(s, n + direction * (e.shiftKey ? 0.001 : 0.01)));
              }
            }
          }}
          onKeyUp={e => {
            if (['Home', 'End', 'ArrowUp', 'ArrowRight', 'ArrowDown', 'ArrowLeft'].includes(e.key)) onEnd();
          }}
          onBlur={end}
          onPointerDown={e => {
            if (toggle || e.button !== 0 || e.altKey || e.ctrlKey || e.metaKey) return;
            e.currentTarget.setPointerCapture(e.pointerId);
            drag.current = { y: e.clientY, n };
            onBegin(s.id);
          }}
          onPointerMove={e => {
            if (drag.current) {
              const sensitivity = e.shiftKey ? 480 : 160;
              onChange(s.id, denormalized(s, drag.current.n + (drag.current.y - e.clientY) / sensitivity));
            }
          }}
          onPointerUp={end}
          onPointerCancel={end}
          onLostPointerCapture={end}
        >
          {toggle ? (
            value >= 0.5 ? 'ON' : 'OFF'
          ) : (
            <span style={{ transform: `rotate(${-135 + 270 * n}deg)` }}>
              <i />
            </span>
          )}
        </button>
        {learnedCc != null && <span className="cc-pill">CC{learnedCc}</span>}
      </div>

      <label>{shortLabel}</label>

      {editing ? (
        <div className="numeric-editor">
          <input
            aria-label={`${s.name} value`}
            aria-invalid={Boolean(error)}
            type="text"
            inputMode="decimal"
            value={text}
            autoFocus
            onFocus={e => e.currentTarget.select()}
            onChange={e => {
              setText(e.target.value);
              setError('');
            }}
            onKeyDown={e => {
              if (e.key === 'Enter') {
                e.preventDefault();
                commit();
              }
              if (e.key === 'Escape') {
                e.preventDefault();
                setEditing(false);
                setError('');
              }
            }}
            onBlur={() => {
              setEditing(false);
              setError('');
            }}
          />
          {unit(s) && <small>{unit(s)}</small>}
          {error && <span role="alert">{error}</span>}
        </div>
      ) : (
        <button
          type="button"
          className="readout"
          aria-label={`Edit ${s.name}`}
          disabled={disabled}
          onClick={() => {
            setText(String(value));
            setEditing(true);
            setError('');
          }}
        >
          {formatValue(s, value)}
        </button>
      )}

      {reason && (
        <small id={`hint-${s.id}`} className="control-hint">
          {reason}
        </small>
      )}
    </div>
  );
});
