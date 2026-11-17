// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII Plugin Wrapper (`prophet-plugin`) — MIDI/MPE Processor, Preset Manager & IPC Integration
// Reference: NIH-plug VST3/CLAP architecture (https://github.com/robbert-vdh/nih-plug)

mod engine;
mod ownership;
#[path = "../../prophet-dsp-core/src/port.rs"]
pub mod port;
pub use engine::PreparedEngine;
pub use ownership::Note;
pub use prophet_params::ipc_bridge;
pub use prophet_params::presets;

use core::sync::atomic::Ordering;
use ipc_bridge::{IpcUiEvent, ParamId, WebViewIpcBridge};
use presets::{factory_presets, ProphetPreset};
use prophet_dsp::{ProphetXiiKernel, MAX_BLOCK_SIZE};

/// Real-Time Plugin Instance uniting the `ProphetXiiKernel`, `WebViewIpcBridge`,
/// MIDI/MPE Per-Channel Expressive Router, and Calibrated Factory Presets.
pub struct ProphetXiiPlugin {
    pub kernel: ProphetXiiKernel,
    pub ipc: WebViewIpcBridge,
    pub presets: [ProphetPreset; 10],
    pub current_preset_idx: usize,
    ownership: ownership::Ownership,
}

impl Default for ProphetXiiPlugin {
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

impl ProphetXiiPlugin {
    pub fn new(sample_rate: f32) -> Self {
        let presets = factory_presets();
        let mut plugin = Self {
            kernel: ProphetXiiKernel::new(sample_rate),
            ipc: WebViewIpcBridge::default(),
            presets,
            current_preset_idx: 0,
            ownership: Default::default(),
        };
        plugin.load_preset(0);
        plugin
    }

    /// Loads one of the 10 calibrated presets into both the atomic parameter bank and the DSP kernel.
    pub fn load_preset(&mut self, index: usize) {
        let idx = index.min(self.presets.len() - 1);
        self.current_preset_idx = idx;
        let p = &self.presets[idx];
        self.ipc.params.apply_struct_snapshot(
            &p.voice,
            &p.reverb,
            p.chorus_mode as f32,
            p.chorus_wet,
            2.0,
        );
        self.ipc.params.set(
            ParamId::UnisonEnabled,
            if p.unison_enabled { 1.0 } else { 0.0 },
        );
        self.ipc
            .params
            .set(ParamId::UnisonVoices, p.unison_voices as f32);
        self.ipc
            .params
            .set(ParamId::UnisonDetune, p.unison_detune_cents);
        self.ipc.params.set(ParamId::UnisonSpread, 0.85);
        self.sync_parameters();
    }

    /// Processes incoming raw 3-byte MIDI / MPE messages on the Audio Thread with zero allocations.
    pub fn handle_midi_message(&mut self, status: u8, data1: u8, data2: u8) {
        self.midi_owned(0, status, data1, data2, u32::from(data1));
    }

    fn global_controller(&mut self, status: u8, data1: u8, data2: u8) {
        match status & 0xf0 {
            0xb0 => {
                let cc = usize::from(data1);
                let norm_val = f32::from(data2) / 127.0;
                if self.ipc.params.midi_learn_active.load(Ordering::Relaxed) {
                    let target = self
                        .ipc
                        .params
                        .midi_learn_target_param
                        .load(Ordering::Relaxed);
                    self.ipc.params.remap(cc, target as usize);
                }

                let mapped_param = self.ipc.params.cc_map[cc].load(Ordering::Relaxed);
                if mapped_param == (ParamId::FilterCutoff as u32) {
                    // Exponential MIDI CC to Cutoff mapping (20 Hz .. 18,000 Hz)
                    let hz = 20.0 * (900.0f32).powf(norm_val);
                    self.ipc.params.set(ParamId::FilterCutoff, hz);
                } else if mapped_param < (ipc_bridge::NUM_PARAMS as u32) {
                    if let Some(pid) = ParamId::from_index(mapped_param as usize) {
                        let spec = &ipc_bridge::PARAM_SPECS[mapped_param as usize];
                        let v = if spec.logarithmic {
                            spec.min * (spec.max / spec.min).powf(norm_val)
                        } else {
                            spec.min + norm_val * (spec.max - spec.min)
                        };
                        self.ipc
                            .params
                            .set(pid, if spec.integer { v.round() } else { v });
                    }
                }
            }
            0xD0 => {
                // Channel Pressure / Aftertouch (Global on Ch 0, Per-Voice MPE on Ch 1..15)
                let pressure = (data1 as f32) / 127.0;
                self.kernel.voice_params.aftertouch = pressure;
            }
            0xE0 => {
                // 14-bit Pitch Bend
                let raw14 = ((data2 as u16) << 7) | (data1 as u16);
                let norm_bipolar = ((raw14 as f32) - 8192.0) / 8192.0;
                // Master channel +/- 7 semitone bend
                self.kernel.voice_params.pitch_bend_semis = norm_bipolar * 7.0;
                self.ipc.params.set(ParamId::PitchBend, norm_bipolar * 7.0);
            }
            _ => {}
        }
    }

