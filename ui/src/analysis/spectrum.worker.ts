import init, { WebAnalyzer } from '../../generated/analysis/prophet_analysis.js';
import wasmUrl from '../../generated/analysis/prophet_analysis_bg.wasm?url';
import { decodeCapture, defaultAnalysis, validConfig, type AnalysisConfig } from './capture';
import { spectrumPoints, type SpectrumFrame } from './spectrum';

let config = defaultAnalysis, analyzer: WebAnalyzer | null = null, memory: WebAssembly.Memory;
let socket: WebSocket | null = null, retry: ReturnType<typeof setTimeout> | undefined;
let stopped = true, url = '', lastPublish = 0, resets = 0;
const ready = init({ module_or_path: wasmUrl }).then(instance => { memory = instance.memory; });
let lastStatus = '';
const status = (text: string) => { if (text !== lastStatus) { lastStatus = text; self.postMessage({ kind: 'status', text }); } };
function clearAnalysis() { if (analyzer) { analyzer.free(); analyzer = null; resets++; } }
function create(sampleRate: number) {
  clearAnalysis();
  analyzer = new WebAnalyzer(config.fftSize, sampleRate, ['hann', 'blackman-harris', 'kaiser'].indexOf(config.window), config.averageAlpha, config.peakHoldFrames, config.peakDecayDbPerFrame, config.kaiserBeta);
}
function receive(text: unknown) {
  if (typeof text !== 'string' || text.length > 1048576) { clearAnalysis(); status('Capture payload rejected; overlap reset'); return; }
  let blocks;
  try { blocks = decodeCapture(JSON.parse(text)); } catch { blocks = null; }
  if (!blocks) { clearAnalysis(); status('Invalid capture packet; overlap reset'); return; }
  for (const block of blocks) {
    if (!analyzer || analyzer.sample_rate() !== block.sampleRate) create(block.sampleRate);
    const engine = analyzer!;
    // Reacquire memory views each call: memory.grow can invalidate JS buffers.
    new Float32Array(memory.buffer, engine.input_ptr(), engine.input_capacity()).set(block.samples);
    const result = engine.push(block.sequence, block.firstFrame, block.samples.length);
    if (result === -4) continue;
    if (result < 0) { clearAnalysis(); status('Capture rejected (' + result + '); overlap reset'); continue; }
    if (!engine.has_frame()) { status('Collecting contiguous PCM for STFT…'); continue; }
    if (result > 0 && performance.now() - lastPublish >= 1000 / 30) {
      lastPublish = performance.now(); lastStatus = '';
      const db = new Float32Array(memory.buffer, engine.db_ptr(), engine.bins());
      const peaks = new Float32Array(memory.buffer, engine.peak_ptr(), engine.bins());
      const data: SpectrumFrame = { sequence: engine.sequence(), firstSample: engine.first_frame(), sampleRate: block.sampleRate, fftSize: config.fftSize, hopSize: config.fftSize / 4, window: config.window, coherentGain: engine.coherent_gain(), enbwBins: engine.enbw_bins(), normalization: 'single-sided-amplitude-dbfs', dbfs: Array.from(db), peakDbfs: Array.from(peaks) };
      const points = spectrumPoints(data, 900), peak = spectrumPoints(data, 900, true);
      let maximum = 1;
      for (let k = 2; k < db.length; k++) if (db[k] > db[maximum]) maximum = k;
      self.postMessage({ kind: 'spectrum', ...data, dbfs: undefined, peakDbfs: undefined, points, peak, peakHz: maximum * block.sampleRate / config.fftSize, peakDb: db[maximum], discontinuities: resets + engine.discontinuities(), endFrame: engine.end_frame() }, { transfer: [points.buffer, peak.buffer] });
    }
  }
}
function connect() {
  if (stopped) return;
  socket = new WebSocket(url);
  socket.onopen = () => status('PCM connected; collecting contiguous STFT window…');
  socket.onmessage = event => { try { receive(event.data); } catch (error) { clearAnalysis(); status('Analysis error: ' + String(error)); } };
  socket.onerror = () => status('PCM transport error');
  socket.onclose = () => { clearAnalysis(); if (!stopped) { status('PCM disconnected; retrying with cleared overlap'); retry = setTimeout(connect, 1000); } };
}
self.onmessage = async (event: MessageEvent<{ kind: 'start' | 'config' | 'stop'; url?: string; config?: AnalysisConfig }>) => {
  try {
    if (event.data.config) {
      if (!validConfig(event.data.config)) { status('Invalid analysis configuration'); return; }
      config = event.data.config; clearAnalysis(); status('Configuration changed; collecting a fresh continuous window');
    }
    if (event.data.kind === 'stop') { stopped = true; clearTimeout(retry); socket?.close(); clearAnalysis(); return; }
    if (event.data.kind === 'start' && event.data.url) { url = event.data.url; stopped = false; await ready; if (!stopped) connect(); }
  } catch (error) { status('Rust WASM analysis unavailable: ' + String(error)); }
};
