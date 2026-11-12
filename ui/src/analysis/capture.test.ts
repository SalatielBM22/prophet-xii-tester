import { describe, expect, it } from 'vitest';
import { decodeCapture, defaultAnalysis, DiscontinuityCounter, validConfig } from './capture';
describe('actual Agent B PCM transport', () => {
  const block = { sequence: 1, firstFrame: 64, sampleRate: 48000, samples: Array(64).fill(0) };
  it('accepts bounded continuous chunks and final short chunks, never scope snapshots', () => {
    expect(decodeCapture({ v: 1, blocks: [block, { ...block, sequence: 2, firstFrame: 128, samples: [0] }] })).toHaveLength(2);
    expect(decodeCapture({ v: 1, scope: Array(1024).fill(0) })).toBeNull();
  });
  it('rejects oversized/nonfinite/fractional packets as a complete message', () => {
    for (const changed of [{ sequence: .5 }, { samples: Array(65).fill(0) }, { samples: [NaN] }, { sampleRate: Infinity }, { firstFrame: -1 }]) expect(decodeCapture({ v: 1, blocks: [block, { ...block, ...changed }] })).toBeNull();
    expect(decodeCapture({ v: 1, blocks: Array(257).fill(block) })).toBeNull();
  });
  it('validates all analysis configurations before recreating a plan', () => {
    expect(validConfig(defaultAnalysis)).toBe(true);
    expect(validConfig({ ...defaultAnalysis, averageAlpha: 0 })).toBe(false);
    expect(validConfig({ ...defaultAnalysis, kaiserBeta: NaN })).toBe(false);
    expect(validConfig({ ...defaultAnalysis, peakHoldFrames: -.1 })).toBe(false);
  });
  it('keeps total discontinuities strictly monotonic across analyzer resets', () => {
    const counter = new DiscontinuityCounter();
    expect(counter.total(0)).toBe(0);
    expect(counter.total(2)).toBe(2);
    counter.recordReset(2);
    expect(counter.total(0)).toBe(3);
    counter.recordReset(0);
    expect(counter.total(1)).toBe(5);
  });
});
