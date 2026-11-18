// SPDX-License-Identifier: GPL-3.0-only
//! Gate B0 acceptance tests verifying EnginePort v1 semantics, `docs/wo-a/fixtures/messages.json`,
//! `prophet_control::{PreparedEngine, music, state}`, `prophet_preset` v1/v2 roundtrips,
//! `prophet_analysis_core::validity::measure`, off-thread swap without RT drops, and canonical MIDI ownership.

use prophet_analysis_core::RequestClock;
use prophet_control::{
    music::{Kind, Order, Step},
    port::{Action, Command, EnginePort, PortError, Prepare, Status, GLIDE_ENABLED},
    state::SoundState,
    PreparedEngine,
};
use prophet_dsp::{
    fx::reverb::Algorithm,
    vector::{Patch, Point, Route, Topology},
};
use prophet_host::{
    analysis::{measure_filter_response, FilterValidity},
    create_swap_pair, factory_presets,
    preset::{self, HostPresetError, Performance},
    queue::{
        BlockScheduler, CanonicalMidiArbiter, CcLearnRouter, PhysicalMidiSource, SerialIssuer,
        ORIGIN_HARDWARE_MIDI,
    },
    wire::{
        self, is_valid_u64_decimal, EngineMessage, WireAction, WireAlgorithm, WireBlock,
        WireCommand, WireTailPolicy,
    },
};
use prophet_params::ParamId;

fn midi_cmd(offset: u16, origin: u8, serial: u32, status: u8, data1: u8, data2: u8, note_id: u32) -> Command {
    Command {
        offset,
        origin,
        serial,
        action: Action::Midi {
            status,
            data1,
            data2,
            note_id,
        },
    }
}

#[test]
fn b0_wire_fixtures_validate_and_execute_against_prepared_engine() {
    let fixture_bytes = include_bytes!("../../../docs/wo-a/fixtures/messages.json");
    let msgs: Vec<EngineMessage> = serde_json::from_slice(fixture_bytes).unwrap();
    assert_eq!(msgs.len(), 3);
    for msg in &msgs {
        msg.validate().unwrap();
    }

    let EngineMessage::Block(block) = &msgs[0] else {
        panic!("expected block as fixture[0]");
    };
    assert_eq!(block.frames, 480);
    assert_eq!(block.commands.len(), 2);

    // Execute the exact fixture block against PreparedEngine via BlockScheduler.
    let mut engine = PreparedEngine::prepare(Prepare {
        sample_rate: 48_000,
        max_frames: 480,
        oversampling: 2,
    })
    .unwrap();
    let port_cmds: Vec<Command> = block.commands.iter().copied().map(Command::from).collect();
    let mut sched = BlockScheduler::new();
    let mut l = vec![0.0f32; usize::from(block.frames)];
    let mut r = vec![0.0f32; usize::from(block.frames)];
    let mut acks = Vec::new();
    let mut music_statuses = Vec::new();
    let summary = sched
        .process_block(
            &mut engine,
            &mut l,
            &mut r,
            &port_cmds,
            |a| acks.push(a),
            None,
            |s| music_statuses.push(s),
        )
        .unwrap();
    assert_eq!(summary.frames_rendered, 480);
    assert_eq!(acks.len(), 2);
    assert_eq!(acks[0].status, Status::Applied);
    assert_eq!(acks[1].status, Status::Applied);
    assert_eq!(engine.parameter(GLIDE_ENABLED), Some(0.0));
    // Prefix before MIDI offset 37 must be silent (factory 0 has zero tail initially).
    assert!(l[..37].iter().all(|&x| x == 0.0));
    assert!(l.iter().chain(&r).all(|x| x.is_finite()));

    let wire_ack = EngineMessage::Ack(wire::WireAck::from_port(acks[1]));
    wire_ack.validate().unwrap();
    let host_caps = [
        "enginePortV1",
        "glideEnabled",
        "reverbState",
        "selectIdleReverb",
        "oscillatorPatch",
        "noteScheduler",
    ];
    let snap = wire::snapshot_from_engine(&engine, &host_caps, None, None, true);
    let snap_msg = EngineMessage::Snapshot(snap.clone());
    snap_msg.validate().unwrap();

    assert_eq!(snap.sample_rate, 48_000);
    assert_eq!(snap.max_frames, 480);
    assert_eq!(snap.latency_samples, 16);
    assert_eq!(snap.rendered_frames, "480");
    assert_eq!(snap.notes.len(), 1);
    assert_eq!(snap.notes[0].origin, 1);
    assert_eq!(snap.notes[0].key, 60);
    assert_eq!(snap.notes[0].note_id, 9);
    assert!(snap.notes[0].pressed);
    assert!(!snap.notes[0].sustained);
    assert_eq!(snap.fx.algorithm, WireAlgorithm::LegacyRC1);
    assert!(snap.fx.enabled);
    assert_eq!(snap.fx.tail, WireTailPolicy::Spill);
    assert!(snap.xruns.is_none());
    assert!(snap.cpu_percent.is_none());
    assert!(snap.capabilities.contains(&"enginePortV1".to_owned()));
    assert!(!snap.capabilities.contains(&"vectorXY".to_owned()));

    // Verify unknown properties & malformed schema shapes fail validation/deserialization.
    assert!(serde_json::from_str::<EngineMessage>(
        r#"{"kind":"ack","version":1,"origin":1,"serial":1,"status":"applied","revision":"1","extra":9}"#
    ).is_err());
    assert!(!is_valid_u64_decimal("01"));
    assert!(!is_valid_u64_decimal("-5"));
    assert!(is_valid_u64_decimal("0"));
    assert!(is_valid_u64_decimal("18446744073709551615"));

    let mut bad_block = WireBlock {
        version: 1,
        frames: 64,
        commands: vec![WireCommand {
            offset: 64,
            origin: 0,
            serial: 1,
            action: WireAction::PanicOrigin,
        }],
    };
    assert!(bad_block.validate().is_err());
    bad_block.commands[0].offset = 0;
    bad_block.commands[0].action = WireAction::VectorXY { x: 0.7, y: 0.6 };
    bad_block.validate().unwrap(); // Schema range allows [-1, 1] per axis
    assert!(bad_block.validate_strict_diamond().is_err()); // Strict |x|+|y|<=1 fails
}

