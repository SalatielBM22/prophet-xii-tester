// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII Lock-Free Atomic Parameter Bank & SPSC Ring Buffer IPC Bridge
// Reference: NIH-plug lock-free parameter architecture (https://github.com/robbert-vdh/nih-plug)

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use prophet_dsp::fx::juno_chorus::JunoChorusMode;
use prophet_dsp::fx::solaris_reverb::{EnsembleMode, ReverbTailPolicy, SolarisReverbParams};
use prophet_dsp::osc_cem3340::{OscAWaveforms, OscBWaveforms};
use prophet_dsp::oversample::OversampleFactor;
use prophet_dsp::voice::{GlideMode, PolyModParams, SynthVoiceParams, VoiceTrimpots, NUM_VOICES};
use prophet_dsp::KernelTelemetrySnapshot;

/// Lock-free Atomic Float wrapper storing IEEE-754 `f32` bits in an `AtomicU32` (`Ordering::Relaxed`).
#[repr(transparent)]
pub struct AtomicF32 {
    bits: AtomicU32,
}

impl AtomicF32 {
    #[inline(always)]
    pub const fn new(val: f32) -> Self {
        Self {
            bits: AtomicU32::new(val.to_bits()),
        }
    }

    #[inline(always)]
    pub fn load(&self) -> f32 {
        f32::from_bits(self.bits.load(Ordering::Relaxed))
    }

    #[inline(always)]
    pub fn store(&self, val: f32) {
        self.bits.store(val.to_bits(), Ordering::Relaxed);
    }
}

/// Complete Parameter ID Enumeration (mapped 1:1 with React UI controls & MIDI CC Learn).
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamId {
    OscASemi = 0,
    OscAFine = 1,
    OscAPw = 2,
    OscASaw = 3,
    OscAPulse = 4,
    OscASync = 5,

    OscBSemi = 6,
    OscBFine = 7,
    OscBPw = 8,
    OscBSaw = 9,
    OscBTri = 10,
    OscBPulse = 11,
    OscBLoFreq = 12,
    OscBKeyTrack = 13,

    MixOscA = 14,
    MixOscB = 15,
    MixNoise = 16,
    NoisePinkBlend = 17,

    PolyModFiltEnv = 18,
    PolyModOscB = 19,
    PolyModDestFreqA = 20,
    PolyModDestPwA = 21,
    PolyModDestFilt = 22,

    FilterCutoff = 23,
    FilterResonance = 24,
    FilterDrive = 25,
    FilterEnvAmt = 26,
    FilterKeyTrack = 27,
    FilterMorph = 28,
    MoogBassComp = 29,

    FiltAttack = 30,
    FiltDecay = 31,
    FiltSustain = 32,
    FiltRelease = 33,

    AmpAttack = 34,
    AmpDecay = 35,
    AmpSustain = 36,
    AmpRelease = 37,

    LfoFreq = 38,
    LfoInitialAmt = 39,
    ModWheel = 40,

    VintageDispersion = 41,
    GlideTime = 42,
    UnisonVoices = 43,
    UnisonDetune = 44,
    UnisonSpread = 45,

    ChorusMode = 46,
    ChorusWet = 47,
    ChorusBbdNoise = 48,

    ReverbPreDelay = 49,
    ReverbCascade = 50,
    ReverbEnsembleMode = 51,
    ReverbEnsembleBlend = 52,
    ReverbDecay = 53,
    ReverbShimmer = 54,
    ReverbVowelMorph = 55,
    ReverbVowelMix = 56,
    ReverbHighCut = 57,
    ReverbLowCut = 58,
    ReverbModDepth = 59,
    ReverbModSpeed = 60,
    ReverbFreeze = 61,
    ReverbWet = 62,

    OversamplingMode = 63,
    MasterGain = 64,
    PitchBend = 65,
    UnisonEnabled = 66,
    NotePriority = 67,
    LfoSaw = 68,
    LfoTri = 69,
    LfoSquare = 70,
    LfoToA = 71,
    LfoToB = 72,
    LfoToPw = 73,
    LfoToFilter = 74,
    ShimmerFifth = 75,
    VelocitySensitivity = 76,
    PolyEnvBipolar = 77,
    GlideMode = 78,
    ShimmerSemitones = 79,
    ReverbTempoBpm = 80,
    ReverbPreDelaySync = 81,
    ReverbPreDelayBeats = 82,
}

