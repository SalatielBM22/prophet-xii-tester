// SPDX-License-Identifier: GPL-3.0-only
use prophet_android::{
    aaudio_error_name, build_probe_plan, is_reconnectable_aaudio_error,
    probe::{
        AAUDIO_ERROR_DISCONNECTED, AAUDIO_ERROR_ILLEGAL_ARGUMENT, AAUDIO_ERROR_TIMEOUT,
        AAUDIO_ERROR_UNAVAILABLE,
    },
    AndroidController, AudioFormatChoice, LifecycleState, Midi1StreamDecoder, PerformanceChoice,
    SharingChoice,
};

#[test]
fn probe_plan_orders_low_latency_float_first_and_deduplicates_fallback_rates() {
    let p96 = build_probe_plan(96_000);
    assert!(p96.len() >= 8);
    assert_eq!(p96[0].sample_rate_hz, 96_000);
    assert_eq!(p96[0].format, AudioFormatChoice::Float);
    assert_eq!(p96[0].performance, PerformanceChoice::LowLatency);
    assert_eq!(p96[0].sharing, SharingChoice::Shared);
    assert_eq!(p96[1].sample_rate_hz, 0);
    assert_eq!(p96[1].format, AudioFormatChoice::Float);

    let p48 = build_probe_plan(48_000);
    // Ensure every tuple (sr, fmt, perf, share) is strictly unique.
    for i in 0..p48.len() {
        for j in (i + 1)..p48.len() {
            assert_ne!(
                (p48[i].sample_rate_hz, p48[i].format, p48[i].performance, p48[i].sharing),
                (p48[j].sample_rate_hz, p48[j].format, p48[j].performance, p48[j].sharing),
                "Duplicate probe step between {i} and {j}"
            );
        }
    }

    let pinvalid = build_probe_plan(-1);
    assert_eq!(pinvalid[0].sample_rate_hz, 0);
}

#[test]
fn reconnectable_aaudio_error_classification_includes_android_r_timeout() {
    assert!(is_reconnectable_aaudio_error(AAUDIO_ERROR_DISCONNECTED));
    assert!(is_reconnectable_aaudio_error(AAUDIO_ERROR_TIMEOUT));
    assert!(is_reconnectable_aaudio_error(AAUDIO_ERROR_UNAVAILABLE));
    assert!(!is_reconnectable_aaudio_error(AAUDIO_ERROR_ILLEGAL_ARGUMENT));
    assert_eq!(aaudio_error_name(AAUDIO_ERROR_TIMEOUT), "AAUDIO_ERROR_TIMEOUT");
}

#[test]
fn midi1_stream_decoder_handles_running_status_realtime_and_sysex() {
    let mut dec = Midi1StreamDecoder::default();
    let mut msgs = Vec::new();
    // NoteOn ch0, running status NoteOn, interleaved clock 0xF8, SysEx packet, ProgramChange ch1 (1 data byte), NoteOff.
    let bytes = [
        0x90, 60, 100, // NoteOn C4
        64, 90,        // Running status NoteOn E4
        0xF8,          // Realtime clock mid-stream (ignored safely)
        0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7, // SysEx Identity Request
        0xC1, 5,       // Program change ch1 -> resets running status & expects 1 byte
        0x80, 60, 0,   // NoteOff C4
    ];
    dec.feed(&bytes, |s, d1, d2| msgs.push((s, d1, d2)));
    assert_eq!(
        msgs,
        vec![
            (0x90, 60, 100),
            (0x90, 64, 90),
            (0xC1, 5, 0),
            (0x80, 60, 0),
        ]
    );
}

#[test]
fn controller_preserves_state_across_reconnect_and_processes_usb_midi_and_engineport() {
    let mut ctrl = AndroidController::new(48_000).expect("init AndroidController");
    assert_eq!(ctrl.diagnostics.lifecycle, LifecycleState::Stopped);
    assert!(!ctrl.diagnostics.dev_bridge_enabled);

    // Dispatch p83 (Glide Enabled) + cutoff via EnginePort UiCommand JSON.
    let (reply_glide, action) =
        ctrl.dispatch_json(r#"{"type":"param","id":83,"value":1.0}"#);
    assert!(action.is_none());
    let glide_val: serde_json::Value = serde_json::from_str(&reply_glide).unwrap();
    assert_eq!(glide_val["ok"], true);

    let (reply_bridge, action_bridge) =
        ctrl.dispatch_json(r#"{"type":"hostControl","op":"toggleDevBridge","enabled":true}"#);
    assert_eq!(action_bridge.as_deref(), Some("DEV_BRIDGE_ON"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&reply_bridge).unwrap()["ok"],
        true
    );
    assert!(ctrl.diagnostics.dev_bridge_enabled);

    // Open session at 44.1 kHz, send USB MIDI NoteOn, render PCM I16 + F32, verify non-silent audio.
    let mut rt44 = ctrl.prepare_stream_session(44_100).expect("prepare 44.1k");
    ctrl.mark_ready(
        44_100,
        2,
        AudioFormatChoice::I16,
        PerformanceChoice::LowLatency,
        SharingChoice::Shared,
        192,
        768,
        384,
        Some(true),
    );
    let accepted = ctrl.ingest_usb_midi(&[0x90, 60, 110, 67, 100]);
    assert_eq!(accepted, 2);

    let mut i16_buf = [0i16; 512 * 2];
    rt44.process_interleaved_i16(&mut i16_buf, 512, Some(0), Some(12.5), None)
        .expect("render i16");
    let peak_i16 = i16_buf.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    assert!(peak_i16 > 10, "expected audible PCM_I16 energy, got {peak_i16}");

    // Snapshot JSON reflects EnginePort v1 contract + pressed notes + glideEnabled p83 + diagnostics.
    let snap_str = ctrl.snapshot_json();
    let snap: serde_json::Value = serde_json::from_str(&snap_str).unwrap();
    assert_eq!(snap["version"], 1);
    assert_eq!(snap["ready"], true);
    assert_eq!(snap["enginePort"]["sampleRate"], 44_100);
    assert_eq!(snap["parameters"][83], 1.0);
    let notes = snap["enginePort"]["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n["key"] == 60 && n["origin"] == prophet_host::queue::ORIGIN_HARDWARE_MIDI),
        "expected USB MIDI C4 note on canonical hardware origin {}: {notes:?}",
        prophet_host::queue::ORIGIN_HARDWARE_MIDI
    );
    assert_eq!(snap["diagnostics"]["lifecycle"], "READY");

    // Reconnect at 96 kHz Float (simulating headphone plug/disconnect recovery) and confirm patch persisted!
    let mut rt96 = ctrl.prepare_stream_session(96_000).expect("reconnect 96k");
    let mut f32_buf = [0.0f32; 256 * 2];
    rt96.process_interleaved_f32(&mut f32_buf, 256, None, None, None)
        .expect("render f32");
    let snap96: serde_json::Value = serde_json::from_str(&ctrl.snapshot_json()).unwrap();
    assert_eq!(snap96["parameters"][83], 1.0);
    assert_eq!(snap96["diagnostics"]["devBridgeEnabled"], true);

    // Focus attenuation clamp check.
    ctrl.set_focus_state("AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK", false, 0.25);
    assert!((ctrl.metrics.focus_gain() - 0.25).abs() < 1e-6);
    ctrl.set_focus_state("AUDIOFOCUS_LOSS_TRANSIENT", true, 0.0);
    assert_eq!(ctrl.metrics.focus_gain(), 0.0);
}
