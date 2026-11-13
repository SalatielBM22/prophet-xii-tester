/** Display adapter for spectra computed by a non-real-time backend worker. */
export interface SpectrumFrame {
  sequence: number;
  firstSample: number;
  sampleRate: number;
  fftSize: 2048 | 4096 | 8192;
  hopSize: number;
  window: 'hann' | 'blackman-harris' | 'kaiser';
  coherentGain: number;
  enbwBins: number;
  normalization: 'single-sided-amplitude-dbfs';
  dbfs: number[];
  peakDbfs?: number[];
}

export function decodeSpectrum(value: unknown): SpectrumFrame | null {
  if (typeof value !== 'object' || value === null) return null;
  const s = value as Record<string, unknown>;
  if (!Number.isSafeInteger(s.sequence) || Number(s.sequence) < 0 || !Number.isSafeInteger(s.firstSample) || Number(s.firstSample) < 0) return null;
  if (typeof s.sampleRate !== 'number' || !Number.isFinite(s.sampleRate) || s.sampleRate < 8000 || s.sampleRate > 384000) return null;
  if (typeof s.fftSize !== 'number' || ![2048, 4096, 8192].includes(s.fftSize) || !Number.isInteger(s.hopSize) || Number(s.hopSize) < 1 || Number(s.hopSize) > Number(s.fftSize)) return null;
  if (!['hann', 'blackman-harris', 'kaiser'].includes(String(s.window)) || s.normalization !== 'single-sided-amplitude-dbfs') return null;
  if (typeof s.coherentGain !== 'number' || !Number.isFinite(s.coherentGain) || s.coherentGain <= 0 || s.coherentGain > 1 || typeof s.enbwBins !== 'number' || !Number.isFinite(s.enbwBins) || s.enbwBins <= 0) return null;
  const bins = Number(s.fftSize) / 2 + 1;
  const valid = (a: unknown) => Array.isArray(a) && a.length === bins && a.every(v => typeof v === 'number' && Number.isFinite(v));
  if (!valid(s.dbfs) || (s.peakDbfs !== undefined && !valid(s.peakDbfs))) return null;
  return s as unknown as SpectrumFrame;
}

/** Logarithmic x coordinates; DC has no coordinate on a log axis. */
export function spectrumPoints(s: SpectrumFrame, width: number, peak = false): Float32Array {
  const values = peak ? s.peakDbfs ?? s.dbfs : s.dbfs;
  const min = 20, max = Math.min(20000, s.sampleRate / 2);
  const points: number[] = [];
  for (let k = 1; k < values.length; k++) {
    const hz = k * s.sampleRate / s.fftSize;
    if (hz < min || hz > max) continue;
    points.push(Math.log(hz / min) / Math.log(max / min) * width, Math.max(-160, Math.min(12, values[k])));
  }
  return Float32Array.from(points);
}

export interface FilterResponse {
  sampleRate: number;
  hz: number[];
  db: number[];
  phase: (number | null)[];
  legacy: boolean;
  fftSize?: number;
  impulseSamples?: number;
  tailConverged?: boolean;
  tailRelativeRms?: number;
  binHz?: number;
}

export function decodeFilter(value: unknown, legacy: boolean, floorDb = -100): FilterResponse | null {
  if (typeof value !== 'object' || value === null) return null;
  const f = value as Record<string, unknown>;
  const hz = f.hz, db = f.db, phase = f.phase;
  if (!Array.isArray(hz) || hz.length < 2 || hz.length > 8192 || !Array.isArray(db) || !Array.isArray(phase) || hz.length !== db.length || hz.length !== phase.length) return null;
  if (!hz.every((x, i) => typeof x === 'number' && Number.isFinite(x) && x > 0 && (i === 0 || x > hz[i - 1]))) return null;
  if (!db.every(x => typeof x === 'number' && Number.isFinite(x))) return null;
  if (!phase.every(x => x === null || typeof x === 'number' && Number.isFinite(x))) return null;
  const sampleRate = legacy ? 48000 : f.sampleRate;
  if (typeof sampleRate !== 'number' || !Number.isFinite(sampleRate) || sampleRate < 8000 || sampleRate > 384000 || hz[hz.length - 1] > sampleRate / 2) return null;
  let fftSize: number | undefined;
  let impulseSamples: number | undefined;
  let tailConverged: boolean | undefined;
  let tailRelativeRms: number | undefined;
  let binHz: number | undefined;
  if (!legacy) {
    if (f.fftSize !== undefined) {
      if (typeof f.fftSize !== 'number' || !Number.isInteger(f.fftSize) || f.fftSize < 2048) return null;
      fftSize = f.fftSize;
    }
    if (f.impulseSamples !== undefined) {
      if (typeof f.impulseSamples !== 'number' || !Number.isInteger(f.impulseSamples) || f.impulseSamples < 1) return null;
      impulseSamples = f.impulseSamples;
    }
    if (f.tailConverged !== undefined) {
      if (typeof f.tailConverged !== 'boolean') return null;
      tailConverged = f.tailConverged;
    }
    if (f.tailRelativeRms !== undefined) {
      if (typeof f.tailRelativeRms !== 'number' || !Number.isFinite(f.tailRelativeRms) || f.tailRelativeRms < 0) return null;
      tailRelativeRms = f.tailRelativeRms;
    }
    if (f.binHz !== undefined) {
      if (typeof f.binHz !== 'number' || !Number.isFinite(f.binHz) || f.binHz <= 0) return null;
      binHz = f.binHz;
    }
  }
  let last: number | null = null;
  const unwrapped = phase.map((p, i) => {
    if (p === null || db[i] < floorDb) { last = null; return null; }
    let result = p as number;
    if (legacy && last !== null) result += Math.round((last - result) / (2 * Math.PI)) * 2 * Math.PI;
    last = result;
    return result;
  });
  return { sampleRate, hz, db, phase: unwrapped, legacy, fftSize, impulseSamples, tailConverged, tailRelativeRms, binHz };
}

export function filterPath(f: FilterResponse, phase: boolean): string {
  const lo = f.hz[0], hi = f.hz[f.hz.length - 1];
  let drawing = false;
  return f.hz.map((hz, i) => {
    const value = phase ? f.phase[i] : f.db[i];
    if (value === null) { drawing = false; return ''; }
    const x = 500 * Math.log(hz / lo) / Math.log(hi / lo);
    const y = phase ? 75 - value * 12 : Math.max(2, Math.min(148, 35 - value));
    const command = drawing ? 'L' : 'M'; drawing = true;
    return `${command}${x.toFixed(3)},${y.toFixed(3)}`;
  }).join(' ');
}
