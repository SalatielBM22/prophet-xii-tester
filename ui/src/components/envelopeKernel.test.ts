import { describe, expect, it } from 'vitest';
import oracle from '../../tests/fixtures/envelope-kernel.json';
import { programmedEnvelope } from './envelopeModel';

describe('programmed envelope against the actual Rust baseline kernel', () => {
  it.each(oracle)('tracks threshold times at $sampleRate Hz, A=$attack S=$sustain R=$release', entry => {
    const model = programmedEnvelope(entry, entry.sampleRate);
    for (const stage of ['attack', 'decay', 'release'] as const) {
      const actualSamples = entry[`${stage}Samples`];
      // f32 recurrence in Rust and ideal f64 exponentials differ slightly.
      // Permit one sample or 0.3% relative error; this is nominal UI timing,
      // not voice-dispersion/retrigger tracking or physical IC calibration.
      expect(Math.abs(model[stage] * entry.sampleRate - actualSamples)).toBeLessThanOrEqual(Math.max(1, actualSamples * .003));
    }
    expect(model.points.every(p => Number.isFinite(p.time) && p.level >= 0 && p.level <= 1)).toBe(true);
  });
});
