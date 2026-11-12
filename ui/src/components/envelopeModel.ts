/** Nominal baseline kernel RC model; not a device-emulation claim or voice tracker. */
export interface Adsr { attack: number; decay: number; sustain: number; release: number }
export interface EnvelopePoint { time: number; level: number }
export function programmedEnvelope(adsr: Adsr, sampleRate: number, hold = .25) {
  const sr = Math.max(8000, sampleRate), sustain = Math.max(0, Math.min(1, adsr.sustain)), eps = .0015;
  const step = (seconds: number, exponent: number) => exponent / Math.max(1, Math.max(.001, Math.min(25, seconds)) * sr);
  const att = step(adsr.attack, 1.7917595), dec = step(adsr.decay, 4.8), rel = step(adsr.release, 4.8);
  const attackSamples = Math.max(1, Math.ceil(Math.log(6) / att));
  const decaySamples = Math.max(1, Math.ceil(Math.log((1 - sustain + eps) / eps) / dec));
  const releaseSamples = Math.max(1, Math.ceil(Math.log((sustain + eps) / (.0001 + eps)) / rel));
  const attack = attackSamples / sr, decay = decaySamples / sr, release = releaseSamples / sr;
  const points: EnvelopePoint[] = [{ time: 0, level: 0 }];
  const add = (start: number, samples: number, value: (sample: number) => number) => {
    for (let i = 1; i <= 64; i++) { const sample = Math.max(1, Math.round(samples * i / 64)); points.push({ time: start + sample / sr, level: value(sample) }); }
  };
  add(0, attackSamples, k => Math.min(1, 1.2 * (1 - Math.exp(-att * k))));
  add(attack, decaySamples, k => Math.max(sustain, sustain - eps + (1 - sustain + eps) * Math.exp(-dec * k)));
  points.push({ time: attack + decay + hold, level: sustain });
  add(attack + decay + hold, releaseSamples, k => Math.max(0, -eps + (sustain + eps) * Math.exp(-rel * k)));
  points[points.length - 1].level = 0;
  return { points, attack, decay, hold, release, total: attack + decay + hold + release };
}
