// SPDX-License-Identifier: GPL-3.0-only
//! Android API 26+ AAudio + JNI host backed by `prophet-host` (`RtHostSlot` + `HostControlPlane`).
//! Zero allocations, locks, JSON parsing, or JNI calls happen on the AAudio real-time thread.

pub mod controller;
pub mod probe;

pub use controller::{AndroidController, Midi1StreamDecoder, SharedRtMetrics, ANDROID_CAPABILITIES};
pub use probe::{
    aaudio_error_name, build_probe_plan, is_reconnectable_aaudio_error, AndroidAudioDiagnostics,
    AudioFormatChoice, LifecycleState, PerformanceChoice, ProbeAttemptRecord, ProbeStep,
    SharingChoice,
};

#[cfg(target_os = "android")]
mod android_jni {
    use crate::{
        aaudio_error_name, build_probe_plan, is_reconnectable_aaudio_error, AndroidController,
        AudioFormatChoice, PerformanceChoice, ProbeAttemptRecord, SharedRtMetrics, SharingChoice,
        probe::{
            AAUDIO_CALLBACK_RESULT_CONTINUE, AAUDIO_CALLBACK_RESULT_STOP, AAUDIO_CONTENT_TYPE_MUSIC,
            AAUDIO_DIRECTION_OUTPUT, AAUDIO_FORMAT_PCM_FLOAT, AAUDIO_FORMAT_PCM_I16, AAUDIO_OK,
            AAUDIO_USAGE_MEDIA,
        },
    };
    use jni::{
        objects::{JByteArray, JClass, JString},
        sys::{jboolean, jfloat, jint, jstring},
        JNIEnv,
    };
    use prophet_host::RtHostSlot;
    use std::{
        ffi::{c_char, c_void, CStr},
        ptr,
        sync::{
            atomic::Ordering,
            Arc, Mutex, OnceLock,
        },
        time::Instant,
    };

    #[repr(C)]
    struct AAudioStream {
        _opaque: [u8; 0],
    }

    #[repr(C)]
    struct AAudioStreamBuilder {
        _opaque: [u8; 0],
    }

