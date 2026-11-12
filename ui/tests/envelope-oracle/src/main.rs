//! Read-only reference generator using the actual baseline kernel, not a JS model.
use prophet_dsp::env_cem3310::{Cem3310Envelope, EnvStage};

fn main() {
    println!("[");
    let mut separator = "";
    for sr in [48000.0, 96000.0] {
        for attack in [0.001, 1.0] {
            for sustain in [0.0, 0.5, 1.0] {
                for release in [0.01, 5.0] {
                    let mut e = Cem3310Envelope::new(sr, 1.0);
                    e.set_adsr(attack, 0.55, sustain, release);
                    e.note_on(true);
                    let mut a = 0u64;
                    while e.stage == EnvStage::Attack { e.process_sample(); a += 1; }
                    let mut d = 0u64;
                    while e.stage == EnvStage::Decay { e.process_sample(); d += 1; }
                    assert_eq!(e.stage, EnvStage::Sustain);
                    e.note_off();
                    let mut r = 0u64;
                    while e.stage == EnvStage::Release { e.process_sample(); r += 1; }
                    assert_eq!(e.stage, EnvStage::Idle);
                    print!("{separator}{{\"sampleRate\":{sr},\"attack\":{attack},\"decay\":0.55,\"sustain\":{sustain},\"release\":{release},\"attackSamples\":{a},\"decaySamples\":{d},\"releaseSamples\":{r}}}");
                    separator = ",\n";
                }
            }
        }
    }
    println!("\n]");
}
