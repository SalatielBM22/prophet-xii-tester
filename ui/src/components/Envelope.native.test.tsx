import React from 'react';
import { describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import specs from '../param-specs.json';
import { Envelope } from './Envelope';
import type { VoiceEnvelope } from '../protocol/types';
describe('native envelope telemetry consumer', () => {
  it('displays producer stage/progress, never guesses a stage from identical levels', () => {
    const voices: VoiceEnvelope[] = Array.from({ length: 12 }, (_, i) => ({ stage: i === 0 ? 1 : i === 1 ? 4 : 3, elapsedSamples: 24000, level: .5, progress: i === 0 ? .25 : i === 1 ? .8 : 0, timed: i < 2 }));
    render(<Envelope values={specs.map(s => s.min)} amp sampleRate={48000} voices={voices} levels={Array(12).fill(.5)} disabled={false} onBegin={vi.fn()} onEnd={vi.fn()} onChange={vi.fn()} />);
    const running = within(screen.getByRole('list', { name: 'Running amp envelopes' }));
    expect(running.getAllByRole('listitem')).toHaveLength(12);
    expect(running.getAllByRole('listitem')[0]).toHaveTextContent('Attack · 0.500 · 0.500 s · 25%');
    expect(running.getAllByRole('listitem')[1]).toHaveTextContent('Release · 0.500 · 0.500 s · 80%');
    expect(running.getAllByRole('listitem')[2]).toHaveTextContent('Sustain · 0.500 · 0.500 s · indefinite');
  });
  it('rebases elapsed display on current sample rate without invented note generation', () => {
    const voices: VoiceEnvelope[] = Array.from({ length: 12 }, () => ({ stage: 1, elapsedSamples: 24000, level: .2, progress: .1, timed: true }));
    const props = { values: specs.map(s => s.min), amp: false, voices, levels: Array(12).fill(.2), disabled: false, onBegin: vi.fn(), onEnd: vi.fn(), onChange: vi.fn() };
    const { rerender } = render(<Envelope {...props} sampleRate={48000} />);
    expect(screen.getAllByRole('listitem')[0]).toHaveTextContent('0.500 s · 10%');
    rerender(<Envelope {...props} sampleRate={96000} />);
    expect(screen.getAllByRole('listitem')[0]).toHaveTextContent('0.250 s · 10%');
  });
});