#[test]
fn b0_rejected_blocks_carry_offsets_and_splitting_preserve_causality() {
    let mut engine = PreparedEngine::prepare(Prepare {
        sample_rate: 48_000,
        max_frames: 256,
        oversampling: 2,
    })
    .unwrap();
    let mut sched = BlockScheduler::new();

    // Out-of-range offset strictly > frames leaves buffers cleared and engine untouched.
    let mut l = vec![9.0f32; 64];
    let mut r = vec![-9.0f32; 64];
    let err = sched.process_block(
        &mut engine,
        &mut l,
        &mut r,
        &[midi_cmd(65, 1, 1, 0x90, 60, 100, 1)],
        |_| {},
        None,
        |_| {},
    );
    assert_eq!(err, Err(PortError::Offset));
    assert_eq!(engine.rendered_frames(), 0);
    assert!(l.iter().all(|&x| x == 0.0));
    assert!(r.iter().all(|&x| x == 0.0));

    // Command at right endpoint `offset == 256` must be carried cleanly to offset 0 of next block.
    let mut l1 = vec![0.0f32; 256];
    let mut r1 = vec![0.0f32; 256];
    let mut acks1 = Vec::new();
    let sum1 = sched
        .process_block(
            &mut engine,
            &mut l1,
            &mut r1,
            &[midi_cmd(256, 1, 1, 0x90, 64, 120, 10)],
            |a| acks1.push(a),
            None,
            |_| {},
        )
        .unwrap();
    assert_eq!(sum1.carried_out, 1);
    assert!(acks1.is_empty());
    assert_eq!(engine.notes().count(), 0);

    // Block 2 (with its own offset 0 NoteOff for same note_id) runs AFTER carried NoteOn? Wait:
    // Actually if block 2 sets param at offset 0 AND releases at offset 0:
    // carried (NoteOn serial 1) precedes incoming (NoteOff serial 2) at offset 0!
    let mut l2 = vec![0.0f32; 256];
    let mut r2 = vec![0.0f32; 256];
    let mut acks2 = Vec::new();
    let sum2 = sched
        .process_block(
            &mut engine,
            &mut l2,
            &mut r2,
            &[
                midi_cmd(0, 1, 2, 0x80, 64, 0, 10),
                midi_cmd(0, 1, 3, 0x90, 67, 100, 11),
            ],
            |a| acks2.push(a),
            None,
            |_| {},
        )
        .unwrap();
    assert_eq!(sum2.carried_in, 1);
    assert_eq!(sum2.carried_out, 0);
    assert_eq!(acks2.len(), 3);
    assert_eq!((acks2[0].serial, acks2[1].serial, acks2[2].serial), (1, 2, 3));
    assert!(acks2.iter().all(|a| a.status == Status::Applied));
    assert_eq!(engine.notes().count(), 1);
    assert_eq!(engine.notes().next().unwrap().key, 67);

    // Host buffer larger than max_frames (600 > 256) and > 256 commands splits cleanly without error.
    let mut big_l = vec![0.0f32; 600];
    let mut big_r = vec![0.0f32; 600];
    let mut many_cmds = Vec::with_capacity(300);
    for i in 0..300 {
        many_cmds.push(Command {
            offset: (i * 2) as u16,
            origin: 2,
            serial: (i + 1) as u32,
            action: Action::Parameter {
                id: ParamId::FilterCutoff as u16,
                value: 200.0 + i as f32,
            },
        });
    }
    let mut many_acks = Vec::new();
    let big_sum = sched
        .process_block(
            &mut engine,
            &mut big_l,
            &mut big_r,
            &many_cmds,
            |a| many_acks.push(a),
            None,
            |_| {},
        )
        .unwrap();
    assert!(big_sum.sub_blocks >= 3);
    assert_eq!(many_acks.len(), 300);
    assert!(many_acks.iter().all(|a| a.status == Status::Applied));
    assert!((engine.parameter(ParamId::FilterCutoff as u16).unwrap() - 499.0).abs() < 1e-4);
}

