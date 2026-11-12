use prophet_dsp::fx::solaris_reverb::ReverbTailPolicy;
use prophet_params::ParamId;
use prophet_standalone::*;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
fn processor() -> (
    rtrb::Producer<Command>,
    rtrb::Consumer<Telemetry>,
    AudioProcessor,
) {
    let (tx, rx) = rtrb::RingBuffer::new(512);
    let (tt, tr) = rtrb::RingBuffer::new(8);
    (
        tx,
        tr,
        AudioProcessor::new(48000., rx, tt, Arc::new(AtomicU64::new(0))),
    )
}
#[test]
fn learn_cancel_unlearn_remap_and_reserved_controllers() {
    let (_, _, mut p) = processor();
    p.command(Command::Learn(23));
    p.command(Command::CancelLearn);
    p.command(Command::Midi {
        status: 0xb0,
        data1: 16,
        data2: 64,
    });
    assert_eq!(
        p.engine.ipc.params.cc_map[16].load(Ordering::Relaxed),
        u32::MAX
    );
    p.command(Command::Learn(23));
    p.command(Command::Midi {
        status: 0xb0,
        data1: 16,
        data2: 64,
    });
    assert!(!p
        .engine
        .ipc
        .params
        .midi_learn_active
        .load(Ordering::Relaxed));
    assert_eq!(p.engine.ipc.params.cc_map[16].load(Ordering::Relaxed), 23);
    assert_eq!(
        p.engine.ipc.params.cc_map[74].load(Ordering::Relaxed),
        u32::MAX
    );
    p.command(Command::Remap { cc: 17, id: 23 });
    assert_eq!(
        p.engine.ipc.params.cc_map[16].load(Ordering::Relaxed),
        u32::MAX
    );
    p.command(Command::Remap { cc: 64, id: 23 });
    assert_eq!(p.engine.ipc.params.cc_map[17].load(Ordering::Relaxed), 23);
    p.command(Command::Unlearn(23));
    assert_eq!(
        p.engine.ipc.params.cc_map[17].load(Ordering::Relaxed),
        u32::MAX
    );
    p.command(Command::Learn(usize::MAX));
    assert!(!p
        .engine
        .ipc
        .params
        .midi_learn_active
        .load(Ordering::Relaxed));
}
#[test]
fn queued_parameter_precedes_note_on_and_revisions_are_observable() {
    let (mut tx, mut tr, mut p) = processor();
    let before = p.engine.ipc.params.revision.load(Ordering::Relaxed);
    tx.push(Command::Param {
        id: ParamId::UnisonEnabled as usize,
        value: 1.,
    })
    .unwrap();
    tx.push(Command::Param {
        id: ParamId::UnisonVoices as usize,
        value: 12.,
    })
    .unwrap();
    tx.push(Command::Midi {
        status: 0x90,
        data1: 60,
        data2: 100,
    })
    .unwrap();
    let mut out = [0.; 512];
    p.process(&mut out, 2);
    assert_eq!(
        p.engine
            .kernel
            .last_telemetry
            .active_voice_mask
            .count_ones(),
        12
    );
    for _ in 0..8 {
        p.process(&mut out, 2);
    }
    let t = tr.pop().unwrap();
    assert!(t.revision > before);
    assert_eq!(t.kernel.amp_envelopes[0].stage, 2);
    assert!(t.kernel.amp_envelopes[0].elapsed_samples > 0);
    assert!(t.kernel.amp_envelopes[0].timed);
}
#[test]
fn capture_is_contiguous_bounded_and_gaps_have_sequence_numbers() {
    let (_, _, mut p) = processor();
    let (ct, mut cr) = rtrb::RingBuffer::new(4);
    p.attach_capture(ct);
    let mut out = [0.; 512];
    p.process(&mut out, 2);
    p.process(&mut out, 2);
    for i in 0..4 {
        let c = cr.pop().unwrap();
        assert_eq!(c.sequence, i);
        assert_eq!(c.first_frame, i * 64);
        assert_eq!(c.len, 64);
        assert_eq!(c.sample_rate, 48000.);
    }
    p.process(&mut out, 2);
    assert_eq!(cr.pop().unwrap().sequence, 8);
}
#[test]
fn command_wait_and_bypass_status_are_measured() {
    let (mut tx, rx) = rtrb::RingBuffer::new(8);
    let (tt, mut tr) = rtrb::RingBuffer::new(8);
    let mut p = AudioProcessor::new_stamped(48000., rx, tt, Arc::new(AtomicU64::new(0)));
    let now = Instant::now();
    tx.push(StampedCommand {
        command: Command::ReverbBypass {
            enabled: false,
            tail: ReverbTailPolicy::Kill,
        },
        id: 42,
        received: now - Duration::from_millis(3),
        queued: now - Duration::from_millis(1),
    })
    .unwrap();
    let mut out = [0.; 512];
    for _ in 0..9 {
        p.process(&mut out, 2);
    }
    let t = tr.pop().unwrap();
    assert_eq!(t.last_command_id, 42);
    assert_eq!(t.applied_frame, 0);
    assert!(t.control_wait_us >= 3000);
    assert!(t.relay_wait_us >= 2000);
    assert!(t.rt_queue_wait_us >= 1000);
    assert!(!t.reverb_enabled && t.reverb_kill && t.reverb_dormant);
    assert_eq!(t.reverb_enable_gain, 0.);
    assert_eq!(t.callback_frames, 256);
    assert!(t.processing_ns > 0 && t.timing_count > 0);
}
#[test]
fn invalid_callbacks_produce_silence_instead_of_dividing_by_zero() {
    let (_, _, mut p) = processor();
    let mut out = [1.; 3];
    p.process(&mut out, 0);
    assert_eq!(out, [0.; 3]);
    out.fill(1.);
    p.process(&mut out, 2);
    assert_eq!(out, [0.; 3]);
}
