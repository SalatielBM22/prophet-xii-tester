// SPDX-License-Identifier: GPL-3.0-only
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import App from './App';
import {
  ORIGIN_HARDWARE_MIDI,
  ORIGIN_LOCAL_UI,
  ORIGIN_MUSIC_SCHEDULER,
  type UiCommand,
  type UiSnapshot,
} from './protocol/enginePort';

function createBaseSnapshot(overrides: Partial<UiSnapshot> = {}): UiSnapshot {
  const parameters = Array.from({ length: 84 }, () => 0);
  parameters[0] = 2; // OscA Wave
  parameters[5] = 0.8; // OscA Level
  parameters[23] = 6400; // FilterCutoff
  parameters[24] = 0.25; // Resonance
  parameters[46] = 0.72; // Master Volume
  parameters[63] = 1; // OversamplingMode 1x
  parameters[79] = 0; // ReverbBypass off
  parameters[80] = 0; // ReverbTailPolicy spill
  parameters[83] = 0; // GlideEnabled off

  return {
    version: 1,
    backend: 'webkit2gtk',
    ready: true,
    enginePort: {
      version: 1,
      revision: '42',
      renderedFrames: '9600',
      sampleRate: 48000,
      maxFrames: 256,
      latencySamples: 0,
      notes: [
        { origin: ORIGIN_HARDWARE_MIDI, channel: 0, key: 60, noteId: 1, pressed: true, sustained: false },
        { origin: ORIGIN_MUSIC_SCHEDULER, channel: 0, key: 64, noteId: 2, pressed: true, sustained: false },
        { origin: ORIGIN_LOCAL_UI, channel: 0, key: 67, noteId: 3, pressed: false, sustained: true },
      ],
      voices: [],
      fx: {
        algorithm: 'LegacyRC1',
        version: 1,
        enabled: true,
        tail: 'spill',
        frozen: false,
        dormant: true,
      },
      capabilities: ['oversampling', 'vector4', 'sequencer', 'preset-v2'],
      xruns: 0,
      cpuPercent: 9.4,
    },
    parameters,
    scope: Array.from({ length: 128 }, (_, i) => Math.sin(i * 0.2) * 0.4),
    patch: {
      topology: 'Vector4',
      routes: ['Sum', 'Frequency'],
      ratios: [1, 2],
      position: { x: 0.25, y: 0.25 },
      weights: [0.375, 0.375, 0.125, 0.125],
      stages: [
        { target: { x: 0, y: 1 }, millis: 0 },
        { target: { x: 1, y: 0 }, millis: 180 },
        { target: { x: 0, y: -1 }, millis: 240 },
        { target: { x: -1, y: 0 }, millis: 180 },
        { target: { x: 0, y: 0 }, millis: 120 },
      ],
      modulation: [
        { x: 0.1, y: -0.1 },
        { x: 0, y: 0 },
        { x: 0, y: 0 },
      ],
      loopHeld: false,
      idleForTopologyChange: true,
    },
    kernelTelemetry: {
      activeVoiceMask: 0b11,
      voiceVcaLevels: [0.8, 0.4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
      voiceFiltEnvLevels: [0.7, 0.3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
      voiceMidiNotes: [60, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
      ampEnvelopes: [],
      filterEnvelopes: [],
      vectorWeights: [[0.375, 0.375, 0.125, 0.125], null, null, null, null, null, null, null, null, null, null, null],
    },
    music: {
      playing: false,
      activePattern: 0,
      patternLen: 16,
      stepIndex: 0,
      tempoMilli: 120000,
      swing: 0,
      arpEnabled: false,
      arpOrder: 'Up',
      arpOctaves: 1,
      arpLatch: false,
      arpSeed: 7,
      currentPattern: {
        index: 0,
        len: 16,
        steps: Array.from({ length: 32 }, (_, idx) => ({
          kind: idx % 2 === 0 ? 'note' : 'rest',
          key: 60 + (idx % 12),
          velocity: 0.85,
          accent: idx === 0,
          glide: false,
          gate: 0.75,
          ratchet: 1,
        })),
      },
    },
    midiLearn: {
      learningTarget: null,
      ccMap: Array(84).fill(null),
      physicalSource: 'native',
    },
    driveMakeup: true,
    reverbBypassEffective: false,
    reverbTailEffective: 'spill',
    presets: [
      { id: 0, name: 'Warm Brass', category: 'Brass', glide: false },
      { id: 1, name: 'Vector Pad', category: 'Pad', glide: true },
    ],
    filterAnalysis: {
      sampleRate: 48000,
      windowSamples: 4096,
      validity: 'Measurable',
      points: [
        { hz: 40, db: 0, phase: 0 },
        { hz: 1000, db: -0.5, phase: -0.6 },
        { hz: 8000, db: -18.4, phase: -2.9 },
      ],
      tailRms: 0.0001,
      relativeScalingError: 0.002,
      reason: 'Linear small-signal transfer response',
    },
    lastNotice: null,
    diagnostics: null,
    ...overrides,
  };
}

describe('App native EnginePort v1 integration', () => {
  const win = window as unknown as Record<string, unknown>;

  afterEach(() => {
    cleanup();
    delete win.__PROPHET_HOST__;
    delete win.ProphetAndroidBridge;
    delete win.Native;
    vi.restoreAllMocks();
  });

  function installEmbeddedHost(initialSnap: UiSnapshot) {
    const dispatched: UiCommand[] = [];
    const setZoom = vi.fn();
    let currentSnap = initialSnap;

    const dispatchAsync = vi.fn(async (cmd: UiCommand) => {
      dispatched.push(cmd);
      if (cmd.type === 'hostControl' && cmd.op === 'getFactoryPresets') {
        const vals = [...currentSnap.parameters];
        vals[23] = 4200; // Confirmed factory FilterCutoff
        return {
          ok: true,
          status: 'applied',
          message: 'Factory presets loaded',
          data: { presets: [{ id: 0, name: 'Warm Brass', values: vals, glide: false }] },
        };
      }
      if (cmd.type === 'presetSaveJson') {
        return {
          ok: true,
          status: 'applied',
          message: 'Exported preset v2',
          presetJson: '{"version":2,"crc32":"deadbeef","name":"Warm Brass"}',
        };
      }
      if (cmd.type === 'presetLoadJson' && cmd.json.includes('bad-crc')) {
        return {
          ok: false,
          status: 'invalid',
          message: 'CRC-32 checksum mismatch',
        };
      }
      return { ok: true, status: 'applied', message: 'ok' };
    });

    win.__PROPHET_HOST__ = {
      backend: 'webkit2gtk',
      dispatchAsync,
      requestSnapshot: () => JSON.stringify(currentSnap),
      setZoom,
      getZoom: () => 1.0,
    };

    const pushSnapshot = (next: UiSnapshot) => {
      currentSnap = next;
      window.dispatchEvent(new CustomEvent('prophet-snapshot', { detail: next }));
    };

    return { dispatched, setZoom, pushSnapshot };
  }

  it('renders clean knobs without intrusive RESET buttons and supports confirmed factory double-click reset', async () => {
    const snap = createBaseSnapshot();
    const { dispatched } = installEmbeddedHost(snap);
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText(/EnginePort v1 \(embedded-host\)/)).toBeTruthy();
    });

    // No intrusive per-knob RESET button or unavailable text
    expect(screen.queryByText(/Factory reset unavailable/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /^RESET$/i })).toBeNull();

    // Wait for getFactoryPresets hydration then double-click Filter Cutoff knob to confirm reset to 4200 Hz
    await waitFor(() => {
      expect(dispatched.some(c => c.type === 'hostControl' && c.op === 'getFactoryPresets')).toBe(true);
    });

    const cutoffSlider = screen.getByRole('slider', { name: /Filter Cutoff/i });
    fireEvent.doubleClick(cutoffSlider);

    await waitFor(() => {
      expect(dispatched).toContainEqual({ type: 'param', id: 23, value: 4200 });
    });
  });

  it('dispatches GlideEnabled (p83), Oversampling (1x/2x/4x), Zoom, and enforces Reverb active-tail lock', async () => {
    const snap = createBaseSnapshot();
    const { dispatched, setZoom, pushSnapshot } = installEmbeddedHost(snap);
    render(<App />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: '4× OS' })).toBeTruthy();
    });

    // Oversampling 4x
    fireEvent.click(screen.getByRole('button', { name: '4× OS' }));
    await waitFor(() => {
      expect(dispatched).toContainEqual({ type: 'setOversampling', factor: 4 });
    });

    // Zoom 125%
    fireEvent.click(screen.getByRole('button', { name: '125%' }));
    expect(setZoom).toHaveBeenCalledWith(1.25);

    // Reverb algorithm switch allowed when dormant === true
    const diffuseBtn = screen.getByRole('button', { name: 'DiffuseV1 (Exp)' });
    expect((diffuseBtn as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(diffuseBtn);
    await waitFor(() => {
      expect(dispatched.some(c => c.type === 'setReverb' && c.algorithm === 'DiffuseV1')).toBe(true);
    });

    // Active-tail lock guard: when dormant === false, switching algorithm is disabled
    const activeTailSnap = createBaseSnapshot({
      enginePort: {
        ...snap.enginePort,
        fx: { ...snap.enginePort.fx, dormant: false },
      },
    });
    pushSnapshot(activeTailSnap);
    await waitFor(() => {
      expect((screen.getByRole('button', { name: 'DiffuseV1 (Exp)' }) as HTMLButtonElement).disabled).toBe(true);
    });
  });

  it('guards topology switch when voices are active and renders VCF Measurable vs SelfOscillating fallback', async () => {
    const snap = createBaseSnapshot();
    const { pushSnapshot } = installEmbeddedHost(snap);
    render(<App />);

    await waitFor(() => {
      expect(screen.getByTestId('vcf-response-plot')).toBeTruthy();
    });

    // Measurable Bode curve SVG present
    expect(screen.getByRole('img', { name: /Adjacent VCF magnitude and phase transfer response/i })).toBeTruthy();
    expect(screen.queryByTestId('vcf-invalid-fallback')).toBeNull();

    // Switch to SelfOscillating + non-idle topology
    pushSnapshot(
      createBaseSnapshot({
        patch: { ...snap.patch, idleForTopologyChange: false },
        filterAnalysis: {
          sampleRate: 48000,
          windowSamples: 4096,
          validity: 'SelfOscillating',
          points: [],
          tailRms: 0.4123,
          relativeScalingError: 0.351,
          reason: 'Filter is self-oscillating; small-signal Bode curve withheld',
        },
      }),
    );

    await waitFor(() => {
      expect(screen.getByTestId('topology-idle-guard')).toBeTruthy();
      expect(screen.getByTestId('vcf-invalid-fallback')).toBeTruthy();
    });

    expect(screen.getByTestId('vcf-response-plot').textContent).toContain('SELF-OSCILLATING');
    expect(screen.getByTestId('vcf-invalid-fallback').textContent).toContain('4.12e-1');
    expect(screen.getByTestId('vcf-invalid-fallback').textContent).toContain('35.10%');
    expect((screen.getByRole('button', { name: 'Classic2' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('drives 32-step Sequencer/Arpeggiator, v2 CRC-32 Preset Modal, and Keyboard origin highlights', async () => {
    const snap = createBaseSnapshot();
    const { dispatched } = installEmbeddedHost(snap);
    render(<App />);

    await waitFor(() => {
      expect(screen.getByTestId('sequencer-step-grid')).toBeTruthy();
    });

    // Sequencer Play sends Rust musicControl start
    fireEvent.click(screen.getByRole('button', { name: /PLAY/i }));
    await waitFor(() => {
      expect(dispatched).toContainEqual({ type: 'musicControl', op: 'start' });
    });

    // Switch to page 4 (steps 25-32)
    fireEvent.click(screen.getByRole('tab', { name: '25–32' }));
    expect(screen.getByLabelText('Step 32 kind')).toBeTruthy();

    // Export v2 CRC-32 preset modal
    fireEvent.click(screen.getByRole('button', { name: 'EXPORT V2' }));
    await waitFor(() => {
      expect(screen.getByRole('dialog', { name: /Preset v2 CRC-32 Codec/i })).toBeTruthy();
      expect((screen.getByLabelText('Preset JSON payload') as HTMLTextAreaElement).value).toContain('deadbeef');
    });
    fireEvent.click(screen.getByRole('button', { name: 'CLOSE' }));

    // Import invalid CRC-32 preset shows error
    fireEvent.click(screen.getByRole('button', { name: 'PASTE V2 JSON' }));
    fireEvent.change(screen.getByLabelText('Preset JSON payload'), {
      target: { value: '{"version":2,"crc32":"bad-crc"}' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'VALIDATE & LOAD VIA RUST CODEC' }));
    await waitFor(() => {
      expect(screen.getByText(/CRC-32 checksum mismatch/i)).toBeTruthy();
    });

    // Keyboard highlights HW, SEQ, and Sustained origins distinctly
    const hwKey = screen.getByRole('button', { name: 'MIDI note 60' });
    const seqKey = screen.getByRole('button', { name: 'MIDI note 64' });
    const susKey = screen.getByRole('button', { name: 'MIDI note 67' });
    expect(hwKey.className).toContain('origin-hw');
    expect(seqKey.className).toContain('origin-seq');
    expect(susKey.className).toContain('sustained');
  });

  it('exposes Android AAudio Diagnostics drawer when ProphetAndroidBridge is present', async () => {
    const snap = createBaseSnapshot({ backend: 'android-aaudio' });
    win.ProphetAndroidBridge = {
      dispatch: vi.fn().mockReturnValue(JSON.stringify({ ok: true, status: 'applied', message: 'ok' })),
      pollSnapshot: () => JSON.stringify(snap),
      diagnostics: () =>
        JSON.stringify({
          lifecycle: 'Running',
          format: 'Float32',
          sampleRate: 48000,
          framesPerBurst: 192,
          xruns: 0,
          focusState: 'AUDIOFOCUS_GAIN',
          route: 'Speaker',
          usbMidiDevices: ['Arturia KeyStep 37'],
          devBridgeEnabled: false,
        }),
    };

    render(<App />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /ANDROID AUDIO/i })).toBeTruthy();
    });

    fireEvent.click(screen.getByRole('button', { name: /ANDROID AUDIO/i }));
    const diag = screen.getByLabelText('Android AAudio Diagnostics');
    expect(diag.textContent).toContain('Running');
    expect(diag.textContent).toContain('192');
    expect(diag.textContent).toContain('Arturia KeyStep 37');
  });
});