    /// Drains all lock-free UI events from the SPSC ring buffer and renders an audio block.
    pub fn process_audio_block(&mut self, out_l: &mut [f32], out_r: &mut [f32]) {
        while let Some(ev) = self.ipc.ui_to_audio_queue.pop() {
            match ev {
                IpcUiEvent::NoteOn {
                    note,
                    velocity,
                    channel,
                } => {
                    if velocity.is_finite() && (0.0..=1.0).contains(&velocity) && channel < 16 {
                        self.midi_owned(
                            1,
                            0x90 | channel,
                            note,
                            (velocity * 127.0).round() as u8,
                            u32::from(note),
                        );
                    }
                }
                IpcUiEvent::NoteOff { note, channel } => {
                    if channel < 16 {
                        self.midi_owned(1, 0x80 | channel, note, 0, u32::from(note));
                    }
                }
                IpcUiEvent::MpePitchBend { channel, semitones } => {
                    if channel < 16 && semitones.is_finite() {
                        let bend = ((semitones / 48.0 + 1.0) * 8192.0)
                            .round()
                            .clamp(0.0, 16383.0) as u16;
                        self.midi_owned(
                            1,
                            0xe0 | channel,
                            (bend & 127) as u8,
                            (bend >> 7) as u8,
                            0,
                        );
                    }
                }
                IpcUiEvent::MpePressure { channel, pressure } => {
                    if channel < 16 && pressure.is_finite() {
                        self.midi_owned(
                            1,
                            0xd0 | channel,
                            (pressure.clamp(0., 1.) * 127.).round() as u8,
                            0,
                            0,
                        );
                    }
                }
                IpcUiEvent::MpeTimbre { channel, timbre } => {
                    if channel < 16 && timbre.is_finite() {
                        self.midi_owned(
                            1,
                            0xb0 | channel,
                            74,
                            ((timbre + 0.5).clamp(0., 1.) * 127.).round() as u8,
                            0,
                        );
                    }
                }
                IpcUiEvent::SelectPreset { index } => {
                    self.load_preset(index);
                }
                IpcUiEvent::SetUnison {
                    enabled,
                    voices,
                    detune,
                    spread,
                } => {
                    let bank = &self.ipc.params;
                    bank.set(ParamId::UnisonEnabled, f32::from(enabled));
                    bank.set(ParamId::UnisonVoices, voices as f32);
                    bank.set(ParamId::UnisonDetune, detune);
                    bank.set(ParamId::UnisonSpread, spread);
                }
            }
        }

        self.sync_parameters();

        // Process in fixed-size 64-sample sub-blocks
        let total = out_l.len().min(out_r.len());
        let mut offset = 0;
        while offset < total {
            let chunk = (total - offset).min(MAX_BLOCK_SIZE);
            self.kernel.process_block(
                &mut out_l[offset..offset + chunk],
                &mut out_r[offset..offset + chunk],
                chunk,
            );
            offset += chunk;
        }

        // Push telemetry snapshot to UI ring buffer (non-blocking)
        let _ = self
            .ipc
            .audio_to_ui_telemetry
            .push(self.kernel.last_telemetry);
    }
    pub fn sync_parameters(&mut self) {
        // Pull atomic parameters into DSP kernel
        let (chorus_mode, chorus_wet, chorus_noise, os_factor, master_gain) =
            self.ipc.params.read_into_dsp(
                &mut self.kernel.voice_params,
                &mut self.kernel.reverb_params,
            );

        self.kernel.chorus.mode = chorus_mode;
        self.kernel.chorus.wet_mix = chorus_wet;
        self.kernel.chorus.bbd_noise_level = chorus_noise;
        self.kernel.master_gain = master_gain;

        if self.kernel.allocator.voices[0].oversampler.factor != os_factor {
            self.kernel.prepare(self.kernel.sample_rate, os_factor);
        }

        // Sync individual voice trimpots from calibration drawer
        for i in 0..prophet_dsp::voice::NUM_VOICES {
            self.kernel.allocator.voices[i].trimpots.vco_a_cents =
                self.ipc.params.voice_vco_a_cents[i].load();
            self.kernel.allocator.voices[i].trimpots.vco_b_cents =
                self.ipc.params.voice_vco_b_cents[i].load();
            self.kernel.allocator.voices[i].trimpots.vcf_cutoff_pct =
                self.ipc.params.voice_vcf_pct[i].load();
            self.kernel.allocator.voices[i].trimpots.env_rc_pct =
                self.ipc.params.voice_env_rc_pct[i].load();
        }

        let b = &self.ipc.params;
        self.kernel.global_lfo.freq_hz = b.get(ParamId::LfoFreq);
        self.kernel.global_lfo.initial_amount = b.get(ParamId::LfoInitialAmt);
        self.kernel.global_lfo.mod_wheel = b.get(ParamId::ModWheel);
        self.kernel.global_lfo.wave_saw = b.get(ParamId::LfoSaw) >= 0.5;
        self.kernel.global_lfo.wave_tri = b.get(ParamId::LfoTri) >= 0.5;
        self.kernel.global_lfo.wave_square = b.get(ParamId::LfoSquare) >= 0.5;
        let a = &mut self.kernel.allocator;
        let unison = b.get(ParamId::UnisonEnabled) >= 0.5;
        let voices = b.get(ParamId::UnisonVoices).round() as usize;
        let returning_to_poly = a.unison_enabled && !unison;
        let resize = unison && (!a.unison_enabled || a.unison_voices != voices);
        a.configure_unison(
            unison,
            voices,
            b.get(ParamId::UnisonDetune),
            b.get(ParamId::UnisonSpread),
            &self.kernel.voice_params,
        );
        a.note_priority = match b.get(ParamId::NotePriority).round() as u32 {
            1 => prophet_dsp::voice::NotePriority::Low,
            2 => prophet_dsp::voice::NotePriority::High,
            _ => prophet_dsp::voice::NotePriority::Last,
        };
        if resize {
            self.refresh_stack();
        }
        if returning_to_poly {
            self.restore_poly_owners();
        }
    }
    pub fn panic(&mut self) {
        self.ownership = Default::default();
        self.kernel.allocator.all_notes_off();
    }
    /// Events must be sorted, offsets are relative to this block; out-of-range events are rejected.
    pub fn process_events(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        events: &[MidiEvent],
    ) -> Result<(), &'static str> {
        let len = l.len().min(r.len());
        let mut last = 0;
        for e in events {
            if e.offset < last || e.offset >= len {
                return Err("invalid MIDI offset");
            }
            last = e.offset;
        }
        self.sync_parameters();
        let mut pos = 0;
        for e in events {
            if e.offset > pos {
                self.process_audio_block(&mut l[pos..e.offset], &mut r[pos..e.offset]);
            }
            self.handle_midi_message(e.status, e.data1, e.data2);
            self.sync_parameters();
            pos = e.offset;
        }
        if pos < len {
            self.process_audio_block(&mut l[pos..len], &mut r[pos..len]);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MidiEvent {
    pub offset: usize,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

#[path = "../../prophet-dsp-core/src/music.rs"]
pub mod music;
pub mod state;
