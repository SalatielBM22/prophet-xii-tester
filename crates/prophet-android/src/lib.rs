// SPDX-License-Identifier: GPL-3.0-only
//! Android API 26+ AAudio host. JNI and JSON run outside the audio callback.
#[cfg(target_os = "android")]
mod android {
    use jni::{
        objects::{JClass, JFloatArray},
        sys::{jfloat, jint, jstring},
        JNIEnv,
    };
    use prophet_control::ProphetXiiPlugin;
    use prophet_params::{ParamId, NUM_PARAMS};
    use std::{
        ffi::c_void,
        ptr,
        sync::{
            atomic::{AtomicI32, Ordering},
            Arc, Mutex,
        },
        time::Instant,
    };
    enum Stream {}
    enum Builder {}
    #[link(name = "aaudio")]
    extern "C" {
        fn AAudio_createStreamBuilder(builder: *mut *mut Builder) -> i32;
        fn AAudioStreamBuilder_delete(builder: *mut Builder) -> i32;
        fn AAudioStreamBuilder_setDirection(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setFormat(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setChannelCount(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setSampleRate(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setPerformanceMode(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setSharingMode(b: *mut Builder, v: i32);
        fn AAudioStreamBuilder_setDataCallback(
            b: *mut Builder,
            cb: unsafe extern "C" fn(*mut Stream, *mut c_void, *mut c_void, i32) -> i32,
            user: *mut c_void,
        );
        fn AAudioStreamBuilder_setErrorCallback(
            b: *mut Builder,
            cb: unsafe extern "C" fn(*mut Stream, *mut c_void, i32),
            user: *mut c_void,
        );
        fn AAudioStreamBuilder_openStream(b: *mut Builder, s: *mut *mut Stream) -> i32;
        fn AAudioStream_getSampleRate(s: *mut Stream) -> i32;
        fn AAudioStream_getChannelCount(s: *mut Stream) -> i32;
        fn AAudioStream_getFormat(s: *mut Stream) -> i32;
        fn AAudioStream_getXRunCount(s: *mut Stream) -> i32;
        fn AAudioStream_requestStart(s: *mut Stream) -> i32;
        fn AAudioStream_requestStop(s: *mut Stream) -> i32;
        fn AAudioStream_close(s: *mut Stream) -> i32;
    }
    #[derive(Clone, Copy)]
    enum Command {
        Param(usize, f32),
        Midi(u8, u8, u8),
        Preset(usize),
        Panic,
        Learn(usize),
        CancelLearn,
        Unlearn(usize),
        Remap(usize, usize),
        ReverbBypass(bool, bool),
        Snapshot([f32; NUM_PARAMS]),
        Trimpot(usize, usize, f32),
    }
    #[derive(Clone, Copy)]
    struct Telemetry {
        kernel: prophet_dsp::KernelTelemetrySnapshot,
        scope: [f32; 1024],
        values: [f32; NUM_PARAMS],
        trimpots: [[f32; 4]; 12],
        cpu: f32,
        frames: u64,
        misses: u64,
        sr: f32,
        revision: u64,
        learn_active: bool,
        learn_target: u32,
        cc_map: [u32; 128],
        reverb_enabled: bool,
        reverb_kill: bool,
        reverb_dormant: bool,
    }
    struct Audio {
        engine: ProphetXiiPlugin,
        commands: rtrb::Consumer<Command>,
        telemetry: rtrb::Producer<Telemetry>,
        scope: [f32; 1024],
        pos: usize,
        frames: u64,
        published: u64,
        cpu: f32,
        misses: u64,
    }
    struct Host {
        stream: *mut Stream,
        audio: *mut Audio,
        commands: rtrb::Producer<Command>,
        telemetry: rtrb::Consumer<Telemetry>,
        error: Arc<AtomicI32>,
    }
    // Host ownership is serialized by JNI's mutex; the audio callback exclusively owns Audio.
    unsafe impl Send for Host {}
    static HOST: Mutex<Option<Host>> = Mutex::new(None);
    unsafe extern "C" fn error(_: *mut Stream, user: *mut c_void, e: i32) {
        unsafe {
            (*(user as *const AtomicI32)).store(e, Ordering::Relaxed);
        }
    }
    unsafe extern "C" fn callback(
        _: *mut Stream,
        user: *mut c_void,
        buffer: *mut c_void,
        frames: i32,
    ) -> i32 {
        if frames <= 0 {
            return 0;
        }
        let a = unsafe { &mut *(user as *mut Audio) };
        let start = Instant::now();
        for _ in 0..256 {
            let Ok(c) = a.commands.pop() else {
                break;
            };
            let e = &mut a.engine;
            match c {
                Command::Param(id, v) => {
                    if let Some(id) = ParamId::from_index(id) {
                        e.ipc.params.set(id, v)
                    }
                }
                Command::Midi(s, d1, d2) => {
                    e.sync_parameters();
                    e.handle_midi_message(s, d1, d2);
                }
                Command::Preset(i) => {
                    e.panic();
                    e.load_preset(i)
                }
                Command::Panic => e.panic(),
                Command::Learn(id) => {
                    e.ipc.params.begin_learn(id);
                }
                Command::CancelLearn => e.ipc.params.cancel_learn(),
                Command::Unlearn(id) => {
                    e.ipc.params.unlearn(id);
                }
                Command::Remap(cc, id) => {
                    e.ipc.params.remap(cc, id);
                }
                Command::ReverbBypass(enabled, kill) => e.ipc.params.set_reverb_bypass(
                    enabled,
                    if kill {
                        prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Kill
                    } else {
                        prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Spill
                    },
                ),
                Command::Snapshot(v) => {
                    for (i, x) in v.into_iter().enumerate() {
                        e.ipc.params.set(ParamId::from_index(i).unwrap(), x)
                    }
                }
                Command::Trimpot(i, f, v) => {
                    if i < 12 {
                        let p = &e.ipc.params;
                        match f {
                            0 => p.voice_vco_a_cents[i].store(v.clamp(-6.0, 6.0)),
                            1 => p.voice_vco_b_cents[i].store(v.clamp(-6.0, 6.0)),
                            2 => p.voice_vcf_pct[i].store(v.clamp(-0.05, 0.05)),
                            3 => p.voice_env_rc_pct[i].store(v.clamp(-0.1, 0.1)),
                            _ => {}
                        }
                    }
                }
            }
        }
        let output =
            unsafe { std::slice::from_raw_parts_mut(buffer as *mut f32, frames as usize * 2) };
        let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
        let mut offset = 0;
        while offset < frames as usize {
            let n = (frames as usize - offset).min(64);
            a.engine.process_audio_block(&mut l[..n], &mut r[..n]);
            for i in 0..n {
                output[2 * (offset + i)] = l[i];
                output[2 * (offset + i) + 1] = r[i];
                a.scope[a.pos] = l[i];
                a.pos = (a.pos + 1) % 1024;
            }
            offset += n;
        }
        a.frames += frames as u64;
        let sr = a.engine.kernel.sample_rate;
        let cpu = (start.elapsed().as_secs_f64() / (frames as f64 / sr as f64) * 100.0) as f32;
        a.cpu = 0.9 * a.cpu + 0.1 * cpu;
        if cpu > 100.0 {
            a.misses += 1;
        }
        if a.frames - a.published >= (sr / 25.0) as u64 {
            let p = &a.engine.ipc.params;
            let t = Telemetry {
                kernel: a.engine.kernel.telemetry_snapshot(),
                scope: std::array::from_fn(|i| a.scope[(a.pos + i) % 1024]),
                values: std::array::from_fn(|i| p.get(ParamId::from_index(i).unwrap())),
                trimpots: std::array::from_fn(|i| {
                    [
                        p.voice_vco_a_cents[i].load(),
                        p.voice_vco_b_cents[i].load(),
                        p.voice_vcf_pct[i].load(),
                        p.voice_env_rc_pct[i].load(),
                    ]
                }),
                cpu: a.cpu,
                frames: a.frames,
                misses: a.misses,
                sr,
                revision: p.revision.load(Ordering::Relaxed),
                learn_active: p.midi_learn_active.load(Ordering::Relaxed),
                learn_target: p.midi_learn_target_param.load(Ordering::Relaxed),
                cc_map: std::array::from_fn(|i| p.cc_map[i].load(Ordering::Relaxed)),
                reverb_enabled: a.engine.kernel.reverb_params.enabled,
                reverb_kill: a.engine.kernel.reverb_params.bypass_tail
                    == prophet_dsp::fx::solaris_reverb::ReverbTailPolicy::Kill,
                reverb_dormant: a.engine.kernel.reverb.is_dormant(),
            };
            let _ = a.telemetry.push(t);
            a.published = a.frames;
        }
        0
    }
    fn stop(h: Host) {
        unsafe {
            AAudioStream_requestStop(h.stream);
            AAudioStream_close(h.stream);
            drop(Box::from_raw(h.audio));
        }
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_start(_: JNIEnv, _: JClass) -> jint {
        let Ok(mut host) = HOST.lock() else {
            return -1;
        };
        if host.is_some() {
            return 0;
        }
        let (tx, rx) = rtrb::RingBuffer::new(1024);
        let (tt, tr) = rtrb::RingBuffer::new(4);
        let audio = Box::into_raw(Box::new(Audio {
            engine: ProphetXiiPlugin::new(48000.0),
            commands: rx,
            telemetry: tt,
            scope: [0.0; 1024],
            pos: 0,
            frames: 0,
            published: 0,
            cpu: 0.0,
            misses: 0,
        }));
        let error_state = Arc::new(AtomicI32::new(0));
        let mut builder = ptr::null_mut();
        let mut stream = ptr::null_mut();
        unsafe {
            let result = AAudio_createStreamBuilder(&mut builder);
            if result != 0 {
                drop(Box::from_raw(audio));
                return result;
            }
            AAudioStreamBuilder_setDirection(builder, 0);
            AAudioStreamBuilder_setFormat(builder, 2);
            AAudioStreamBuilder_setChannelCount(builder, 2);
            AAudioStreamBuilder_setSampleRate(builder, 48000);
            AAudioStreamBuilder_setPerformanceMode(builder, 12);
            AAudioStreamBuilder_setSharingMode(builder, 1);
            AAudioStreamBuilder_setDataCallback(builder, callback, audio as *mut c_void);
            AAudioStreamBuilder_setErrorCallback(
                builder,
                error,
                Arc::as_ptr(&error_state) as *mut c_void,
            );
            let result = AAudioStreamBuilder_openStream(builder, &mut stream);
            AAudioStreamBuilder_delete(builder);
            if result != 0 {
                drop(Box::from_raw(audio));
                return result;
            }
            if AAudioStream_getChannelCount(stream) != 2 || AAudioStream_getFormat(stream) != 2 {
                AAudioStream_close(stream);
                drop(Box::from_raw(audio));
                return -2;
            }
            let sr = AAudioStream_getSampleRate(stream);
            (*audio)
                .engine
                .kernel
                .prepare(sr as f32, prophet_dsp::oversample::OversampleFactor::X2);
            let result = AAudioStream_requestStart(stream);
            if result != 0 {
                AAudioStream_close(stream);
                drop(Box::from_raw(audio));
                return result;
            }
            *host = Some(Host {
                stream,
                audio,
                commands: tx,
                telemetry: tr,
                error: error_state,
            });
            0
        }
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_stop(_: JNIEnv, _: JClass) {
        if let Ok(mut h) = HOST.lock() {
            if let Some(h) = h.take() {
                stop(h);
            }
        }
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_healthy(_: JNIEnv, _: JClass) -> jint {
        HOST.lock()
            .ok()
            .and_then(|h| h.as_ref().map(|h| h.error.load(Ordering::Relaxed)))
            .unwrap_or(-1)
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_command(
        _: JNIEnv,
        _: JClass,
        kind: jint,
        a: jint,
        b: jint,
        value: jfloat,
    ) -> jint {
        if !value.is_finite() {
            return -2;
        }
        let c = match kind {
            0 if a >= 0 && (a as usize) < NUM_PARAMS => Command::Param(a as usize, value),
            1 if (128..240).contains(&a)
                && (0..128).contains(&b)
                && (0.0..128.0).contains(&value) =>
            {
                Command::Midi(a as u8, b as u8, value as u8)
            }
            2 if (0..10).contains(&a) => Command::Preset(a as usize),
            3 => Command::Panic,
            6 => Command::CancelLearn,
            7 if a >= 0 && (a as usize) < NUM_PARAMS => Command::Unlearn(a as usize),
            8 if a >= 0
                && prophet_params::AtomicParamBank::learnable_cc(a as usize)
                && b >= 0
                && (b as usize) < NUM_PARAMS =>
            {
                Command::Remap(a as usize, b as usize)
            }
            9 if (0..=1).contains(&a) && (0..=1).contains(&b) => {
                Command::ReverbBypass(a == 1, b == 1)
            }
            5 if a >= 0 && (a as usize) < NUM_PARAMS => Command::Learn(a as usize),
            4 if (0..12).contains(&a) && (0..4).contains(&b) => {
                Command::Trimpot(a as usize, b as usize, value)
            }
            _ => return -2,
        };
        if let Ok(mut h) = HOST.lock() {
            if let Some(h) = h.as_mut() {
                return if h.commands.push(c).is_ok() { 0 } else { -3 };
            }
        }
        -1
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_snapshot(
        env: JNIEnv,
        _: JClass,
        values: JFloatArray,
    ) -> jint {
        if env.get_array_length(&values).unwrap_or(0) != NUM_PARAMS as i32 {
            return -2;
        }
        let mut data = [0.0; NUM_PARAMS];
        if env.get_float_array_region(&values, 0, &mut data).is_err()
            || data.iter().any(|v| !v.is_finite())
        {
            return -2;
        }
        if let Ok(mut h) = HOST.lock() {
            if let Some(h) = h.as_mut() {
                return if h.commands.push(Command::Snapshot(data)).is_ok() {
                    0
                } else {
                    -3
                };
            }
        }
        -1
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_state(env: JNIEnv, _: JClass) -> jstring {
        let mut result = "{\"v\":1,\"ready\":false}".to_string();
        if let Ok(mut h) = HOST.lock() {
            if let Some(h) = h.as_mut() {
                let mut last = None;
                while let Ok(t) = h.telemetry.pop() {
                    last = Some(t);
                }
                if let Some(t) = last {
                    let envelope = |e: &prophet_dsp::env_cem3310::EnvelopeTelemetry| serde_json::json!({"stage":e.stage,"elapsedSamples":e.elapsed_samples,"progress":e.progress,"level":e.level,"timed":e.timed});
                    result=serde_json::json!({"v":1,"ready":true,"sampleRate":t.sr,"values":t.values.as_slice(),"scope":t.scope.as_slice(),"trimpots":t.trimpots,"activeVoiceMask":t.kernel.active_voice_mask,"voiceVcaLevels":t.kernel.voice_vca_levels,"voiceFiltEnvLevels":t.kernel.voice_filt_env_levels,"voiceMidiNotes":t.kernel.voice_midi_notes,"dspLoadPercent":t.cpu,"frames":t.frames,"deadlineMisses":t.misses,"revision":t.revision,"ampEnvelopes":t.kernel.amp_envelopes.iter().map(envelope).collect::<Vec<_>>(),"filterEnvelopes":t.kernel.filter_envelopes.iter().map(envelope).collect::<Vec<_>>(),"midiLearn":{"active":t.learn_active,"target":if (t.learn_target as usize)<NUM_PARAMS{Some(t.learn_target)}else{None},"ccMap":t.cc_map.iter().map(|v|if (*v as usize)<NUM_PARAMS{Some(*v)}else{None}).collect::<Vec<_>>()},"reverb":{"enabled":t.reverb_enabled,"tail":if t.reverb_kill{"kill"}else{"spill"},"dormant":t.reverb_dormant},"underruns":unsafe{AAudioStream_getXRunCount(h.stream)}}).to_string();
                }
            }
        }
        env.new_string(result)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    }
    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_meta(env: JNIEnv, _: JClass) -> jstring {
        let result=serde_json::json!({"v":1,"presets":prophet_params::presets::factory_presets().iter().map(|p|serde_json::json!({"id":p.id,"name":p.name,"category":p.category.as_str()})).collect::<Vec<_>>()}).to_string();
        env.new_string(result)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    }
}
