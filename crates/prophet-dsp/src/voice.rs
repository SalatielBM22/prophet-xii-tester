// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII DSP Kernel — 12-Voice Allocator, Poly-Mod Matrix, Glide & Vintage Dispersion
// Mathematical grounding:
// - Sequential Prophet-10 Rev3 12-voice circular board rotation & Release-priority stealing
// - 1.5 ms anti-click linear/cosine crossfade on voice steal
// - OB-Xd deterministic per-voice component tolerance modeling (https://github.com/2DaT/Obxd)

use crate::env_cem3310::{Cem3310Envelope, EnvStage};
use crate::osc_cem3340::{Cem3340DualVco, OscAWaveforms, OscBWaveforms, ProphetNoiseGen};
use crate::oversample::{OversampleFactor, VoiceOversampler};
use crate::vcf_moog::DualVcfStage;

pub const NUM_VOICES: usize = 12;

/// Selectable Note Priority for Unison / Mono modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotePriority {
    Last,
    Low,
    High,
}

/// Portamento / Polyphonic Glide Law.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlideMode {
    ConstantRate,
    ConstantTime,
}

/// Per-Voice Trimpot Offsets (accessible from the 12-Voice Diagnostic & Calibration Drawer).
#[derive(Clone, Copy, Debug)]
pub struct VoiceTrimpots {
    /// Oscillator A pitch offset in cents (-3.0 .. +3.0 nominal at Vintage = 1.0)
    pub vco_a_cents: f32,
    /// Oscillator B pitch offset in cents (-3.0 .. +3.0 nominal at Vintage = 1.0)
    pub vco_b_cents: f32,
    /// Filter cutoff multiplier offset (-0.02 .. +0.02 nominal, i.e. +/- 2%)
    pub vcf_cutoff_pct: f32,
    /// Envelope RC time constant offset (-0.04 .. +0.04 nominal, i.e. +/- 4%)
    pub env_rc_pct: f32,
    /// VCA gain trim offset (-0.04 .. +0.04 nominal)
    pub vca_gain_trim: f32,
    /// Voice stereo pan position (-1.0 Left .. +1.0 Right)
    pub pan: f32,
}

impl VoiceTrimpots {
    /// Deterministic hardware board dispersion seeded uniquely for each of the 12 voice cards.
    pub fn from_voice_index(voice_idx: usize) -> Self {
        // Deterministic LCG/Murmur-style hash per voice card (1..=12)
        let mut h = ((voice_idx as u32) + 1).wrapping_mul(0x9E37_79B9);
        let mut next_bipolar = || -> f32 {
            h ^= h << 13;
            h ^= h >> 17;
            h ^= h << 5;
            h = h.wrapping_mul(0x85EB_CA6B);
            ((h as f32) / (u32::MAX as f32)) * 2.0 - 1.0
        };
        let pan_positions: [f32; NUM_VOICES] = [
            -0.55, 0.55, -0.35, 0.35, -0.75, 0.75, -0.15, 0.15, -0.65, 0.65, -0.25, 0.25,
        ];
        Self {
            vco_a_cents: next_bipolar() * 3.0,
            vco_b_cents: next_bipolar() * 3.0,
            vcf_cutoff_pct: next_bipolar() * 0.02,
            env_rc_pct: next_bipolar() * 0.04,
            vca_gain_trim: next_bipolar() * 0.035,
            pan: pan_positions[voice_idx % NUM_VOICES],
        }
    }
}

/// Prophet-10 Rev3 Audio-Rate Poly-Mod Matrix Parameters.
#[derive(Clone, Copy, Debug)]
pub struct PolyModParams {
    /// Filter Envelope -> poly-mod bus amount (-1.0 .. +1.0, bipolar/unipolar)
    pub src_filter_env: f32,
    /// True = Filter Env is bipolar (-0.5 .. +0.5), False = unipolar (0.0 .. +1.0)
    pub filter_env_bipolar: bool,
    /// Oscillator B -> poly-mod bus amount (0.0 .. 1.0, audio-rate FM / cross-mod)
    pub src_osc_b: f32,
    /// Poly-Mod Bus -> Osc A Frequency (in octaves, up to +/- 4 octaves)
    pub dest_osc_a_freq: f32,
    /// Poly-Mod Bus -> Osc A Pulse Width (0.0 .. 1.0)
    pub dest_osc_a_pw: f32,
    /// Poly-Mod Bus -> Filter Cutoff (in octaves, up to +/- 5 octaves)
    pub dest_filter_cutoff: f32,
}

