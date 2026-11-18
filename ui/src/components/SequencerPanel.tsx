// SPDX-License-Identifier: GPL-3.0-only
import React, { memo, useState } from 'react';
import {
  ARP_ORDERS,
  STEP_KINDS,
  formatMidiNote,
  type UiArpOrder,
  type UiCommand,
  type UiMusicState,
  type UiStep,
  type UiStepKind,
} from '../protocol/enginePort';

type MusicCmd = Extract<UiCommand, { type: 'musicControl' }>;

interface Props {
  music: UiMusicState | null;
  disabled: boolean;
  onMusicControl: (cmd: Omit<MusicCmd, 'type'>) => void;
}

export const SequencerPanel = memo(function SequencerPanel({ music, disabled, onMusicControl }: Props) {
  const [page, setPage] = useState<number>(0);

  if (!music) {
    return (
      <section className="panel seq-panel" aria-label="Sequencer and Arpeggiator">
        <div className="panel-head">
          <h2>SEQUENCER &amp; ARPEGGIATOR</h2>
          <span className="pill muted">HOST LEGACY MODE</span>
        </div>
        <p className="muted-note">
          The sample-accurate 32-step / 64-pattern Rust sequencer and arpeggiator require an EnginePort v1 host. React
          never runs its own audio clock.
        </p>
      </section>
    );
  }

  const bpm = music.tempoMilli / 1000;
  const activeStepInPattern = music.patternLen > 0 ? music.stepIndex % music.patternLen : 0;
  const pageStart = page * 8;
  const visibleSteps = music.currentPattern.steps.slice(pageStart, pageStart + 8);

  const editStep = (globalIndex: number, patch: Partial<UiStep>) => {
    const base = music.currentPattern.steps[globalIndex];
    if (!base) return;
    const next: UiStep = {
      kind: patch.kind ?? base.kind,
      key: Math.min(127, Math.max(0, Math.round(patch.key ?? base.key))),
      velocity: Math.min(127, Math.max(1, Math.round(patch.velocity ?? base.velocity))),
      accent: patch.accent ?? base.accent,
      glide: patch.glide ?? base.glide,
      gate: Math.min(100, Math.max(1, Math.round(patch.gate ?? base.gate))),
      ratchet: Math.min(4, Math.max(1, Math.round(patch.ratchet ?? base.ratchet))),
    };
    onMusicControl({
      op: 'setStep',
      pattern: music.activePattern,
      step_index: globalIndex,
      step: next,
    });
  };

  return (
    <section className="panel seq-panel wide" aria-label="Sequencer and Arpeggiator">
      <div className="panel-head">
        <div className="seq-title-row">
          <h2>SEQUENCER &amp; ARPEGGIATOR</h2>
          <span className={`pill ${music.playing ? 'live' : 'muted'}`}>
            {music.playing ? `RUN · STEP ${activeStepInPattern + 1}/${music.patternLen}` : 'STOPPED'}
          </span>
        </div>
        <div className="seq-transport">
          <button
            type="button"
            className={`seg-btn ${music.playing ? 'selected' : ''}`}
            disabled={disabled}
            onClick={() => onMusicControl({ op: music.playing ? 'stop' : 'start' })}
          >
            {music.playing ? '■ STOP' : '▶ PLAY'}
          </button>
          <label className="compact-field">
            BPM
            <input
              type="number"
              aria-label="Sequencer Tempo BPM"
              min={20}
              max={400}
              step={0.5}
              disabled={disabled}
              value={Number(bpm.toFixed(1))}
              onChange={e => {
                const v = Number(e.target.value);
                if (Number.isFinite(v) && v >= 20 && v <= 400) {
                  onMusicControl({ op: 'setTempo', tempo_milli: Math.round(v * 1000) });
                }
              }}
            />
          </label>
          <label className="compact-field">
            SWING%
            <input
              type="number"
              aria-label="Sequencer Swing Percent"
              min={0}
              max={75}
              step={1}
              disabled={disabled}
              value={music.swing}
              onChange={e => {
                const v = Math.round(Number(e.target.value));
                if (Number.isFinite(v) && v >= 0 && v <= 75) {
                  onMusicControl({ op: 'setSwing', swing: v });
                }
              }}
            />
          </label>
          <label className="compact-field">
            PAT
            <select
              aria-label="Sequencer Pattern Slot"
              disabled={disabled}
              value={music.activePattern}
              onChange={e => onMusicControl({ op: 'setPattern', pattern: Number(e.target.value) })}
            >
              {Array.from({ length: 64 }, (_, i) => (
                <option key={i} value={i}>
                  {String(i + 1).padStart(2, '0')}
                </option>
              ))}
            </select>
          </label>
          <label className="compact-field">
            LEN
            <input
              type="number"
              aria-label="Sequencer Pattern Length"
              min={1}
              max={32}
              step={1}
              disabled={disabled}
              value={music.patternLen}
              onChange={e => {
                const len = Math.round(Number(e.target.value));
                if (Number.isFinite(len) && len >= 1 && len <= 32) {
                  onMusicControl({ op: 'setPatternLen', pattern: music.activePattern, pattern_len: len });
                }
              }}
            />
          </label>
        </div>
      </div>

      {/* Arpeggiator Strip */}
      <div className="arp-strip" role="group" aria-label="Arpeggiator controls">
        <button
          type="button"
          className={`seg-btn ${music.arpEnabled ? 'selected' : ''}`}
          aria-pressed={music.arpEnabled}
          disabled={disabled}
          onClick={() => onMusicControl({ op: 'setArp', arp_enabled: !music.arpEnabled })}
        >
          ARP {music.arpEnabled ? 'ON' : 'OFF'}
        </button>
        <label className="compact-field">
          ORDER
          <select
            aria-label="Arpeggiator Order"
            disabled={disabled}
            value={music.arpOrder}
            onChange={e => onMusicControl({ op: 'setArp', arp_order: e.target.value as UiArpOrder })}
          >
            {ARP_ORDERS.map(o => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
        </label>
        <label className="compact-field">
          OCT
          <select
            aria-label="Arpeggiator Octaves"
            disabled={disabled}
            value={music.arpOctaves}
            onChange={e => onMusicControl({ op: 'setArp', arp_octaves: Number(e.target.value) })}
          >
            {[1, 2, 3, 4].map(o => (
              <option key={o} value={o}>
                {o}
              </option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className={`mini-btn ${music.arpLatch ? 'selected' : ''}`}
          aria-pressed={music.arpLatch}
          disabled={disabled}
          onClick={() => onMusicControl({ op: 'setArp', arp_latch: !music.arpLatch })}
        >
          LATCH
        </button>
        <label className="compact-field">
          SEED
          <input
            type="number"
            aria-label="Arpeggiator Seed"
            min={0}
            max={65535}
            disabled={disabled}
            value={music.arpSeed}
            onChange={e => {
              const seed = Math.max(0, Math.round(Number(e.target.value)));
              if (Number.isFinite(seed)) onMusicControl({ op: 'setArp', arp_seed: seed });
            }}
          />
        </label>

        {/* 4 Pages x 8 Steps */}
        <div className="page-tabs" role="tablist" aria-label="Sequencer step pages">
          {[0, 1, 2, 3].map(p => (
            <button
              key={p}
              type="button"
              role="tab"
              aria-selected={page === p}
              className={`mini-btn ${page === p ? 'selected' : ''}`}
              onClick={() => setPage(p)}
            >
              {`${p * 8 + 1}–${p * 8 + 8}`}
            </button>
          ))}
        </div>
      </div>

      {/* 8-Step Page Grid */}
      <div className="step-grid" data-testid="sequencer-step-grid">
        {visibleSteps.map((step, localIdx) => {
          const idx = pageStart + localIdx;
          const outOfLen = idx >= music.patternLen;
          const isCurrent = music.playing && activeStepInPattern === idx;
          return (
            <div
              key={idx}
              className={`step-card ${step.kind} ${isCurrent ? 'playing' : ''} ${outOfLen ? 'beyond-len' : ''}`}
              data-step-index={idx}
            >
              <div className="step-head">
                <strong>{String(idx + 1).padStart(2, '0')}</strong>
                <button
                  type="button"
                  className={`step-kind-btn kind-${step.kind}`}
                  disabled={disabled}
                  aria-label={`Step ${idx + 1} kind`}
                  onClick={() => {
                    const nextKind = STEP_KINDS[(STEP_KINDS.indexOf(step.kind) + 1) % STEP_KINDS.length] as UiStepKind;
                    editStep(idx, { kind: nextKind });
                  }}
                >
                  {step.kind.toUpperCase()}
                </button>
              </div>
              <label className="step-row">
                <span>{formatMidiNote(step.key)}</span>
                <input
                  type="number"
                  aria-label={`Step ${idx + 1} MIDI key`}
                  min={0}
                  max={127}
                  disabled={disabled || step.kind === 'rest'}
                  value={step.key}
                  onChange={e => editStep(idx, { key: Number(e.target.value) })}
                />
              </label>
              <div className="step-pair">
                <label>
                  V
                  <input
                    type="number"
                    aria-label={`Step ${idx + 1} velocity`}
                    min={1}
                    max={127}
                    disabled={disabled || step.kind !== 'note'}
                    value={step.velocity}
                    onChange={e => editStep(idx, { velocity: Number(e.target.value) })}
                  />
                </label>
                <label>
                  G%
                  <input
                    type="number"
                    aria-label={`Step ${idx + 1} gate`}
                    min={1}
                    max={100}
                    disabled={disabled || step.kind === 'rest'}
                    value={step.gate}
                    onChange={e => editStep(idx, { gate: Number(e.target.value) })}
                  />
                </label>
              </div>
              <div className="step-flags">
                <button
                  type="button"
                  className={`flag-btn ${step.accent ? 'on' : ''}`}
                  disabled={disabled || step.kind !== 'note'}
                  aria-pressed={step.accent}
                  title="Accent"
                  onClick={() => editStep(idx, { accent: !step.accent })}
                >
                  ACC
                </button>
                <button
                  type="button"
                  className={`flag-btn ${step.glide ? 'on' : ''}`}
                  disabled={disabled || step.kind !== 'note'}
                  aria-pressed={step.glide}
                  title="Glide"
                  onClick={() => editStep(idx, { glide: !step.glide })}
                >
                  GLD
                </button>
                <button
                  type="button"
                  className={`flag-btn ${step.ratchet > 1 ? 'on' : ''}`}
                  disabled={disabled || step.kind !== 'note'}
                  title="Ratchet subdivisions (1-4)"
                  aria-label={`Step ${idx + 1} ratchet`}
                  onClick={() => editStep(idx, { ratchet: (step.ratchet % 4) + 1 })}
                >
                  {step.ratchet}×
                </button>
              </div>
            </div>
          );
        })}
      </div>
      <small className="vector-caption">
        Sample-accurate Rust NoteScheduler authority · React edits pattern bank and visualizes clock only.
      </small>
    </section>
  );
});
