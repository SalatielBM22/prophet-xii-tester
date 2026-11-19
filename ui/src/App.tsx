// SPDX-License-Identifier: GPL-3.0-only
import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import specs from './param-specs.json';
import { Knob } from './controls/Knob';
import { ParameterState } from './controls/parameterState';
import { inactiveReason, normalize, paramId } from './controls/parameters';
import { Envelope } from './components/Envelope';
import { VectorPanel } from './components/VectorPanel';
import { SequencerPanel } from './components/SequencerPanel';
import { VcfResponsePlot } from './components/VcfResponsePlot';
import { Keyboard } from './components/Keyboard';
import { AnalysisPanel } from './analysis/AnalysisPanel';
import { decodeFilter, type FilterResponse } from './analysis/spectrum';
import { TransferQueue } from './analysis/transferQueue';
import { ControlTransport } from './protocol/transport';
import { decodeFrame, decodeMetadata, withFrameCapabilities } from './protocol/types';
import type { Command, EnvelopeStage, Frame, Metadata, VoiceEnvelope } from './protocol/types';
import {
  GLIDE_ENABLED_PARAM_ID,
  GLIDE_ENABLED_SPEC,
  formatMidiNote,
  originShortLabel,
  type AndroidAudioDiagnostics,
  type FilterResponseReport,
  type PhysicalMidiSource,
  type UiCommand,
  type UiSnapshot,
  type WireAlgorithm,
  type WireTailPolicy,
} from './protocol/enginePort';
import { createNativeTransport, type NativeUiTransport } from './protocol/uiTransport';
import './style.css';

const emptyMetadata: Metadata = { protocolVersion: 1, capabilities: [], presets: [] };
const ZOOM_LEVELS = [0.75, 1, 1.25, 1.5] as const;

const PANEL_ORDER = [
  'OSCILLATOR A',
  'OSCILLATOR B',
  'MIXER',
  'FILTER',
  'FILTER ENVELOPE',
  'AMPLIFIER ENVELOPE',
  'POLY-MOD',
  'LFO',
  'PERFORMANCE',
  'CHORUS',
  'REVERB',
] as const;

type NativeEdit = { id: number; before: number; after: number };
type PendingReverb = {
  seq: number;
  enabled: boolean;
  tail: 'kill' | 'spill';
  commandId: number | null;
  submitted: boolean;
  since: number;
};

