// SPDX-License-Identifier: GPL-3.0-only
//! Gate B5 deterministic RT callback benchmark across topologies, voice counts,
//! sample rates, oversampling modes, block sizes, filters, reverbs, sequencer,
//! and multi-instance host loads — with a global RT allocation counter.

use prophet_control::port::Prepare;
use prophet_host::{
    create_host_session, HostControlPlane, RtHostSlot, UiCommand, UiRoute, UiTopology,
    WireAlgorithm, WireTailPolicy,
};
use prophet_params::ParamId;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

struct CountingAlloc;

static COUNT_ALLOCS: AtomicBool = AtomicBool::new(false);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static DEALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNT_ALLOCS.load(Ordering::Relaxed) {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if COUNT_ALLOCS.load(Ordering::Relaxed) {
            DEALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[derive(Clone)]
struct BenchCase {
    name: &'static str,
    topology: UiTopology,
    voices: usize,
    sample_rate: u32,
    oversampling: u8,
    block_frames: usize,
    moog_morph: f32,
    reverb: WireAlgorithm,
    sequencer: bool,
    instances: usize,
    measured_blocks: usize,
}

fn configure_session(case: &BenchCase) -> (RtHostSlot, HostControlPlane) {
    let prepare = Prepare {
        sample_rate: case.sample_rate,
        max_frames: case.block_frames as u16,
        oversampling: case.oversampling,
    };
    let (mut rt, mut ctrl) =
        create_host_session("bench-host", vec!["rt-bench"], prepare, true).expect("prepare");

    let snap = ctrl.snapshot();
    let _ = ctrl.dispatch(UiCommand::SetOversampling {
        factor: case.oversampling,
    });
    let _ = ctrl.dispatch(UiCommand::SetPatch {
        topology: case.topology,
        routes: [UiRoute::Sum, UiRoute::Frequency],
        ratios: [1.0, 2.0],
        stages: snap.patch.stages,
        modulation: snap.patch.modulation,
        loop_held: true,
    });
    let _ = ctrl.dispatch(UiCommand::SetReverb {
        algorithm: Some(case.reverb),
        enabled: Some(true),
        tail: Some(WireTailPolicy::Spill),
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::FilterMorph as u16,
        value: case.moog_morph,
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::ReverbWet as u16,
        value: 0.35,
    });
    let _ = ctrl.dispatch(UiCommand::VectorXY { x: 0.2, y: 0.25 });

    // Pump 4 silent blocks to apply prepared oversampling/patch swaps before triggering voices
    let mut left = vec![0.0f32; case.block_frames];
    let mut right = vec![0.0f32; case.block_frames];
    for _ in 0..4 {
        let _ = rt.process_planar_f32(&mut left, &mut right, &[], None, None, None);
        ctrl.poll();
    }

    if case.sequencer {
        let _ = ctrl.dispatch(UiCommand::MusicControl {
            op: "start".to_string(),
            pattern: None,
            pattern_len: None,
            step_index: None,
            step: None,
            tempo_milli: Some(128_000),
            swing: Some(10),
            arp_enabled: Some(true),
            arp_order: None,
            arp_octaves: Some(2),
            arp_latch: Some(true),
            arp_seed: None,
            grid_step: None,
            frame: None,
        });
    }

    // Trigger chord of `case.voices` active polyphonic notes
    for v in 0..case.voices {
        let key = (48 + v * 2) as u8;
        let _ = ctrl.dispatch(UiCommand::Midi {
            status: 0x90,
            data1: key,
            data2: 100,
            note_id: Some((v + 1) as u32),
        });
    }
    (rt, ctrl)
}

fn run_case(case: &BenchCase) -> String {
    let mut sessions: Vec<(RtHostSlot, HostControlPlane)> = (0..case.instances)
        .map(|_| configure_session(case))
        .collect();
    let mut left = vec![0.0f32; case.block_frames];
    let mut right = vec![0.0f32; case.block_frames];

    // Warmup 32 blocks so swaps, voices, and reverb state are fully active
    for _ in 0..32 {
        for (rt, _) in &mut sessions {
            let _ = rt.process_planar_f32(&mut left, &mut right, &[], None, None, None);
        }
        for (_, ctrl) in &mut sessions {
            ctrl.poll();
        }
    }

    let mut times_us = Vec::with_capacity(case.measured_blocks);
    let mut non_finite = 0usize;
    let mut max_peak = 0.0f32;
    let mut max_boundary_jump = 0.0f32;
    let mut prev_l = left[case.block_frames - 1];

    ALLOC_COUNT.store(0, Ordering::SeqCst);
    DEALLOC_COUNT.store(0, Ordering::SeqCst);
    COUNT_ALLOCS.store(true, Ordering::SeqCst);

    for _ in 0..case.measured_blocks {
        let t0 = Instant::now();
        for (rt, _) in &mut sessions {
            let _ = rt.process_planar_f32(&mut left, &mut right, &[], None, None, None);
        }
        let elapsed_us = t0.elapsed().as_secs_f64() * 1_000_000.0;
        times_us.push(elapsed_us);

        let jump = (left[0] - prev_l).abs();
        if jump > max_boundary_jump {
            max_boundary_jump = jump;
        }
        prev_l = left[case.block_frames - 1];
        for (&l, &r) in left.iter().zip(right.iter()) {
            if !l.is_finite() || !r.is_finite() {
                non_finite += 1;
            }
            let p = l.abs().max(r.abs());
            if p > max_peak {
                max_peak = p;
            }
        }
    }

    COUNT_ALLOCS.store(false, Ordering::SeqCst);
    let rt_allocs = ALLOC_COUNT.load(Ordering::SeqCst);
    let rt_deallocs = DEALLOC_COUNT.load(Ordering::SeqCst);

    times_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = times_us.len();
    let pct = |p: f64| -> f64 {
        let idx = ((n as f64 - 1.0) * p).round() as usize;
        times_us[idx.min(n - 1)] / 1000.0
    };
    let mean_ms = (times_us.iter().sum::<f64>() / n as f64) / 1000.0;
    let p50_ms = pct(0.50);
    let p95_ms = pct(0.95);
    let p99_ms = pct(0.99);
    let max_ms = times_us[n - 1] / 1000.0;
    let budget_ms = (case.block_frames as f64 / case.sample_rate as f64) * 1000.0;
    let over_budget = times_us
        .iter()
        .filter(|&&u| (u / 1000.0) > budget_ms)
        .count();
    let cpu_pct = (mean_ms / budget_ms) * 100.0;
    let status = if over_budget == 0 && rt_allocs == 0 && rt_deallocs == 0 && non_finite == 0 {
        "PASS"
    } else if p99_ms <= budget_ms && rt_allocs == 0 && rt_deallocs == 0 && non_finite == 0 {
        "PARTIAL"
    } else {
        "FAILED"
    };

    format!(
        "{{\"name\":\"{}\",\"topology\":\"{:?}\",\"voices\":{},\"sr\":{},\"os\":{},\"frames\":{},\"filter\":\"{}\",\"reverb\":\"{:?}\",\"seq\":{},\"instances\":{},\"blocks\":{},\"budget_ms\":{:.3},\"mean_ms\":{:.3},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"over_budget\":{},\"cpu_pct\":{:.2},\"rt_allocs\":{},\"rt_deallocs\":{},\"non_finite\":{},\"peak\":{:.4},\"max_boundary_jump\":{:.4},\"status\":\"{}\"}}",
        case.name,
        case.topology,
        case.voices,
        case.sample_rate,
        case.oversampling,
        case.block_frames,
        if case.moog_morph < 0.5 { "Curtis" } else { "Moog" },
        case.reverb,
        case.sequencer,
        case.instances,
        case.measured_blocks,
        budget_ms,
        mean_ms,
        p50_ms,
        p95_ms,
        p99_ms,
        max_ms,
        over_budget,
        cpu_pct,
        rt_allocs,
        rt_deallocs,
        non_finite,
        max_peak,
        max_boundary_jump,
        status
    )
}

fn main() {
    let cases = vec![
        BenchCase { name: "classic2_1v_48k_1x_256_curtis_legacy", topology: UiTopology::Classic2, voices: 1, sample_rate: 48000, oversampling: 1, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "classic2_6v_48k_2x_256_curtis_legacy", topology: UiTopology::Classic2, voices: 6, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "classic2_12v_44k1_2x_128_moog_legacy", topology: UiTopology::Classic2, voices: 12, sample_rate: 44100, oversampling: 2, block_frames: 128, moog_morph: 1.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "classic2_12v_48k_2x_256_moog_diffuse", topology: UiTopology::Classic2, voices: 12, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 1.0, reverb: WireAlgorithm::DiffuseV1, sequencer: true, instances: 1, measured_blocks: 300 },
        BenchCase { name: "classic2_12v_48k_4x_256_curtis_legacy", topology: UiTopology::Classic2, voices: 12, sample_rate: 48000, oversampling: 4, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "extended3_12v_48k_2x_256_curtis_legacy", topology: UiTopology::Extended3, voices: 12, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "extended4_12v_48k_2x_256_moog_diffuse", topology: UiTopology::Extended4, voices: 12, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 1.0, reverb: WireAlgorithm::DiffuseV1, sequencer: true, instances: 1, measured_blocks: 300 },
        BenchCase { name: "vector4_1v_48k_2x_256_curtis_legacy", topology: UiTopology::Vector4, voices: 1, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "vector4_6v_48k_2x_128_curtis_diffuse", topology: UiTopology::Vector4, voices: 6, sample_rate: 48000, oversampling: 2, block_frames: 128, moog_morph: 0.0, reverb: WireAlgorithm::DiffuseV1, sequencer: true, instances: 1, measured_blocks: 300 },
        BenchCase { name: "vector4_12v_48k_1x_64_curtis_legacy", topology: UiTopology::Vector4, voices: 12, sample_rate: 48000, oversampling: 1, block_frames: 64, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "vector4_12v_48k_2x_256_curtis_legacy", topology: UiTopology::Vector4, voices: 12, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 300 },
        BenchCase { name: "vector4_12v_48k_2x_512_moog_diffuse_seq", topology: UiTopology::Vector4, voices: 12, sample_rate: 48000, oversampling: 2, block_frames: 512, moog_morph: 1.0, reverb: WireAlgorithm::DiffuseV1, sequencer: true, instances: 1, measured_blocks: 200 },
        BenchCase { name: "vector4_12v_48k_4x_256_curtis_legacy", topology: UiTopology::Vector4, voices: 12, sample_rate: 48000, oversampling: 4, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 250 },
        BenchCase { name: "vector4_12v_96k_2x_480_curtis_legacy", topology: UiTopology::Vector4, voices: 12, sample_rate: 96000, oversampling: 2, block_frames: 480, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 250 },
        BenchCase { name: "CRITICAL_vector4_12v_96k_4x_480_curtis_legacy", topology: UiTopology::Vector4, voices: 12, sample_rate: 96000, oversampling: 4, block_frames: 480, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 1, measured_blocks: 250 },
        BenchCase { name: "CRITICAL_vector4_12v_96k_4x_480_moog_diffuse_seq", topology: UiTopology::Vector4, voices: 12, sample_rate: 96000, oversampling: 4, block_frames: 480, moog_morph: 1.0, reverb: WireAlgorithm::DiffuseV1, sequencer: true, instances: 1, measured_blocks: 250 },
        BenchCase { name: "multi_4inst_classic2_6v_48k_2x_256", topology: UiTopology::Classic2, voices: 6, sample_rate: 48000, oversampling: 2, block_frames: 256, moog_morph: 0.0, reverb: WireAlgorithm::LegacyRC1, sequencer: false, instances: 4, measured_blocks: 200 },
    ];

    println!("[");
    for (i, c) in cases.iter().enumerate() {
        let line = run_case(c);
        if i + 1 < cases.len() {
            println!("  {},", line);
        } else {
            println!("  {}", line);
        }
    }
    println!("]");
}