pub const NUM_PARAMS: usize = 83;

/// Lock-free Atomic Parameter Bank shared between the Audio Thread and UI/Host Thread.
pub struct AtomicParamBank {
    pub values: [AtomicF32; NUM_PARAMS],
    pub revision: AtomicU64,
    pub reverb_enabled: AtomicBool,
    pub reverb_tail: AtomicU32,
    /// Individual per-voice calibration trimpots (`12` voices x `4` offsets)
    pub voice_vco_a_cents: [AtomicF32; NUM_VOICES],
    pub voice_vco_b_cents: [AtomicF32; NUM_VOICES],
    pub voice_vcf_pct: [AtomicF32; NUM_VOICES],
    pub voice_env_rc_pct: [AtomicF32; NUM_VOICES],
    /// MIDI CC -> ParamId lock-free mapping table (`128` CCs, `u32::MAX` = unmapped)
    pub cc_map: [AtomicU32; 128],
    pub midi_learn_active: AtomicBool,
    pub midi_learn_target_param: AtomicU32,
}

impl Default for AtomicParamBank {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicParamBank {
    pub fn new() -> Self {
        let defaults = SynthVoiceParams::default();
        let rev_defaults = SolarisReverbParams::default();
        let values = core::array::from_fn(|i| AtomicF32::new(PARAM_SPECS[i].min));
        let voice_vco_a_cents = core::array::from_fn(|i| {
            AtomicF32::new(VoiceTrimpots::from_voice_index(i).vco_a_cents)
        });
        let voice_vco_b_cents = core::array::from_fn(|i| {
            AtomicF32::new(VoiceTrimpots::from_voice_index(i).vco_b_cents)
        });
        let voice_vcf_pct = core::array::from_fn(|i| {
            AtomicF32::new(VoiceTrimpots::from_voice_index(i).vcf_cutoff_pct)
        });
        let voice_env_rc_pct =
            core::array::from_fn(|i| AtomicF32::new(VoiceTrimpots::from_voice_index(i).env_rc_pct));
        let cc_map = core::array::from_fn(|_| AtomicU32::new(u32::MAX));

        // Default performance CC assignments (CC 1 = ModWheel, CC 74 = Filter Cutoff, CC 71 = Resonance)
        cc_map[1].store(ParamId::ModWheel as u32, Ordering::Relaxed);
        cc_map[74].store(ParamId::FilterCutoff as u32, Ordering::Relaxed);
        cc_map[71].store(ParamId::FilterResonance as u32, Ordering::Relaxed);

        let bank = Self {
            values,
            revision: AtomicU64::new(0),
            reverb_enabled: AtomicBool::new(true),
            reverb_tail: AtomicU32::new(0),
            voice_vco_a_cents,
            voice_vco_b_cents,
            voice_vcf_pct,
            voice_env_rc_pct,
            cc_map,
            midi_learn_active: AtomicBool::new(false),
            midi_learn_target_param: AtomicU32::new(u32::MAX),
        };
        bank.apply_struct_snapshot(&defaults, &rev_defaults, 1.0, 0.5, 2.0);
        bank
    }

    #[inline(always)]
    pub fn get(&self, id: ParamId) -> f32 {
        self.values[id as usize].load()
    }

