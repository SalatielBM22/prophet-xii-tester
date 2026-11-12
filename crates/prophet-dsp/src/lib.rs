// SPDX-License-Identifier: GPL-3.0-only
// PROPHET-XII Deterministic Real-Time DSP Kernel (`prophet-dsp`)
// Strictly zero dynamic heap allocations inside `process_block()`.
// Aligned fixed blocks (`MAX_BLOCK_SIZE = 64`, `#[repr(align(32))]`).

pub mod env_cem3310;
pub mod fx;
pub mod osc_cem3340;
pub mod oversample;
pub mod vcf_cem3320;
pub mod vcf_moog;
pub mod voice;

use env_cem3310::ProphetGlobalLfo;
use fx::juno_chorus::Juno106BbdChorus;
use fx::solaris_reverb::{SolarisCloudburstReverb, SolarisReverbParams};
use oversample::OversampleFactor;
use voice::{ProphetVoiceAllocator, SynthVoiceParams, NUM_VOICES};

/// Fixed maximum SIMD-friendly audio block size (`64` samples, 32-byte aligned).
pub const MAX_BLOCK_SIZE: usize = 64;
/// Maximum supported host sample rate (`192,000 Hz`).
pub const MAX_SAMPLE_RATE: f32 = 192_000.0;

/// 32-byte SIMD-aligned Audio Block Buffer (`64` samples).
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub struct AlignedAudioBlock {
    pub samples: [f32; MAX_BLOCK_SIZE],
}

impl Default for AlignedAudioBlock {
    fn default() -> Self {
        Self {
            samples: [0.0; MAX_BLOCK_SIZE],
        }
    }
}

/// 1-Pole Exponential Parameter Smoother to eliminate zipper noise on atomic UI changes.
#[derive(Clone, Copy, Debug)]
pub struct OnePoleSmoother {
    pub current: f32,
    pub target: f32,
    coeff: f32,
}

impl OnePoleSmoother {
    pub fn new(initial: f32, smoothing_ms: f32, sample_rate: f32) -> Self {
        let mut s = Self {
            current: initial,
            target: initial,
            coeff: 0.01,
        };
        s.set_time_ms(smoothing_ms, sample_rate);
        s
    }

    #[inline]
    pub fn set_time_ms(&mut self, smoothing_ms: f32, sample_rate: f32) {
        let samples = (smoothing_ms * 0.001 * sample_rate.max(8000.0)).max(1.0);
        self.coeff = 1.0 - (-1.0 / samples).exp();
    }

    #[inline(always)]
    pub fn next(&mut self) -> f32 {
        let diff = self.target - self.current;
        if diff.abs() <= 1.0e-7 * (1.0 + self.target.abs()) {
            self.current = self.target;
        } else {
            self.current += diff * self.coeff;
        }
        self.current
    }
}

/// Real-Time Diagnostic Snapshot for the 60 FPS React/Canvas UI Telemetry Bridge.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KernelTelemetrySnapshot {
    pub active_voice_mask: u16,
    pub voice_vca_levels: [f32; NUM_VOICES],
    pub voice_filt_env_levels: [f32; NUM_VOICES],
    pub voice_midi_notes: [u8; NUM_VOICES],
    pub amp_envelopes: [env_cem3310::EnvelopeTelemetry; NUM_VOICES],
    pub filter_envelopes: [env_cem3310::EnvelopeTelemetry; NUM_VOICES],
    pub scope_samples_l: [f32; MAX_BLOCK_SIZE],
    pub scope_samples_r: [f32; MAX_BLOCK_SIZE],
    pub dsp_load_percent: f32,
}

impl Default for KernelTelemetrySnapshot {
    fn default() -> Self {
        Self {
            active_voice_mask: 0,
            voice_vca_levels: [0.0; NUM_VOICES],
            voice_filt_env_levels: [0.0; NUM_VOICES],
            voice_midi_notes: [0; NUM_VOICES],
            amp_envelopes: [env_cem3310::EnvelopeTelemetry::default(); NUM_VOICES],
            filter_envelopes: [env_cem3310::EnvelopeTelemetry::default(); NUM_VOICES],
            scope_samples_l: [0.0; MAX_BLOCK_SIZE],
            scope_samples_r: [0.0; MAX_BLOCK_SIZE],
            dsp_load_percent: 0.0,
        }
    }
}

