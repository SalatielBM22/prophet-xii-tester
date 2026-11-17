use prophet_control::ProphetXiiPlugin;
use prophet_params::ParamId::*;

#[test]
fn held_stack_survives_live_count_and_spread() {
    let mut p = ProphetXiiPlugin::default();
    p.ipc.params.set(UnisonEnabled, 1.0);
    p.ipc.params.set(UnisonVoices, 2.0);
    p.sync_parameters();
    p.handle_midi_message(0x90, 64, 100);
    let age = p.kernel.allocator.voices[0].note_age;
    p.ipc.params.set(UnisonVoices, 5.0);
    p.ipc.params.set(UnisonDetune, 32.0);
    p.ipc.params.set(UnisonSpread, 0.25);
    p.sync_parameters();
    assert_eq!(p.kernel.allocator.voices.iter().filter(|v|v.gate).count(),5);
    assert_eq!(p.kernel.allocator.voices[0].note_age, age);
    let (mut l,mut r)=([0.;480],[0.;480]);
    p.process_audio_block(&mut l,&mut r);
    assert_eq!(p.kernel.allocator.voices[0].unison_pan,-0.25);
    assert!(l.iter().all(|v|v.is_finite()));
    p.ipc.params.set(UnisonVoices, 1.0);
    p.sync_parameters();
    assert_eq!(p.kernel.allocator.voices.iter().filter(|v|v.gate).count(),1);
    p.handle_midi_message(0x80,64,0);
    assert!(p.kernel.allocator.voices.iter().all(|v|!v.gate));
}