impl Default for PolyModParams {
    fn default() -> Self {
        Self {
            src_filter_env: 0.0,
            filter_env_bipolar: false,
            src_osc_b: 0.0,
            dest_osc_a_freq: 0.0,
            dest_osc_a_pw: 0.0,
            dest_filter_cutoff: 0.0,
        }
    }
}

/// Global Voice Parameters shared across all 12 voices during a processing block.
#[derive(Clone, Copy, Debug)]
pub struct SynthVoiceParams {
    pub osc_a_semi: f32,
    pub osc_a_fine_cents: f32,
    pub osc_a_pw: f32,
    pub osc_a_waves: OscAWaveforms,
    pub osc_a_sync: bool,

    pub osc_b_semi: f32,
    pub osc_b_fine_cents: f32,
    pub osc_b_pw: f32,
    pub osc_b_waves: OscBWaveforms,

    pub mix_osc_a: f32,
    pub mix_osc_b: f32,
    pub mix_noise: f32,
    pub noise_pink_blend: f32,

    pub filter_cutoff_hz: f32,
    pub filter_resonance: f32,
    pub filter_drive: f32,
    pub filter_env_amount: f32,
    /// Keyboard tracking: 0.0 (Off), 0.5 (Half), 1.0 (Full)
    pub filter_key_track: f32,
    /// 0.0 = CEM3320 (Prophet Rev3), 1.0 = Moog Transistor Ladder 24 dB/oct
    pub filter_topology_morph: f32,
    pub moog_bass_comp: f32,

    pub filt_attack: f32,
    pub filt_decay: f32,
    pub filt_sustain: f32,
    pub filt_release: f32,

    pub amp_attack: f32,
    pub amp_decay: f32,
    pub amp_sustain: f32,
    pub amp_release: f32,

    pub poly_mod: PolyModParams,
    /// Macro Vintage Dispersion Knob (0.0 = factory trimmed, 0.35 = Prophet Rev3, 1.0 = Rev1 unstable)
    pub vintage_amount: f32,
    pub glide_time_sec: f32,
    pub glide_mode: GlideMode,
    pub pitch_bend_semis: f32,
    pub aftertouch: f32,
    pub velocity_sensitivity: f32,
    pub lfo_routes: [bool; 4],
}

impl Default for SynthVoiceParams {
    fn default() -> Self {
        Self {
            osc_a_semi: 0.0,
            osc_a_fine_cents: 0.0,
            osc_a_pw: 0.5,
            osc_a_waves: OscAWaveforms::default(),
            osc_a_sync: false,
            osc_b_semi: 0.0,
            osc_b_fine_cents: 7.0,
            osc_b_pw: 0.5,
            osc_b_waves: OscBWaveforms::default(),
            mix_osc_a: 0.8,
            mix_osc_b: 0.6,
            mix_noise: 0.0,
            noise_pink_blend: 0.7,
            filter_cutoff_hz: 2400.0,
            filter_resonance: 0.25,
            filter_drive: 1.1,
            filter_env_amount: 0.45,
            filter_key_track: 0.5,
            filter_topology_morph: 0.0,
            moog_bass_comp: 0.5,
            filt_attack: 0.015,
            filt_decay: 0.45,
            filt_sustain: 0.35,
            filt_release: 0.60,
            amp_attack: 0.005,
            amp_decay: 0.35,
            amp_sustain: 0.80,
            amp_release: 0.45,
            poly_mod: PolyModParams::default(),
            vintage_amount: 0.35,
            glide_time_sec: 0.0,
            glide_mode: GlideMode::ConstantRate,
            pitch_bend_semis: 0.0,
            aftertouch: 0.0,
            velocity_sensitivity: 0.65,
            lfo_routes: [true, false, false, false],
        }
    }
}