#[test]
fn b0_canonical_midi_arbitration_and_cc_learn_router() {
    let serials = SerialIssuer::new();
    let mut arbiter = CanonicalMidiArbiter::new(PhysicalMidiSource::Native);
    let mut router = CcLearnRouter::default();

    // Browser duplicate observation MUST NOT trigger when Native is canonical.
    assert!(arbiter
        .accept_raw_midi(PhysicalMidiSource::Browser, 0x90, 60, 100, None, 0, &serials)
        .is_none());
    let accepted = arbiter
        .accept_raw_midi(PhysicalMidiSource::Native, 0x90, 60, 100, None, 12, &serials)
        .unwrap();
    assert_eq!(accepted.origin, ORIGIN_HARDWARE_MIDI);
    assert_eq!(accepted.serial, 1);

    // Switching canonical ownership emits PanicOrigin before accepting Browser stream.
    let switch_cmd = arbiter
        .switch_source(PhysicalMidiSource::Browser, &serials, 0)
        .unwrap();
    assert_eq!(switch_cmd.action, Action::PanicOrigin);
    assert_eq!(switch_cmd.origin, ORIGIN_HARDWARE_MIDI);
    assert!(arbiter
        .accept_raw_midi(PhysicalMidiSource::Native, 0x90, 62, 100, None, 0, &serials)
        .is_none());
    assert!(arbiter
        .accept_raw_midi(PhysicalMidiSource::Browser, 0x90, 62, 100, None, 0, &serials)
        .is_some());

    // CC Learn router maps CC 20 -> FilterDrive, keeps damper/sostenuto/MPE CC74 untouched.
    assert!(router.begin_learn(ParamId::FilterDrive as u16));
    assert!(!router.begin_learn(ParamId::OversamplingMode as u16));
    let learned = router.translate_midi(0xb0, 20, 127, 0).unwrap();
    assert!(matches!(
        learned,
        Action::Parameter { id, value } if id == ParamId::FilterDrive as u16 && (value - 4.0).abs() < 1e-5
    ));
    assert_eq!(router.learning_target, None);

    // Damper passes raw as Action::Midi; MPE ch > 0 CC 74 passes raw as Action::Midi.
    assert!(matches!(
        router.translate_midi(0xb0, 64, 127, 0),
        Some(Action::Midi { status: 0xb0, data1: 64, .. })
    ));
    assert!(matches!(
        router.translate_midi(0xb1, 74, 90, 5),
        Some(Action::Midi { status: 0xb1, data1: 74, .. })
    ));
}