export default function App() {
  const nativeTransport = useMemo<NativeUiTransport | null>(() => createNativeTransport(), []);
  const isNative = nativeTransport !== null;

  // --- Shared UI state ---
  const [zoom, setZoom] = useState<number>(() => nativeTransport?.getZoom() ?? 1);
  const [notice, setNotice] = useState('');
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('All');
  const [selectedPreset, setSelectedPreset] = useState('');
  const [drawer, setDrawer] = useState(false);
  const [analysisOpen, setAnalysisOpen] = useState(!isNative);
  const [diagOpen, setDiagOpen] = useState(false);
  const [presetModal, setPresetModal] = useState<{ mode: 'import' | 'export'; text: string; error: string } | null>(null);
  const [presetName, setPresetName] = useState('User Patch');
  const [midiTarget, setMidiTarget] = useState<number | null>(null);
  const [midiCC, setMidiCC] = useState(1);
  const [slot, setSlot] = useState(0);
  const slots = useRef<Array<number[] | undefined>>([]);
  const [held, setHeld] = useState<Set<number>>(new Set());
  const heldRef = useRef(new Set<number>());
  const scopeCanvasRef = useRef<HTMLCanvasElement>(null);

  // --- Native EnginePort v1 state ---
  const [uiSnap, setUiSnap] = useState<UiSnapshot | null>(null);
  const [nativeOverrides, setNativeOverrides] = useState<Record<number, number>>({});
  const [nativeHistory, setNativeHistory] = useState<NativeEdit[][]>([]);
  const [nativeFuture, setNativeFuture] = useState<NativeEdit[][]>([]);
  const nativeGesture = useRef<{ before: number[]; ids: Set<number> } | null>(null);
  const nativeFactoryBank = useRef<Map<number, { values: number[]; glide: boolean }>>(new Map());
  const [nativeFactoryRef, setNativeFactoryRef] = useState<{ id: number; values: number[]; glide: boolean } | null>(null);
  const [nativeFilterReport, setNativeFilterReport] = useState<FilterResponseReport | null>(null);
  const [nativeFilterStatus, setNativeFilterStatus] = useState('Ready');
  const [androidDiag, setAndroidDiag] = useState<AndroidAudioDiagnostics | null>(null);
  const lastNoticeSeq = useRef(0);
  const lastVectorSendMs = useRef(0);

  // --- Legacy HTTP/WebSocket (--http-dev / RC1) state ---
  const [token, setToken] = useState(() => sessionStorage.getItem('prophet-token') || '');
  const transport = useMemo(() => new ControlTransport(token), [token]);
  const parameters = useMemo(() => new ParameterState(transport), [transport]);
  const legacyState = useSyncExternalStore(parameters.subscribe, parameters.getSnapshot);
  const [frame, setFrame] = useState<Frame | null>(null);
  const [httpConnected, setHttpConnected] = useState(false);
  const [metadata, setMetadata] = useState<Metadata>(emptyMetadata);
  const [presetLoading, setPresetLoading] = useState(false);
  const [factoryReference, setFactoryReference] = useState<{ id: number; commandId: number; values: number[] } | null>(null);
  const presetRequest = useRef<{ id: number; commandId: number; values: number[]; since: number } | null>(null);
  const [presetAcceptance, setPresetAcceptance] = useState(0);
  const [pendingReverb, setPendingReverb] = useState<PendingReverb | null>(null);
  const pendingReverbRef = useRef<PendingReverb | null>(null);
  const reverbSeq = useRef(0);
  const latestFrame = useRef<Frame | null>(null);
  const [legacyFilter, setLegacyFilter] = useState<FilterResponse | null>(null);
  const [legacyFilterStatus, setLegacyFilterStatus] = useState('Waiting for host');
  const legacyTransfer = useRef<TransferQueue<FilterResponse> | null>(null);

  const midiInputs = useRef<Array<{ onmidimessage: ((e: { data: Uint8Array }) => void) | null }>>([]);
  const midiGeneration = useRef(0);

  const setPendingReverbState = useCallback((next: PendingReverb | null) => {
    pendingReverbRef.current = next;
    setPendingReverb(next);
  }, []);

  const clearMIDI = useCallback(() => {
    midiGeneration.current++;
    midiInputs.current.forEach(input => {
      input.onmidimessage = null;
    });
    midiInputs.current = [];
  }, []);
  useEffect(() => clearMIDI, [token, clearMIDI]);

  // --- Effective unified values & connection ---
  const connected = isNative ? Boolean(uiSnap?.ready) : httpConnected;
  const values = useMemo(() => {
    if (!isNative) return legacyState.values;
    const base = uiSnap?.parameters ?? specs.map(s => s.min);
    const merged = specs.map((s, idx) => nativeOverrides[idx] ?? base[idx] ?? s.min);
    return merged;
  }, [isNative, legacyState.values, uiSnap, nativeOverrides]);

  const glideEnabledValue = useMemo(() => {
    if (!isNative) return null;
    if (nativeOverrides[GLIDE_ENABLED_PARAM_ID] !== undefined) return nativeOverrides[GLIDE_ENABLED_PARAM_ID];
    return uiSnap?.parameters[GLIDE_ENABLED_PARAM_ID] ?? 0;
  }, [isNative, uiSnap, nativeOverrides]);

  const valuesRef = useRef(values);
  valuesRef.current = values;

  // --- Native EnginePort Transport Lifecycle ---
  useEffect(() => {
    if (!nativeTransport) return;
    let alive = true;
    const stop = nativeTransport.start(
      snap => {
        if (!alive) return;
        setUiSnap(snap);
        if (snap.filterAnalysis) setNativeFilterReport(snap.filterAnalysis);
        if (snap.lastNotice && snap.lastNotice.seq !== lastNoticeSeq.current) {
          lastNoticeSeq.current = snap.lastNotice.seq;
          setNotice(snap.lastNotice.message);
        }
        // Clear optimistic overrides once host snapshot reflects updated state
        setNativeOverrides(prev => {
          const keys = Object.keys(prev);
          if (!keys.length) return prev;
          const next: Record<number, number> = {};
          let changed = false;
          for (const k of keys) {
            const id = Number(k);
            const target = prev[id];
            const hostVal = snap.parameters[id];
            if (hostVal !== undefined && Math.abs(hostVal - target) <= Math.max(1e-4, Math.abs(target) * 1e-4)) {
              changed = true;
            } else {
              next[id] = target;
            }
          }
          return changed ? next : prev;
        });
      },
      err => {
        if (alive) setNotice(err);
      },
    );

    // Fetch immutable confirmed factory preset defaults once off-audio
    void nativeTransport.dispatch({ type: 'hostControl', op: 'getFactoryPresets' }).then(reply => {
      if (!alive || !reply.ok || !reply.data) return;
      const list = reply.data.presets;
      if (Array.isArray(list)) {
        const map = new Map<number, { values: number[]; glide: boolean }>();
        for (const item of list) {
          if (item && typeof item === 'object' && typeof (item as { id?: unknown }).id === 'number') {
            const entry = item as { id: number; values?: unknown; glide?: unknown };
            if (Array.isArray(entry.values) && entry.values.length >= specs.length) {
              map.set(entry.id, {
                values: (entry.values as number[]).slice(0, specs.length),
                glide: Boolean(entry.glide),
              });
            }
          }
        }
        nativeFactoryBank.current = map;
        const bootPreset = map.get(0);
        if (bootPreset) {
          setNativeFactoryRef(prev => prev ?? { id: 0, values: [...bootPreset.values], glide: bootPreset.glide });
        }
      }
    });

    const diagTimer =
      nativeTransport.kind === 'android-bridge'
        ? setInterval(() => {
            if (!alive) return;
            const d = nativeTransport.pollDiagnostics();
            if (d) setAndroidDiag(d);
          }, 500)
        : undefined;

    return () => {
      alive = false;
      stop();
      if (diagTimer) clearInterval(diagTimer);
    };
  }, [nativeTransport]);

  const dispatchNative = useCallback(
    async (cmd: UiCommand) => {
      if (!nativeTransport) return { ok: false, message: 'Native transport unavailable' };
      const reply = await nativeTransport.dispatch(cmd);
      if (!reply.ok && reply.message) {
        setNotice(reply.message);
      }
      return reply;
    },
    [nativeTransport],
  );

  // --- Native Debounced VCF Small-Signal Analysis ---
  const filterSignature = useMemo(() => {
    const ids = ['FilterCutoff', 'FilterResonance', 'FilterDrive', 'FilterMorph', 'MoogBassComp'].map(paramId);
    return ids.map(id => Number((values[id] ?? 0).toFixed(4))).join(',');
  }, [values]);

  // One native measurement at a time (~50 ms of synchronous host work each).
  // A change that arrives meanwhile is measured right after the reply.
  const nativeFilterInFlight = useRef(false);
  const nativeFilterDirty = useRef(false);
  const triggerNativeFilterRef = useRef<() => void>(() => {});
  const triggerNativeFilterMeasure = useCallback(() => {
    if (!nativeTransport || !connected) return;
    if (nativeFilterInFlight.current) {
      nativeFilterDirty.current = true;
      return;
    }
    nativeFilterInFlight.current = true;
    setNativeFilterStatus('Measuring small-signal VCF response…');
    void nativeTransport
      .dispatch({ type: 'analyzeFilter', sample_rate: uiSnap?.enginePort.sampleRate })
      .then(reply => {
        if (reply.filterAnalysis) {
          setNativeFilterReport(reply.filterAnalysis);
          setNativeFilterStatus(reply.filterAnalysis.reason);
        } else {
          setNativeFilterStatus(reply.message || 'VCF analysis unavailable');
        }
      })
      .catch(error => setNativeFilterStatus(String(error)))
      .finally(() => {
        nativeFilterInFlight.current = false;
        if (nativeFilterDirty.current) {
          nativeFilterDirty.current = false;
          triggerNativeFilterRef.current();
        }
      });
  }, [nativeTransport, connected, uiSnap?.enginePort.sampleRate]);
  triggerNativeFilterRef.current = triggerNativeFilterMeasure;

  // Throttled (not debounced) so the curve follows a continuous knob drag.
  const nativeFilterTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!isNative || !connected || nativeFilterTimer.current) return;
    nativeFilterTimer.current = setTimeout(() => {
      nativeFilterTimer.current = null;
      triggerNativeFilterRef.current();
    }, 120);
  }, [isNative, connected, filterSignature]);
  useEffect(() => () => {
    if (nativeFilterTimer.current) clearTimeout(nativeFilterTimer.current);
    nativeFilterTimer.current = null;
  }, []);

  // --- Legacy HTTP/WebSocket Connection Lifecycle (runs ONLY when !isNative) ---
  const capability = useCallback(
    (name: Metadata['capabilities'][number]) => metadata.capabilities.includes(name),
    [metadata.capabilities],
  );

  const sendLegacy = useCallback(
    async (command: Command) => {
      const generation = transport.generation;
      try {
        await transport.send(command);
        return true;
      } catch (error) {
        if (generation === transport.generation) setNotice(String(error));
        return false;
      }
    },
    [transport],
  );

  const reconcileReverb = useCallback(
    (f: Frame | null, caps: Metadata['capabilities']) => {
      const pending = pendingReverbRef.current;
      if (!pending) return;
      if (!f?.reverb || !caps.includes('reverbBypass')) {
        setPendingReverbState(null);
        return;
      }
      if (!pending.submitted) return;
      if (caps.includes('parameterAck')) {
        if (pending.commandId === null) {
          setPendingReverbState(null);
        } else if (f.control?.lastDroppedCommandId === pending.commandId) {
          setPendingReverbState(null);
          setNotice('Host dropped the reverb bypass request');
        } else if ((f.control?.lastAppliedCommandId ?? 0) >= pending.commandId) {
          setPendingReverbState(null);
        }
      } else if (f.reverb.enabled === pending.enabled && f.reverb.tail === pending.tail) {
        setPendingReverbState(null);
      }
    },
    [setPendingReverbState],
  );

  useEffect(() => {
    if (isNative) return;
    sessionStorage.setItem('prophet-token', token);
    let connectionGeneration = 0;
    let alive = true;
    let socket: WebSocket | null = null;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let raf = 0;
    let latest: Frame | null = null;
    let previousFrames: number | null = null;
    let lastProgress = Date.now();
    const abort = new AbortController();
    setHttpConnected(false);
    setFrame(null);
    setMetadata(emptyMetadata);
    setFactoryReference(null);
    setPendingReverbState(null);

    const open = async () => {
      const generation = ++connectionGeneration;
      try {
        const response = await fetch('/api/v1/meta', { signal: abort.signal });
        if (!response.ok) throw Error(`Metadata returned ${response.status}`);
        const meta = decodeMetadata(await response.json());
        if (!alive || generation !== connectionGeneration) return;
        transport.metadata = meta;
        setMetadata(meta);
      } catch (error) {
        if (alive) {
          transport.metadata = emptyMetadata;
          setMetadata(emptyMetadata);
          setNotice(String(error));
        }
      }
      if (!alive) return;
      socket = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/ws${transport.auth}`);
      socket.onmessage = event => {
        if (!alive || generation !== connectionGeneration) return;
        try {
          const raw = JSON.parse(event.data);
          if (raw?.v === 1 && raw?.ready === false) return;
          const parsed = decodeFrame(raw);
          if (!parsed) {
            setNotice('Ignored invalid or unsupported telemetry');
            return;
          }
          if (parsed.v !== transport.metadata.protocolVersion) {
            setNotice('Telemetry protocol does not match host metadata');
            return;
          }
          latest = parsed;
          if (!raf) {
            raf = requestAnimationFrame(() => {
              raf = 0;
              if (!alive || !latest) return;
              const f = latest;
              latest = null;
              if (parameters.receive(f)) {
                const nextMeta = withFrameCapabilities(transport.metadata, f);
                transport.metadata = nextMeta;
                setMetadata(current => (current.capabilities.join(',') === nextMeta.capabilities.join(',') ? current : nextMeta));
                const advanced = previousFrames !== null && f.frames > previousFrames;
                if (advanced) {
                  lastProgress = Date.now();
                  setNotice(current => (current === 'Audio telemetry stopped advancing; reconnecting' ? '' : current));
                }
                previousFrames = f.frames;
                latestFrame.current = f;
                setFrame(f);
                if (advanced) setHttpConnected(true);
              }
            });
          }
        } catch {
          setNotice('Ignored malformed telemetry');
        }
      };
      socket.onclose = () => {
        if (!alive || generation !== connectionGeneration) return;
        latest = null;
        previousFrames = null;
        lastProgress = Date.now();
        cancelAnimationFrame(raf);
        raf = 0;
        parameters.disconnect();
        transport.reset();
        clearMIDI();
        setHttpConnected(false);
        setFrame(null);
        setFactoryReference(null);
        presetRequest.current = null;
        setPresetLoading(false);
        setPendingReverbState(null);
        latestFrame.current = null;
        heldRef.current.clear();
        setHeld(new Set());
        retry = setTimeout(() => void open(), 1000);
      };
      socket.onerror = () => {
        if (alive) setNotice('WebSocket transport error');
      };
    };
    void open();

    const expiration = setInterval(() => {
      parameters.expire();
      if (socket?.readyState === WebSocket.OPEN && Date.now() - lastProgress > 3000) {
        setNotice('Audio telemetry stopped advancing; reconnecting');
        setHttpConnected(false);
        socket.close();
      }
      if (presetRequest.current && Date.now() - presetRequest.current.since > 4000) {
        presetRequest.current = null;
        setPresetLoading(false);
        setFactoryReference(null);
        setNotice('Host did not confirm factory preset application within 4 seconds');
      }
      if (pendingReverbRef.current && Date.now() - pendingReverbRef.current.since > 4000) {
        setPendingReverbState(null);
        setNotice('Host did not confirm reverb state change within 4 seconds');
      }
    }, 200);

    return () => {
      alive = false;
      abort.abort();
      clearTimeout(retry);
      clearInterval(expiration);
      cancelAnimationFrame(raf);
      socket?.close();
      parameters.disconnect();
      transport.reset();
      setPendingReverbState(null);
    };
  }, [isNative, token, transport, parameters, clearMIDI, setPendingReverbState]);

  useEffect(() => {
    if (isNative) return;
    const f = latestFrame.current;
    reconcileReverb(f, metadata.capabilities);
    if (!f?.parameterCommandIds || !metadata.capabilities.includes('factoryPresetValues')) {
      setFactoryReference(null);
      return;
    }
    const requested = presetRequest.current;
    if (requested && f.parameterCommandIds.every(id => id === requested.commandId)) {
      setFactoryReference({ id: requested.id, commandId: requested.commandId, values: [...requested.values] });
      presetRequest.current = null;
      setPresetLoading(false);
    } else if (requested && f.control?.lastDroppedCommandId === requested.commandId) {
      presetRequest.current = null;
      setPresetLoading(false);
      setFactoryReference(null);
      setNotice('Host dropped the factory preset request');
    }
    const stamp = f.parameterCommandIds[0];
    if (stamp > 0 && f.parameterCommandIds.every(id => id === stamp) && !transport.isOwnHostCommand(stamp)) {
      setFactoryReference(current => (current?.commandId === stamp ? current : null));
    }
  }, [isNative, frame, metadata, presetAcceptance, transport, reconcileReverb]);

  // Legacy adjacent VCF plot queue
  useEffect(() => {
    if (isNative) return;
    setLegacyFilter(null);
    if (!httpConnected) {
      setLegacyFilterStatus('Disconnected');
      return;
    }
    const queue = new TransferQueue<FilterResponse>(
      async (params, signal) => {
        const res = await fetch('/api/v1/filter' + (token ? `?token=${encodeURIComponent(token)}` : ''), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(params),
          signal,
        });
        if (res.status === 409) throw Error('Superseded');
        if (!res.ok) throw Error(`Transfer HTTP ${res.status}`);
        const raw: unknown = await res.json();
        const isLeg = typeof raw !== 'object' || raw === null || typeof (raw as { sampleRate?: unknown }).sampleRate !== 'number';
        const parsed = decodeFilter(raw, isLeg);
        if (!parsed) throw Error('Invalid transfer response');
        return parsed;
      },
      parsed => {
        setLegacyFilter(parsed);
        setLegacyFilterStatus('Small-signal transfer response');
      },
      err => {
        const msg = err instanceof Error ? err.message : String(err);
        if (msg === 'Superseded') setLegacyFilterStatus('Superseded by newer request');
        else setLegacyFilterStatus(msg);
      },
    );
    legacyTransfer.current = queue;
    queue.schedule(valuesRef.current);
    return () => {
      queue.dispose();
      legacyTransfer.current = null;
    };
  }, [isNative, httpConnected, token]);

  useEffect(() => {
    if (!isNative && httpConnected) {
      legacyTransfer.current?.schedule(valuesRef.current);
    }
  }, [isNative, httpConnected, filterSignature]);

  // --- Compact Oscilloscope Drawing (shared native + legacy) ---
  const scopeSamples = isNative ? uiSnap?.scope : frame?.scope;
  useEffect(() => {
    const canvas = scopeCanvasRef.current;
    const ctx = canvas?.getContext('2d');
    if (!canvas || !ctx) return;
    ctx.fillStyle = '#0a0f0c';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.strokeStyle = '#1f2b22';
    ctx.beginPath();
    ctx.moveTo(0, canvas.height / 2);
    ctx.lineTo(canvas.width, canvas.height / 2);
    ctx.stroke();
    if (!connected || !scopeSamples || scopeSamples.length === 0) return;
    ctx.strokeStyle = '#e7bd70';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let x = 0; x < canvas.width; x++) {
      const idx = Math.min(scopeSamples.length - 1, Math.floor((x * scopeSamples.length) / canvas.width));
      const v = Math.max(-1, Math.min(1, scopeSamples[idx] ?? 0));
      const y = canvas.height / 2 - v * canvas.height * 0.43;
      if (x === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();
  }, [scopeSamples, connected]);

  // --- Unified Parameter Edit / Gesture / Undo / Redo ---
  const beginParam = useCallback(
    (id: number) => {
      if (!isNative) {
        parameters.begin(id);
        return;
      }
      if (!nativeGesture.current) {
        nativeGesture.current = {
          before: [...valuesRef.current, glideEnabledValue ?? 0],
          ids: new Set(),
        };
      }
      nativeGesture.current.ids.add(id);
    },
    [isNative, parameters, glideEnabledValue],
  );

  const endParam = useCallback(() => {
    if (!isNative) {
      parameters.end();
      return;
    }
    if (!nativeGesture.current) return;
    const currentAll = [...valuesRef.current, glideEnabledValue ?? 0];
    const edits: NativeEdit[] = [...nativeGesture.current.ids]
      .map(id => ({ id, before: nativeGesture.current!.before[id] ?? 0, after: currentAll[id] ?? 0 }))
      .filter(e => Math.abs(e.before - e.after) > 1e-6);
    nativeGesture.current = null;
    if (edits.length) {
      setNativeHistory(h => [...h.slice(-49), edits]);
      setNativeFuture([]);
    }
  }, [isNative, parameters, glideEnabledValue]);

  const editParam = useCallback(
    (id: number, rawValue: number) => {
      if (!isNative) {
        parameters.edit(id, rawValue);
        return;
      }
      if (id === paramId('OversamplingMode')) {
        const factor = (rawValue < 1.5 ? 1 : rawValue < 3 ? 2 : 4) as 1 | 2 | 4;
        setNativeOverrides(prev => ({ ...prev, [id]: factor }));
        void dispatchNative({ type: 'setOversampling', factor });
        return;
      }
      const spec = id === GLIDE_ENABLED_PARAM_ID ? GLIDE_ENABLED_SPEC : specs[id];
      if (!spec) return;
      const value = normalize(spec, rawValue);
      const atomic = !nativeGesture.current;
      if (atomic) beginParam(id);
      nativeGesture.current?.ids.add(id);
      setNativeOverrides(prev => ({ ...prev, [id]: value }));
      void dispatchNative({ type: 'param', id, value }).then(reply => {
        if (!reply.ok) {
          setNativeOverrides(prev => {
            const copy = { ...prev };
            delete copy[id];
            return copy;
          });
        }
      });
      if (atomic) endParam();
    },
    [isNative, parameters, beginParam, endParam, dispatchNative],
  );

  const undo = useCallback(() => {
    if (!isNative) {
      parameters.undo();
      return;
    }
    endParam();
    setNativeHistory(prev => {
      if (!prev.length) return prev;
      const edits = prev[prev.length - 1];
      setNativeFuture(f => [...f, edits]);
      for (const e of edits) {
        setNativeOverrides(o => ({ ...o, [e.id]: e.before }));
        if (e.id === paramId('OversamplingMode')) {
          void dispatchNative({ type: 'setOversampling', factor: (e.before < 1.5 ? 1 : e.before < 3 ? 2 : 4) as 1 | 2 | 4 });
        } else {
          void dispatchNative({ type: 'param', id: e.id, value: e.before });
        }
      }
      return prev.slice(0, -1);
    });
  }, [isNative, parameters, endParam, dispatchNative]);

  const redo = useCallback(() => {
    if (!isNative) {
      parameters.redo();
      return;
    }
    endParam();
    setNativeFuture(prev => {
      if (!prev.length) return prev;
      const edits = prev[prev.length - 1];
      setNativeHistory(h => [...h, edits]);
      for (const e of edits) {
        setNativeOverrides(o => ({ ...o, [e.id]: e.after }));
        if (e.id === paramId('OversamplingMode')) {
          void dispatchNative({ type: 'setOversampling', factor: (e.after < 1.5 ? 1 : e.after < 3 ? 2 : 4) as 1 | 2 | 4 });
        } else {
          void dispatchNative({ type: 'param', id: e.id, value: e.after });
        }
      }
      return prev.slice(0, -1);
    });
  }, [isNative, parameters, endParam, dispatchNative]);

  // --- Reverb Update (EnginePort v1 + Legacy RC1 Optimistic Guard) ---
  const updateReverb = useCallback(
    async (patch: Partial<{ algorithm: WireAlgorithm; enabled: boolean; tail: WireTailPolicy }>) => {
      if (isNative) {
        const reply = await dispatchNative({
          type: 'setReverb',
          algorithm: patch.algorithm,
          enabled: patch.enabled,
          tail: patch.tail,
        });
        if (reply.message) setNotice(reply.message);
        return;
      }
      const current = pendingReverbRef.current ?? latestFrame.current?.reverb ?? frame?.reverb;
      if (!current) return;
      const enabled = patch.enabled ?? current.enabled;
      const tail = patch.tail ?? current.tail;
      const seq = ++reverbSeq.current;
      const epoch = transport.generation;
      const since = Date.now();
      setPendingReverbState({ seq, enabled, tail, commandId: null, submitted: false, since });
      try {
        const submission = await transport.send({ type: 'reverb_bypass', enabled, tail });
        if (epoch !== transport.generation || pendingReverbRef.current?.seq !== seq) return;
        if (!submission.submitted) {
          setPendingReverbState(null);
          return;
        }
        setPendingReverbState({ seq, enabled, tail, commandId: submission.acceptedCommandId, submitted: true, since });
        reconcileReverb(latestFrame.current, transport.metadata.capabilities);
      } catch (error) {
        if (epoch === transport.generation && pendingReverbRef.current?.seq === seq) {
          setPendingReverbState(null);
          setNotice(String(error));
        }
      }
    },
    [isNative, dispatchNative, frame, transport, setPendingReverbState, reconcileReverb],
  );

  // --- Notes & Panic ---
  const note = useCallback(
    (midiNumber: number, on: boolean) => {
      if (on === heldRef.current.has(midiNumber)) return;
      if (on) heldRef.current.add(midiNumber);
      else heldRef.current.delete(midiNumber);
      setHeld(new Set(heldRef.current));
      if (isNative) {
        void dispatchNative({
          type: 'midi',
          status: on ? 0x90 : 0x80,
          data1: midiNumber,
          data2: on ? 100 : 0,
          note_id: midiNumber,
        });
      } else {
        void sendLegacy({ type: 'midi', status: on ? 0x90 : 0x80, data1: midiNumber, data2: on ? 100 : 0 });
      }
    },
    [isNative, dispatchNative, sendLegacy],
  );

  const panic = useCallback(() => {
    heldRef.current.clear();
    setHeld(new Set());
    if (isNative) {
      void dispatchNative({ type: 'panic', all_origins: true });
    } else {
      void sendLegacy({ type: 'panic' });
    }
  }, [isNative, dispatchNative, sendLegacy]);

  useEffect(() => {
    window.addEventListener('blur', panic);
    return () => window.removeEventListener('blur', panic);
  }, [panic]);

  // --- Presets (Factory + v2 CRC-32 Rust Codec) ---
  const loadPreset = async (value: string) => {
    const idx = Number(value);
    setSelectedPreset(value);
    if (isNative) {
      setNativeOverrides({});
      setNativeHistory([]);
      setNativeFuture([]);
      const reply = await dispatchNative({ type: 'factoryPreset', index: idx });
      if (reply.ok) {
        const dataValues = Array.isArray(reply.data?.values) ? (reply.data?.values as number[]) : nativeFactoryBank.current.get(idx)?.values;
        const dataGlide = typeof reply.data?.glide === 'boolean' ? reply.data.glide : nativeFactoryBank.current.get(idx)?.glide ?? false;
        if (dataValues && dataValues.length === specs.length) {
          setNativeFactoryRef({ id: idx, values: [...dataValues], glide: dataGlide });
        } else {
          setNativeFactoryRef(null);
        }
        setNotice(reply.message);
      }
      return;
    }
    setFactoryReference(null);
    presetRequest.current = null;
    parameters.end();
    setPresetLoading(true);
    const commandId = transport.nextId();
    const epoch = transport.generation;
    try {
      const submission = await transport.send({ type: 'preset', index: idx }, commandId);
      if (epoch !== transport.generation) return;
      const preset = metadata.presets.find(p => p.id === idx);
      parameters.acceptPresetBarrier(commandId, submission, preset?.values);
      if (transport.supportsAck && submission.acceptedCommandId && preset?.values) {
        presetRequest.current = { id: preset.id, commandId: submission.acceptedCommandId, values: [...preset.values], since: Date.now() };
        setPresetAcceptance(n => n + 1);
      } else {
        setPresetLoading(false);
      }
    } catch (error) {
      if (epoch === transport.generation) {
        setSelectedPreset('');
        setNotice(String(error));
        setPresetLoading(false);
      }
    }
  };

  const exportPreset = async () => {
    if (isNative) {
      const reply = await dispatchNative({ type: 'presetSaveJson', name: presetName, category: 'User' });
      if (reply.ok && reply.presetJson) {
        setPresetModal({ mode: 'export', text: reply.presetJson, error: '' });
        try {
          const blob = new Blob([reply.presetJson], { type: 'application/json' });
          const url = URL.createObjectURL(blob);
          const anchor = document.createElement('a');
          anchor.href = url;
          anchor.download = `${presetName.toLowerCase().replace(/[^a-z0-9]+/g, '-') || 'prophet-patch'}.v2.json`;
          anchor.click();
          URL.revokeObjectURL(url);
        } catch {
          // Embedded WebViews may disallow blob downloads; modal text remains copyable.
        }
      }
      return;
    }
    const blob = new Blob([JSON.stringify({ format: 'PROPHET-XII', version: 1, values }, null, 2)], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = 'prophet-preset.json';
    anchor.click();
    URL.revokeObjectURL(url);
  };

  const importPresetJson = async (text: string) => {
    if (isNative) {
      const reply = await dispatchNative({ type: 'presetLoadJson', json: text });
      if (reply.ok) {
        setNativeOverrides({});
        setNativeFactoryRef(null);
        setPresetModal(null);
        setNotice(reply.message);
      } else {
        setPresetModal(prev => (prev ? { ...prev, error: reply.message } : { mode: 'import', text, error: reply.message }));
        setNotice(reply.message);
      }
      return;
    }
    try {
      const p = JSON.parse(text);
      if (p.format !== 'PROPHET-XII' || p.version !== 1 || !Array.isArray(p.values)) throw Error('Invalid preset');
      parameters.applySnapshot(p.values);
      setFactoryReference(null);
      setPresetModal(null);
    } catch (error) {
      const msg = String(error);
      setNotice(msg);
      setPresetModal(prev => (prev ? { ...prev, error: msg } : null));
    }
  };

  // --- MIDI Learn ---
  const midiAction = async (action: 'start' | 'cancel' | 'remove' | 'replace') => {
    if (midiTarget === null) return;
    if (isNative) {
      if (action === 'start') {
        const r = await dispatchNative({ type: 'midiLearn', action: 'begin', param_id: midiTarget });
        if (r.ok) setNotice(r.message);
      } else if (action === 'cancel') {
        const r = await dispatchNative({ type: 'midiLearn', action: 'cancel' });
        if (r.ok) setNotice(r.message);
      } else if (action === 'remove') {
        const r = await dispatchNative({ type: 'midiLearn', action: 'unlearn', param_id: midiTarget });
        if (r.ok) setNotice(r.message);
      } else if (action === 'replace') {
        if (!Number.isInteger(midiCC) || midiCC < 0 || midiCC > 127 || [64, 66, 120, 123].includes(midiCC)) {
          setNotice('Choose CC 0–127 except reserved CC64, CC66, CC120 and CC123');
          return;
        }
        const r = await dispatchNative({ type: 'midiLearn', action: 'remap', cc: midiCC, param_id: midiTarget });
        if (r.ok) setNotice(r.message);
      }
      return;
    }
    if (action === 'start') {
      if ((await sendLegacy({ type: 'learn', id: midiTarget })) && !capability('midiLearnLifecycle')) {
        setNotice('Legacy host accepted MIDI Learn start. Capture completion, cancel and mapping queries are unavailable.');
      }
    } else if (capability('midiLearnLifecycle')) {
      if (action === 'cancel') await sendLegacy({ type: 'cancel_learn' });
      if (action === 'remove') await sendLegacy({ type: 'unlearn', id: midiTarget });
      if (action === 'replace') {
        if (!Number.isInteger(midiCC) || midiCC < 0 || midiCC > 127 || [64, 120, 123].includes(midiCC)) {
          setNotice('Choose CC 0–127 except reserved CC64, CC120 and CC123');
          return;
        }
        await sendLegacy({ type: 'remap', cc: midiCC, id: midiTarget });
      }
    }
  };

  // --- Web MIDI Connection (Respecting PhysicalMidiSource Arbitration) ---
  const connectWebMidi = async () => {
    try {
      const nav = navigator as unknown as {
        requestMIDIAccess?: () => Promise<{
          inputs: Map<string, { onmidimessage: ((e: { data: Uint8Array }) => void) | null }>;
        }>;
      };
      if (typeof nav.requestMIDIAccess !== 'function') {
        setNotice('Web MIDI API is unavailable in this WebView/browser; use native OS MIDI input.');
        return;
      }
      clearMIDI();
      const generation = midiGeneration.current;
      const midi = await nav.requestMIDIAccess();
      if (generation !== midiGeneration.current) return;
      midiInputs.current = [...midi.inputs.values()];
      midi.inputs.forEach(input => {
        input.onmidimessage = e => {
          const bytes = e.data;
          if (bytes.length >= 2 && bytes[0] >= 0x80 && bytes[0] < 0xf0) {
            if (isNative) {
              void dispatchNative({ type: 'midi', status: bytes[0], data1: bytes[1], data2: bytes[2] ?? 0 });
            } else {
              void sendLegacy({ type: 'midi', status: bytes[0], data1: bytes[1], data2: bytes[2] ?? 0 });
            }
          }
        };
      });
      setNotice(`Web MIDI: ${midi.inputs.size} inputs`);
    } catch (error) {
      setNotice(String(error));
    }
  };

  // --- Zoom handler ---
  const applyZoom = (nextZoom: number) => {
    setZoom(nextZoom);
    if (nativeTransport) {
      nativeTransport.setZoom(nextZoom);
    } else {
      document.documentElement.style.zoom = String(nextZoom);
    }
  };

  // --- Vector4 & Sequencer callbacks ---
  const handleVectorXY = useCallback(
    (x: number, y: number) => {
      const now = performance.now();
      if (now - lastVectorSendMs.current < 14 && (Math.abs(x) > 0.001 || Math.abs(y) > 0.001)) return;
      lastVectorSendMs.current = now;
      void dispatchNative({ type: 'vectorXY', x, y });
    },
    [dispatchNative],
  );

  // --- Derived telemetry & display helpers ---
  const presetList = isNative ? uiSnap?.presets ?? [] : metadata.presets;
  const activeFactoryValues = isNative ? nativeFactoryRef?.values : factoryReference?.values;
  const activeSampleRate = isNative ? uiSnap?.enginePort.sampleRate : frame?.sampleRate;
  const activeVoiceMask = isNative ? uiSnap?.kernelTelemetry.activeVoiceMask ?? 0 : frame?.activeVoiceMask ?? 0;
  const voiceMidiNotes = isNative ? uiSnap?.kernelTelemetry.voiceMidiNotes ?? [] : frame?.voiceMidiNotes ?? [];
  const voiceVcaLevels = isNative ? uiSnap?.kernelTelemetry.voiceVcaLevels ?? [] : frame?.voiceVcaLevels ?? [];
  const voiceFiltEnvLevels = isNative ? uiSnap?.kernelTelemetry.voiceFiltEnvLevels ?? [] : frame?.voiceFiltEnvLevels ?? [];

  const nativeAmpEnvelopes: VoiceEnvelope[] | undefined = useMemo(
    () =>
      uiSnap?.kernelTelemetry.ampEnvelopes.map(e => ({
        stage: Math.min(4, Math.max(0, e.stage)) as EnvelopeStage,
        elapsedSamples: e.elapsedSamples,
        level: e.level,
        progress: e.progress,
        timed: e.timed,
      })),
    [uiSnap?.kernelTelemetry.ampEnvelopes],
  );
  const nativeFilterEnvelopes: VoiceEnvelope[] | undefined = useMemo(
    () =>
      uiSnap?.kernelTelemetry.filterEnvelopes.map(e => ({
        stage: Math.min(4, Math.max(0, e.stage)) as EnvelopeStage,
        elapsedSamples: e.elapsedSamples,
        level: e.level,
        progress: e.progress,
        timed: e.timed,
      })),
    [uiSnap?.kernelTelemetry.filterEnvelopes],
  );

  const midiState = isNative
    ? uiSnap
      ? {
          active: uiSnap.midiLearn.learningTarget !== null,
          target: uiSnap.midiLearn.learningTarget,
          ccMap: uiSnap.midiLearn.ccMap,
        }
      : undefined
    : capability('midiLearnLifecycle')
      ? frame?.midiLearn
      : undefined;

  const ccByParam = useMemo(() => {
    const map = new Map<number, number>();
    if (midiState) {
      midiState.ccMap.forEach((pId, cc) => {
        if (pId !== null && !map.has(pId)) map.set(pId, cc);
      });
    }
    return map;
  }, [midiState]);

  const selectedLearnState = midiState
    ? midiState.active
      ? 'pending'
      : midiTarget !== null && midiState.ccMap.includes(midiTarget)
        ? 'assigned'
        : 'idle'
    : null;

  const reverbDisplay = isNative
    ? uiSnap
      ? {
          enabled: uiSnap.reverbBypassEffective,
          tail: uiSnap.reverbTailEffective,
          dormant: uiSnap.enginePort.fx.dormant,
          frozen: uiSnap.enginePort.fx.frozen,
          algorithm: uiSnap.enginePort.fx.algorithm,
          version: uiSnap.enginePort.fx.version,
          enableGain: uiSnap.reverbBypassEffective ? 1 : 0,
          pending: false,
        }
      : null
    : frame?.reverb && capability('reverbBypass')
      ? {
          enabled: pendingReverb?.enabled ?? frame.reverb.enabled,
          tail: pendingReverb?.tail ?? frame.reverb.tail,
          dormant: frame.reverb.dormant,
          frozen: false,
          algorithm: 'LegacyRC1' as WireAlgorithm,
          version: 1,
          enableGain: frame.reverb.enableGain,
          pending: Boolean(pendingReverb),
        }
      : null;

  const canUndo = isNative ? nativeHistory.length > 0 : legacyState.canUndo;
  const canRedo = isNative ? nativeFuture.length > 0 : legacyState.canRedo;
  const currentOs = Math.round(values[paramId('OversamplingMode')] ?? 2);
  const effectiveAndroidDiag = androidDiag ?? (uiSnap?.diagnostics && 'lifecycle' in uiSnap.diagnostics ? (uiSnap.diagnostics as AndroidAudioDiagnostics) : null);
  const hostBadge = isNative ? (uiSnap?.backend ?? nativeTransport.backendLabel).toUpperCase() : 'HTTP-DEV';

  const voiceOriginMap = useMemo(() => {
    const map = new Map<number, string>();
    if (!uiSnap) return map;
    for (const n of uiSnap.enginePort.notes) {
      if (n.pressed || n.sustained) {
        map.set(n.key, originShortLabel(n.origin));
      }
    }
    return map;
  }, [uiSnap]);

  const switchSlot = (targetSlot: number) => {
    if (isNative) {
      slots.current[slot] = [...valuesRef.current];
      const next = slots.current[targetSlot];
      if (next && next.length === specs.length) {
        beginParam(0);
        next.forEach((v, id) => {
          if (id !== paramId('OversamplingMode') && Math.abs(v - valuesRef.current[id]) > 1e-6) {
            editParam(id, v);
          }
        });
        endParam();
        setNativeFactoryRef(null);
      }
      setSlot(targetSlot);
      return;
    }
    slots.current[slot] = [...parameters.getSnapshot().values];
    const next = slots.current[targetSlot];
    if (next) {
      parameters.applySnapshot(next);
      setFactoryReference(null);
    }
    setSlot(targetSlot);
  };

  return (
    <main data-transport={isNative ? nativeTransport.kind : 'http-dev'}>
      {/* TOP INSTRUMENT HEADER */}
      <header className="top-header">
        <div className="brand-block">
          <small>TWELVE VOICE · VIRTUAL ANALOG · VECTOR4</small>
          <h1>
            PROPHET<span>—XII</span>
          </h1>
        </div>

        <div className="connection">
          <div className="conn-row">
            <b className={connected ? 'online' : ''}>
              ● {connected ? `RUST ENGINE CONNECTED (${hostBadge})` : `Audio host disconnected (${hostBadge})`}
            </b>
            {isNative && uiSnap && (
              <span className="pill subtle">
                {uiSnap.patch.topology} · {uiSnap.enginePort.fx.algorithm}
              </span>
            )}
          </div>
          {isNative && uiSnap ? (
            <span>
              CPU {uiSnap.enginePort.cpuPercent !== null ? `${uiSnap.enginePort.cpuPercent.toFixed(1)}%` : '—'} · XRuns{' '}
              {uiSnap.enginePort.xruns !== null ? uiSnap.enginePort.xruns : '—'} · Latency{' '}
              {uiSnap.enginePort.latencySamples} smp ({currentOs}× OS)
            </span>
          ) : (
            <span>
              DSP {connected ? `${frame?.dspLoadPercent.toFixed(1)}%` : '—'} · deadline misses {frame?.deadlineMisses ?? '—'} ·
              device errors {frame?.underruns ?? '—'}
            </span>
          )}
          <span>
            {activeSampleRate ?? '—'} Hz ·{' '}
            {isNative ? `${uiSnap?.enginePort.renderedFrames ?? '0'} frames · rev ${uiSnap?.enginePort.revision ?? '0'}` : `${frame?.frames ?? 0} rendered frames`}
          </span>
        </div>

        <div className="header-actions">
          <div className="zoom-bar" role="group" aria-label="Interface zoom">
            {ZOOM_LEVELS.map(z => (
              <button
                key={z}
                type="button"
                className={`mini-btn ${Math.abs(zoom - z) < 0.01 ? 'selected' : ''}`}
                aria-pressed={Math.abs(zoom - z) < 0.01}
                onClick={() => applyZoom(z)}
              >
                {`${Math.round(z * 100)}%`}
              </button>
            ))}
          </div>
          <button type="button" className="panic-btn" disabled={!connected || presetLoading} onClick={panic}>
            ALL NOTES OFF
          </button>
        </div>
      </header>

      {/* STATUS & PROTOCOL BAR */}
      <div className="status-strip">
        <div className="control-status" role="status">
          {(!isNative && legacyState.error) ||
            notice ||
            (!isNative && legacyState.pending ? `${legacyState.pending} control changes awaiting host` : '')}
        </div>
        <small className="protocol-status">
          {isNative
            ? `EnginePort v1 (${nativeTransport.kind}) · ${uiSnap?.enginePort.capabilities.length ?? 0} negotiated capabilities`
            : `Protocol ${metadata.protocolVersion}${!capability('parameterAck') ? ' · legacy matching-value confirmation; causal acknowledgments unavailable' : ''}`}
        </small>
      </div>

      {/* PRESET & PERFORMANCE TOOLBAR */}
      <section className="presetbar" aria-label="Preset and host controls">
        <select aria-label="Preset category" value={category} onChange={e => setCategory(e.target.value)}>
          <option>All</option>
          {Array.from(new Set(presetList.map(p => p.category))).map(c => (
            <option key={c}>{c}</option>
          ))}
        </select>

        <input aria-label="Search presets" placeholder="Search presets" value={query} onChange={e => setQuery(e.target.value)} />

        <select
          aria-label="Factory preset"
          disabled={!connected || presetLoading}
          value={selectedPreset}
          onChange={e => void loadPreset(e.target.value)}
        >
          <option value="" disabled>
            Factory presets
          </option>
          {presetList
            .filter(p => (category === 'All' || p.category === category) && p.name.toLowerCase().includes(query.toLowerCase()))
            .map(p => (
              <option value={p.id} key={p.id}>
                {p.name}
              </option>
            ))}
        </select>

        <button type="button" disabled={!connected || !canUndo} onClick={undo}>
          UNDO
        </button>
        <button type="button" disabled={!connected || !canRedo} onClick={redo}>
          REDO
        </button>

        {['A', 'B'].map((name, i) => (
          <button
            type="button"
            disabled={!connected || presetLoading}
            className={slot === i ? 'selected' : ''}
            key={name}
            onClick={() => switchSlot(i)}
          >
            {`SLOT ${name}`}
          </button>
        ))}

        {isNative && (
          <div className="os-group" role="group" aria-label="Oversampling factor">
            {([1, 2, 4] as const).map(f => (
              <button
                key={f}
                type="button"
                className={`mini-btn ${currentOs === f ? 'selected' : ''}`}
                disabled={!connected}
                aria-pressed={currentOs === f}
                onClick={() => editParam(paramId('OversamplingMode'), f)}
              >
                {`${f}× OS`}
              </button>
            ))}
          </div>
        )}

        {isNative && uiSnap && (
          <button
            type="button"
            className={uiSnap.driveMakeup ? 'selected' : ''}
            disabled={!connected}
            aria-pressed={uiSnap.driveMakeup}
            onClick={() => void dispatchNative({ type: 'setDriveMakeup', enabled: !uiSnap.driveMakeup })}
          >
            {`DRIVE MAKEUP ${uiSnap.driveMakeup ? 'ON' : 'OFF'}`}
          </button>
        )}

        <button type="button" disabled={!connected || presetLoading} onClick={() => void exportPreset()}>
          {isNative ? 'EXPORT V2' : 'EXPORT'}
        </button>

        <label className="file">
          IMPORT
          <input
            disabled={!connected || presetLoading}
            type="file"
            accept="application/json"
            onChange={async e => {
              const text = await e.target.files?.[0]?.text();
              if (text) await importPresetJson(text);
              e.target.value = '';
            }}
          />
        </label>

        {isNative && (
          <button
            type="button"
            disabled={!connected}
            onClick={() => setPresetModal({ mode: 'import', text: '', error: '' })}
          >
            PASTE V2 JSON
          </button>
        )}

        {!isNative && (
          <input
            type="password"
            aria-label="Remote access token"
            placeholder="Remote token if required"
            value={token}
            onChange={e => setToken(e.target.value)}
          />
        )}

        {effectiveAndroidDiag && (
          <button type="button" className={diagOpen ? 'selected' : ''} onClick={() => setDiagOpen(d => !d)}>
            ANDROID AUDIO {diagOpen ? '−' : '+'}
          </button>
        )}
      </section>

      {/* V2 PRESET IMPORT/EXPORT MODAL */}
      {presetModal && (
        <section className="preset-modal" role="dialog" aria-label="Preset v2 CRC-32 Codec">
          <div className="modal-head">
            <h2>{presetModal.mode === 'export' ? 'EXPORTED PRESET V2 (CRC-32)' : 'IMPORT PRESET V1/V2 JSON'}</h2>
            <button type="button" className="mini-btn" onClick={() => setPresetModal(null)}>
              CLOSE
            </button>
          </div>
          {presetModal.mode === 'export' && (
            <div className="modal-row">
              <label>
                Patch name
                <input value={presetName} onChange={e => setPresetName(e.target.value)} />
              </label>
              <button type="button" onClick={() => void exportPreset()}>
                RE-ENCODE
              </button>
            </div>
          )}
          <textarea
            aria-label="Preset JSON payload"
            rows={6}
            value={presetModal.text}
            readOnly={presetModal.mode === 'export'}
            placeholder="Paste PROPHET-XII v1/v2 JSON payload..."
            onChange={e => setPresetModal(m => (m ? { ...m, text: e.target.value, error: '' } : null))}
          />
          {presetModal.error && (
            <p className="modal-error" role="alert">
              Validation failed: {presetModal.error}
            </p>
          )}
          <div className="modal-actions">
            {presetModal.mode === 'import' && (
              <button type="button" disabled={!presetModal.text.trim()} onClick={() => void importPresetJson(presetModal.text)}>
                VALIDATE &amp; LOAD VIA RUST CODEC
              </button>
            )}
            <small>Validated off-audio by `prophet_preset` (version, CRC-32 checksum, algorithm &amp; pattern bounds).</small>
          </div>
        </section>
      )}

      {/* ANDROID AUDIO DIAGNOSTICS DRAWER */}
      {diagOpen && effectiveAndroidDiag && (
        <section className="android-diag" aria-label="Android AAudio Diagnostics">
          <div className="diag-grid">
            <span>Lifecycle: <strong>{effectiveAndroidDiag.lifecycle ?? '—'}</strong></span>
            <span>Format: <strong>{effectiveAndroidDiag.format ?? '—'}</strong></span>
            <span>SR: <strong>{effectiveAndroidDiag.sampleRate ?? '—'} Hz</strong></span>
            <span>Burst: <strong>{effectiveAndroidDiag.framesPerBurst ?? '—'}</strong></span>
            <span>XRuns: <strong>{effectiveAndroidDiag.xruns ?? '—'}</strong></span>
            <span>Focus: <strong>{effectiveAndroidDiag.focusState ?? '—'}</strong></span>
            <span>Route: <strong>{effectiveAndroidDiag.route ?? '—'}</strong></span>
            <span>USB MIDI: <strong>{effectiveAndroidDiag.usbMidiDevices?.join(', ') || 'None'}</strong></span>
          </div>
          {effectiveAndroidDiag.lastErrorText && (
            <p className="modal-error">
              Last AAudio error ({effectiveAndroidDiag.lastErrorCode}): {effectiveAndroidDiag.lastErrorText}
            </p>
          )}
          <div className="diag-actions">
            <button type="button" onClick={() => void dispatchNative({ type: 'hostControl', op: 'retryAudio' })}>
              RETRY AAUDIO
            </button>
            <button
              type="button"
              className={effectiveAndroidDiag.devBridgeEnabled ? 'selected' : ''}
              onClick={() =>
                void dispatchNative({
                  type: 'hostControl',
                  op: 'toggleDevBridge',
                  enabled: !effectiveAndroidDiag.devBridgeEnabled,
                })
              }
            >
              {`OPT-IN LAN DEV BRIDGE: ${effectiveAndroidDiag.devBridgeEnabled ? 'ON' : 'OFF'}`}
            </button>
          </div>
        </section>
      )}

      {/* 12-VOICE MATRIX & COMPACT SCOPE STRIP */}
      <section className="diagnostics">
        <div className="voice-bank">
          <small>12 VOICE CARDS</small>
          <div className="leds">
            {Array.from({ length: 12 }, (_, i) => {
              const active = Boolean(activeVoiceMask & (1 << i));
              const midiNote = voiceMidiNotes[i];
              const originTag = midiNote !== undefined && midiNote >= 0 ? voiceOriginMap.get(midiNote) : undefined;
              const releasing = isNative ? Boolean(uiSnap?.enginePort.voices[i]?.releasing) : false;
              return (
                <div
                  key={i}
                  title={`Voice ${i + 1} · ${midiNote !== undefined && midiNote >= 0 ? formatMidiNote(midiNote) : 'idle'}${originTag ? ` · ${originTag}` : ''}`}
                >
                  <i className={`${active ? 'active' : ''} ${releasing ? 'releasing' : ''} ${originTag === 'HW' ? 'hw' : originTag === 'SEQ' ? 'seq' : ''}`} />
                  <span>{String(i + 1).padStart(2, '0')}</span>
                  <em className="voice-tag">{active && originTag ? originTag : active && midiNote >= 0 ? formatMidiNote(midiNote) : '·'}</em>
                </div>
              );
            })}
          </div>
        </div>

        <div className="mini-scope-wrap">
          <canvas ref={scopeCanvasRef} width={280} height={44} aria-label="Live output oscilloscope" />
        </div>

        <div className="diag-right">
          {!isNative && (
            <button type="button" onClick={() => setDrawer(d => !d)}>
              VOICE TRIMPOTS {drawer ? '−' : '+'}
            </button>
          )}
          <button type="button" className={analysisOpen ? 'selected' : ''} onClick={() => setAnalysisOpen(o => !o)}>
            SPECTRUM {analysisOpen ? '−' : '+'}
          </button>
          <p className="learn-hint">
            Right-click knob: MIDI Learn ({midiState ? (midiState.active ? 'ARMED' : 'idle') : 'legacy'})
            {activeFactoryValues ? ' · Double-click / Ctrl+click resets to factory preset' : ''}
          </p>
        </div>
      </section>

      {/* MIDI LEARN DIALOG */}
      {midiTarget !== null && (
        <section className="midi-dialog" role="dialog" aria-label="MIDI Learn">
          <h2>MIDI Learn · {midiTarget === GLIDE_ENABLED_PARAM_ID ? GLIDE_ENABLED_SPEC.name : specs[midiTarget]?.name}</h2>
          <p>
            {midiState
              ? `State: ${selectedLearnState}${midiState.target !== null ? ` · target ${specs[midiState.target]?.name ?? `p${midiState.target}`}` : ''}`
              : 'This host supports starting capture only. Dismiss does not cancel capture.'}
          </p>
          <button type="button" disabled={!connected || presetLoading} onClick={() => void midiAction('start')}>
            Start MIDI Learn
          </button>
          <button
            type="button"
            disabled={!connected || (!isNative && !capability('midiLearnLifecycle'))}
            onClick={() => void midiAction('cancel')}
          >
            Cancel MIDI Learn
          </button>
          <button
            type="button"
            disabled={!connected || (!isNative && !capability('midiLearnLifecycle'))}
            onClick={() => void midiAction('remove')}
          >
            Remove MIDI mapping
          </button>
          <p>Mappings are global across channels. CC64/66/120/123 are reserved; MPE channel CC74 takes precedence.</p>
          <label>
            CC
            <input
              aria-label="MIDI mapping CC"
              disabled={!isNative && !capability('midiLearnLifecycle')}
              type="number"
              min="0"
              max="127"
              value={midiCC}
              onChange={e => setMidiCC(Number(e.target.value))}
            />
          </label>
          <button
            type="button"
            disabled={!connected || (!isNative && !capability('midiLearnLifecycle'))}
            onClick={() => void midiAction('replace')}
          >
            Replace MIDI mapping
          </button>
          <p>
            {midiState
              ? midiState.ccMap.flatMap((id, cc) => (id === midiTarget ? [`CC ${cc}`] : [])).join(' · ') || 'No assigned mapping'
              : 'Cancel, remove, replace and mapping feedback: not supported by this legacy host.'}
          </p>
          <button type="button" onClick={() => setMidiTarget(null)}>
            Dismiss MIDI Learn panel
          </button>
        </section>
      )}

      {/* LEGACY VOICE TRIMPOTS DRAWER */}
      {!isNative && drawer && (
        <section className="calibration">
          {frame?.trimpots.map((t, i) => (
            <fieldset key={i}>
              <legend>VOICE {i + 1}</legend>
              {['A cents', 'B cents', 'Cutoff %', 'Envelope %'].map((label, j) => (
                <label key={j}>
                  {label}
                  <input
                    aria-label={`Voice ${i + 1} ${label}`}
                    disabled={!connected || presetLoading}
                    type="number"
                    step="0.1"
                    value={Number((t[j] * (j >= 2 ? 100 : 1)).toFixed(2))}
                    onChange={e => {
                      const val = Number(e.target.value) / (j >= 2 ? 100 : 1);
                      if (Number.isFinite(val)) void sendLegacy({ type: 'trimpot', voice: i, field: j, value: val });
                    }}
                  />
                </label>
              ))}
            </fieldset>
          ))}
        </section>
      )}

      {/* EXPANDABLE SPECTRUM / CAPTURE DRAWER */}
      {analysisOpen && !isNative && (
        <AnalysisPanel frame={frame} values={values} connected={connected} token={token} metadata={metadata} filterChart={false} />
      )}
      {analysisOpen && isNative && (
        <section className="panel native-spectrum-drawer" aria-label="Oscilloscope and Spectrum Info">
          <div className="panel-head">
            <h2>OUTPUT WAVEFORM &amp; SPECTRUM</h2>
            <span className="pill subtle">{activeSampleRate ?? '—'} Hz · 1024-SAMPLE SCOPE</span>
          </div>
          <p className="muted-note">
            Real-time scope snapshot is streamed directly from `RtHostSlot` with zero TCP sockets. High-resolution
            continuous FFT capture is available in developer `--http-dev` mode (`continuous-capture` extension).
          </p>
        </section>
      )}

      {/* MAIN INSTRUMENT PANEL GRID */}
      <div className="panels">
        {/* Vector4 & Oscillator Topology */}
        <VectorPanel
          patch={
            uiSnap?.patch ?? {
              topology: 'Classic2',
              routes: ['Sum', 'Sum'],
              ratios: [0.5, 1.5],
              position: { x: 0, y: 0 },
              weights: [0.25, 0.25, 0.25, 0.25],
              stages: [
                { target: { x: 0, y: 0 }, millis: 100 },
                { target: { x: 0, y: 0 }, millis: 100 },
                { target: { x: 0, y: 0 }, millis: 100 },
                { target: { x: 0, y: 0 }, millis: 100 },
                { target: { x: 0, y: 0 }, millis: 100 },
              ],
              modulation: [
                { x: 0, y: 0 },
                { x: 0, y: 0 },
                { x: 0, y: 0 },
              ],
              loopHeld: false,
              idleForTopologyChange: true,
            }
          }
          disabled={!connected || presetLoading}
          supported={isNative}
          onVectorXY={handleVectorXY}
          onUpdatePatch={next => {
            void dispatchNative({
              type: 'setPatch',
              topology: next.topology,
              routes: next.routes,
              ratios: next.ratios,
              stages: next.stages,
              modulation: next.modulation,
              loop_held: next.loopHeld,
            });
          }}
        />

        {/* Synthesis Sections following classic Prophet operational hierarchy */}
        {PANEL_ORDER.map(group => {
          const groupSpecs = specs.filter(s => s.group === group);
          const isFilter = group === 'FILTER';
          const isReverb = group === 'REVERB';
          const isPerf = group === 'PERFORMANCE';

          return (
            <section className={`panel ${isReverb || isFilter ? 'wide' : ''}`} key={group}>
              <h2>{group}</h2>

              {group === 'CHORUS' && (
                <div className="chorus-buttons">
                  {[1, 2].map((bit, i) => (
                    <button
                      type="button"
                      disabled={!connected || presetLoading}
                      key={bit}
                      className={Math.round(values[paramId('ChorusMode')]) & bit ? 'selected' : ''}
                      aria-pressed={Boolean(Math.round(values[paramId('ChorusMode')]) & bit)}
                      onClick={() => editParam(paramId('ChorusMode'), Math.round(valuesRef.current[paramId('ChorusMode')]) ^ bit)}
                    >
                      CHORUS {i === 0 ? 'I' : 'II'}
                    </button>
                  ))}
                </div>
              )}

              {isReverb && (
                <div className="reverb-status">
                  <div className="reverb-top-controls">
                    <button
                      type="button"
                      disabled={!connected || !reverbDisplay}
                      onClick={() => reverbDisplay && void updateReverb({ enabled: !reverbDisplay.enabled })}
                    >
                      {reverbDisplay ? `Reverb ${reverbDisplay.enabled ? 'ON' : 'BYPASSED'}` : 'Reverb bypass unavailable'}
                    </button>

                    <label>
                      Tail policy
                      <select
                        aria-label="Reverb tail policy"
                        disabled={!connected || !reverbDisplay}
                        value={reverbDisplay?.tail ?? 'spill'}
                        onChange={e => reverbDisplay && void updateReverb({ tail: e.target.value as 'kill' | 'spill' })}
                      >
                        <option value="spill">Spill</option>
                        <option value="kill">Kill</option>
                      </select>
                    </label>

                    {isNative && reverbDisplay && (
                      <div className="reverb-alg-group" role="group" aria-label="Reverb algorithm">
                        {(['LegacyRC1', 'DiffuseV1'] as const).map(alg => {
                          const selected = reverbDisplay.algorithm === alg;
                          const locked = !selected && !reverbDisplay.dormant;
                          return (
                            <button
                              key={alg}
                              type="button"
                              className={`seg-btn ${selected ? 'selected' : ''}`}
                              disabled={!connected || locked}
                              aria-pressed={selected}
                              title={
                                locked
                                  ? 'Reverb tank tail is active; wait until dormant before switching algorithms'
                                  : alg === 'DiffuseV1'
                                    ? 'Experimental candidate (not yet perceptually approved)'
                                    : 'Verified default RC1 reverb algorithm'
                              }
                              onClick={() => {
                                if (!selected) void updateReverb({ algorithm: alg });
                              }}
                            >
                              {alg === 'DiffuseV1' ? 'DiffuseV1 (Exp)' : 'LegacyRC1'}
                            </button>
                          );
                        })}
                      </div>
                    )}
                  </div>

                  {isNative && reverbDisplay && !reverbDisplay.dormant && (
                    <div className="inline-guard" role="status">
                      Tank active — algorithm switching unlocks once the reverb tail becomes dormant.
                    </div>
                  )}

                  <p>
                    {reverbDisplay
                      ? `Tank ${reverbDisplay.dormant ? 'dormant' : 'processing'} · ${reverbDisplay.tail} · enable gain ${reverbDisplay.enableGain.toFixed(3)}${reverbDisplay.pending ? ' · updating' : ''}${isNative ? ` · ${reverbDisplay.algorithm} v${reverbDisplay.version}${reverbDisplay.frozen ? ' · FROZEN' : ''}` : ''}`
                      : 'Tail status and independent bypass are not supported by this host.'}
                  </p>
                  <p className="reverb-note">
                    Wet controls dry/wet mix; zero is dry, not a bypass. Ensemble Off disables only Ensemble. Decay calibrates neutral RT60; artistic FX alter perceived tail.
                  </p>
                </div>
              )}

              {(group === 'AMPLIFIER ENVELOPE' || group === 'FILTER ENVELOPE') && (
                <Envelope
                  values={values}
                  amp={group.startsWith('AMP')}
                  sampleRate={activeSampleRate ?? 48000}
                  activeMask={activeVoiceMask}
                  voices={
                    isNative
                      ? group.startsWith('AMP')
                        ? nativeAmpEnvelopes
                        : nativeFilterEnvelopes
                      : capability('voiceEnvelopes')
                        ? group.startsWith('AMP')
                          ? frame?.ampEnvelopes
                          : frame?.filterEnvelopes
                        : undefined
                  }
                  levels={group.startsWith('AMP') ? voiceVcaLevels : voiceFiltEnvLevels}
                  disabled={!connected || presetLoading}
                  onChange={editParam}
                  onBegin={beginParam}
                  onEnd={endParam}
                />
              )}

              {isFilter ? (
                <div className="filter-split">
                  <div className="controls">
                    {groupSpecs.map(s => {
                      const reason = inactiveReason(s, values);
                      return (
                        <Knob
                          key={s.id}
                          spec={s}
                          value={values[s.id] ?? s.min}
                          disabled={!connected || presetLoading || Boolean(reason)}
                          reason={reason}
                          resetValue={activeFactoryValues?.[s.id]}
                          learnedCc={ccByParam.get(s.id)}
                          onChange={editParam}
                          onBegin={beginParam}
                          onEnd={endParam}
                          onLearn={setMidiTarget}
                        />
                      );
                    })}
                  </div>
                  <VcfResponsePlot
                    report={isNative ? nativeFilterReport : null}
                    legacyFilter={!isNative ? legacyFilter : null}
                    status={isNative ? nativeFilterStatus : legacyFilterStatus}
                    cutoffHz={values[paramId('FilterCutoff')] ?? 1200}
                    resonance={values[paramId('FilterResonance')] ?? 0}
                    morph={values[paramId('FilterMorph')] ?? 0}
                    connected={connected}
                    onRefresh={isNative ? triggerNativeFilterMeasure : undefined}
                  />
                </div>
              ) : (
                <div className="controls">
                  {isPerf && isNative && glideEnabledValue !== null && (
                    <Knob
                      key={GLIDE_ENABLED_PARAM_ID}
                      spec={GLIDE_ENABLED_SPEC}
                      value={glideEnabledValue}
                      disabled={!connected || presetLoading}
                      reason=""
                      resetValue={nativeFactoryRef ? (nativeFactoryRef.glide ? 1 : 0) : undefined}
                      learnedCc={ccByParam.get(GLIDE_ENABLED_PARAM_ID)}
                      onChange={editParam}
                      onBegin={beginParam}
                      onEnd={endParam}
                      onLearn={setMidiTarget}
                    />
                  )}
                  {groupSpecs.map(s => {
                    const reason = inactiveReason(s, values);
                    return (
                      <Knob
                        key={s.id}
                        spec={s}
                        value={values[s.id] ?? s.min}
                        disabled={!connected || presetLoading || Boolean(reason)}
                        reason={reason}
                        resetValue={activeFactoryValues?.[s.id]}
                        learnedCc={ccByParam.get(s.id)}
                        onChange={editParam}
                        onBegin={beginParam}
                        onEnd={endParam}
                        onLearn={setMidiTarget}
                      />
                    );
                  })}
                </div>
              )}
            </section>
          );
        })}

        {/* Sample-Accurate 32-Step / 64-Pattern Sequencer & Arpeggiator */}
        <SequencerPanel
          music={isNative ? uiSnap?.music ?? null : null}
          disabled={!connected || presetLoading}
          onMusicControl={cmd => void dispatchNative({ type: 'musicControl', ...cmd })}
        />
      </div>

      {/* PERFORMANCE KEYBOARD */}
      <Keyboard
        disabled={!connected || presetLoading}
        heldLocal={held}
        engineNotes={uiSnap?.enginePort.notes ?? []}
        engineVoices={uiSnap?.enginePort.voices ?? []}
        pitchBend={values[paramId('PitchBend')] ?? 0}
        physicalSource={isNative ? uiSnap?.midiLearn.physicalSource ?? 'native' : null}
        onPitchBendBegin={() => beginParam(paramId('PitchBend'))}
        onPitchBendChange={v => editParam(paramId('PitchBend'), v)}
        onPitchBendEnd={() => {
          editParam(paramId('PitchBend'), 0);
          endParam();
        }}
        onNote={note}
        onConnectWebMidi={() => void connectWebMidi()}
        onSelectPhysicalSource={
          isNative ? (source: PhysicalMidiSource) => void dispatchNative({ type: 'setPhysicalMidiSource', source }) : undefined
        }
      />

      <p className="provenance">
        Independent virtual analog DSP interpretation. Audio is generated by the Rust host. Control mappings alone do not establish audible sensitivity or hardware fidelity.
      </p>
    </main>
  );
}