/// Single Hardware Voice Card of the PROPHET-XII.
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct ProphetVoice {
    pub voice_index: usize,
    pub midi_note: u8,
    pub velocity: f32,
    pub gate: bool,
    pub note_age: u64,
    /// Current portamento pitch in MIDI semitones (69.0 = A4 = 440 Hz)
    pub current_note_pitch: f32,
    pub target_note_pitch: f32,
    pub glide_rate_semis_per_sample: f32,

    /// Unison detune offset in semitones & stereo pan override
    pub unison_detune_semis: f32,
    pub unison_pan: f32,

    /// Per-note MPE expressions
    pub mpe_pitch_bend_semis: f32,
    pub mpe_pressure: f32,
    pub mpe_timbre: f32,

    /// 1.5 ms Voice Steal Anti-Click Crossfade State
    pub steal_fade_remaining: u32,
    pub steal_fade_total: u32,
    pub steal_held_sample: f32,

    /// Last Osc B output sample for audio-rate Poly-Mod feedback
    pub last_osc_b: f32,
    /// Current VCA output level for diagnostic LEDs and voice-stealing priority
    pub current_vca_level: f32,

    pub trimpots: VoiceTrimpots,
    pub vco: Cem3340DualVco,
    pub noise: ProphetNoiseGen,
    pub vcf: DualVcfStage,
    pub filter_env: Cem3310Envelope,
    pub amp_env: Cem3310Envelope,
    pub oversampler: VoiceOversampler,
    pub base_sample_rate: f32,
    pub last_mono: f32,
}

impl ProphetVoice {
    pub fn new(voice_index: usize, sample_rate: f32) -> Self {
        let trimpots = VoiceTrimpots::from_voice_index(voice_index);
        let sr = sample_rate.max(8000.0);
        let os_factor = OversampleFactor::X2;
        let os_sr = sr * (os_factor.as_usize() as f32);

        Self {
            voice_index,
            midi_note: 60,
            velocity: 0.0,
            gate: false,
            note_age: 0,
            current_note_pitch: 60.0,
            target_note_pitch: 60.0,
            glide_rate_semis_per_sample: 100.0,
            unison_detune_semis: 0.0,
            unison_pan: trimpots.pan,
            mpe_pitch_bend_semis: 0.0,
            mpe_pressure: 0.0,
            mpe_timbre: 0.0,
            steal_fade_remaining: 0,
            steal_fade_total: ((0.0015 * sr) as u32).max(8),
            steal_held_sample: 0.0,
            last_osc_b: 0.0,
            current_vca_level: 0.0,
            trimpots,
            vco: Cem3340DualVco::new(os_sr, trimpots.vco_a_cents, trimpots.vco_b_cents),
            noise: ProphetNoiseGen::new(0x1357_9BDFu32.wrapping_add((voice_index as u32) * 7919)),
            vcf: DualVcfStage::new(os_sr, 1.0 + trimpots.vcf_cutoff_pct),
            filter_env: Cem3310Envelope::new(sr, 1.0 + trimpots.env_rc_pct),
            amp_env: Cem3310Envelope::new(sr, 1.0 - trimpots.env_rc_pct * 0.8),
            oversampler: VoiceOversampler::new(os_factor),
            base_sample_rate: sr,
            last_mono: 0.0,
        }
    }

    pub fn set_sample_rate_and_oversampling(&mut self, sample_rate: f32, factor: OversampleFactor) {
        let sr = sample_rate.max(8000.0);
        self.base_sample_rate = sr;
        self.oversampler.factor = factor;
        self.oversampler.reset();
        let os_sr = sr * (factor.as_usize() as f32);
        self.vco.set_sample_rate(os_sr);
        self.vcf.update_sample_rate(os_sr);
        self.filter_env.set_sample_rate(sr);
        self.amp_env.set_sample_rate(sr);
        self.steal_fade_total = ((0.0015 * sr) as u32).max(8);
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        self.amp_env.is_active() || self.steal_fade_remaining > 0
    }

