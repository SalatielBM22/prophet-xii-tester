import { describe, expect, it } from 'vitest';
import { programmedEnvelope } from './envelopeModel';
describe('programmed kernel RC envelope', () => {
  it.each([48000, 96000])('uses actual attack and threshold-derived release timing at %i Hz', sr => {
    const short = programmedEnvelope({ attack: .001, decay: .55, sustain: .5, release: .01 }, sr);
    const long = programmedEnvelope({ attack: 1, decay: .55, sustain: .5, release: 5 }, sr);
    expect(short.attack).toBeCloseTo(.001, 4); expect(long.attack).toBeCloseTo(1, 4);
    expect(long.release / short.release).toBeGreaterThan(490);
    expect(long.points[long.points.length - 1].level).toBe(0);
    expect(long.points.every(p => Number.isFinite(p.time) && p.level >= 0 && p.level <= 1)).toBe(true);
    expect(short.points[64].level).toBe(1);
    expect(long.points[64].time / long.total).not.toBeCloseTo(short.points[64].time / short.total);
  });
  it.each([0, .5, 1])('reaches sustain %f at the kernel undershoot threshold', sustain => {
    const result = programmedEnvelope({ attack: .01, decay: .5, sustain, release: .4 }, 48000);
    expect(result.points[128].level).toBeCloseTo(sustain, 5);
    expect(result.points[129].level).toBe(sustain);
    expect(result.points.every((p, i, arr) => i === 0 || p.time >= arr[i - 1].time)).toBe(true);
  });
});
