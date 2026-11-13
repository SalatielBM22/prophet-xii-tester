import React, { memo, useEffect, useRef, useState } from 'react';
import specs from '../param-specs.json';
import type { Frame, Metadata } from '../protocol/types';
import { decodeFilter, filterPath, type FilterResponse } from './spectrum';
import { defaultAnalysis, type AnalysisConfig } from './capture';
import { TransferQueue } from './transferQueue';
import './analysis.css';

function drawSpectrum(canvas: HTMLCanvasElement, points: Float32Array, peak: Float32Array | null, sampleRate: number) {
  const ctx = canvas.getContext('2d'); if (!ctx) return;
  const { width, height } = canvas;
  ctx.fillStyle = '#0b1010'; ctx.fillRect(0, 0, width, height);
  const y = (db: number) => 18 + (12 - db) / 172 * (height - 38);
  ctx.font = '11px monospace';
  for (const db of [0, -40, -80, -120, -160]) {
    ctx.strokeStyle = '#25362b'; ctx.beginPath(); ctx.moveTo(0, y(db)); ctx.lineTo(width, y(db)); ctx.stroke();
    ctx.fillStyle = '#9faf9b'; ctx.fillText(`${db} dBFS`, 5, y(db) - 2);
  }
  const upper = Math.min(20000, sampleRate / 2);
  for (const hz of [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]) {
    if (hz > upper) continue;
    const x = width * Math.log(hz / 20) / Math.log(upper / 20);
    ctx.strokeStyle = '#25362b'; ctx.beginPath(); ctx.moveTo(x, 18); ctx.lineTo(x, height - 25); ctx.stroke();
    ctx.fillStyle = '#9faf9b'; ctx.textAlign = x > width - 35 ? 'right' : 'left';
    ctx.fillText(hz >= 1000 ? `${hz / 1000}k Hz` : `${hz} Hz`, Math.max(2, x), height - 4);
  }
  ctx.textAlign = 'left';
  const line = (data: Float32Array, color: string) => {
    ctx.strokeStyle = color; ctx.beginPath();
    for (let i = 0; i < data.length; i += 2) { if (i) ctx.lineTo(data[i], y(data[i + 1])); else ctx.moveTo(data[i], y(data[i + 1])); }
    ctx.stroke();
  };
  line(points, '#7fcbac'); if (peak) line(peak, '#e7bd70');
}

class SupersededTransferError extends Error {
  constructor() {
    super('Transfer request superseded');
  }
}

function formatFilterCaption(filter: FilterResponse): string {
  if (filter.legacy) {
    return `${filter.sampleRate} Hz fixed legacy DTFT measurement; independent of audio sample rate`;
  }
  if (filter.fftSize && filter.binHz) {
    const tail = filter.tailConverged === undefined
      ? ''
      : filter.tailConverged
        ? ' · tail converged'
        : ' · tail unconverged (self-oscillation/slow decay)';
    const samples = filter.impulseSamples ? ` (${filter.impulseSamples} samples)` : '';
    return `${filter.sampleRate} Hz · ${filter.fftSize}-point FFT${samples} · Δf ${filter.binHz.toFixed(3)} Hz${tail}`;
  }
  return `${filter.sampleRate} Hz FFT transfer measurement`;
}