    /// Triggers Note On on this voice card, engaging 1.5 ms crossfade if stolen while active.
    pub fn trigger_note_on(
        &mut self,
        midi_note: u8,
        velocity: f32,
        previous_pitch: Option<f32>,
        glide_time_sec: f32,
        glide_mode: GlideMode,
        age: u64,
        was_stolen: bool,
        last_output_sample: f32,
    ) {
        if was_stolen && self.is_active() {
            self.steal_fade_remaining = self.steal_fade_total;
            self.steal_held_sample = self.last_mono;
            let _ = last_output_sample;
        }
        self.midi_note = midi_note;
        self.velocity = velocity.clamp(0.0, 1.0);
        self.gate = true;
        self.note_age = age;
        self.target_note_pitch = midi_note as f32;

        if glide_time_sec <= 0.0005 {
            self.current_note_pitch = self.target_note_pitch;
            self.glide_rate_semis_per_sample = 1000.0;
        } else {
            if let Some(prev) = previous_pitch {
                self.current_note_pitch = prev;
            }
            let dist = (self.target_note_pitch - self.current_note_pitch)
                .abs()
                .max(0.01);
            let samples = (glide_time_sec * self.base_sample_rate).max(1.0);
            self.glide_rate_semis_per_sample = match glide_mode {
                GlideMode::ConstantTime => dist / samples,
                GlideMode::ConstantRate => 12.0 / samples, // 1 octave per glide_time_sec
            };
        }

        // Smooth analog envelope retrigger from current capacitor voltage
        self.filter_env.note_on(false);
        self.amp_env.note_on(false);
    }

    #[inline(always)]
    pub fn trigger_note_off(&mut self) {
        self.gate = false;
        self.filter_env.note_off();
        self.amp_env.note_off();
    }

