// SPDX-License-Identifier: GPL-3.0-only
import React, { memo, useMemo } from 'react';
import {
  ORIGIN_DAW_HOST,
  ORIGIN_HARDWARE_MIDI,
  ORIGIN_LOCAL_UI,
  ORIGIN_MUSIC_SCHEDULER,
  formatMidiNote,
  type PhysicalMidiSource,
  type WireNote,
  type WireVoice,
} from '../protocol/enginePort';

interface Props {
  disabled: boolean;
  heldLocal: Set<number>;
  engineNotes: WireNote[];
  engineVoices: WireVoice[];
  pitchBend: number;
  physicalSource: PhysicalMidiSource | null;
  onPitchBendBegin: () => void;
  onPitchBendChange: (value: number) => void;
  onPitchBendEnd: () => void;
  onNote: (midiNote: number, on: boolean) => void;
  onConnectWebMidi: () => void;
  onSelectPhysicalSource?: (source: PhysicalMidiSource) => void;
}

const BLACK_SEMIS = new Set([1, 3, 6, 8, 10]);

interface KeyLayout {
  note: number;
  isBlack: boolean;
  whiteIndex: number;
  label: string;
}

function build61KeyLayout(): { whites: KeyLayout[]; blacks: KeyLayout[]; totalWhites: number } {
  const whites: KeyLayout[] = [];
  const blacks: KeyLayout[] = [];
  let whiteCount = 0;
  for (let i = 0; i < 61; i++) {
    const note = 36 + i; // C2..C7
    const semi = note % 12;
    const isBlack = BLACK_SEMIS.has(semi);
    if (!isBlack) {
      whites.push({
        note,
        isBlack: false,
        whiteIndex: whiteCount,
        label: semi === 0 ? `C${Math.floor(note / 12) - 1}` : '',
      });
      whiteCount++;
    } else {
      blacks.push({
        note,
        isBlack: true,
        whiteIndex: whiteCount - 1, // Positioned over boundary between whiteIndex and whiteIndex + 1
        label: '',
      });
    }
  }
  return { whites, blacks, totalWhites: whiteCount };
}

const KEY_LAYOUT = build61KeyLayout();

function classifyKeyState(
  note: number,
  heldLocal: Set<number>,
  notesByKey: Map<number, WireNote[]>,
  releasingKeys: Set<number>,
): { classes: string; title: string; pressed: boolean } {
  const list = notesByKey.get(note) ?? [];
  const hasHw = list.some(n => n.origin === ORIGIN_HARDWARE_MIDI && (n.pressed || n.sustained));
  const hasDaw = list.some(n => n.origin === ORIGIN_DAW_HOST && (n.pressed || n.sustained));
  const hasSeq = list.some(n => n.origin === ORIGIN_MUSIC_SCHEDULER && (n.pressed || n.sustained));
  const hasUi = heldLocal.has(note) || list.some(n => n.origin === ORIGIN_LOCAL_UI && n.pressed);
  const sustainedOnly = list.length > 0 && !list.some(n => n.pressed) && list.some(n => n.sustained);
  const releasing = !hasHw && !hasDaw && !hasSeq && !hasUi && !sustainedOnly && releasingKeys.has(note);

  const cls: string[] = [];
  if (hasHw) cls.push('origin-hw');
  else if (hasDaw) cls.push('origin-daw');
  else if (hasSeq) cls.push('origin-seq');
  else if (hasUi) cls.push('held');
  if (sustainedOnly) cls.push('sustained');
  if (releasing) cls.push('releasing');

  const states: string[] = [];
  if (hasHw) states.push('Hardware MIDI');
  if (hasDaw) states.push('DAW Host');
  if (hasSeq) states.push('Sequencer/Arp');
  if (hasUi) states.push('UI');
  if (sustainedOnly) states.push('Sustained');
  if (releasing) states.push('Releasing');

  return {
    classes: cls.join(' '),
    title: `${formatMidiNote(note)} (MIDI ${note})${states.length ? ` · ${states.join(', ')}` : ''}`,
    pressed: hasHw || hasDaw || hasSeq || hasUi || sustainedOnly,
  };
}