    #[link(name = "aaudio")]
    extern "C" {
        fn AAudio_createStreamBuilder(builder: *mut *mut AAudioStreamBuilder) -> i32;
        fn AAudioStreamBuilder_delete(builder: *mut AAudioStreamBuilder) -> i32;
        fn AAudioStreamBuilder_setDirection(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setFormat(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setChannelCount(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setSampleRate(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setPerformanceMode(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setSharingMode(b: *mut AAudioStreamBuilder, v: i32);
        fn AAudioStreamBuilder_setDataCallback(
            b: *mut AAudioStreamBuilder,
            cb: unsafe extern "C" fn(
                *mut AAudioStream,
                *mut c_void,
                *mut c_void,
                i32,
            ) -> i32,
            user: *mut c_void,
        );
        fn AAudioStreamBuilder_setErrorCallback(
            b: *mut AAudioStreamBuilder,
            cb: unsafe extern "C" fn(*mut AAudioStream, *mut c_void, i32),
            user: *mut c_void,
        );
        fn AAudioStreamBuilder_openStream(
            b: *mut AAudioStreamBuilder,
            s: *mut *mut AAudioStream,
        ) -> i32;
        fn AAudioStream_getSampleRate(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getChannelCount(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getFormat(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getSharingMode(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getPerformanceMode(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getFramesPerBurst(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getBufferCapacityInFrames(s: *mut AAudioStream) -> i32;
        fn AAudioStream_getBufferSizeInFrames(s: *mut AAudioStream) -> i32;
        fn AAudioStream_setBufferSizeInFrames(s: *mut AAudioStream, v: i32) -> i32;
        fn AAudioStream_getXRunCount(s: *mut AAudioStream) -> i32;
        fn AAudioStream_requestStart(s: *mut AAudioStream) -> i32;
        fn AAudioStream_requestStop(s: *mut AAudioStream) -> i32;
        fn AAudioStream_close(s: *mut AAudioStream) -> i32;
    }

    type SetUsageFn = unsafe extern "C" fn(*mut AAudioStreamBuilder, i32);
    type SetContentTypeFn = unsafe extern "C" fn(*mut AAudioStreamBuilder, i32);
    type IsMMapUsedFn = unsafe extern "C" fn(*mut AAudioStream) -> bool;
    type ConvertResultToTextFn = unsafe extern "C" fn(i32) -> *const c_char;

    struct DynamicAAudioSyms {
        set_usage: Option<SetUsageFn>,
        set_content_type: Option<SetContentTypeFn>,
        is_mmap_used: Option<IsMMapUsedFn>,
        convert_result_to_text: Option<ConvertResultToTextFn>,
    }

    unsafe fn dlsym_opt<T>(name: &'static [u8]) -> Option<T> {
        let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char) };
        if sym.is_null() {
            None
        } else {
            Some(unsafe { core::mem::transmute_copy(&sym) })
        }
    }

    fn dynamic_syms() -> &'static DynamicAAudioSyms {
        static SYMS: OnceLock<DynamicAAudioSyms> = OnceLock::new();
        SYMS.get_or_init(|| unsafe {
            DynamicAAudioSyms {
                set_usage: dlsym_opt(b"AAudioStreamBuilder_setUsage\0"),
                set_content_type: dlsym_opt(b"AAudioStreamBuilder_setContentType\0"),
                is_mmap_used: dlsym_opt(b"AAudioStream_isMMapUsed\0"),
                convert_result_to_text: dlsym_opt(b"AAudio_convertResultToText\0"),
            }
        })
    }

    fn describe_aaudio_result(code: i32) -> String {
        if let Some(f) = dynamic_syms().convert_result_to_text {
            let ptr = unsafe { f(code) };
            if !ptr.is_null() {
                if let Ok(s) = unsafe { CStr::from_ptr(ptr) }.to_str() {
                    return s.to_owned();
                }
            }
        }
        aaudio_error_name(code).to_owned()
    }

    struct AudioCallbackCtx {
        rt: RtHostSlot,
        metrics: Arc<SharedRtMetrics>,
        format: AudioFormatChoice,
        channels: usize,
        sample_rate_f64: f64,
        smoothed_focus_gain: f32,
        mono_scratch: [f32; 2048],
    }

    struct ActiveStream {
        stream: *mut AAudioStream,
        ctx_ptr: *mut AudioCallbackCtx,
        metrics_raw: *mut Arc<SharedRtMetrics>,
        metrics: Arc<SharedRtMetrics>,
    }

    unsafe impl Send for ActiveStream {}

    struct HostState {
        controller: AndroidController,
        active: Option<ActiveStream>,
    }

    static HOST: OnceLock<Mutex<HostState>> = OnceLock::new();

    fn with_host<R>(f: impl FnOnce(&mut HostState) -> R) -> Option<R> {
        let mutex = HOST.get_or_init(|| {
            let controller = AndroidController::new(48_000).expect("AndroidController init");
            Mutex::new(HostState {
                controller,
                active: None,
            })
        });
        mutex.lock().ok().map(|mut guard| f(&mut guard))
    }

    fn tear_down_stream(active: ActiveStream) {
        unsafe {
            AAudioStream_requestStop(active.stream);
            AAudioStream_close(active.stream);
            // Only free callback context & error Arc clone AFTER AAudioStream_close joins RT/error threads.
            drop(Box::from_raw(active.ctx_ptr));
            drop(Box::from_raw(active.metrics_raw));
        }
    }

    unsafe extern "C" fn aaudio_error_callback(
        _: *mut AAudioStream,
        user: *mut c_void,
        error: i32,
    ) {
        if user.is_null() {
            return;
        }
        let metrics = unsafe { &*(user as *const Arc<SharedRtMetrics>) };
        metrics.error_code.store(error, Ordering::Relaxed);
    }

    unsafe extern "C" fn aaudio_data_callback(
        _: *mut AAudioStream,
        user: *mut c_void,
        audio_data: *mut c_void,
        frames_i32: i32,
    ) -> i32 {
        if user.is_null() || audio_data.is_null() || frames_i32 <= 0 {
            return AAUDIO_CALLBACK_RESULT_CONTINUE;
        }
        let ctx = unsafe { &mut *(user as *mut AudioCallbackCtx) };
        let frames = frames_i32 as usize;
        let t0 = Instant::now();

        let budget_secs = frames as f64 / ctx.sample_rate_f64.max(8000.0);
        let xruns = None; // Populated off-RT from AAudioStream_getXRunCount on JNI poll.
        let prev_cpu_pct = ctx.metrics.last_cpu_pct();

        // Measure zero-alloc rendering of synthesizer into float or i16 HAL output buffer.
        let target_gain = ctx.metrics.focus_gain();

        match (ctx.format, ctx.channels) {
            (AudioFormatChoice::Float, 2) => {
                let out = unsafe { core::slice::from_raw_parts_mut(audio_data as *mut f32, frames * 2) };
                if ctx
                    .rt
                    .process_interleaved_f32(out, frames, xruns, prev_cpu_pct, None)
                    .is_err()
                {
                    out.fill(0.0);
                    return AAUDIO_CALLBACK_RESULT_STOP;
                }
                apply_focus_ramp_f32(out, &mut ctx.smoothed_focus_gain, target_gain);
            }
            (AudioFormatChoice::I16, 2) => {
                let out = unsafe { core::slice::from_raw_parts_mut(audio_data as *mut i16, frames * 2) };
                if (ctx.smoothed_focus_gain - 1.0).abs() < 1e-4 && (target_gain - 1.0).abs() < 1e-4 {
                    if ctx
                        .rt
                        .process_interleaved_i16(out, frames, xruns, prev_cpu_pct, None)
                        .is_err()
                    {
                        out.fill(0);
                        return AAUDIO_CALLBACK_RESULT_STOP;
                    }
                } else {
                    let mut pos = 0usize;
                    while pos < frames {
                        let c = (frames - pos).min(1024);
                        if ctx
                            .rt
                            .process_interleaved_f32(
                                &mut ctx.mono_scratch[..c * 2],
                                c,
                                xruns,
                                prev_cpu_pct,
                                None,
                            )
                            .is_err()
                        {
                            out.fill(0);
                            return AAUDIO_CALLBACK_RESULT_STOP;
                        }
                        apply_focus_ramp_f32(
                            &mut ctx.mono_scratch[..c * 2],
                            &mut ctx.smoothed_focus_gain,
                            target_gain,
                        );
                        for i in 0..(c * 2) {
                            let s = ctx.mono_scratch[i].clamp(-1.0, 1.0);
                            out[pos * 2 + i] = (s * 32767.0).round() as i16;
                        }
                        pos += c;
                    }
                }
            }
            (AudioFormatChoice::Float, 1) => {
                let out = unsafe { core::slice::from_raw_parts_mut(audio_data as *mut f32, frames) };
                let mut pos = 0usize;
                while pos < frames {
                    let c = (frames - pos).min(1024);
                    if ctx
                        .rt
                        .process_interleaved_f32(
                            &mut ctx.mono_scratch[..c * 2],
                            c,
                            xruns,
                            prev_cpu_pct,
                            None,
                        )
                        .is_err()
                    {
                        out.fill(0.0);
                        return AAUDIO_CALLBACK_RESULT_STOP;
                    }
                    apply_focus_ramp_f32(
                        &mut ctx.mono_scratch[..c * 2],
                        &mut ctx.smoothed_focus_gain,
                        target_gain,
                    );
                    for i in 0..c {
                        out[pos + i] = 0.5 * (ctx.mono_scratch[i * 2] + ctx.mono_scratch[i * 2 + 1]);
                    }
                    pos += c;
                }
            }
            (AudioFormatChoice::I16, 1) => {
                let out = unsafe { core::slice::from_raw_parts_mut(audio_data as *mut i16, frames) };
                let mut pos = 0usize;
                while pos < frames {
                    let c = (frames - pos).min(1024);
                    if ctx
                        .rt
                        .process_interleaved_f32(
                            &mut ctx.mono_scratch[..c * 2],
                            c,
                            xruns,
                            prev_cpu_pct,
                            None,
                        )
                        .is_err()
                    {
                        out.fill(0);
                        return AAUDIO_CALLBACK_RESULT_STOP;
                    }
                    apply_focus_ramp_f32(
                        &mut ctx.mono_scratch[..c * 2],
                        &mut ctx.smoothed_focus_gain,
                        target_gain,
                    );
                    for i in 0..c {
                        let m = (0.5 * (ctx.mono_scratch[i * 2] + ctx.mono_scratch[i * 2 + 1]))
                            .clamp(-1.0, 1.0);
                        out[pos + i] = (m * 32767.0).round() as i16;
                    }
                    pos += c;
                }
            }
            _ => return AAUDIO_CALLBACK_RESULT_STOP,
        }

        if budget_secs > 0.0 {
            let elapsed_pct = ((t0.elapsed().as_secs_f64() / budget_secs) * 100.0) as f32;
            ctx.metrics.store_cpu_pct(elapsed_pct);
        }
        ctx.metrics
            .callback_invocations
            .fetch_add(1, Ordering::Relaxed);
        ctx.metrics
            .frames_rendered
            .fetch_add(frames as u64, Ordering::Relaxed);

        AAUDIO_CALLBACK_RESULT_CONTINUE
    }

    #[inline(always)]
    fn apply_focus_ramp_f32(interleaved: &mut [f32], current_gain: &mut f32, target_gain: f32) {
        if (*current_gain - 1.0).abs() < 1e-4 && (target_gain - 1.0).abs() < 1e-4 {
            return;
        }
        let step = 0.002f32;
        for frame in interleaved.chunks_exact_mut(2) {
            if *current_gain < target_gain {
                *current_gain = (*current_gain + step).min(target_gain);
            } else if *current_gain > target_gain {
                *current_gain = (*current_gain - step).max(target_gain);
            }
            frame[0] *= *current_gain;
            frame[1] *= *current_gain;
        }
    }

    fn start_stream(state: &mut HostState, hint_sr: i32, hint_burst: i32) -> i32 {
        if let Some(prev) = state.active.take() {
            tear_down_stream(prev);
        }
        state.controller.mark_starting();

        let plan = build_probe_plan(hint_sr);
        let syms = dynamic_syms();
        let mut last_err = -1;

        for (step_idx, step) in plan.iter().enumerate() {
            // Prepare temporary RtHostSlot at fallback rate; once negotiated, re-sync cleanly before start.
            let initial_sr = if step.sample_rate_hz > 0 {
                step.sample_rate_hz as u32
            } else {
                hint_sr.clamp(8000, 192_000) as u32
            };
            let Ok(rt) = state.controller.prepare_stream_session(initial_sr) else {
                last_err = -3;
                continue;
            };
            let metrics_raw = Box::into_raw(Box::new(Arc::clone(&state.controller.metrics)));
            let ctx_ptr = Box::into_raw(Box::new(AudioCallbackCtx {
                rt,
                metrics: Arc::clone(&state.controller.metrics),
                format: step.format,
                channels: 2,
                sample_rate_f64: f64::from(initial_sr),
                smoothed_focus_gain: state.controller.metrics.focus_gain(),
                mono_scratch: [0.0; 2048],
            }));

            let mut builder = ptr::null_mut();
            let create_res = unsafe { AAudio_createStreamBuilder(&mut builder) };
            if create_res != AAUDIO_OK || builder.is_null() {
                unsafe {
                    drop(Box::from_raw(ctx_ptr));
                    drop(Box::from_raw(metrics_raw));
                }
                last_err = create_res;
                state.controller.record_probe_attempt(ProbeAttemptRecord {
                    step_index: step_idx,
                    requested_sr: step.sample_rate_hz,
                    requested_format: step.format,
                    requested_performance: step.performance,
                    requested_sharing: step.sharing,
                    open_result: create_res,
                    open_result_name: describe_aaudio_result(create_res),
                    start_result: None,
                    negotiated_sr: None,
                    negotiated_channels: None,
                    negotiated_format: None,
                    negotiated_performance: None,
                    negotiated_sharing: None,
                    frames_per_burst: None,
                    buffer_capacity_frames: None,
                    buffer_size_frames: None,
                    mmap_used: None,
                    accepted: false,
                    detail: format!(
                        "AAudio_createStreamBuilder failed: {}",
                        describe_aaudio_result(create_res)
                    ),
                });
                continue;
            }

            unsafe {
                AAudioStreamBuilder_setDirection(builder, AAUDIO_DIRECTION_OUTPUT);
                AAudioStreamBuilder_setFormat(builder, step.format.to_aaudio());
                AAudioStreamBuilder_setChannelCount(builder, 2);
                if step.sample_rate_hz > 0 {
                    AAudioStreamBuilder_setSampleRate(builder, step.sample_rate_hz);
                }
                AAudioStreamBuilder_setPerformanceMode(builder, step.performance.to_aaudio());
                AAudioStreamBuilder_setSharingMode(builder, step.sharing.to_aaudio());
                if let Some(f) = syms.set_usage {
                    f(builder, AAUDIO_USAGE_MEDIA);
                }
                if let Some(f) = syms.set_content_type {
                    f(builder, AAUDIO_CONTENT_TYPE_MUSIC);
                }
                AAudioStreamBuilder_setDataCallback(
                    builder,
                    aaudio_data_callback,
                    ctx_ptr as *mut c_void,
                );
                AAudioStreamBuilder_setErrorCallback(
                    builder,
                    aaudio_error_callback,
                    metrics_raw as *mut c_void,
                );
            }

            let mut stream = ptr::null_mut();
            let open_res = unsafe { AAudioStreamBuilder_openStream(builder, &mut stream) };
            unsafe { AAudioStreamBuilder_delete(builder) };

            if open_res != AAUDIO_OK || stream.is_null() {
                unsafe {
                    drop(Box::from_raw(ctx_ptr));
                    drop(Box::from_raw(metrics_raw));
                }
                last_err = open_res;
                state.controller.record_probe_attempt(ProbeAttemptRecord {
                    step_index: step_idx,
                    requested_sr: step.sample_rate_hz,
                    requested_format: step.format,
                    requested_performance: step.performance,
                    requested_sharing: step.sharing,
                    open_result: open_res,
                    open_result_name: describe_aaudio_result(open_res),
                    start_result: None,
                    negotiated_sr: None,
                    negotiated_channels: None,
                    negotiated_format: None,
                    negotiated_performance: None,
                    negotiated_sharing: None,
                    frames_per_burst: None,
                    buffer_capacity_frames: None,
                    buffer_size_frames: None,
                    mmap_used: None,
                    accepted: false,
                    detail: format!("openStream failed ({})", step.rationale),
                });
                continue;
            }

            let neg_sr = unsafe { AAudioStream_getSampleRate(stream) };
            let neg_ch = unsafe { AAudioStream_getChannelCount(stream) };
            let neg_fmt_raw = unsafe { AAudioStream_getFormat(stream) };
            let neg_fmt = AudioFormatChoice::from_aaudio(neg_fmt_raw);
            let neg_perf = PerformanceChoice::from_aaudio(unsafe { AAudioStream_getPerformanceMode(stream) });
            let neg_share = SharingChoice::from_aaudio(unsafe { AAudioStream_getSharingMode(stream) });
            let burst = unsafe { AAudioStream_getFramesPerBurst(stream) };
            let cap = unsafe { AAudioStream_getBufferCapacityInFrames(stream) };
            let mmap = syms.is_mmap_used.map(|f| unsafe { f(stream) });

            let compatible_ch = matches!(neg_ch, 1 | 2);
            let compatible_fmt = matches!(
                neg_fmt_raw,
                AAUDIO_FORMAT_PCM_FLOAT | AAUDIO_FORMAT_PCM_I16
            );
            let valid_sr = (8000..=192_000).contains(&neg_sr);

            if !compatible_ch || !compatible_fmt || !valid_sr {
                unsafe {
                    AAudioStream_close(stream);
                    drop(Box::from_raw(ctx_ptr));
                    drop(Box::from_raw(metrics_raw));
                }
                last_err = -2;
                state.controller.record_probe_attempt(ProbeAttemptRecord {
                    step_index: step_idx,
                    requested_sr: step.sample_rate_hz,
                    requested_format: step.format,
                    requested_performance: step.performance,
                    requested_sharing: step.sharing,
                    open_result: open_res,
                    open_result_name: describe_aaudio_result(open_res),
                    start_result: None,
                    negotiated_sr: Some(neg_sr),
                    negotiated_channels: Some(neg_ch),
                    negotiated_format: neg_fmt,
                    negotiated_performance: neg_perf,
                    negotiated_sharing: neg_share,
                    frames_per_burst: Some(burst),
                    buffer_capacity_frames: Some(cap),
                    buffer_size_frames: None,
                    mmap_used: mmap,
                    accepted: false,
                    detail: format!(
                        "Unsupported HAL negotiation: sr={neg_sr} ch={neg_ch} fmt={neg_fmt_raw}"
                    ),
                });
                continue;
            }

            // Tune buffer size conservatively to 2 bursts (or hint burst) clamped to capacity.
            let desired_burst = if burst > 0 { burst } else { hint_burst.max(64) };
            if cap > desired_burst {
                let target_buf = (desired_burst * 2).min(cap);
                unsafe {
                    AAudioStream_setBufferSizeInFrames(stream, target_buf);
                }
            }
            let actual_buf_size = unsafe { AAudioStream_getBufferSizeInFrames(stream) };

            // Reconcile prepared DSP sample rate if HAL chose a different rate than initial guess.
            let negotiated_sr_u32 = neg_sr as u32;
            unsafe {
                (*ctx_ptr).format = match neg_fmt_raw {
                    AAUDIO_FORMAT_PCM_I16 => AudioFormatChoice::I16,
                    _ => AudioFormatChoice::Float,
                };
                (*ctx_ptr).channels = neg_ch as usize;
                (*ctx_ptr).sample_rate_f64 = f64::from(negotiated_sr_u32);
                if (*ctx_ptr).rt.slot.engine.configuration().sample_rate != negotiated_sr_u32 {
                    let _ = state.controller.control.worker.prepare_reconfig(
                        &state.controller.control.sound,
                        negotiated_sr_u32,
                        1024,
                    );
                    (*ctx_ptr).rt.slot.poll_swap(None);
                    state.controller.control.poll();
                }
            }

            let start_res = unsafe { AAudioStream_requestStart(stream) };
            if start_res != AAUDIO_OK {
                unsafe {
                    AAudioStream_close(stream);
                    drop(Box::from_raw(ctx_ptr));
                    drop(Box::from_raw(metrics_raw));
                }
                last_err = start_res;
                state.controller.record_probe_attempt(ProbeAttemptRecord {
                    step_index: step_idx,
                    requested_sr: step.sample_rate_hz,
                    requested_format: step.format,
                    requested_performance: step.performance,
                    requested_sharing: step.sharing,
                    open_result: open_res,
                    open_result_name: describe_aaudio_result(open_res),
                    start_result: Some(start_res),
                    negotiated_sr: Some(neg_sr),
                    negotiated_channels: Some(neg_ch),
                    negotiated_format: neg_fmt,
                    negotiated_performance: neg_perf,
                    negotiated_sharing: neg_share,
                    frames_per_burst: Some(burst),
                    buffer_capacity_frames: Some(cap),
                    buffer_size_frames: Some(actual_buf_size),
                    mmap_used: mmap,
                    accepted: false,
                    detail: format!("requestStart failed: {}", describe_aaudio_result(start_res)),
                });
                continue;
            }

            let accepted_fmt = match neg_fmt_raw {
                AAUDIO_FORMAT_PCM_I16 => AudioFormatChoice::I16,
                _ => AudioFormatChoice::Float,
            };
            let accepted_perf = neg_perf.unwrap_or(step.performance);
            let accepted_share = neg_share.unwrap_or(step.sharing);
            state.controller.record_probe_attempt(ProbeAttemptRecord {
                step_index: step_idx,
                requested_sr: step.sample_rate_hz,
                requested_format: step.format,
                requested_performance: step.performance,
                requested_sharing: step.sharing,
                open_result: open_res,
                open_result_name: describe_aaudio_result(open_res),
                start_result: Some(start_res),
                negotiated_sr: Some(neg_sr),
                negotiated_channels: Some(neg_ch),
                negotiated_format: Some(accepted_fmt),
                negotiated_performance: Some(accepted_perf),
                negotiated_sharing: Some(accepted_share),
                frames_per_burst: Some(burst),
                buffer_capacity_frames: Some(cap),
                buffer_size_frames: Some(actual_buf_size),
                mmap_used: mmap,
                accepted: true,
                detail: step.rationale.to_owned(),
            });
            state.controller.mark_ready(
                neg_sr,
                neg_ch,
                accepted_fmt,
                accepted_perf,
                accepted_share,
                burst,
                cap,
                actual_buf_size,
                mmap,
            );
            state.active = Some(ActiveStream {
                stream,
                ctx_ptr,
                metrics_raw,
                metrics: Arc::clone(&state.controller.metrics),
            });
            return AAUDIO_OK;
        }

        state
            .controller
            .mark_failed(last_err, describe_aaudio_result(last_err));
        last_err
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_isReconnectableError(
        _: JNIEnv,
        _: JClass,
        error_code: jint,
    ) -> jboolean {
        u8::from(is_reconnectable_aaudio_error(error_code))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_start(
        _: JNIEnv,
        _: JClass,
        hint_sample_rate: jint,
        hint_frames_per_burst: jint,
    ) -> jint {
        with_host(|st| {
            if st.active.is_some() {
                return AAUDIO_OK;
            }
            start_stream(st, hint_sample_rate, hint_frames_per_burst)
        })
        .unwrap_or(-1)
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_stop(_: JNIEnv, _: JClass) {
        with_host(|st| {
            if let Some(active) = st.active.take() {
                tear_down_stream(active);
            }
            st.controller.mark_stopped();
        });
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_healthy(_: JNIEnv, _: JClass) -> jint {
        with_host(|st| {
            st.active
                .as_ref()
                .map(|a| a.metrics.error_code.load(Ordering::Relaxed))
                .unwrap_or(-1)
        })
        .unwrap_or(-1)
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_dispatchJson(
        mut env: JNIEnv,
        _: JClass,
        cmd_json: JString,
    ) -> jstring {
        let Ok(input) = env.get_string(&cmd_json) else {
            return ptr::null_mut();
        };
        let input_str: String = input.into();
        let reply = with_host(|st| st.controller.dispatch_json(&input_str).0)
            .unwrap_or_else(|| "{\"ok\":false,\"message\":\"Host lock poisoned\"}".to_owned());
        env.new_string(reply)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_pollSnapshotJson(
        env: JNIEnv,
        _: JClass,
    ) -> jstring {
        let json = with_host(|st| {
            if let Some(active) = &st.active {
                let xruns = unsafe { AAudioStream_getXRunCount(active.stream) };
                st.controller.update_xruns((xruns >= 0).then_some(xruns));
            } else {
                st.controller.update_xruns(None);
            }
            st.controller.snapshot_json()
        })
        .unwrap_or_else(|| "{\"version\":1,\"ready\":false}".to_owned());
        env.new_string(json)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_diagnosticsJson(
        env: JNIEnv,
        _: JClass,
    ) -> jstring {
        let json = with_host(|st| {
            if let Some(active) = &st.active {
                let xruns = unsafe { AAudioStream_getXRunCount(active.stream) };
                st.controller.update_xruns((xruns >= 0).then_some(xruns));
            }
            st.controller.diagnostics_json()
        })
        .unwrap_or_else(|| "{}".to_owned());
        env.new_string(json)
            .map(|s| s.into_raw())
            .unwrap_or(ptr::null_mut())
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_usbMidi(
        env: JNIEnv,
        _: JClass,
        data: JByteArray,
        offset: jint,
        length: jint,
    ) -> jint {
        if offset < 0 || length <= 0 || length > 4096 {
            return 0;
        }
        let Ok(bytes) = env.convert_byte_array(&data) else {
            return 0;
        };
        let start = offset as usize;
        let end = start.saturating_add(length as usize).min(bytes.len());
        if start >= end {
            return 0;
        }
        with_host(|st| st.controller.ingest_usb_midi(&bytes[start..end]) as jint).unwrap_or(0)
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_setFocusGain(
        mut env: JNIEnv,
        _: JClass,
        state: JString,
        muted: jboolean,
        gain: jfloat,
    ) {
        let desc = env
            .get_string(&state)
            .map(|s| String::from(s))
            .unwrap_or_else(|_| "UNKNOWN".to_owned());
        with_host(|st| st.controller.set_focus_state(&desc, muted != 0, gain));
    }

    #[no_mangle]
    pub extern "system" fn Java_org_prophetxii_Native_setRouteInfo(
        mut env: JNIEnv,
        _: JClass,
        route: JString,
        usb_devices_csv: JString,
    ) {
        let r = env
            .get_string(&route)
            .map(|s| String::from(s))
            .unwrap_or_else(|_| "unknown".to_owned());
        let devs: Vec<String> = env
            .get_string(&usb_devices_csv)
            .map(|s| {
                String::from(s)
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        with_host(|st| st.controller.set_route_info(r, devs));
    }
}