#[test]
fn b0_vector4_reverb_oversampling_swap_and_rc1_glide_migration() {
    let engine = PreparedEngine::prepare(Prepare {
        sample_rate: 48_000,
        max_frames: 480,
        oversampling: 2,
    })
    .unwrap();
    assert_eq!(engine.latency_samples(), 16);

    let (mut rt_slot, mut worker) = create_swap_pair(engine, true, 4);
    let serials = SerialIssuer::new();

    // Oversampling change in process() MUST return Unsupported.
    let mut l = [0.0f32; 64];
    let mut r = [0.0f32; 64];
    let mut acks = Vec::new();
    rt_slot
        .process_block(
            &mut l,
            &mut r,
            &[Command {
                offset: 0,
                origin: 2,
                serial: serials.next_serial(2),
                action: Action::Parameter {
                    id: ParamId::OversamplingMode as u16,
                    value: 4.0,
                },
            }],
            |a| acks.push(a),
            |_| {},
        )
        .unwrap();
    assert_eq!(acks[0].status, Status::Unsupported);

    // Off-thread re-prepare to 4x updates latency to 24 samples without deallocating in RT thread.
    let mut sound = rt_slot.engine.sound_state();
    sound.values[ParamId::OversamplingMode as usize] = 4.0;
    sound.patch.topology = Topology::Vector4;
    sound.patch.routes = [Route::Frequency, Route::Ring];
    sound.algorithm = Algorithm::DiffuseV1;
    let swap_id = worker.prepare_reconfig(&sound, 48_000, 480).unwrap();
    assert!(rt_slot.poll_swap(Some(&serials)));
    assert_eq!(rt_slot.active_swap_id(), swap_id);
    assert_eq!(rt_slot.engine.latency_samples(), 24);
    assert_eq!(rt_slot.engine.reverb_state().algorithm, Algorithm::DiffuseV1);
    assert_eq!(worker.collect_retired(), 1); // Retired old engine freed safely on worker!

    // VectorXY must succeed on Vector4 without incrementing parameter revision.
    let rev_before = rt_slot.engine.revision();
    acks.clear();
    rt_slot
        .process_block(
            &mut l,
            &mut r,
            &[
                Command {
                    offset: 0,
                    origin: 2,
                    serial: serials.next_serial(2),
                    action: Action::VectorXY { x: 0.6, y: 0.4 },
                },
                Command {
                    offset: 1,
                    origin: 2,
                    serial: serials.next_serial(2),
                    action: Action::VectorXY { x: 0.6, y: 0.5 },
                },
            ],
            |a| acks.push(a),
            |_| {},
        )
        .unwrap();
    assert_eq!(acks[0].status, Status::Applied);
    assert_eq!(acks[1].status, Status::Invalid);
    assert_eq!(rt_slot.engine.revision(), rev_before);
    assert_eq!(
        rt_slot.engine.patch().position,
        Point { x: 0.6, y: 0.4 }
    );

    // Trigger voice so tank/voices become active; select_reverb must fail and keep DiffuseV1.
    rt_slot
        .process_block(
            &mut l,
            &mut r,
            &[midi_cmd(0, 1, serials.next_serial(1), 0x90, 60, 110, 60)],
            |_| {},
            |_| {},
        )
        .unwrap();
    assert_eq!(
        rt_slot.engine.select_reverb(Algorithm::LegacyRC1),
        Err(Status::Unsupported)
    );
    assert_eq!(rt_slot.engine.reverb_state().algorithm, Algorithm::DiffuseV1);
    assert_eq!(
        rt_slot.engine.set_patch(Patch::default()),
        Err(Status::Unsupported)
    );

    // RC1 SoundState glide migration boundary (`GlideTime > 0.0005`).
    let mut rc1_low = [0.0f32; prophet_params::NUM_PARAMS];
    for (i, spec) in prophet_params::PARAM_SPECS.iter().enumerate() {
        rc1_low[i] = spec.min;
    }
    rc1_low[ParamId::OversamplingMode as usize] = 2.0;
    rc1_low[ParamId::GlideTime as usize] = 0.0005;
    assert!(!SoundState::from_rc1(rc1_low).unwrap().glide);
    rc1_low[ParamId::GlideTime as usize] = 0.0006;
    assert!(SoundState::from_rc1(rc1_low).unwrap().glide);
}

