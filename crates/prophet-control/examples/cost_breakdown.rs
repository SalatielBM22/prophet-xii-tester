//! Per-component DSP cost on the current machine (issue #4).
//!
//! Medians over many 256-frame blocks are robust to scheduler jitter, unlike max/p99.
//! Run release: `cargo run --release -p prophet-control --example cost_breakdown`.
use prophet_control::ProphetXiiPlugin;
use prophet_dsp::fx::solaris_reverb::ReverbTailPolicy;
use prophet_params::ParamId;
use std::time::Instant;

const SR: f32 = 48_000.0;
const FRAMES: usize = 256;
const BLOCKS: usize = 1500;

fn median_us(p: &mut ProphetXiiPlugin) -> f64 {
    let (mut l, mut r) = ([0.0f32; FRAMES], [0.0f32; FRAMES]);
    for _ in 0..100 {
        p.process_audio_block(&mut l, &mut r);
    }
    let mut t: Vec<f64> = (0..BLOCKS)
        .map(|_| {
            let s = Instant::now();
            p.process_audio_block(&mut l, &mut r);
            s.elapsed().as_secs_f64() * 1e6
        })
        .collect();
    t.sort_by(f64::total_cmp);
    t[t.len() / 2]
}

fn synth(os: f32, reverb: bool, chorus: bool, voices: u8) -> ProphetXiiPlugin {
    let mut p = ProphetXiiPlugin::new(SR);
    p.load_preset(0);
    let b = &p.ipc.params;
    b.set(ParamId::OversamplingMode, os);
    b.set(ParamId::ChorusMode, if chorus { 1.0 } else { 0.0 });
    b.set_reverb_bypass(reverb, ReverbTailPolicy::Kill);
    p.sync_parameters();
    for k in 0..voices {
        p.handle_midi_message(0x90, 48 + 2 * k, 100);
    }
    p
}

fn main() {
    let budget = FRAMES as f64 / SR as f64 * 1e6;
    println!("block {FRAMES} frames @ {SR} Hz, budget {budget:.0} us; medians of {BLOCKS} blocks");
    let base = median_us(&mut synth(2.0, false, false, 0));
    let chorus = median_us(&mut synth(2.0, false, true, 0)) - base;
    // The reverb goes dormant on silence, so measure it with six voices feeding it.
    let six_dry = median_us(&mut synth(2.0, false, false, 6));
    let reverb = median_us(&mut synth(2.0, true, false, 6)) - six_dry;
    let both = median_us(&mut synth(2.0, true, true, 6)) - six_dry;
    println!("idle kernel            {base:8.1} us");
    println!("chorus (mode I)       +{chorus:8.1} us  ({:.1}% budget)", chorus / budget * 100.0);
    println!("reverb, fed (preset 0)+{reverb:8.1} us  ({:.1}% budget)", reverb / budget * 100.0);
    println!("chorus+reverb, fed    +{both:8.1} us  ({:.1}% budget)", both / budget * 100.0);
    for os in [1.0, 2.0, 4.0] {
        let one = median_us(&mut synth(os, false, false, 1)) - base;
        let twelve = median_us(&mut synth(os, false, false, 12)) - base;
        println!(
            "voices @ {os}x: 1 voice +{one:7.1} us, 12 voices +{twelve:7.1} us ({:.1} us/voice, {:.1}% budget for 12)",
            twelve / 12.0,
            twelve / budget * 100.0
        );
    }
}
