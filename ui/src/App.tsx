// SPDX-License-Identifier: GPL-3.0-only
import React, { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import specs from './param-specs.json';
import { Knob } from './controls/Knob';
import { ParameterState } from './controls/parameterState';
import { inactiveReason, paramId } from './controls/parameters';
import { Envelope } from './components/Envelope';
import { Scope } from './components/LegacyMeters';
import { ControlTransport } from './protocol/transport';
import { decodeFrame, decodeMetadata } from './protocol/types';
import type { Command, Frame, Metadata } from './protocol/types';
import './style.css';

const emptyMetadata: Metadata = { protocolVersion: 1, capabilities: [], presets: [] };
type Filter = { hz: number[]; db: number[]; phase: number[] };
export default function App() {
  const [token, setToken] = useState(() => sessionStorage.getItem('prophet-token') || '');
  const transport = useMemo(() => new ControlTransport(token), [token]);
  const parameters = useMemo(() => new ParameterState(transport), [transport]);
  const state = useSyncExternalStore(parameters.subscribe, parameters.getSnapshot);
  const { values } = state;
  const [frame, setFrame] = useState<Frame | null>(null), [connected, setConnected] = useState(false);
  const [metadata, setMetadata] = useState<Metadata>(emptyMetadata), [notice, setNotice] = useState('');
  const [query, setQuery] = useState(''), [category, setCategory] = useState('All'), [drawer, setDrawer] = useState(false);
  const [midiTarget, setMidiTarget] = useState<number | null>(null), [midiChannel, setMidiChannel] = useState(0), [midiCC, setMidiCC] = useState(1);
  const [presetLoading, setPresetLoading] = useState(false);
  const [selectedPreset, setSelectedPreset] = useState(''), [factoryReference, setFactoryReference] = useState<{ id: number; revision: number; values: number[] } | null>(null);
  const selectedPresetRef = useRef('');
  const [slot, setSlot] = useState(0), slots = useRef<Array<number[] | undefined>>([]);
  const [held, setHeld] = useState<Set<number>>(new Set()), heldRef = useRef(new Set<number>());
  const midiInputs = useRef<Array<{ onmidimessage: ((e: { data: Uint8Array }) => void) | null }>>([]);
  const midiGeneration = useRef(0);
  const clearMIDI = useCallback(() => { midiGeneration.current++; midiInputs.current.forEach(input => { input.onmidimessage = null; }); midiInputs.current = []; }, []);
  useEffect(() => clearMIDI, [token, clearMIDI]);
  const [filter, setFilter] = useState<Filter | null>(null);
  const capability = (name: Metadata['capabilities'][number]) => metadata.capabilities.includes(name);
  const send = useCallback(async (command: Command) => {
    const generation = transport.generation;
    try { await transport.send(command); return true; }
    catch (error) { if (generation === transport.generation) setNotice(String(error)); return false; }
  }, [transport]);
  const learn = useCallback((id: number) => setMidiTarget(id), []);
  useEffect(() => {
    sessionStorage.setItem('prophet-token', token);
    let alive = true, socket: WebSocket | null = null, retry: ReturnType<typeof setTimeout> | undefined, raf = 0;
    let latest: Frame | null = null;
    const abort = new AbortController();
    setConnected(false); setFrame(null); setMetadata(emptyMetadata); setFactoryReference(null);
    const open = async () => {
      try {
        const response = await fetch('/api/v1/meta', { signal: abort.signal });
        if (!response.ok) throw Error(`Metadata returned ${response.status}`);
        const meta = decodeMetadata(await response.json());
        if (!alive) return;
        transport.metadata = meta; setMetadata(meta);
      } catch (error) { if (alive) { transport.metadata = emptyMetadata; setMetadata(emptyMetadata); setNotice(String(error)); } }
      if (!alive) return;
      socket = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/ws${transport.auth}`);
      socket.onmessage = event => {
        if (!alive) return;
        try {
          const parsed = decodeFrame(JSON.parse(event.data));
          if (!parsed) { setNotice('Ignored invalid or unsupported telemetry'); return; }
          if (parsed.v !== transport.metadata.protocolVersion) { setNotice('Telemetry protocol does not match host metadata'); return; }
          if (parsed.v === 2 && transport.metadata.sessionId && parsed.sessionId !== transport.metadata.sessionId) { setNotice('Host session changed; reconnect required'); socket?.close(); return; }
          latest = parsed;
          if (!raf) raf = requestAnimationFrame(() => {
            raf = 0; if (!alive || !latest) return;
            const f = latest; latest = null;
            if (parameters.receive(f)) { setFrame(f); setConnected(true); }
          });
        } catch { setNotice('Ignored malformed telemetry'); }
      };
      socket.onclose = () => {
        if (!alive) return;
        latest = null; cancelAnimationFrame(raf); raf = 0;
        parameters.disconnect(); transport.reset(); clearMIDI(); setConnected(false); setFrame(null); setFactoryReference(null);
        heldRef.current.clear(); setHeld(new Set());
        retry = setTimeout(() => void open(), 1000);
      };
      socket.onerror = () => { if (alive) setNotice('WebSocket transport error'); };
    };
    void open();
    const expiration = setInterval(parameters.expire, 200);
    return () => { alive = false; abort.abort(); clearTimeout(retry); clearInterval(expiration); cancelAnimationFrame(raf); socket?.close(); parameters.disconnect(); transport.reset(); };
  }, [token, transport, parameters, clearMIDI]);
  useEffect(() => {
    if (!frame || !metadata.capabilities.includes('factoryPresetValues')) { setFactoryReference(null); return; }
    const preset = metadata.presets.find(p => String(p.id) === selectedPresetRef.current);
    if (preset?.values && preset.referenceRevision !== undefined && frame.presetId === preset.id && frame.factoryPresetRevision === preset.referenceRevision) {
      setFactoryReference(current => current?.id === preset.id && current.revision === preset.referenceRevision ? current : { id: preset.id, revision: preset.referenceRevision!, values: [...preset.values!] });
    } else setFactoryReference(null);
  }, [frame, metadata]);
  const note = useCallback((number: number, on: boolean) => {
    if (on === heldRef.current.has(number)) return;
    if (on) heldRef.current.add(number); else heldRef.current.delete(number);
    setHeld(new Set(heldRef.current));
    void send({ type: 'midi', status: on ? 0x90 : 0x80, data1: number, data2: on ? 100 : 0 });
  }, [send]);
  const panic = useCallback(() => { heldRef.current.clear(); setHeld(new Set()); void send({ type: 'panic' }); }, [send]);
  useEffect(() => { window.addEventListener('blur', panic); return () => window.removeEventListener('blur', panic); }, [panic]);
  const signature = ['FilterCutoff', 'FilterResonance', 'FilterDrive', 'FilterMorph', 'MoogBassComp'].map(key => values[paramId(key)]).join(',');
  useEffect(() => {
    if (!connected) { setFilter(null); return; }
    let alive = true;
    const abort = new AbortController();
    const timer = setTimeout(() => {
      fetch('/api/v1/filter' + transport.auth, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(parameters.getSnapshot().values), signal: abort.signal })
        .then(r => { if (!r.ok) throw Error('Filter response unavailable'); return r.json(); })
        .then((data: Filter) => { if (alive && Array.isArray(data.hz) && data.hz.length === data.db?.length && data.db.length === data.phase?.length && [...data.hz, ...data.db, ...data.phase].every(Number.isFinite)) setFilter(data); })
        .catch(() => { if (alive) setFilter(null); });
    }, 180);
    return () => { alive = false; clearTimeout(timer); abort.abort(); };
  }, [signature, connected, token, transport, parameters]);
  const loadPreset = async (value: string) => {
    selectedPresetRef.current = value; setSelectedPreset(value); setFactoryReference(null);
    parameters.end(); setPresetLoading(true);
    const commandId = transport.nextId();
    try {
      await transport.send({ type: 'preset', index: Number(value) }, commandId);
      parameters.acceptPresetBarrier(commandId);
    } catch (error) { selectedPresetRef.current = ''; setSelectedPreset(''); setNotice(String(error)); }
    finally { setPresetLoading(false); }
  };
  const midiAction = async (action: 'start' | 'cancel' | 'remove' | 'replace' | 'query') => {
    if (midiTarget === null) return;
    if (capability('midiLearnLifecycle')) {
      if (!Number.isInteger(midiChannel) || midiChannel < 0 || midiChannel > 15 || !Number.isInteger(midiCC) || midiCC < 0 || midiCC > 127) { setNotice('MIDI channel must be 0–15; CC must be 0–127'); return; }
      await send({ type: 'midi_learn', action, parameterId: midiTarget, ...(action === 'replace' ? { channel: midiChannel, cc: midiCC } : {}) });
    } else if (action === 'start' && await send({ type: 'learn', id: midiTarget })) setNotice('Host v1 accepted MIDI Learn start. Capture completion, cancel and mapping queries are unavailable.');
  };
  const groups = Array.from(new Set(specs.map(s => s.group)));
  const midiState = capability('midiLearnLifecycle') ? frame?.midiLearn : undefined;
  return <main>
    <header><div><small>TWELVE VOICE · VIRTUAL ANALOG</small><h1>PROPHET<span>—XII</span></h1></div><div className="connection"><b className={connected ? 'online' : ''}>● {connected ? 'RUST ENGINE CONNECTED' : 'Audio host disconnected'}</b><span>DSP {connected ? frame?.dspLoadPercent.toFixed(1) + '%' : '—'} · deadline misses {frame?.deadlineMisses ?? '—'} · device errors {frame?.underruns ?? '—'}</span><span>{frame?.sampleRate ?? '—'} Hz · {frame?.frames ?? 0} rendered frames</span></div><button disabled={!connected || presetLoading} onClick={panic}>ALL NOTES OFF</button></header>
    <div className="control-status" role="status">{state.error || notice || (state.pending ? `${state.pending} control changes awaiting host` : '')}</div>
    <small className="protocol-status">Protocol {metadata.protocolVersion}{!capability('parameterAck') && ' · v1 matching-value confirmation; causal acknowledgments unavailable'}</small>
    <section className="presetbar"><select aria-label="Preset category" value={category} onChange={e => setCategory(e.target.value)}><option>All</option>{Array.from(new Set(metadata.presets.map(p => p.category))).map(c => <option key={c}>{c}</option>)}</select><input aria-label="Search presets" placeholder="Search presets" value={query} onChange={e => setQuery(e.target.value)} /><select aria-label="Factory preset" disabled={!connected || presetLoading} value={selectedPreset} onChange={e => void loadPreset(e.target.value)}><option value="" disabled>Factory presets</option>{metadata.presets.filter(p => (category === 'All' || p.category === category) && p.name.toLowerCase().includes(query.toLowerCase())).map(p => <option value={p.id} key={p.id}>{p.name}</option>)}</select><button disabled={!connected || !state.canUndo} onClick={parameters.undo}>UNDO</button><button disabled={!connected || !state.canRedo} onClick={parameters.redo}>REDO</button>{['A', 'B'].map((name, i) => <button disabled={!connected || presetLoading} className={slot === i ? 'selected' : ''} key={name} onClick={() => { slots.current[slot] = [...parameters.getSnapshot().values]; const next = slots.current[i]; if (next) parameters.applySnapshot(next); setSlot(i); }}>{`SLOT ${name}`}</button>)}<button disabled={!connected || presetLoading} onClick={() => { const blob = new Blob([JSON.stringify({ format: 'PROPHET-XII', version: 1, values }, null, 2)], { type: 'application/json' }), url = URL.createObjectURL(blob), anchor = document.createElement('a'); anchor.href = url; anchor.download = 'prophet-preset.json'; anchor.click(); URL.revokeObjectURL(url); }}>EXPORT</button><label className="file">IMPORT<input disabled={!connected || presetLoading} type="file" accept="application/json" onChange={async e => { try { const text = await e.target.files?.[0]?.text(); if (!text) return; const p = JSON.parse(text); if (p.format !== 'PROPHET-XII' || p.version !== 1 || !Array.isArray(p.values)) throw Error('Invalid preset'); parameters.applySnapshot(p.values); setFactoryReference(null); } catch (error) { setNotice(String(error)); } }} /></label><input type="password" aria-label="Remote access token" placeholder="Remote token if required" value={token} onChange={e => setToken(e.target.value)} /></section>
    <small>{factoryReference ? 'Double-click a knob to restore its confirmed factory-preset value.' : 'Factory reset unavailable: host does not provide confirmed factory preset values.'}</small>
    <section className="diagnostics"><div><small>VOICE CARDS</small><div className="leds">{Array.from({ length: 12 }, (_, i) => <div key={i} title={`Voice ${i + 1} · MIDI ${frame?.voiceMidiNotes[i] ?? '—'}`}><i className={frame && (frame.activeVoiceMask & (1 << i)) ? 'active' : ''} /><span>{String(i + 1).padStart(2, '0')}</span></div>)}</div></div><button onClick={() => setDrawer(!drawer)}>VOICE TRIMPOTS {drawer ? '−' : '+'}</button><p>Right-click a control for MIDI Learn. {midiState ? `Host learn state: ${midiState.state}` : 'MIDI mappings and capture completion unavailable on v1.'}</p></section>
    {midiTarget !== null && <section className="midi-dialog" role="dialog" aria-label="MIDI Learn"><h2>MIDI Learn · {specs[midiTarget].name}</h2><p>{midiState ? `State: ${midiState.state}` : 'This host supports starting capture only. Dismiss does not cancel capture.'}</p><button disabled={!connected || presetLoading} onClick={() => void midiAction('start')}>Start MIDI Learn</button><button disabled={!connected || !capability('midiLearnLifecycle')} onClick={() => void midiAction('cancel')}>Cancel MIDI Learn</button><button disabled={!connected || !capability('midiLearnLifecycle')} onClick={() => void midiAction('remove')}>Remove MIDI mapping</button><button disabled={!connected || !capability('midiLearnLifecycle')} onClick={() => void midiAction('query')}>Query MIDI mappings</button><label>Channel (0–15)<input aria-label="MIDI mapping channel" disabled={!capability('midiLearnLifecycle')} type="number" min="0" max="15" value={midiChannel} onChange={e => setMidiChannel(Number(e.target.value))} /></label><label>CC<input aria-label="MIDI mapping CC" disabled={!capability('midiLearnLifecycle')} type="number" min="0" max="127" value={midiCC} onChange={e => setMidiCC(Number(e.target.value))} /></label><button disabled={!connected || !capability('midiLearnLifecycle')} onClick={() => void midiAction('replace')}>Replace MIDI mapping</button><p>{midiState ? midiState.bindings.filter(b => b.parameterId === midiTarget).map(b => `Channel ${b.channel}, CC ${b.cc}`).join(' · ') || 'No assigned mapping' : 'Cancel, remove, replace and query: not supported by protocol v1.'}</p><button onClick={() => setMidiTarget(null)}>Dismiss MIDI Learn panel</button></section>}
    {drawer && <section className="calibration">{frame?.trimpots.map((t, i) => <fieldset key={i}><legend>VOICE {i + 1}</legend>{['A cents', 'B cents', 'Cutoff %', 'Envelope %'].map((label, j) => <label key={j}>{label}<input aria-label={`Voice ${i + 1} ${label}`} disabled={!connected || presetLoading} type="number" step="0.1" value={Number((t[j] * (j >= 2 ? 100 : 1)).toFixed(2))} onChange={e => { const value = Number(e.target.value) / (j >= 2 ? 100 : 1); if (Number.isFinite(value)) void send({ type: 'trimpot', voice: i, field: j, value }); }} /></label>)}</fieldset>)}</section>}
    <section className="meters"><Scope frame={frame} /><div className="filterchart"><small>MEASURED RUST VCF · SMALL SIGNAL · FIXED 48 kHz (legacy endpoint)</small><svg viewBox="0 0 500 150"><path d={filter ? 'M' + filter.db.map((db, i) => `${i * 500 / filter.db.length},${Math.max(5, Math.min(145, 45 - db))}`).join(' L') : ''} fill="none" stroke="#e9b95e" strokeWidth="2" /><path d={filter ? 'M' + filter.phase.map((phase, i) => `${i * 500 / filter.phase.length},${80 - phase * 15}`).join(' L') : ''} fill="none" stroke="#80cdae" strokeWidth="1" /></svg><span>20 Hz — 20 kHz · amber magnitude · green phase</span></div></section>
    <div className="panels">{groups.map(group => <section className={'panel ' + (group === 'REVERB' ? 'wide' : '')} key={group}><h2>{group}</h2>
      {group === 'CHORUS' && <div className="chorus-buttons">{[1, 2].map((bit, i) => <button disabled={!connected || presetLoading} key={bit} className={(Math.round(values[paramId('ChorusMode')]) & bit) ? 'selected' : ''} aria-pressed={Boolean(Math.round(values[paramId('ChorusMode')]) & bit)} onClick={() => parameters.edit(paramId('ChorusMode'), Math.round(parameters.getSnapshot().values[paramId('ChorusMode')]) ^ bit)}>CHORUS {i === 0 ? 'I' : 'II'}</button>)}</div>}
      {group === 'REVERB' && <div className="reverb-status"><button disabled={!connected || !capability('reverbBypass') || !frame?.reverb} onClick={() => frame?.reverb && void send({ type: 'reverb_bypass', enabled: !frame.reverb.enabled, tailPolicy: frame.reverb.tailPolicy })}>{capability('reverbBypass') && frame?.reverb ? `Reverb ${frame.reverb.enabled ? 'ON' : 'BYPASSED'}` : 'Reverb bypass unavailable'}</button><label>Tail policy<select aria-label="Reverb tail policy" disabled={!connected || !capability('reverbBypass') || !frame?.reverb} value={frame?.reverb?.tailPolicy ?? 'spill'} onChange={e => frame?.reverb && void send({ type: 'reverb_bypass', enabled: frame.reverb.enabled, tailPolicy: e.target.value as 'kill' | 'spill' })}><option value="spill">Spill</option><option value="kill">Kill</option></select></label><p>{frame?.reverb && capability('reverbBypass') ? `Tail: ${frame.reverb.tailStatus} · ${frame.reverb.transition}` : 'Tail status and independent bypass are not supported by this host.'}</p><p>Wet controls dry/wet mix; zero is dry, not a bypass. Ensemble Off disables only Ensemble. Decay is uncalibrated; it is not room size.</p></div>}
      {(group === 'AMPLIFIER ENVELOPE' || group === 'FILTER ENVELOPE') && <Envelope values={values} amp={group.startsWith('AMP')} sampleRate={frame?.sampleRate ?? 48000} voices={capability('voiceEnvelopes') ? (group.startsWith('AMP') ? frame?.envelopes?.amp : frame?.envelopes?.filter) : undefined} levels={(group.startsWith('AMP') ? frame?.voiceVcaLevels : frame?.voiceFiltEnvLevels) ?? []} disabled={!connected || presetLoading} onChange={parameters.edit} onBegin={parameters.begin} onEnd={parameters.end} />}
      <div className="controls">{specs.filter(s => s.group === group).map(s => { const reason = inactiveReason(s, values); return <Knob key={s.id} spec={s} value={values[s.id] ?? s.min} disabled={!connected || presetLoading || Boolean(reason)} reason={reason} resetValue={factoryReference?.values[s.id]} onChange={parameters.edit} onBegin={parameters.begin} onEnd={parameters.end} onLearn={learn} />; })}</div></section>)}</div>
    <footer><div className="performance"><label>PITCH BEND<input disabled={!connected || presetLoading} aria-label="Pitch bend" type="range" min="-7" max="7" step="0.01" value={values[paramId('PitchBend')] ?? 0} onPointerDown={() => parameters.begin(paramId('PitchBend'))} onChange={e => parameters.edit(paramId('PitchBend'), Number(e.target.value))} onPointerUp={() => { parameters.edit(paramId('PitchBend'), 0); parameters.end(); }} onPointerCancel={() => { parameters.edit(paramId('PitchBend'), 0); parameters.end(); }} onBlur={() => { parameters.edit(paramId('PitchBend'), 0); parameters.end(); }} /></label><button disabled={!connected || presetLoading} onClick={async () => { try { const nav = navigator as unknown as { requestMIDIAccess: () => Promise<{ inputs: Map<string, { onmidimessage: ((e: { data: Uint8Array }) => void) | null }> }> }; clearMIDI(); const generation = midiGeneration.current; const midi = await nav.requestMIDIAccess(); if (generation !== midiGeneration.current) return; midiInputs.current = [...midi.inputs.values()]; midi.inputs.forEach(input => { input.onmidimessage = e => { const bytes = e.data; if (bytes.length >= 2 && bytes[0] >= 0x80 && bytes[0] < 0xf0) void send({ type: 'midi', status: bytes[0], data1: bytes[1], data2: bytes[2] ?? 0 }); }; }); setNotice(`Web MIDI: ${midi.inputs.size} inputs`); } catch (error) { setNotice(String(error)); } }}>CONNECT WEB MIDI</button><small>C2 — C7 · velocity 100 · Space/Enter plays focused key</small></div><div className="keyboard">{Array.from({ length: 61 }, (_, i) => { const n = 36 + i, black = [1, 3, 6, 8, 10].includes(n % 12); return <button key={n} disabled={!connected || presetLoading} aria-label={`MIDI note ${n}`} aria-pressed={held.has(n)} className={`${black ? 'black' : 'white'} ${held.has(n) ? 'held' : ''}`} onPointerDown={e => { if (e.button !== 0) return; e.preventDefault(); e.currentTarget.setPointerCapture(e.pointerId); note(n, true); }} onPointerUp={() => note(n, false)} onPointerCancel={() => note(n, false)} onLostPointerCapture={() => note(n, false)} onKeyDown={e => { if ([' ', 'Enter'].includes(e.key)) { e.preventDefault(); if (!e.repeat) note(n, true); } }} onKeyUp={e => { if ([' ', 'Enter'].includes(e.key)) { e.preventDefault(); note(n, false); } }} onBlur={() => note(n, false)}>{n % 12 === 0 ? `C${Math.floor(n / 12) - 1}` : ''}</button>; })}</div></footer>
    <p className="provenance">Independent virtual analog DSP interpretation. Audio is generated by the Rust host. Control mappings alone do not establish audible sensitivity or hardware fidelity.</p>
  </main>;
}