    #[inline(always)]
    pub fn set(&self, id: ParamId, val: f32) {
        let spec = &PARAM_SPECS[id as usize];
        if val.is_finite() {
            let value = val.clamp(spec.min, spec.max);
            let value = if spec.integer { value.round() } else { value };
            if self.values[id as usize].load().to_bits() != value.to_bits() {
                self.values[id as usize].store(value);
                self.revision.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// MIDI CC64 and channel-mode CC120/123 are reserved by the engine.
    pub fn learnable_cc(cc: usize) -> bool {
        cc < 128 && !matches!(cc, 64 | 120 | 123)
    }
    pub fn begin_learn(&self, id: usize) -> bool {
        if id >= NUM_PARAMS {
            return false;
        }
        self.midi_learn_target_param
            .store(id as u32, Ordering::Relaxed);
        self.midi_learn_active.store(true, Ordering::Relaxed);
        self.revision.fetch_add(1, Ordering::Relaxed);
        true
    }
    pub fn cancel_learn(&self) {
        self.midi_learn_active.store(false, Ordering::Relaxed);
        self.midi_learn_target_param
            .store(u32::MAX, Ordering::Relaxed);
        self.revision.fetch_add(1, Ordering::Relaxed);
    }
    pub fn unlearn(&self, id: usize) -> bool {
        if id >= NUM_PARAMS {
            return false;
        }
        for mapping in &self.cc_map {
            if mapping.load(Ordering::Relaxed) == id as u32 {
                mapping.store(u32::MAX, Ordering::Relaxed);
            }
        }
        if self.midi_learn_target_param.load(Ordering::Relaxed) == id as u32 {
            self.cancel_learn();
        }
        self.revision.fetch_add(1, Ordering::Relaxed);
        true
    }
    pub fn remap(&self, cc: usize, id: usize) -> bool {
        if !Self::learnable_cc(cc) || id >= NUM_PARAMS {
            return false;
        }
        self.unlearn(id);
        self.cc_map[cc].store(id as u32, Ordering::Relaxed);
        self.cancel_learn();
        true
    }

    pub fn set_reverb_bypass(&self, enabled: bool, tail: ReverbTailPolicy) {
        let policy = u32::from(tail == ReverbTailPolicy::Kill);
        let changed = self.reverb_enabled.swap(enabled, Ordering::Relaxed) != enabled;
        let changed = (self.reverb_tail.swap(policy, Ordering::Relaxed) != policy) || changed;
        if changed {
            self.revision.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn apply_struct_snapshot(
        &self,
        vp: &SynthVoiceParams,
        rp: &SolarisReverbParams,
        chorus_mode: f32,
        chorus_wet: f32,
        oversample_idx: f32,
    ) {
        self.set(ParamId::OscASemi, vp.osc_a_semi);
        self.set(ParamId::OscAFine, vp.osc_a_fine_cents);
        self.set(ParamId::OscAPw, vp.osc_a_pw);
        self.set(ParamId::OscASaw, if vp.osc_a_waves.saw { 1.0 } else { 0.0 });
        self.set(
            ParamId::OscAPulse,
            if vp.osc_a_waves.pulse { 1.0 } else { 0.0 },
        );
        self.set(ParamId::OscASync, if vp.osc_a_sync { 1.0 } else { 0.0 });

        self.set(ParamId::OscBSemi, vp.osc_b_semi);
        self.set(ParamId::OscBFine, vp.osc_b_fine_cents);
        self.set(ParamId::OscBPw, vp.osc_b_pw);
        self.set(ParamId::OscBSaw, if vp.osc_b_waves.saw { 1.0 } else { 0.0 });
        self.set(
            ParamId::OscBTri,
            if vp.osc_b_waves.triangle { 1.0 } else { 0.0 },
        );
        self.set(
            ParamId::OscBPulse,
            if vp.osc_b_waves.pulse { 1.0 } else { 0.0 },
        );
        self.set(
            ParamId::OscBLoFreq,
            if vp.osc_b_waves.lo_freq { 1.0 } else { 0.0 },
        );
        self.set(
            ParamId::OscBKeyTrack,
            if vp.osc_b_waves.key_tracking {
                1.0
            } else {
                0.0
            },
        );

        self.set(ParamId::MixOscA, vp.mix_osc_a);
        self.set(ParamId::MixOscB, vp.mix_osc_b);
        self.set(ParamId::MixNoise, vp.mix_noise);
        self.set(ParamId::NoisePinkBlend, vp.noise_pink_blend);

        self.set(ParamId::PolyModFiltEnv, vp.poly_mod.src_filter_env);
        self.set(ParamId::PolyModOscB, vp.poly_mod.src_osc_b);
        self.set(ParamId::PolyModDestFreqA, vp.poly_mod.dest_osc_a_freq);
        self.set(ParamId::PolyModDestPwA, vp.poly_mod.dest_osc_a_pw);
        self.set(ParamId::PolyModDestFilt, vp.poly_mod.dest_filter_cutoff);

        self.set(ParamId::FilterCutoff, vp.filter_cutoff_hz);
        self.set(ParamId::FilterResonance, vp.filter_resonance);
        self.set(ParamId::FilterDrive, vp.filter_drive);
        self.set(ParamId::FilterEnvAmt, vp.filter_env_amount);
        self.set(ParamId::FilterKeyTrack, vp.filter_key_track);
        self.set(ParamId::FilterMorph, vp.filter_topology_morph);
        self.set(ParamId::MoogBassComp, vp.moog_bass_comp);

        self.set(ParamId::FiltAttack, vp.filt_attack);
        self.set(ParamId::FiltDecay, vp.filt_decay);
        self.set(ParamId::FiltSustain, vp.filt_sustain);
        self.set(ParamId::FiltRelease, vp.filt_release);

        self.set(ParamId::AmpAttack, vp.amp_attack);
        self.set(ParamId::AmpDecay, vp.amp_decay);
        self.set(ParamId::AmpSustain, vp.amp_sustain);
        self.set(ParamId::AmpRelease, vp.amp_release);

        self.set(ParamId::VintageDispersion, vp.vintage_amount);
        self.set(ParamId::GlideTime, vp.glide_time_sec);
        self.set(ParamId::ChorusMode, chorus_mode);
        self.set(ParamId::ChorusWet, chorus_wet);
        self.set(ParamId::ChorusBbdNoise, 0.0);

        self.set(ParamId::ReverbPreDelay, rp.pre_delay_ms);
        self.set(ParamId::ReverbCascade, rp.cascade_diffusion);
        let ens_idx = match rp.ensemble_mode {
            EnsembleMode::Off => 0.0,
            EnsembleMode::MezzoPiano => 1.0,
            EnsembleMode::Forte => 2.0,
        };
        self.set(ParamId::ReverbEnsembleMode, ens_idx);
        self.set(ParamId::ReverbEnsembleBlend, rp.ensemble_blend);
        self.set(ParamId::ReverbDecay, rp.decay_sec);
        self.set(ParamId::ReverbShimmer, rp.shimmer_amount);
        self.set(ParamId::ReverbVowelMorph, rp.vowel_morph);
        self.set(ParamId::ReverbVowelMix, rp.vowel_mix);
        self.set(ParamId::ReverbHighCut, rp.high_cut_hz);
        self.set(ParamId::ReverbLowCut, rp.low_cut_hz);
        self.set(ParamId::ReverbModDepth, rp.mod_depth);
        self.set(ParamId::ReverbModSpeed, rp.mod_speed_hz);
        self.set(ParamId::ReverbFreeze, if rp.freeze { 1.0 } else { 0.0 });
        self.set(ParamId::ReverbWet, rp.wet_mix);
        self.set_reverb_bypass(rp.enabled, rp.bypass_tail);

        self.set(ParamId::OversamplingMode, oversample_idx);
        self.set(ParamId::MasterGain, 0.65);
        self.set(ParamId::LfoFreq, 2.5);
        self.set(ParamId::LfoTri, 1.0);
        self.set(ParamId::LfoToA, 1.0);
        self.set(ParamId::LfoInitialAmt, 0.0);
        self.set(ParamId::ModWheel, 0.0);
        self.set(ParamId::PitchBend, 0.0);
        self.set(ParamId::VelocitySensitivity, vp.velocity_sensitivity);
        self.set(
            ParamId::PolyEnvBipolar,
            if vp.poly_mod.filter_env_bipolar {
                1.0
            } else {
                0.0
            },
        );
        self.set(
            ParamId::GlideMode,
            if vp.glide_mode == GlideMode::ConstantTime {
                1.0
            } else {
                0.0
            },
        );
        self.set(ParamId::ShimmerFifth, rp.shimmer_fifth_blend);
        self.set(ParamId::ShimmerSemitones, rp.shimmer_semitones);
        self.set(ParamId::ReverbTempoBpm, rp.tempo_bpm);
        self.set(
            ParamId::ReverbPreDelaySync,
            if rp.predelay_sync { 1.0 } else { 0.0 },
        );
        self.set(ParamId::ReverbPreDelayBeats, rp.predelay_beats);
    }

    /// Reads all atomic parameters in `O(1)` lock-free relaxed loads into the DSP structs.
    #[inline]
    pub fn read_into_dsp(
        &self,
        vp: &mut SynthVoiceParams,
        rp: &mut SolarisReverbParams,
    ) -> (JunoChorusMode, f32, f32, OversampleFactor, f32) {
        vp.osc_a_semi = self.get(ParamId::OscASemi);
        vp.osc_a_fine_cents = self.get(ParamId::OscAFine);
        vp.osc_a_pw = self.get(ParamId::OscAPw);
        vp.osc_a_waves = OscAWaveforms {
            saw: self.get(ParamId::OscASaw) >= 0.5,
            pulse: self.get(ParamId::OscAPulse) >= 0.5,
        };
        vp.osc_a_sync = self.get(ParamId::OscASync) >= 0.5;

        vp.osc_b_semi = self.get(ParamId::OscBSemi);
        vp.osc_b_fine_cents = self.get(ParamId::OscBFine);
        vp.osc_b_pw = self.get(ParamId::OscBPw);
        vp.osc_b_waves = OscBWaveforms {
            saw: self.get(ParamId::OscBSaw) >= 0.5,
            triangle: self.get(ParamId::OscBTri) >= 0.5,
            pulse: self.get(ParamId::OscBPulse) >= 0.5,
            lo_freq: self.get(ParamId::OscBLoFreq) >= 0.5,
            key_tracking: self.get(ParamId::OscBKeyTrack) >= 0.5,
        };

        vp.mix_osc_a = self.get(ParamId::MixOscA);
        vp.mix_osc_b = self.get(ParamId::MixOscB);
        vp.mix_noise = self.get(ParamId::MixNoise);
        vp.noise_pink_blend = self.get(ParamId::NoisePinkBlend);

        vp.poly_mod = PolyModParams {
            src_filter_env: self.get(ParamId::PolyModFiltEnv),
            filter_env_bipolar: self.get(ParamId::PolyEnvBipolar) >= 0.5,
            src_osc_b: self.get(ParamId::PolyModOscB),
            dest_osc_a_freq: self.get(ParamId::PolyModDestFreqA),
            dest_osc_a_pw: self.get(ParamId::PolyModDestPwA),
            dest_filter_cutoff: self.get(ParamId::PolyModDestFilt),
        };

        vp.filter_cutoff_hz = self.get(ParamId::FilterCutoff);
        vp.filter_resonance = self.get(ParamId::FilterResonance);
        vp.filter_drive = self.get(ParamId::FilterDrive);
        vp.filter_env_amount = self.get(ParamId::FilterEnvAmt);
        vp.filter_key_track = self.get(ParamId::FilterKeyTrack);
        vp.filter_topology_morph = self.get(ParamId::FilterMorph);
        vp.moog_bass_comp = self.get(ParamId::MoogBassComp);

        vp.filt_attack = self.get(ParamId::FiltAttack);
        vp.filt_decay = self.get(ParamId::FiltDecay);
        vp.filt_sustain = self.get(ParamId::FiltSustain);
        vp.filt_release = self.get(ParamId::FiltRelease);

        vp.amp_attack = self.get(ParamId::AmpAttack);
        vp.amp_decay = self.get(ParamId::AmpDecay);
        vp.amp_sustain = self.get(ParamId::AmpSustain);
        vp.amp_release = self.get(ParamId::AmpRelease);

        vp.vintage_amount = self.get(ParamId::VintageDispersion);
        vp.glide_time_sec = self.get(ParamId::GlideTime);
        vp.glide_mode = if self.get(ParamId::GlideMode) >= 0.5 {
            GlideMode::ConstantTime
        } else {
            GlideMode::ConstantRate
        };
        vp.pitch_bend_semis = self.get(ParamId::PitchBend);
        vp.velocity_sensitivity = self.get(ParamId::VelocitySensitivity);
        vp.lfo_routes = [
            self.get(ParamId::LfoToA) >= 0.5,
            self.get(ParamId::LfoToB) >= 0.5,
            self.get(ParamId::LfoToPw) >= 0.5,
            self.get(ParamId::LfoToFilter) >= 0.5,
        ];
        rp.shimmer_fifth_blend = self.get(ParamId::ShimmerFifth);
        rp.shimmer_semitones = self.get(ParamId::ShimmerSemitones);
        rp.tempo_bpm = self.get(ParamId::ReverbTempoBpm);
        rp.predelay_sync = self.get(ParamId::ReverbPreDelaySync) >= 0.5;
        rp.predelay_beats = self.get(ParamId::ReverbPreDelayBeats);

        rp.pre_delay_ms = self.get(ParamId::ReverbPreDelay);
        rp.cascade_diffusion = self.get(ParamId::ReverbCascade);
        rp.ensemble_mode = match self.get(ParamId::ReverbEnsembleMode).round() as i32 {
            0 => EnsembleMode::Off,
            1 => EnsembleMode::MezzoPiano,
            _ => EnsembleMode::Forte,
        };
        rp.ensemble_blend = self.get(ParamId::ReverbEnsembleBlend);
        rp.decay_sec = self.get(ParamId::ReverbDecay);
        rp.shimmer_amount = self.get(ParamId::ReverbShimmer);
        rp.vowel_morph = self.get(ParamId::ReverbVowelMorph);
        rp.vowel_mix = self.get(ParamId::ReverbVowelMix);
        rp.high_cut_hz = self.get(ParamId::ReverbHighCut);
        rp.low_cut_hz = self.get(ParamId::ReverbLowCut);
        rp.mod_depth = self.get(ParamId::ReverbModDepth);
        rp.mod_speed_hz = self.get(ParamId::ReverbModSpeed);
        rp.freeze = self.get(ParamId::ReverbFreeze) >= 0.5;
        rp.wet_mix = self.get(ParamId::ReverbWet);
        rp.enabled = self.reverb_enabled.load(Ordering::Relaxed);
        rp.bypass_tail = if self.reverb_tail.load(Ordering::Relaxed) == 1 {
            ReverbTailPolicy::Kill
        } else {
            ReverbTailPolicy::Spill
        };

        let chorus_mode = match self.get(ParamId::ChorusMode).round() as i32 {
            0 => JunoChorusMode::Off,
            1 => JunoChorusMode::ModeI,
            2 => JunoChorusMode::ModeII,
            _ => JunoChorusMode::ModeIPlusII,
        };
        let chorus_wet = self.get(ParamId::ChorusWet);
        let chorus_noise = self.get(ParamId::ChorusBbdNoise);

        let os = match self.get(ParamId::OversamplingMode).round() as i32 {
            1 => OversampleFactor::X1,
            4 => OversampleFactor::X4,
            _ => OversampleFactor::X2,
        };
        let master = self.get(ParamId::MasterGain);

        (chorus_mode, chorus_wet, chorus_noise, os, master)
    }
}

/// Lock-Free Single-Producer Single-Consumer (SPSC) Ring Buffer (`N` must be a power of 2).
/// Used for UI -> Audio Thread MIDI/Preset events and Audio -> UI 60 FPS Telemetry snapshots.
pub struct SpscRingBuffer<T: Copy, const N: usize> {
    head: AtomicUsize,
    tail: AtomicUsize,
    buffer: [UnsafeCell<MaybeUninit<T>>; N],
}

unsafe impl<T: Copy + Send, const N: usize> Send for SpscRingBuffer<T, N> {}
// Deliberately !Sync: public push/pop do not enforce the single-producer contract.
// Cross-thread transport is provided by split rtrb endpoints in the host.

impl<T: Copy, const N: usize> Default for SpscRingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Copy, const N: usize> SpscRingBuffer<T, N> {
    pub fn new() -> Self {
        assert!(
            N.is_power_of_two(),
            "SPSC capacity N must be a power of two"
        );
        Self {
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            buffer: core::array::from_fn(|_| UnsafeCell::new(MaybeUninit::uninit())),
        }
    }

    #[inline]
    pub fn push(&self, item: T) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        if head.wrapping_sub(tail) >= N {
            return false; // Queue full
        }
        unsafe {
            (*self.buffer[head & (N - 1)].get()).write(item);
        }
        self.head.store(head.wrapping_add(1), Ordering::Release);
        true
    }

    #[inline]
    pub fn pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail == head {
            return None; // Queue empty
        }
        let item = unsafe { (*self.buffer[tail & (N - 1)].get()).assume_init_read() };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Some(item)
    }
}

/// Lock-Free UI <-> Kernel IPC Event Protocol.
#[derive(Clone, Copy, Debug)]
pub enum IpcUiEvent {
    NoteOn {
        note: u8,
        velocity: f32,
        channel: u8,
    },
    NoteOff {
        note: u8,
        channel: u8,
    },
    MpePitchBend {
        channel: u8,
        semitones: f32,
    },
    MpePressure {
        channel: u8,
        pressure: f32,
    },
    MpeTimbre {
        channel: u8,
        timbre: f32,
    },
    SelectPreset {
        index: usize,
    },
    SetUnison {
        enabled: bool,
        voices: usize,
        detune: f32,
        spread: f32,
    },
}

/// Bidirectional WebView IPC Bridge combining Atomic Parameters + SPSC Event & Telemetry Queues.
pub struct WebViewIpcBridge {
    pub params: AtomicParamBank,
    pub ui_to_audio_queue: SpscRingBuffer<IpcUiEvent, 256>,
    pub audio_to_ui_telemetry: SpscRingBuffer<KernelTelemetrySnapshot, 16>,
}

impl Default for WebViewIpcBridge {
    fn default() -> Self {
        Self {
            params: AtomicParamBank::new(),
            ui_to_audio_queue: SpscRingBuffer::new(),
            audio_to_ui_telemetry: SpscRingBuffer::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub id: ParamId,
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub integer: bool,
    pub logarithmic: bool,
    pub group: &'static str,
}
pub const PARAM_SPECS: [ParamSpec; NUM_PARAMS] = [
    ParamSpec {
        id: ParamId::OscASemi,
        name: "OscA Semi",
        min: -36.0,
        max: 36.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscAFine,
        name: "OscA Fine",
        min: -100.0,
        max: 100.0,
        integer: false,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscAPw,
        name: "OscA Pw",
        min: 0.01,
        max: 0.99,
        integer: false,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscASaw,
        name: "OscA Saw",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscAPulse,
        name: "OscA Pulse",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscASync,
        name: "OscA Sync",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR A",
    },
    ParamSpec {
        id: ParamId::OscBSemi,
        name: "OscB Semi",
        min: -36.0,
        max: 36.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBFine,
        name: "OscB Fine",
        min: -100.0,
        max: 100.0,
        integer: false,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBPw,
        name: "OscB Pw",
        min: 0.01,
        max: 0.99,
        integer: false,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBSaw,
        name: "OscB Saw",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBTri,
        name: "OscB Tri",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBPulse,
        name: "OscB Pulse",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBLoFreq,
        name: "OscB Lo Freq",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::OscBKeyTrack,
        name: "OscB Key Track",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "OSCILLATOR B",
    },
    ParamSpec {
        id: ParamId::MixOscA,
        name: "Mix OscA",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "MIXER",
    },
    ParamSpec {
        id: ParamId::MixOscB,
        name: "Mix OscB",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "MIXER",
    },
    ParamSpec {
        id: ParamId::MixNoise,
        name: "Mix Noise",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "MIXER",
    },
    ParamSpec {
        id: ParamId::NoisePinkBlend,
        name: "Noise Pink Blend",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "MIXER",
    },
    ParamSpec {
        id: ParamId::PolyModFiltEnv,
        name: "Poly Mod Filt Env",
        min: -1.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::PolyModOscB,
        name: "Poly Mod OscB",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::PolyModDestFreqA,
        name: "Poly Mod Dest FreqA",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::PolyModDestPwA,
        name: "Poly Mod Dest PwA",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::PolyModDestFilt,
        name: "Poly Mod Dest Filt",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::FilterCutoff,
        name: "Filter Cutoff",
        min: 20.0,
        max: 18000.0,
        integer: false,
        logarithmic: true,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FilterResonance,
        name: "Filter Resonance",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FilterDrive,
        name: "Filter Drive",
        min: 0.25,
        max: 4.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FilterEnvAmt,
        name: "Filter Env Amt",
        min: -1.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FilterKeyTrack,
        name: "Filter Key Track",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FilterMorph,
        name: "Filter Morph",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::MoogBassComp,
        name: "Moog Bass Comp",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER",
    },
    ParamSpec {
        id: ParamId::FiltAttack,
        name: "Filt Attack",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "FILTER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::FiltDecay,
        name: "Filt Decay",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "FILTER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::FiltSustain,
        name: "Filt Sustain",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "FILTER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::FiltRelease,
        name: "Filt Release",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "FILTER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::AmpAttack,
        name: "Amp Attack",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "AMPLIFIER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::AmpDecay,
        name: "Amp Decay",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "AMPLIFIER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::AmpSustain,
        name: "Amp Sustain",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "AMPLIFIER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::AmpRelease,
        name: "Amp Release",
        min: 0.001,
        max: 25.0,
        integer: false,
        logarithmic: true,
        group: "AMPLIFIER ENVELOPE",
    },
    ParamSpec {
        id: ParamId::LfoFreq,
        name: "Lfo Freq",
        min: 0.04,
        max: 30.0,
        integer: false,
        logarithmic: true,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoInitialAmt,
        name: "Lfo Initial Amt",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::ModWheel,
        name: "Mod Wheel",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::VintageDispersion,
        name: "Vintage Dispersion",
        min: 0.0,
        max: 1.5,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::GlideTime,
        name: "Glide Time",
        min: 0.0,
        max: 5.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::UnisonVoices,
        name: "Unison Voices",
        min: 1.0,
        max: 12.0,
        integer: true,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::UnisonDetune,
        name: "Unison Detune",
        min: 0.0,
        max: 60.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::UnisonSpread,
        name: "Unison Spread",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::ChorusMode,
        name: "Chorus Mode",
        min: 0.0,
        max: 3.0,
        integer: true,
        logarithmic: false,
        group: "CHORUS",
    },
    ParamSpec {
        id: ParamId::ChorusWet,
        name: "Chorus Wet",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "CHORUS",
    },
    ParamSpec {
        id: ParamId::ChorusBbdNoise,
        name: "Chorus Bbd Noise",
        min: 0.0,
        max: 0.005,
        integer: false,
        logarithmic: false,
        group: "CHORUS",
    },
    ParamSpec {
        id: ParamId::ReverbPreDelay,
        name: "Reverb Pre Delay",
        min: 0.0,
        max: 500.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbCascade,
        name: "Reverb Cascade",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbEnsembleMode,
        name: "Reverb Ensemble Mode",
        min: 0.0,
        max: 2.0,
        integer: true,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbEnsembleBlend,
        name: "Reverb Ensemble Blend",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbDecay,
        name: "Reverb Decay",
        min: 0.2,
        max: 60.0,
        integer: false,
        logarithmic: true,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbShimmer,
        name: "Reverb Shimmer",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbVowelMorph,
        name: "Reverb Vowel Morph",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbVowelMix,
        name: "Reverb Vowel Mix",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbHighCut,
        name: "Reverb High Cut",
        min: 800.0,
        max: 18000.0,
        integer: false,
        logarithmic: true,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbLowCut,
        name: "Reverb Low Cut",
        min: 20.0,
        max: 1000.0,
        integer: false,
        logarithmic: true,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbModDepth,
        name: "Reverb Mod Depth",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbModSpeed,
        name: "Reverb Mod Speed",
        min: 0.05,
        max: 5.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbFreeze,
        name: "Reverb Freeze",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbWet,
        name: "Reverb Wet",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::OversamplingMode,
        name: "Oversampling Mode",
        min: 1.0,
        max: 4.0,
        integer: true,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::MasterGain,
        name: "Master Gain",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::PitchBend,
        name: "Pitch Bend",
        min: -7.0,
        max: 7.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::UnisonEnabled,
        name: "Unison Enabled",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::NotePriority,
        name: "Note Priority",
        min: 0.0,
        max: 2.0,
        integer: true,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::LfoSaw,
        name: "Lfo Saw",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoTri,
        name: "Lfo Tri",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoSquare,
        name: "Lfo Square",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoToA,
        name: "Lfo ToA",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoToB,
        name: "Lfo ToB",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoToPw,
        name: "Lfo To Pw",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::LfoToFilter,
        name: "Lfo To Filter",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "LFO",
    },
    ParamSpec {
        id: ParamId::ShimmerFifth,
        name: "Shimmer Fifth",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::VelocitySensitivity,
        name: "Velocity Sensitivity",
        min: 0.0,
        max: 1.0,
        integer: false,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::PolyEnvBipolar,
        name: "Poly Env Bipolar",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "POLY-MOD",
    },
    ParamSpec {
        id: ParamId::GlideMode,
        name: "Glide Mode",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "PERFORMANCE",
    },
    ParamSpec {
        id: ParamId::ShimmerSemitones,
        name: "Shimmer Semitones",
        min: -12.0,
        max: 12.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbTempoBpm,
        name: "Reverb Tempo Bpm",
        min: 20.0,
        max: 300.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbPreDelaySync,
        name: "Reverb Pre Delay Sync",
        min: 0.0,
        max: 1.0,
        integer: true,
        logarithmic: false,
        group: "REVERB",
    },
    ParamSpec {
        id: ParamId::ReverbPreDelayBeats,
        name: "Reverb Pre Delay Beats",
        min: 0.0,
        max: 4.0,
        integer: false,
        logarithmic: false,
        group: "REVERB",
    },
];
impl ParamId {
    pub fn from_index(i: usize) -> Option<Self> {
        PARAM_SPECS.get(i).map(|s| s.id)
    }
}