export const AnalysisPanel = memo(function AnalysisPanel({ frame, values, connected, token, metadata }: { frame: Frame | null; values: number[]; connected: boolean; token: string; metadata: Metadata }) {
  const scope = useRef<HTMLCanvasElement>(null), spectrum = useRef<HTMLCanvasElement>(null);
  const worker = useRef<Worker | null>(null);
  const transfer = useRef<TransferQueue<FilterResponse> | null>(null);
  const [config, setConfig] = useState<AnalysisConfig>(defaultAnalysis);
  const [spectralInfo, setSpectralInfo] = useState('Waiting for continuous PCM capture');
  const [measurement, setMeasurement] = useState<{ peakHz: number; peakDb: number; firstSample: number; endFrame: number; discontinuities: number } | null>(null);
  const [filter, setFilter] = useState<FilterResponse | null>(null), [filterStatus, setFilterStatus] = useState('Waiting for host');
  const [showPeak, setShowPeak] = useState(true);
  const peakRef = useRef(showPeak); peakRef.current = showPeak;
  const continuous = metadata.extensions?.includes('continuous-capture') ?? false;
  const keys = ['FilterCutoff', 'FilterResonance', 'FilterDrive', 'FilterMorph', 'MoogBassComp'];
  const signature = keys.map(k => values[specs.find(s => s.key === k)!.id]).join(',');
  const currentValues = useRef(values); currentValues.current = values;

  useEffect(() => {
    if (!continuous || !connected) return;
    let alive = true;
    let instance: Worker;
    try { instance = new Worker(new URL('./spectrum.worker.ts', import.meta.url), { type: 'module' }); }
    catch { setSpectralInfo('Spectral display worker unavailable'); return; }
    worker.current = instance;
    instance.onmessage = event => {
      if (!alive) return;
      const data = event.data;
      if (data.kind === 'status') { setSpectralInfo(String(data.text)); return; }
      if (data.kind !== 'spectrum') return;
      if (spectrum.current) drawSpectrum(spectrum.current, data.points, peakRef.current ? data.peak : null, data.sampleRate);
      setMeasurement(data);
      setSpectralInfo(`${data.fftSize}-point ${data.window} · ${data.sampleRate} Hz · Δf ${(data.sampleRate / data.fftSize).toFixed(3)} Hz · hop ${data.hopSize} · gain ${data.coherentGain.toFixed(4)} · ENBW ${data.enbwBins.toFixed(3)} bins · single-sided amplitude dBFS`);
    };
    instance.onerror = () => { if (alive) setSpectralInfo('Rust spectral worker failed'); };
    instance.postMessage({ kind: 'start', url: `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/capture${token ? `?token=${encodeURIComponent(token)}` : ''}`, config });
    return () => { alive = false; instance.postMessage({ kind: 'stop' }); instance.terminate(); worker.current = null; setMeasurement(null); };
  }, [continuous, connected, token]);

  useEffect(() => { worker.current?.postMessage({ kind: 'config', config }); setMeasurement(null); }, [config]);

  useEffect(() => {
    const canvas = scope.current, ctx = canvas?.getContext('2d'); if (!canvas || !ctx) return;
    ctx.fillStyle = '#0b1010'; ctx.fillRect(0, 0, canvas.width, canvas.height);
    if (!frame || !connected) return;
    ctx.strokeStyle = '#e7bd70'; ctx.beginPath();
    for (let x = 0; x < canvas.width; x++) {
      const value = frame.scope[Math.min(frame.scope.length - 1, Math.floor(x * frame.scope.length / canvas.width))] ?? 0;
      const y = canvas.height / 2 - value * canvas.height * .45;
      if (x) ctx.lineTo(x, y); else ctx.moveTo(x, y);
    }
    ctx.stroke();
  }, [frame, connected]);

  useEffect(() => {
    setFilter(null);
    if (!connected) { setFilterStatus('Disconnected'); return; }
    const queue = new TransferQueue(async (parameters, signal) => {
        const response = await fetch('/api/v1/filter' + (token ? `?token=${encodeURIComponent(token)}` : ''), { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(parameters), signal });
        if (response.status === 409) throw new SupersededTransferError();
        if (!response.ok) throw Error(`Transfer response HTTP ${response.status}`);
        const raw: unknown = await response.json();
        const isLegacy = typeof raw !== 'object' || raw === null || typeof (raw as { sampleRate?: unknown }).sampleRate !== 'number';
        const parsed = decodeFilter(raw, isLegacy);
        if (!parsed) throw Error('Invalid transfer response');
        return parsed;
    }, parsed => { setFilter(parsed); setFilterStatus('Small-signal transfer response'); }, error => {
      if (error instanceof SupersededTransferError) { setFilterStatus('Measurement superseded by newer request'); return; }
      setFilterStatus(String(error));
    });
    transfer.current = queue; queue.schedule(currentValues.current);
    return () => { queue.dispose(); transfer.current = null; };
  }, [connected, token]);
  useEffect(() => { if (connected) { setFilterStatus('Measuring transfer response…'); transfer.current?.schedule(currentValues.current); } }, [signature, connected]);

  return <section className="analysis" aria-label="Signal analysis">
    <div className="meters">
      <div><h2>OUTPUT WAVEFORM</h2><canvas ref={scope} width={900} height={130} aria-label="Real Rust waveform snapshot" /><p>Scope snapshot · {connected ? frame?.sampleRate : '—'} Hz; no continuous acquisition implied.</p></div>
      <div className="filterchart"><h2>VCF SMALL-SIGNAL TRANSFER</h2><p role="status">{filterStatus}</p><svg viewBox="0 0 500 150" role="img" aria-label="VCF magnitude and unwrapped phase"><path d={filter ? filterPath(filter, false) : ''} fill="none" stroke="#e9b95e" strokeWidth="2" /><path d={filter ? filterPath(filter, true) : ''} fill="none" stroke="#80cdae" /></svg><span>{filter ? formatFilterCaption(filter) : 'Sample rate unavailable'} · amber |H| dB · green phase rad; phase masked below −100 dB</span><span>Log frequency: 20 Hz — {(filter ? filter.hz[filter.hz.length - 1] : undefined) ?? '—'} Hz. Transfer response is separate from live audio.</span></div>
    </div>
    <h2>LIVE OUTPUT SPECTRUM</h2>
    <p role="status">{!connected ? 'Spectrum disconnected' : continuous ? spectralInfo : 'Continuous spectrum unavailable: legacy scope snapshots are discontinuous.'}</p>
    <div className="analysis-config">
      <label>FFT size<select aria-label="FFT size" disabled={!continuous} value={config.fftSize} onChange={e => setConfig(c => ({ ...c, fftSize: Number(e.target.value) as AnalysisConfig['fftSize'] }))}>{[2048, 4096, 8192].map(n => <option key={n}>{n}</option>)}</select></label>
      <label>Window<select aria-label="Spectrum window" disabled={!continuous} value={config.window} onChange={e => setConfig(c => ({ ...c, window: e.target.value as AnalysisConfig['window'] }))}><option value="hann">Hann</option><option value="blackman-harris">Blackman–Harris</option><option value="kaiser">Kaiser</option></select></label>
      <label>Power average α<input aria-label="Spectrum average alpha" type="number" min="0.01" max="1" step="0.01" value={config.averageAlpha} onChange={e => { const n = Number(e.target.value); if (Number.isFinite(n) && n > 0 && n <= 1) setConfig(c => ({ ...c, averageAlpha: n })); }} /></label>
      <label>Peak hold frames<input aria-label="Spectrum peak hold frames" type="number" min="0" max="10000" value={config.peakHoldFrames} onChange={e => { const n = Number(e.target.value); if (Number.isInteger(n) && n >= 0 && n <= 10000) setConfig(c => ({ ...c, peakHoldFrames: n })); }} /></label>
      <label>Peak decay dB/frame<input aria-label="Spectrum peak decay" type="number" min="0" max="100" step="0.1" value={config.peakDecayDbPerFrame} onChange={e => { const n = Number(e.target.value); if (Number.isFinite(n) && n >= 0 && n <= 100) setConfig(c => ({ ...c, peakDecayDbPerFrame: n })); }} /></label>
      {config.window === 'kaiser' && <label>Kaiser β<input aria-label="Kaiser beta" type="number" min="0" max="20" step="0.1" value={config.kaiserBeta} onChange={e => { const n = Number(e.target.value); if (Number.isFinite(n) && n >= 0 && n <= 20) setConfig(c => ({ ...c, kaiserBeta: n })); }} /></label>}
    </div>
    <label><input type="checkbox" checked={showPeak} disabled={!continuous} onChange={e => setShowPeak(e.target.checked)} />Show producer peak hold</label>
    {continuous && connected && <canvas ref={spectrum} width={900} height={250} aria-label="Continuous output spectrum dBFS" />}
    {measurement && <p data-testid="spectrum-measurement" data-first-frame={measurement.firstSample} data-end-frame={measurement.endFrame} data-discontinuities={measurement.discontinuities}>Peak {measurement.peakHz.toFixed(2)} Hz · {measurement.peakDb.toFixed(2)} dBFS · PCM frames {measurement.firstSample}–{measurement.endFrame} · resets/gaps {measurement.discontinuities}</p>}
    <p>Positive log-frequency axis: 20 Hz — {frame ? Math.min(20000, frame.sampleRate / 2) : 20000} Hz. Rust STFT in a dedicated worker, 75% overlap; missing PCM resets overlap and averages. Capture is actual post-FX/master mono-left output, not device playback timing.</p>
  </section>;
});