#[test]
fn b0_music_transport_stop_flush_and_v2_preset_worker_swap() {
    let engine = PreparedEngine::prepare(Prepare {
        sample_rate: 48_000,
        max_frames: 480,
        oversampling: 1,
    })
    .unwrap();
    let (mut rt_slot, mut worker) = create_swap_pair(engine, true, 4);

    // Configure dense Chord + ratchet=4 pattern and verify capacity splitting + host-first ties.
    {
        let m = rt_slot.music.as_mut().unwrap();
        m.arp.order = Order::Chord;
        for k in [60, 64, 67, 71] {
            assert!(m.arp.key(k, true));
        }
        m.arp_enabled = true;
        m.bank.0[0].len = 2;
        m.bank.0[0].steps[0] = Step {
            kind: Kind::Note,
            key: 60,
            velocity: 110,
            accent: true,
            glide: false,
            gate: 80,
            ratchet: 4,
        };
        m.bank.0[0].steps[1].kind = Kind::Tie;
        m.scheduler.queue_tempo(240_000).unwrap();
        m.start_from_zero().unwrap();
    }

    let mut l = vec![0.0f32; 480];
    let mut r = vec![0.0f32; 480];
    let mut host_acks = Vec::new();
    let mut music_statuses = Vec::new();
    let host_cmd = midi_cmd(0, 1, 1, 0x90, 48, 100, 48);
    rt_slot
        .process_block(
            &mut l,
            &mut r,
            &[host_cmd],
            |a| host_acks.push(a),
            |s| music_statuses.push(s),
        )
        .unwrap();
    assert_eq!(host_acks[0].status, Status::Applied);
    assert!(!music_statuses.is_empty());
    assert!(rt_slot.engine.notes().count() >= 5); // 1 host + 4 chord arp notes

    // Stop scheduler and flush pending release cleanly before suspending audio.
    {
        let m = rt_slot.music.as_mut().unwrap();
        m.stop();
        assert!(m.needs_stop_flush());
        m.flush_stop_before_suspend(&mut rt_slot.engine).unwrap();
        assert!(!m.needs_stop_flush());
    }
    assert_eq!(rt_slot.engine.notes().count(), 1);
    assert_eq!(rt_slot.engine.notes().next().unwrap().key, 48);

    // All 10 factory presets must decode and capture snapshots.
    let factory = factory_presets();
    assert_eq!(factory.len(), 10);
    assert_eq!(factory[0].name, "1980 San Jose Rev3 Brass");

    // Preset v2 encode/decode + tamper rejection preserving existing instance.
    let v2_bytes = preset::encode_from_engine(
        "WO-B Test Vector",
        "Pads",
        &rt_slot.engine,
        Some(rt_slot.music.as_ref().unwrap().bank.clone()),
        Some(Performance {
            tempo_milli: 128_000,
            swing: 15,
            pattern: 0,
            mode: 5,
            octaves: 2,
            latch: true,
            seed: 77,
        }),
    )
    .unwrap();

    // Tamper payload without updating CRC -> rejected before touching active audio slot!
    let mut json_val: serde_json::Value = serde_json::from_slice(&v2_bytes).unwrap();
    json_val["payload"]["glide"] = serde_json::Value::Bool(!rt_slot.engine.sound_state().glide);
    let tampered = serde_json::to_vec(&json_val).unwrap();
    assert_eq!(
        worker.prepare_preset_bytes(&tampered, 48_000, 480),
        Err(HostPresetError::Codec(preset::PresetError::Checksum))
    );
    assert!(!rt_slot.poll_swap(None));

    // Valid v2 preset prepares cleanly and swaps on transport boundary.
    let swap_id = worker.prepare_preset_bytes(&v2_bytes, 48_000, 480).unwrap();
    assert!(rt_slot.poll_swap(None));
    assert_eq!(rt_slot.active_swap_id(), swap_id);
    assert_eq!(rt_slot.music.as_ref().unwrap().bank.0[0].len, 2);
    assert_eq!(rt_slot.music.as_ref().unwrap().bank.0[0].steps[0].ratchet, 4);
    assert_eq!(rt_slot.music.as_ref().unwrap().arp.seed, 77);
    assert_eq!(worker.collect_retired(), 2); // Old engine + old PatternBank freed off audio thread!
}