    /// Processes 1 base-rate stereo sample `(left, right)` for this voice, running VCOs,
    /// Poly-Mod Matrix, and Dual ZDF Filter inside the 2x/4x polyphase oversampling loop.
    #[inline]
    pub fn process_sample(&mut self, params: &SynthVoiceParams, lfo_mod_signal: f32) -> (f32, f32) {
        if !self.is_active() {
            self.current_vca_level = 0.0;
            return (0.0, 0.0);
        }

        // Update Vintage Dispersion scaling on VCO and VCF tolerances
        let vintage = params.vintage_amount.clamp(0.0, 1.5);
        self.vco.cents_offset_a = self.trimpots.vco_a_cents * vintage;
        self.vco.cents_offset_b = self.trimpots.vco_b_cents * vintage;
        let vcf_disp = 1.0 + self.trimpots.vcf_cutoff_pct * vintage;
        self.vcf.cem3320.cutoff_dispersion = vcf_disp;
        self.vcf.moog_ladder.cutoff_dispersion = vcf_disp;

        // Update Portamento / Glide pitch
        if (self.current_note_pitch - self.target_note_pitch).abs() > 1.0e-4 {
            if self.current_note_pitch < self.target_note_pitch {
                self.current_note_pitch = (self.current_note_pitch
                    + self.glide_rate_semis_per_sample)
                    .min(self.target_note_pitch);
            } else {
                self.current_note_pitch = (self.current_note_pitch
                    - self.glide_rate_semis_per_sample)
                    .max(self.target_note_pitch);
            }
        } else {
            self.current_note_pitch = self.target_note_pitch;
        }

        // Advance CEM3310 Filter & Amp Envelopes at base sample rate
        self.filter_env.rc_dispersion = 1.0 + self.trimpots.env_rc_pct * vintage;
        self.amp_env.rc_dispersion = 1.0 - self.trimpots.env_rc_pct * vintage * 0.8;
        self.filter_env.set_adsr(
            params.filt_attack,
            params.filt_decay,
            params.filt_sustain,
            params.filt_release,
        );
        self.amp_env.set_adsr(
            params.amp_attack,
            params.amp_decay,
            params.amp_sustain,
            params.amp_release,
        );

        let filt_eg = self.filter_env.process_sample();
        let amp_eg = self.amp_env.process_sample();

        // Velocity & VCA calculation
        let vel_scale =
            (1.0 - params.velocity_sensitivity) + params.velocity_sensitivity * self.velocity;
        let vca_gain = amp_eg * vel_scale * (1.0 + self.trimpots.vca_gain_trim * vintage);
        self.current_vca_level = vca_gain;

        // Base note pitch including Pitch Bend, Unison Detune, and MPE
        let base_pitch = self.current_note_pitch
            + params.pitch_bend_semis
            + self.unison_detune_semis
            + self.mpe_pitch_bend_semis;

        // Filter envelope contribution to Poly-Mod bus
        let filt_eg_polymod = if params.poly_mod.filter_env_bipolar {
            filt_eg - 0.5
        } else {
            filt_eg
        };

        // Global LFO routing contributions
        let lfo_osc_a = if params.lfo_routes[0] {
            lfo_mod_signal
        } else {
            0.0
        };

        let key_track_octaves = ((self.current_note_pitch - 60.0) / 12.0) * params.filter_key_track;
        let aftertouch_total =
            (params.aftertouch + self.mpe_pressure + self.mpe_timbre).clamp(0.0, 1.5);

        // Execute audio-rate VCO + Poly-Mod + ZDF VCF inside the 2x/4x Oversampler
        let vco_ref = &mut self.vco;
        let noise_ref = &mut self.noise;
        let vcf_ref = &mut self.vcf;
        let last_osc_b_ref = &mut self.last_osc_b;

        let lfo_osc_b = if params.lfo_routes[1] {
            lfo_mod_signal * 2.0
        } else {
            0.0
        };
        let lfo_pw = if params.lfo_routes[2] {
            lfo_mod_signal * 0.25
        } else {
            0.0
        };
        let lfo_cutoff = if params.lfo_routes[3] {
            lfo_mod_signal * 1.5
        } else {
            0.0
        };

        // Osc B frequency and PW are always invariant across oversampled substeps.
        let freq_b_hz = if params.osc_b_waves.lo_freq {
            let lo_base = if params.osc_b_waves.key_tracking {
                2.0 * ((base_pitch - 60.0) * (1.0 / 12.0)).exp2()
            } else {
                2.0
            };
            (lo_base
                * ((params.osc_b_semi + params.osc_b_fine_cents * 0.01) * (1.0 / 12.0)).exp2())
            .clamp(0.05, 150.0)
        } else {
            let osc_b_pitch_semis = (if params.osc_b_waves.key_tracking {
                base_pitch
            } else {
                60.0
            }) + params.osc_b_semi
                + params.osc_b_fine_cents * 0.01
                + lfo_osc_b;
            440.0 * ((osc_b_pitch_semis - 69.0) * (1.0 / 12.0)).exp2()
        };
        let pw_b = (params.osc_b_pw + lfo_pw).clamp(0.01, 0.99);

        let static_polymod = params.poly_mod.src_filter_env * filt_eg_polymod;
        let osc_b_polymod = params.poly_mod.src_osc_b;

        let osc_a_pitch_base =
            base_pitch + params.osc_a_semi + params.osc_a_fine_cents * 0.01 + lfo_osc_a * 2.0;
        let pm_freq_scale = osc_b_polymod * params.poly_mod.dest_osc_a_freq * 48.0;
        let fixed_freq_a = if pm_freq_scale == 0.0 {
            Some(
                440.0
                    * ((osc_a_pitch_base
                        + static_polymod * params.poly_mod.dest_osc_a_freq * 48.0
                        - 69.0)
                        * (1.0 / 12.0))
                        .exp2(),
            )
        } else {
            None
        };

        let osc_a_pw_base = params.osc_a_pw + lfo_pw;
        let pm_pw_scale = osc_b_polymod * params.poly_mod.dest_osc_a_pw * 0.48;
        let fixed_pw_a = if pm_pw_scale == 0.0 {
            Some((osc_a_pw_base + static_polymod * params.poly_mod.dest_osc_a_pw * 0.48).clamp(0.01, 0.99))
        } else {
            None
        };

        let cutoff_octaves_base = filt_eg * params.filter_env_amount * 6.0 * vel_scale
            + key_track_octaves
            + aftertouch_total * 2.0
            + lfo_cutoff;
        let pm_filt_scale = osc_b_polymod * params.poly_mod.dest_filter_cutoff * 5.0;
        let cutoff_needs_substep_update = pm_filt_scale != 0.0;
        if !cutoff_needs_substep_update {
            let inst_cutoff = params.filter_cutoff_hz
                * (cutoff_octaves_base + static_polymod * params.poly_mod.dest_filter_cutoff * 5.0)
                    .exp2();
            vcf_ref.set_params(
                inst_cutoff,
                params.filter_resonance,
                params.filter_drive,
                params.moog_bass_comp,
                params.filter_topology_morph,
            );
        }

        let has_noise = params.mix_noise > 1.0e-5;
        let filtered_sample = self.oversampler.process_voice_sample(|| {
            // Audio-rate Poly-Mod bus combining Filter EG and Osc B
            let poly_mod_bus = static_polymod + osc_b_polymod * (*last_osc_b_ref);

            // 1. Osc A Instantaneous Frequency (V/Oct exponential modulation)
            let freq_a_hz = match fixed_freq_a {
                Some(f) => f,
                None => {
                    let osc_a_pitch_semis = osc_a_pitch_base
                        + (poly_mod_bus * params.poly_mod.dest_osc_a_freq * 48.0);
                    440.0 * ((osc_a_pitch_semis - 69.0) * (1.0 / 12.0)).exp2()
                }
            };

            // 2. Pulse Width Modulation (Poly-Mod -> PW A + Global LFO -> PW A+B)
            let pw_a = match fixed_pw_a {
                Some(pw) => pw,
                None => (osc_a_pw_base + poly_mod_bus * params.poly_mod.dest_osc_a_pw * 0.48)
                    .clamp(0.01, 0.99),
            };

            // 3. Generate CEM3340 Dual VCO with Exact Sub-Sample Hard Sync
            let (out_a, out_b) = vco_ref.process_sample(
                freq_a_hz,
                freq_b_hz,
                pw_a,
                pw_b,
                params.osc_a_waves,
                params.osc_b_waves,
                params.osc_a_sync,
            );
            *last_osc_b_ref = out_b;

            // 4. Noise Generator & Mixer Stage
            let noise_sig = if has_noise {
                noise_ref.process(params.noise_pink_blend) * params.mix_noise
            } else {
                0.0
            };
            let mixer_out = out_a * params.mix_osc_a + out_b * params.mix_osc_b + noise_sig;

            // 5. Instantaneous ZDF Filter Cutoff Modulation (V/Oct exponential)
            if cutoff_needs_substep_update {
                let cutoff_octaves =
                    cutoff_octaves_base + poly_mod_bus * params.poly_mod.dest_filter_cutoff * 5.0;
                let inst_cutoff = params.filter_cutoff_hz * cutoff_octaves.exp2();
                vcf_ref.set_params(
                    inst_cutoff,
                    params.filter_resonance,
                    params.filter_drive,
                    params.moog_bass_comp,
                    params.filter_topology_morph,
                );
            }
            vcf_ref.process_sample(mixer_out)
        });

        let mut voice_out = filtered_sample * vca_gain;

        // Apply 1.5 ms anti-click crossfade if this voice card was stolen from an active note
        if self.steal_fade_remaining > 0 {
            let alpha = (self.steal_fade_remaining as f32) / (self.steal_fade_total as f32);
            voice_out = voice_out * (1.0 - alpha) + self.steal_held_sample * alpha;
            self.steal_fade_remaining -= 1;
        }

        self.last_mono = voice_out;
        // Equal-power stereo panning
        let pan = (self.unison_pan * 0.5 + 0.5).clamp(0.0, 1.0);
        let angle = pan * core::f32::consts::FRAC_PI_2;
        let left = voice_out * angle.cos();
        let right = voice_out * angle.sin();
        (left, right)
    }
}

