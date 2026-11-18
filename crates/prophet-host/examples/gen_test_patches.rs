// SPDX-License-Identifier: GPL-3.0-only
//! Generates validated v2 CRC-32 test presets under `docs/wo-b/patches/` using the
//! authoritative Rust `prophet_preset` codec and verifies decode/CRC roundtrip.

use prophet_control::port::Prepare;
use prophet_host::{
    create_host_session, decode_and_prepare, UiArpOrder, UiCommand, UiPoint, UiRoute, UiStage,
    UiStep, UiStepKind, UiTopology, WireAlgorithm, WireTailPolicy,
};
use prophet_params::ParamId;
use std::fs;
use std::path::Path;

fn make_patch(
    filename: &str,
    name: &str,
    category: &str,
    topology: UiTopology,
    routes: [UiRoute; 2],
    ratios: [f32; 2],
    reverb: WireAlgorithm,
    moog_morph: f32,
    with_seq: bool,
) {
    let prepare = Prepare {
        sample_rate: 48000,
        max_frames: 256,
        oversampling: 2,
    };
    let (mut rt, mut ctrl) =
        create_host_session("patch-gen", vec!["preset-v2"], prepare, true).expect("session");

    let stages = [
        UiStage { target: UiPoint { x: 0.0, y: 1.0 }, millis: 0 },
        UiStage { target: UiPoint { x: 1.0, y: 0.0 }, millis: 320 },
        UiStage { target: UiPoint { x: 0.0, y: -1.0 }, millis: 420 },
        UiStage { target: UiPoint { x: -1.0, y: 0.0 }, millis: 360 },
        UiStage { target: UiPoint { x: 0.0, y: 0.0 }, millis: 240 },
    ];
    let modulation = [
        UiPoint { x: 0.2, y: -0.15 },
        UiPoint { x: 0.1, y: 0.25 },
        UiPoint { x: 0.0, y: 0.1 },
    ];

    let _ = ctrl.dispatch(UiCommand::SetPatch {
        topology,
        routes,
        ratios,
        stages,
        modulation,
        loop_held: true,
    });
    let _ = ctrl.dispatch(UiCommand::SetReverb {
        algorithm: Some(reverb),
        enabled: Some(true),
        tail: Some(WireTailPolicy::Spill),
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::FilterCutoff as u16,
        value: 4600.0,
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::FilterResonance as u16,
        value: 0.35,
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::FilterMorph as u16,
        value: moog_morph,
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: ParamId::ReverbWet as u16,
        value: 0.32,
    });
    let _ = ctrl.dispatch(UiCommand::Param {
        id: 83, // p83 GlideEnabled
        value: 1.0,
    });
    let _ = ctrl.dispatch(UiCommand::VectorXY { x: 0.2, y: 0.3 });

    if with_seq {
        let _ = ctrl.dispatch(UiCommand::MusicControl {
            op: "setPatternLen".to_string(),
            pattern: Some(0),
            pattern_len: Some(16),
            step_index: None,
            step: None,
            tempo_milli: None,
            swing: None,
            arp_enabled: None,
            arp_order: None,
            arp_octaves: None,
            arp_latch: None,
            arp_seed: None,
            grid_step: None,
            frame: None,
        });
        let notes: [u8; 16] = [48, 55, 60, 63, 48, 55, 62, 67, 51, 58, 63, 67, 53, 60, 65, 70];
        for (i, &key) in notes.iter().enumerate() {
            let _ = ctrl.dispatch(UiCommand::MusicControl {
                op: "setStep".to_string(),
                pattern: Some(0),
                pattern_len: None,
                step_index: Some(i as u8),
                step: Some(UiStep {
                    kind: UiStepKind::Note,
                    key,
                    velocity: if i % 4 == 0 { 120 } else { 96 },
                    accent: i % 4 == 0,
                    glide: i % 8 == 3,
                    gate: 72,
                    ratchet: if i == 14 { 2 } else { 1 },
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
            });
        }
        let _ = ctrl.dispatch(UiCommand::MusicControl {
            op: "setTempo".to_string(),
            pattern: None,
            pattern_len: None,
            step_index: None,
            step: None,
            tempo_milli: Some(122_000),
            swing: Some(12),
            arp_enabled: Some(true),
            arp_order: Some(UiArpOrder::UpDown),
            arp_octaves: Some(2),
            arp_latch: Some(false),
            arp_seed: Some(7),
            grid_step: None,
            frame: None,
        });
    }

    let mut l = [0.0f32; 256];
    let mut r = [0.0f32; 256];
    for _ in 0..4 {
        let _ = rt.process_planar_f32(&mut l, &mut r, &[], None, None, None);
        ctrl.poll();
    }

    let reply = ctrl.dispatch(UiCommand::PresetSaveJson {
        name: name.to_string(),
        category: category.to_string(),
    });
    assert!(reply.ok, "PresetSaveJson failed for {name}: {}", reply.message);
    let json = reply.preset_json.expect("preset_json");

    // Verify strict Rust CRC-32 decode & prepare roundtrip
    let decoded = decode_and_prepare(json.as_bytes(), 48000, 256).expect("decode_and_prepare roundtrip");
    assert_eq!(decoded.preset.name, name);

    let out_path = Path::new("docs/wo-b/patches").join(filename);
    fs::write(&out_path, format!("{json}\n")).expect("write patch");
    println!("Wrote & verified {} ({} bytes)", out_path.display(), json.len());
}

fn main() {
    fs::create_dir_all("docs/wo-b/patches").expect("mkdir patches");
    make_patch(
        "vector4_diamond_glass.prophet.json",
        "Vector4 Diamond Glass",
        "Vector",
        UiTopology::Vector4,
        [UiRoute::Sum, UiRoute::Phase],
        [1.0, 2.0],
        WireAlgorithm::LegacyRC1,
        0.25,
        false,
    );
    make_patch(
        "extended4_fm_sync_lead.prophet.json",
        "Extended4 FM Sync Lead",
        "Lead",
        UiTopology::Extended4,
        [UiRoute::Frequency, UiRoute::Sync],
        [1.5, 3.0],
        WireAlgorithm::LegacyRC1,
        0.85,
        false,
    );
    make_patch(
        "vector4_sequencer_arp_diffuse_exp.prophet.json",
        "Vector4 Seq Arp Diffuse (Exp)",
        "Sequence",
        UiTopology::Vector4,
        [UiRoute::Sum, UiRoute::Ring],
        [0.5, 2.0],
        WireAlgorithm::DiffuseV1,
        0.5,
        true,
    );
}