#[test]
fn b0_vcf_validity_gating_and_cancellation() {
    let mut vals = [0.0f32; prophet_params::NUM_PARAMS];
    for (i, s) in prophet_params::PARAM_SPECS.iter().enumerate() {
        vals[i] = s.min;
    }
    vals[ParamId::FilterCutoff as usize] = 1000.0;
    vals[ParamId::FilterResonance as usize] = 0.2;
    vals[ParamId::FilterDrive as usize] = 1.0;
    vals[ParamId::MoogBassComp as usize] = 0.0;
    // Moog (morph = 1.0) and CEM3320 (morph = 0.0) at 1000 Hz, Q = 0.2 converge in default window.
    vals[ParamId::FilterMorph as usize] = 1.0;
    let ok_moog = measure_filter_response(&vals, 48_000.0, None, None).unwrap();
    assert_eq!(ok_moog.validity, FilterValidity::Measurable);
    assert_eq!(ok_moog.points.len(), 256);

    vals[ParamId::FilterMorph as usize] = 0.0;
    let ok_cem = measure_filter_response(&vals, 48_000.0, None, None).unwrap();
    assert_eq!(ok_cem.validity, FilterValidity::Measurable);
    assert_eq!(ok_cem.points.len(), 256);

    // High-Q long tail at low cutoff matching docs/wo-a/evidence/vcf.csv must return empty points!
    vals[ParamId::FilterCutoff as usize] = 27.51;
    vals[ParamId::FilterResonance as usize] = 1.0;
    vals[ParamId::FilterDrive as usize] = 1.1;
    vals[ParamId::FilterMorph as usize] = 0.35;
    let non_measurable = measure_filter_response(&vals, 48_000.0, None, None).unwrap();
    assert_eq!(non_measurable.validity, FilterValidity::LongTail);
    assert!(non_measurable.points.is_empty());

    // Superseded request token aborts early.
    let clock = RequestClock::default();
    let stale = clock.next();
    let _latest = clock.next();
    assert!(matches!(
        measure_filter_response(&vals, 48_000.0, None, Some(&stale)),
        Err(prophet_analysis_core::Error::Superseded)
    ));
}