/// 12-Voice Hardware Allocator Kernel (Poly Circular Rotation, Release-Priority Stealing & Unison).
#[repr(C, align(32))]
#[derive(Clone, Debug)]
pub struct ProphetVoiceAllocator {
    pub voices: [ProphetVoice; NUM_VOICES],
    /// Circular hardware cursor rotating sequentially through boards 0..12
    pub rotating_cursor: usize,
    pub event_counter: u64,
    pub unison_enabled: bool,
    pub unison_voices: usize,
    pub unison_detune_cents: f32,
    pub unison_stereo_spread: f32,
    pub note_priority: NotePriority,
    pub last_triggered_pitch: Option<f32>,
    /// Held MIDI notes stack (fixed-size array, zero heap allocations)
    held_notes: [u8; 32],
    held_count: usize,
    last_voice_out: [f32; NUM_VOICES],
}

impl ProphetVoiceAllocator {
    pub fn new(sample_rate: f32) -> Self {
        let voices = core::array::from_fn(|idx| ProphetVoice::new(idx, sample_rate));
        Self {
            voices,
            rotating_cursor: 0,
            event_counter: 0,
            unison_enabled: false,
            unison_voices: 1,
            unison_detune_cents: 18.0,
            unison_stereo_spread: 0.85,
            note_priority: NotePriority::Last,
            last_triggered_pitch: None,
            held_notes: [0; 32],
            held_count: 0,
            last_voice_out: [0.0; NUM_VOICES],
        }
    }

