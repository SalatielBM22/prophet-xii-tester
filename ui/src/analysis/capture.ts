export interface CaptureBlock { sequence: number; firstFrame: number; sampleRate: number; samples: number[] }
export interface AnalysisConfig { fftSize: 2048 | 4096 | 8192; window: 'hann' | 'blackman-harris' | 'kaiser'; averageAlpha: number; peakHoldFrames: number; peakDecayDbPerFrame: number; kaiserBeta: number }
export const defaultAnalysis: AnalysisConfig = { fftSize: 4096, window: 'hann', averageAlpha: .4, peakHoldFrames: 25, peakDecayDbPerFrame: 1, kaiserBeta: 8.6 };
export function validConfig(c: AnalysisConfig) {
  return [2048, 4096, 8192].includes(c.fftSize) && ['hann', 'blackman-harris', 'kaiser'].includes(c.window) && Number.isFinite(c.averageAlpha) && c.averageAlpha > 0 && c.averageAlpha <= 1 && Number.isSafeInteger(c.peakHoldFrames) && c.peakHoldFrames >= 0 && c.peakHoldFrames <= 10000 && Number.isFinite(c.peakDecayDbPerFrame) && c.peakDecayDbPerFrame >= 0 && c.peakDecayDbPerFrame <= 100 && Number.isFinite(c.kaiserBeta) && c.kaiserBeta >= 0 && c.kaiserBeta <= 20;
}
export function decodeCapture(value: unknown): CaptureBlock[] | null {
  if (!value || typeof value !== 'object') return null;
  const input = value as Record<string, unknown>;
  if (input.v !== 1 || !Array.isArray(input.blocks) || input.blocks.length > 256) return null;
  for (const block of input.blocks) {
    if (!block || typeof block !== 'object') return null;
    if (!Number.isSafeInteger(block.sequence) || block.sequence < 0 || !Number.isSafeInteger(block.firstFrame) || block.firstFrame < 0 || !Number.isFinite(block.sampleRate) || block.sampleRate < 8000 || block.sampleRate > 192000) return null;
    if (!Array.isArray(block.samples) || block.samples.length < 1 || block.samples.length > 64 || !block.samples.every((s: unknown) => typeof s === 'number' && Number.isFinite(s) && Math.abs(s) <= 1e10)) return null;
    if (!Number.isSafeInteger(block.firstFrame + block.samples.length)) return null;
  }
  return input.blocks as CaptureBlock[];
}
export class DiscontinuityCounter {
  private prior = 0;
  recordReset(engineDiscontinuities = 0) {
    this.prior += 1 + Math.max(0, Math.trunc(engineDiscontinuities));
  }
  total(engineDiscontinuities: number) {
    return this.prior + Math.max(0, Math.trunc(engineDiscontinuities));
  }
}
