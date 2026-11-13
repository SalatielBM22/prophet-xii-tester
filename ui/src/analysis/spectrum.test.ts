import { describe, expect, it } from 'vitest';
import { decodeFilter, decodeSpectrum, filterPath, spectrumPoints, type SpectrumFrame } from './spectrum';

const frame = (sampleRate = 48000): SpectrumFrame => ({ sequence: 1, firstSample: 0, sampleRate, fftSize: 4096, hopSize: 1024, window: 'hann', coherentGain: .5, enbwBins: 1.5, normalization: 'single-sided-amplitude-dbfs', dbfs: Array(2049).fill(-60) });
describe('spectral transport and axes', () => {
  it('rejects truncated, unknown normalization and nonfinite spectra', () => {
    expect(decodeSpectrum(frame())).not.toBeNull();
    expect(decodeSpectrum({ ...frame(), dbfs: [0] })).toBeNull();
    expect(decodeSpectrum({ ...frame(), normalization: 'power' })).toBeNull();
    expect(decodeSpectrum({ ...frame(), dbfs: Array(2049).fill(NaN) })).toBeNull();
  });
  it.each([44100, 48000, 96000])('places bins on a positive log axis at %i Hz', sampleRate => {
    const points = spectrumPoints(frame(sampleRate), 900);
    expect(points[0]).toBeGreaterThanOrEqual(0);
    for (let i = 2; i < points.length; i += 2) expect(points[i]).toBeGreaterThan(points[i - 2]);
    expect(points[points.length - 2]).toBeLessThanOrEqual(900);
  });
});
describe('transfer display independent from live spectrum', () => {
  it('unwraps legacy phase and breaks paths at low magnitude without joining across a gap', () => {
    const f = decodeFilter({ hz: [20, 200, 2000, 10000, 20000], db: [0, 0, -120, -20, -40], phase: [-3, 3, .2, -3, 3] }, true)!;
    expect(f.sampleRate).toBe(48000);
    expect(Math.abs(f.phase[1]! - f.phase[0]!)).toBeLessThan(1);
    expect(f.phase[2]).toBeNull();
    expect(filterPath(f, true).match(/M/g)).toHaveLength(2);
  });
  it('requires the actual sample rate on a v2 response and preserves pre-unwrapped phase', () => {
    const f = { hz: [20, 200, 20000], db: [0, -10, -120], phase: [0, -7.5, null] };
    expect(decodeFilter(f, false)).toBeNull();
    const parsed = decodeFilter({
      ...f,
      sampleRate: 96000,
      fftSize: 16384,
      impulseSamples: 8192,
      tailConverged: true,
      tailRelativeRms: 1e-5,
      binHz: 96000 / 16384,
    }, false)!;
    expect(parsed.sampleRate).toBe(96000);
    expect(parsed.legacy).toBe(false);
    expect(parsed.fftSize).toBe(16384);
    expect(parsed.impulseSamples).toBe(8192);
    expect(parsed.tailConverged).toBe(true);
    expect(parsed.phase[1]).toBe(-7.5);
    expect(parsed.phase[2]).toBeNull();
    expect(decodeFilter({ ...f, sampleRate: 96000, fftSize: 1024 }, false)).toBeNull();
    expect(decodeFilter({ ...f, sampleRate: 96000, tailConverged: 'yes' }, false)).toBeNull();
  });
});