/// Top-Level PROPHET-XII Real-Time Synthesis & Effects Kernel.
#[repr(C, align(32))]
pub struct ProphetXiiKernel {
    pub allocator: ProphetVoiceAllocator,
    pub global_lfo: ProphetGlobalLfo,
    pub chorus: Juno106BbdChorus,
    pub reverb: SolarisCloudburstReverb,
    pub voice_params: SynthVoiceParams,
    pub reverb_params: SolarisReverbParams,
    pub cutoff_smoother: OnePoleSmoother,
    pub res_smoother: OnePoleSmoother,
    pub master_gain: f32,
    pub sample_rate: f32,
    pub last_telemetry: KernelTelemetrySnapshot,
    smooth: [OnePoleSmoother; 24],
    gain_smoother: OnePoleSmoother,
}

impl ProphetXiiKernel {
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate.clamp(8000.0, MAX_SAMPLE_RATE);
        let voice_params = SynthVoiceParams::default();
        let initial = [
            voice_params.osc_a_semi,
            voice_params.osc_a_fine_cents,
            voice_params.osc_a_pw,
            voice_params.osc_b_semi,
            voice_params.osc_b_fine_cents,
            voice_params.osc_b_pw,
            voice_params.mix_osc_a,
            voice_params.mix_osc_b,
            voice_params.mix_noise,
            voice_params.noise_pink_blend,
            voice_params.filter_drive,
            voice_params.filter_env_amount,
            voice_params.filter_key_track,
            voice_params.filter_topology_morph,
            voice_params.moog_bass_comp,
            voice_params.vintage_amount,
            voice_params.pitch_bend_semis,
            voice_params.aftertouch,
            voice_params.velocity_sensitivity,
            voice_params.poly_mod.src_filter_env,
            voice_params.poly_mod.src_osc_b,
            voice_params.poly_mod.dest_osc_a_freq,
            voice_params.poly_mod.dest_osc_a_pw,
            voice_params.poly_mod.dest_filter_cutoff,
        ];
        Self {
            allocator: ProphetVoiceAllocator::new(sr),
            global_lfo: ProphetGlobalLfo::new(sr),
            chorus: Juno106BbdChorus::new(sr),
            reverb: SolarisCloudburstReverb::new(sr),
            voice_params,
            reverb_params: SolarisReverbParams::default(),
            cutoff_smoother: OnePoleSmoother::new(voice_params.filter_cutoff_hz, 4.0, sr),
            res_smoother: OnePoleSmoother::new(voice_params.filter_resonance, 4.0, sr),
            master_gain: 0.85,
            sample_rate: sr,
            last_telemetry: KernelTelemetrySnapshot::default(),
            smooth: core::array::from_fn(|i| OnePoleSmoother::new(initial[i], 5.0, sr)),
            gain_smoother: OnePoleSmoother::new(0.85, 5.0, sr),
        }
    }

    pub fn reset(&mut self) {
        self.allocator.all_notes_off();
        for v in &mut self.allocator.voices {
            v.amp_env.stage = env_cem3310::EnvStage::Idle;
            v.amp_env.level = 0.0;
            v.amp_env.stage_elapsed_samples = 0;
            v.filter_env.stage = env_cem3310::EnvStage::Idle;
            v.filter_env.level = 0.0;
            v.filter_env.stage_elapsed_samples = 0;
            v.vco.reset(0.0, 0.0);
            v.vcf.reset();
            v.oversampler.reset();
            v.steal_fade_remaining = 0;
            v.last_mono = 0.0;
        }
        self.reverb.reset();
        self.chorus.reset();
        self.last_telemetry = KernelTelemetrySnapshot::default();
    }
    pub fn prepare(&mut self, sample_rate: f32, oversample: OversampleFactor) {
        let sr = sample_rate.clamp(8000.0, MAX_SAMPLE_RATE);
        self.sample_rate = sr;
        for v in &mut self.smooth {
            v.set_time_ms(5.0, sr);
        }
        self.gain_smoother.set_time_ms(5.0, sr);
        self.allocator
            .set_sample_rate_and_oversampling(sr, oversample);
        self.global_lfo.set_sample_rate(sr);
        self.chorus.set_sample_rate(sr);
        self.reverb.set_sample_rate(sr);
        self.cutoff_smoother.set_time_ms(4.0, sr);
        self.res_smoother.set_time_ms(4.0, sr);
    }

    /// Called at bounded diagnostic cadence, not per audio sample.
    pub fn telemetry_snapshot(&self) -> KernelTelemetrySnapshot {
        let mut snapshot = self.last_telemetry;
        for (i, v) in self.allocator.voices.iter().enumerate() {
            snapshot.amp_envelopes[i] = v.amp_env.telemetry();
            snapshot.filter_envelopes[i] = v.filter_env.telemetry();
        }
        snapshot
    }

    /// Hard Real-Time Block Processor (`num_samples <= MAX_BLOCK_SIZE`).
    /// Zero heap allocations, zero locks.
    pub fn process_block(&mut self, out_l: &mut [f32], out_r: &mut [f32], num_samples: usize) {
        let n = num_samples
            .min(MAX_BLOCK_SIZE)
            .min(out_l.len())
            .min(out_r.len());
        self.cutoff_smoother.target = self.voice_params.filter_cutoff_hz;
        self.res_smoother.target = self.voice_params.filter_resonance;

        let mut local_params = self.voice_params;
        self.gain_smoother.target = self.master_gain;
        self.smooth[0].target = self.voice_params.osc_a_semi;
        self.smooth[1].target = self.voice_params.osc_a_fine_cents;
        self.smooth[2].target = self.voice_params.osc_a_pw;
        self.smooth[3].target = self.voice_params.osc_b_semi;
        self.smooth[4].target = self.voice_params.osc_b_fine_cents;
        self.smooth[5].target = self.voice_params.osc_b_pw;
        self.smooth[6].target = self.voice_params.mix_osc_a;
        self.smooth[7].target = self.voice_params.mix_osc_b;
        self.smooth[8].target = self.voice_params.mix_noise;
        self.smooth[9].target = self.voice_params.noise_pink_blend;
        self.smooth[10].target = self.voice_params.filter_drive;
        self.smooth[11].target = self.voice_params.filter_env_amount;
        self.smooth[12].target = self.voice_params.filter_key_track;
        self.smooth[13].target = self.voice_params.filter_topology_morph;
        self.smooth[14].target = self.voice_params.moog_bass_comp;
        self.smooth[15].target = self.voice_params.vintage_amount;
        self.smooth[16].target = self.voice_params.pitch_bend_semis;
        self.smooth[17].target = self.voice_params.aftertouch;
        self.smooth[18].target = self.voice_params.velocity_sensitivity;
        self.smooth[19].target = self.voice_params.poly_mod.src_filter_env;
        self.smooth[20].target = self.voice_params.poly_mod.src_osc_b;
        self.smooth[21].target = self.voice_params.poly_mod.dest_osc_a_freq;
        self.smooth[22].target = self.voice_params.poly_mod.dest_osc_a_pw;
        self.smooth[23].target = self.voice_params.poly_mod.dest_filter_cutoff;

        let settled = self.smooth.iter().all(|s| s.current == s.target)
            && self.cutoff_smoother.current == self.cutoff_smoother.target
            && self.res_smoother.current == self.res_smoother.target
            && self.gain_smoother.current == self.gain_smoother.target;

        if settled {
            local_params.filter_cutoff_hz = self.cutoff_smoother.current;
            local_params.filter_resonance = self.res_smoother.current;
            let gain = self.gain_smoother.current;
            for i in 0..n {
                let (_raw_lfo, lfo_mod) = self.global_lfo.process_sample();
                let (dry_l, dry_r) = self.allocator.process_sample(&local_params, lfo_mod);
                let (chor_l, chor_r) = self.chorus.process_stereo(dry_l, dry_r);
                let (rev_l, rev_r) = self
                    .reverb
                    .process_stereo(chor_l, chor_r, &self.reverb_params);

                let final_l = (rev_l * gain).clamp(-1.0, 1.0);
                let final_r = (rev_r * gain).clamp(-1.0, 1.0);

                out_l[i] = final_l;
                out_r[i] = final_r;
                self.last_telemetry.scope_samples_l[i] = final_l;
                self.last_telemetry.scope_samples_r[i] = final_r;
            }
        } else {
            for i in 0..n {
                local_params.osc_a_semi = self.smooth[0].next();
                local_params.osc_a_fine_cents = self.smooth[1].next();
                local_params.osc_a_pw = self.smooth[2].next();
                local_params.osc_b_semi = self.smooth[3].next();
                local_params.osc_b_fine_cents = self.smooth[4].next();
                local_params.osc_b_pw = self.smooth[5].next();
                local_params.mix_osc_a = self.smooth[6].next();
                local_params.mix_osc_b = self.smooth[7].next();
                local_params.mix_noise = self.smooth[8].next();
                local_params.noise_pink_blend = self.smooth[9].next();
                local_params.filter_drive = self.smooth[10].next();
                local_params.filter_env_amount = self.smooth[11].next();
                local_params.filter_key_track = self.smooth[12].next();
                local_params.filter_topology_morph = self.smooth[13].next();
                local_params.moog_bass_comp = self.smooth[14].next();
                local_params.vintage_amount = self.smooth[15].next();
                local_params.pitch_bend_semis = self.smooth[16].next();
                local_params.aftertouch = self.smooth[17].next();
                local_params.velocity_sensitivity = self.smooth[18].next();
                local_params.poly_mod.src_filter_env = self.smooth[19].next();
                local_params.poly_mod.src_osc_b = self.smooth[20].next();
                local_params.poly_mod.dest_osc_a_freq = self.smooth[21].next();
                local_params.poly_mod.dest_osc_a_pw = self.smooth[22].next();
                local_params.poly_mod.dest_filter_cutoff = self.smooth[23].next();

                local_params.filter_cutoff_hz = self.cutoff_smoother.next();
                local_params.filter_resonance = self.res_smoother.next();

                let (_raw_lfo, lfo_mod) = self.global_lfo.process_sample();
                let (dry_l, dry_r) = self.allocator.process_sample(&local_params, lfo_mod);
                let (chor_l, chor_r) = self.chorus.process_stereo(dry_l, dry_r);
                let (rev_l, rev_r) = self
                    .reverb
                    .process_stereo(chor_l, chor_r, &self.reverb_params);

                let gain = self.gain_smoother.next();
                let final_l = (rev_l * gain).clamp(-1.0, 1.0);
                let final_r = (rev_r * gain).clamp(-1.0, 1.0);

                out_l[i] = final_l;
                out_r[i] = final_r;
                self.last_telemetry.scope_samples_l[i] = final_l;
                self.last_telemetry.scope_samples_r[i] = final_r;
            }
        }

        // Update lock-free telemetry snapshot for 12-Voice Diagnostic Matrix
        let mut mask = 0u16;
        for v_idx in 0..NUM_VOICES {
            let v = &self.allocator.voices[v_idx];
            if v.is_active() {
                mask |= 1 << v_idx;
            }
            self.last_telemetry.voice_vca_levels[v_idx] = v.current_vca_level;
            self.last_telemetry.voice_filt_env_levels[v_idx] = v.filter_env.level;
            self.last_telemetry.voice_midi_notes[v_idx] = v.midi_note;
        }
        self.last_telemetry.active_voice_mask = mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cem3340_hard_sync_and_polyblep_bounded() {
        let mut vco = osc_cem3340::Cem3340DualVco::new(96_000.0, 1.5, -1.2);
        let waves_a = osc_cem3340::OscAWaveforms {
            saw: true,
            pulse: true,
        };
        let waves_b = osc_cem3340::OscBWaveforms {
            saw: true,
            triangle: true,
            pulse: false,
            lo_freq: false,
            key_tracking: true,
        };
        let mut energy = 0.0f32;
        for _ in 0..1024 {
            let (a, b) = vco.process_sample(659.25, 220.0, 0.35, 0.5, waves_a, waves_b, true);
            assert!(a.is_finite() && b.is_finite());
            assert!(a.abs() <= 2.5 && b.abs() <= 2.5);
            energy += a * a;
        }
        assert!(energy > 10.0);
    }

    #[test]
    fn test_zdf_filters_self_oscillation_and_stability() {
        let mut vcf = vcf_moog::DualVcfStage::new(96_000.0, 1.0);
        // Mode 1: CEM3320 high resonance
        vcf.set_params(1200.0, 0.95, 1.4, 0.5, 0.0);
        for i in 0..512 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let y = vcf.process_sample(x);
            assert!(y.is_finite() && y.abs() < 4.0);
        }
        // Mode 2: Moog Transistor Ladder high resonance + Bass Compensation
        vcf.set_params(800.0, 0.95, 1.6, 0.85, 1.0);
        for i in 0..512 {
            let x = ((i as f32) * 0.05).sin() * 0.5;
            let y = vcf.process_sample(x);
            assert!(y.is_finite() && y.abs() < 4.0);
        }
    }

    #[test]
    fn test_kernel_12_voices_and_fx_pipeline() {
        let mut kernel = ProphetXiiKernel::new(48_000.0);
        kernel.allocator.note_on(60, 0.9, &kernel.voice_params);
        kernel.allocator.note_on(64, 0.85, &kernel.voice_params);
        kernel.allocator.note_on(67, 0.8, &kernel.voice_params);

        let mut out_l = [0.0f32; MAX_BLOCK_SIZE];
        let mut out_r = [0.0f32; MAX_BLOCK_SIZE];
        kernel.process_block(&mut out_l, &mut out_r, MAX_BLOCK_SIZE);

        assert_eq!(kernel.last_telemetry.active_voice_mask.count_ones(), 3);
        assert!(out_l.iter().all(|x| x.is_finite()));
        assert!(out_r.iter().all(|x| x.is_finite()));
    }
}