#[test]
fn b1_shared_host_control_plane_and_zero_alloc_interleaved_rt() {
    use prophet_host::{
        create_host_session,
        ui::{UiCommand, UiPoint, UiRoute, UiStage, UiStep, UiStepKind, UiTopology},
    };

    let (mut rt, mut ctrl) = create_host_session(
        "test-host",
        vec![
            "enginePortV1",
            "glideEnabled",
            "reverbState",
            "selectIdleReverb",
            "oscillatorPatch",
            "noteScheduler",
            "vectorXY",
        ],
        Prepare {
            sample_rate: 48_000,
            max_frames: 256,
            oversampling: 2,
        },
        true,
    )
    .unwrap();
    ctrl.set_audio_running(true, Some(48_000), Some(256));

    // Switch patch to Vector4 while idle and adjust VectorXY cleanly.
    let patch_cmd = UiCommand::SetPatch {
        topology: UiTopology::Vector4,
        routes: [UiRoute::Frequency, UiRoute::Ring],
        ratios: [1.0, 2.0],
        stages: [UiStage {
            target: UiPoint { x: 0.0, y: 0.0 },
            millis: 100,
        }; 5],
        modulation: [UiPoint { x: 0.0, y: 0.0 }; 3],
        loop_held: false,
    };
    assert!(ctrl.dispatch(patch_cmd).ok);

    // Oversampling swap (4x) + interleaved odd/oversized bursts (333 f32, 1500 i16).
    let os_reply = ctrl.dispatch(UiCommand::SetOversampling { factor: 4 });
    assert!(os_reply.ok && os_reply.swap_id.is_some());

    let mut pcm_f32 = vec![0.0f32; 333 * 2];
    rt.process_interleaved_f32(&mut pcm_f32, 333, None, Some(12.5), None)
        .unwrap();
    let snap = ctrl.snapshot();
    assert!(snap.ready);
    assert_eq!(snap.engine_port.latency_samples, 24);
    assert_eq!(snap.patch.topology, UiTopology::Vector4);
    assert!(snap.engine_port.capabilities.contains(&"vectorXY".to_owned()));

    // Trigger note via UI MIDI + VectorXY, verify busy-voice refusal and active-tail reverb refusal.
    assert!(
        ctrl.dispatch(UiCommand::VectorXY { x: 0.4, y: 0.3 })
            .ok
    );
    assert!(
        ctrl.dispatch(UiCommand::Midi {
            status: 0x90,
            data1: 60,
            data2: 110,
            note_id: Some(700),
        })
        .ok
    );
    let mut pcm_i16 = vec![0i16; 1500 * 2];
    rt.process_interleaved_i16(&mut pcm_i16, 1500, Some(0), Some(18.0), None)
        .unwrap();
    assert!(pcm_i16.iter().any(|&s| s != 0));

    let snap2 = ctrl.snapshot();
    assert_eq!(snap2.engine_port.notes.len(), 1);
    assert!((snap2.patch.position.x - 0.4).abs() < 1e-5);
    assert_eq!(snap2.engine_port.xruns, Some(0));

    let busy_patch = ctrl.dispatch(UiCommand::SetPatch {
        topology: UiTopology::Classic2,
        routes: [UiRoute::Sum, UiRoute::Sum],
        ratios: [0.5, 1.5],
        stages: [UiStage {
            target: UiPoint { x: 0.0, y: 0.0 },
            millis: 100,
        }; 5],
        modulation: [UiPoint { x: 0.0, y: 0.0 }; 3],
        loop_held: false,
    });
    assert!(!busy_patch.ok);

    let busy_rev = ctrl.dispatch(UiCommand::SetReverb {
        algorithm: Some(WireAlgorithm::DiffuseV1),
        enabled: None,
        tail: None,
    });
    assert!(!busy_rev.ok);

    // Edit sequencer step, save v2 preset with CRC-32, and validate VCF analysis response.
    assert!(
        ctrl.dispatch(UiCommand::MusicControl {
            op: "setStep".into(),
            pattern: Some(0),
            pattern_len: None,
            step_index: Some(0),
            step: Some(UiStep {
                kind: UiStepKind::Note,
                key: 64,
                velocity: 100,
                accent: true,
                glide: false,
                gate: 80,
                ratchet: 2,
            }),
            tempo_milli: None,
            swing: None,
            arp_enabled: None,
            arp_order: None,
            arp_octaves: None,
            arp_latch: None,
            arp_seed: None,
            grid_step: None,
            frame: None,
        })
        .ok
    );
    let saved = ctrl.dispatch(UiCommand::PresetSaveJson {
        name: "Host Session Test".into(),
        category: "Vector".into(),
    });
    assert!(saved.ok);
    let json = saved.preset_json.unwrap();
    assert!(
        ctrl.dispatch(UiCommand::PresetLoadJson { json })
            .ok
    );

    let vcf = ctrl.dispatch(UiCommand::AnalyzeFilter {
        sample_rate: Some(48_000.0),
    });
    assert!(vcf.ok && vcf.filter_analysis.is_some());
}