export const Keyboard = memo(function Keyboard({
  disabled,
  heldLocal,
  engineNotes,
  engineVoices,
  pitchBend,
  physicalSource,
  onPitchBendBegin,
  onPitchBendChange,
  onPitchBendEnd,
  onNote,
  onConnectWebMidi,
  onSelectPhysicalSource,
}: Props) {
  const notesByKey = useMemo(() => {
    const map = new Map<number, WireNote[]>();
    for (const n of engineNotes) {
      const list = map.get(n.key);
      if (list) list.push(n);
      else map.set(n.key, [n]);
    }
    return map;
  }, [engineNotes]);

  const releasingKeys = useMemo(() => {
    const set = new Set<number>();
    for (const v of engineVoices) {
      if (v.releasing && !v.gate) set.add(v.key);
    }
    return set;
  }, [engineVoices]);

  const renderKey = (k: KeyLayout) => {
    const st = classifyKeyState(k.note, heldLocal, notesByKey, releasingKeys);
    const leftPct = k.isBlack ? ((k.whiteIndex + 0.68) / KEY_LAYOUT.totalWhites) * 100 : undefined;
    const widthPct = k.isBlack ? (0.64 / KEY_LAYOUT.totalWhites) * 100 : undefined;
    return (
      <button
        key={k.note}
        type="button"
        disabled={disabled}
        title={st.title}
        aria-label={`MIDI note ${k.note}`}
        aria-pressed={st.pressed}
        className={`${k.isBlack ? 'black' : 'white'} ${st.classes}`}
        style={k.isBlack ? { left: `${leftPct}%`, width: `${widthPct}%` } : undefined}
        onPointerDown={e => {
          if (e.button !== 0) return;
          e.preventDefault();
          e.currentTarget.setPointerCapture(e.pointerId);
          onNote(k.note, true);
        }}
        onPointerUp={() => onNote(k.note, false)}
        onPointerCancel={() => onNote(k.note, false)}
        onLostPointerCapture={() => onNote(k.note, false)}
        onKeyDown={e => {
          if ([' ', 'Enter'].includes(e.key)) {
            e.preventDefault();
            if (!e.repeat) onNote(k.note, true);
          }
        }}
        onKeyUp={e => {
          if ([' ', 'Enter'].includes(e.key)) {
            e.preventDefault();
            onNote(k.note, false);
          }
        }}
        onBlur={() => onNote(k.note, false)}
      >
        {k.label}
      </button>
    );
  };

  return (
    <footer className="keyboard-deck">
      <div className="performance">
        <label className="bend-control">
          PITCH BEND
          <input
            disabled={disabled}
            aria-label="Pitch bend"
            type="range"
            min="-7"
            max="7"
            step="0.01"
            value={pitchBend}
            onPointerDown={onPitchBendBegin}
            onChange={e => onPitchBendChange(Number(e.target.value))}
            onPointerUp={onPitchBendEnd}
            onPointerCancel={onPitchBendEnd}
            onBlur={onPitchBendEnd}
          />
          <code>{pitchBend >= 0 ? `+${pitchBend.toFixed(1)}` : pitchBend.toFixed(1)} st</code>
        </label>

        {physicalSource !== null && onSelectPhysicalSource && (
          <div className="midi-owner-group" role="group" aria-label="Physical MIDI owner">
            <span>MIDI OWNER</span>
            <button
              type="button"
              className={`mini-btn ${physicalSource === 'native' ? 'selected' : ''}`}
              disabled={disabled}
              onClick={() => onSelectPhysicalSource('native')}
            >
              NATIVE OS
            </button>
            <button
              type="button"
              className={`mini-btn ${physicalSource === 'browser' ? 'selected' : ''}`}
              disabled={disabled}
              onClick={() => onSelectPhysicalSource('browser')}
            >
              WEB MIDI
            </button>
          </div>
        )}

        <button
          type="button"
          disabled={disabled || physicalSource === 'native'}
          title={
            physicalSource === 'native'
              ? 'Native OS owns physical MIDI exclusively; switch MIDI Owner to Web MIDI to connect browser inputs'
              : 'Connect Web MIDI input'
          }
          onClick={onConnectWebMidi}
        >
          CONNECT WEB MIDI
        </button>

        <div className="key-legend" aria-label="Keyboard origin legend">
          <span>
            <i className="swatch-ui" /> UI
          </span>
          <span>
            <i className="swatch-hw" /> HW MIDI
          </span>
          <span>
            <i className="swatch-seq" /> SEQ/ARP
          </span>
          <span>
            <i className="swatch-sus" /> SUSTAIN
          </span>
        </div>
        <small>C2–C7 · vel 100 · Space/Enter plays focused key</small>
      </div>

      <div className="keyboard-stage" data-testid="performance-keyboard">
        <div className="keyboard-whites">{KEY_LAYOUT.whites.map(renderKey)}</div>
        <div className="keyboard-blacks">{KEY_LAYOUT.blacks.map(renderKey)}</div>
      </div>
    </footer>
  );
});