    pub fn set_sample_rate_and_oversampling(&mut self, sample_rate: f32, factor: OversampleFactor) {
        for v in &mut self.voices {
            v.set_sample_rate_and_oversampling(sample_rate, factor);
        }
    }

    pub fn note_on(&mut self, midi_note: u8, velocity: f32, params: &SynthVoiceParams) {
        self.event_counter = self.event_counter.wrapping_add(1);

        // Push note into held stack if not already present
        if !self.held_notes[..self.held_count].contains(&midi_note) && self.held_count < 32 {
            self.held_notes[self.held_count] = midi_note;
            self.held_count += 1;
        }

        if self.unison_enabled {
            let target_note = self.select_priority_note(midi_note);
            let n_uni = self.unison_voices.clamp(1, NUM_VOICES);
            let prev_pitch = self.last_triggered_pitch;
            self.last_triggered_pitch = Some(target_note as f32);

            for i in 0..n_uni {
                let norm_pos = if n_uni > 1 {
                    (i as f32) / ((n_uni - 1) as f32) * 2.0 - 1.0
                } else {
                    0.0
                };
                // Non-linear symmetric detune law (cubic emphasis on outer voices)
                let non_linear = 0.65 * norm_pos + 0.35 * norm_pos * norm_pos * norm_pos;
                let detune_semis = (non_linear * self.unison_detune_cents) * 0.01;
                let pan = norm_pos * self.unison_stereo_spread;

                let was_active = self.voices[i].is_active();
                self.voices[i].unison_detune_semis = detune_semis;
                self.voices[i].unison_pan = pan;
                self.voices[i].trigger_note_on(
                    target_note,
                    velocity,
                    prev_pitch,
                    params.glide_time_sec,
                    params.glide_mode,
                    self.event_counter,
                    was_active,
                    self.last_voice_out[i],
                );
            }
            return;
        }

        // Polyphonic Circular Allocation + Intelligent Voice Stealing
        let voice_idx = self.find_voice_for_note(midi_note);
        let was_active = self.voices[voice_idx].is_active();
        let prev_pitch = self.last_triggered_pitch;
        self.last_triggered_pitch = Some(midi_note as f32);

        self.voices[voice_idx].unison_detune_semis = 0.0;
        self.voices[voice_idx].unison_pan = self.voices[voice_idx].trimpots.pan;
        self.voices[voice_idx].trigger_note_on(
            midi_note,
            velocity,
            prev_pitch,
            params.glide_time_sec,
            params.glide_mode,
            self.event_counter,
            was_active,
            self.last_voice_out[voice_idx],
        );
    }

