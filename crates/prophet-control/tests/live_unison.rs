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
    assert_eq!(
        p.kernel.allocator.voices.iter().filter(|v| v.gate).count(),
        5
    );
    assert_eq!(p.kernel.allocator.voices[0].note_age, age);
    let (mut l, mut r) = ([0.; 480], [0.; 480]);
    p.process_audio_block(&mut l, &mut r);
    assert_eq!(p.kernel.allocator.voices[0].unison_pan, -0.25);
    assert!(l.iter().all(|v| v.is_finite()));
    p.ipc.params.set(UnisonVoices, 1.0);
    p.sync_parameters();
    assert_eq!(
        p.kernel.allocator.voices.iter().filter(|v| v.gate).count(),
        1
    );
    p.handle_midi_message(0x80, 64, 0);
    assert!(p.kernel.allocator.voices.iter().all(|v| !v.gate));
}

#[test]
fn mpe_follows_owner_through_pedal_stack_edits_and_fallback() {
    let mut p = ProphetXiiPlugin::default();
    p.ipc.params.set(UnisonEnabled, 1.);
    p.ipc.params.set(UnisonVoices, 2.);
    p.sync_parameters();
    p.handle_midi_message(0x91, 64, 96);
    p.handle_midi_message(0xd1, 95, 0);
    p.handle_midi_message(0xb1, 64, 127);
    p.handle_midi_message(0x81, 64, 0);
    p.ipc.params.set(UnisonVoices, 5.);
    p.sync_parameters();
    let held = || 95. / 127.;
    assert!(p.kernel.allocator.voices[..5]
        .iter()
        .all(|v| v.gate && v.mpe_pressure == held()));
    p.handle_midi_message(0x92, 64, 110);
    p.handle_midi_message(0xd2, 30, 0);
    p.handle_midi_message(0xd1, 100, 0);
    assert!(p.kernel.allocator.voices[..5]
        .iter()
        .all(|v| v.mpe_pressure == 30. / 127.));
    p.handle_midi_message(0x82, 64, 0);
    assert!(p.kernel.allocator.voices[..5]
        .iter()
        .all(|v| v.mpe_pressure == 100. / 127.));
    p.handle_midi_message(0xb1, 64, 0);
    assert!(p.kernel.allocator.voices.iter().all(|v| !v.gate));
}

#[test]
fn glide_laws_and_mid_note_disable_at_three_rates() {
    for sr in [44100., 48000., 96000.] {
        for law in [0., 1.] {
            let mut p = ProphetXiiPlugin::new(sr);
            p.handle_midi_message(0x90, 48, 96);
            p.handle_midi_message(0x80, 48, 0);
            p.ipc.params.set(GlideTime, 0.1);
            p.ipc.params.set(GlideMode, law);
            p.ipc.params.set_glide_enabled(true);
            p.sync_parameters();
            p.handle_midi_message(0x90, 72, 96);
            let samples = (0.05_f32 * sr).round() as usize;
            let mut left = [0.; 480];
            let mut right = left;
            for i in (0..samples).step_by(480) {
                let n = (samples - i).min(480);
                p.process_audio_block(&mut left[..n], &mut right[..n]);
            }
            let pitch = |p: &ProphetXiiPlugin| {
                p.kernel
                    .allocator
                    .voices
                    .iter()
                    .find(|v| v.gate)
                    .unwrap()
                    .current_note_pitch
            };
            let expected = if law == 0. { 54. } else { 60. };
            assert!(
                (pitch(&p) - expected).abs() < 0.02,
                "rate={sr} law={law} pitch={}",
                pitch(&p)
            );
            p.ipc.params.set_glide_enabled(false);
            p.process_audio_block(&mut left[..1], &mut right[..1]);
            assert_eq!(pitch(&p), 72.);
            assert_eq!(p.ipc.params.get(GlideTime), 0.1);
        }
    }
}