    pub fn note_off(&mut self, midi_note: u8, params: &SynthVoiceParams) {
        if let Some(pos) = self.held_notes[..self.held_count]
            .iter()
            .position(|&n| n == midi_note)
        {
            for idx in pos..(self.held_count - 1) {
                self.held_notes[idx] = self.held_notes[idx + 1];
            }
            self.held_count -= 1;
        }

        if self.unison_enabled {
            if self.held_count > 0 {
                let fallback_note = self.select_priority_note(self.held_notes[self.held_count - 1]);
                let n_uni = self.unison_voices.clamp(1, NUM_VOICES);
                for i in 0..n_uni {
                    self.voices[i].midi_note = fallback_note;
                    self.voices[i].target_note_pitch = fallback_note as f32;
                    let dist = (self.voices[i].target_note_pitch
                        - self.voices[i].current_note_pitch)
                        .abs()
                        .max(0.01);
                    let samples =
                        (params.glide_time_sec * self.voices[i].base_sample_rate).max(1.0);
                    self.voices[i].glide_rate_semis_per_sample = dist / samples;
                }
            } else {
                for v in &mut self.voices {
                    if v.gate {
                        v.trigger_note_off();
                    }
                }
            }
            return;
        }

        for v in &mut self.voices {
            if v.gate && v.midi_note == midi_note {
                v.trigger_note_off();
            }
        }
    }

    pub fn all_notes_off(&mut self) {
        self.held_count = 0;
        self.last_triggered_pitch = None;
        for v in &mut self.voices {
            v.trigger_note_off();
        }
    }
    fn select_priority_note(&self, candidate: u8) -> u8 {
        if self.held_count == 0 {
            return candidate;
        }
        match self.note_priority {
            NotePriority::Last => self.held_notes[self.held_count - 1],
            NotePriority::Low => *self.held_notes[..self.held_count]
                .iter()
                .min()
                .unwrap_or(&candidate),
            NotePriority::High => *self.held_notes[..self.held_count]
                .iter()
                .max()
                .unwrap_or(&candidate),
        }
    }

    /// Prophet-10 Circular Allocation + Release-Priority Lowest-Amplitude Stealing.
    fn find_voice_for_note(&mut self, midi_note: u8) -> usize {
        // 1. Re-use same physical voice board if retriggering an identical note
        for (i, v) in self.voices.iter().enumerate() {
            if v.midi_note == midi_note && v.is_active() {
                self.rotating_cursor = (i + 1) % NUM_VOICES;
                return i;
            }
        }

        // 2. Circular search starting from `rotating_cursor` for an Idle voice board
        for offset in 0..NUM_VOICES {
            let idx = (self.rotating_cursor + offset) % NUM_VOICES;
            if !self.voices[idx].is_active() {
                self.rotating_cursor = (idx + 1) % NUM_VOICES;
                return idx;
            }
        }

        // 3. Steal voice in Release stage with the lowest residual VCA amplitude
        let mut best_release_idx: Option<usize> = None;
        let mut min_release_amp = f32::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.amp_env.stage == EnvStage::Release && v.current_vca_level < min_release_amp {
                min_release_amp = v.current_vca_level;
                best_release_idx = Some(i);
            }
        }
        if let Some(idx) = best_release_idx {
            self.rotating_cursor = (idx + 1) % NUM_VOICES;
            return idx;
        }

        // 4. Fallback: Steal oldest active voice (lowest `note_age`)
        let mut oldest_idx = 0;
        let mut oldest_age = u64::MAX;
        for (i, v) in self.voices.iter().enumerate() {
            if v.note_age < oldest_age {
                oldest_age = v.note_age;
                oldest_idx = i;
            }
        }
        self.rotating_cursor = (oldest_idx + 1) % NUM_VOICES;
        oldest_idx
    }

    /// Processes 1 stereo sample across all 12 voices.
    #[inline]
    pub fn process_sample(&mut self, params: &SynthVoiceParams, lfo_mod_signal: f32) -> (f32, f32) {
        let mut sum_l = 0.0f32;
        let mut sum_r = 0.0f32;

        for i in 0..NUM_VOICES {
            let (l, r) = self.voices[i].process_sample(params, lfo_mod_signal);
            self.last_voice_out[i] = 0.5 * (l + r);
            sum_l += l;
            sum_r += r;
        }

        // Polyphonic summing headroom normalization
        (sum_l * 0.32, sum_r * 0.32)
    }
}
