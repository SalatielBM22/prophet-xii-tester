// SPDX-License-Identifier: GPL-3.0-only
use axum::{
    extract::{
        ws::{Message, WebSocketUpgrade},
        Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use prophet_control::port::Prepare;
use prophet_dsp::fx::solaris_reverb::ReverbTailPolicy;
use prophet_host::embed::{
    spawn_embedded_editor, EditorViewConfig, EmbeddedEditorHandle, HostBridgeSink, ParentWindow,
};
use prophet_host::{
    create_host_session, ui_snapshot_to_json, HostControlPlane, PhysicalMidiSource, RtHostSlot,
    UiCommand, UiCommandReply, UiSnapshot,
};
use prophet_params::{NUM_PARAMS, PARAM_SPECS};
use prophet_standalone::{AudioProcessor, Command, StampedCommand, Telemetry};
use serde::Deserialize;
use std::time::{Duration, Instant};
use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
};
use tokio::sync::{broadcast, mpsc, watch, Semaphore};
use tower_http::services::ServeDir;

const EDITOR_LOGICAL_WIDTH: u32 = 1180;
const EDITOR_LOGICAL_HEIGHT: u32 = 720;

#[derive(Clone)]
struct AppState {
    tx: mpsc::Sender<IngressCommand>,
    ids: Arc<AtomicU64>,
    capture: broadcast::Sender<String>,
    telemetry: watch::Receiver<String>,
    token: Option<Arc<str>>,
    sample_rate: f64,
    filter_clock: prophet_analysis::RequestClock,
    filter_permits: Arc<Semaphore>,
}

#[derive(Clone, Copy)]
struct IngressCommand {
    command: Command,
    id: u64,
    received: Instant,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Tail {
    Spill,
    Kill,
}

impl From<Tail> for ReverbTailPolicy {
    fn from(t: Tail) -> Self {
        match t {
            Tail::Spill => Self::Spill,
            Tail::Kill => Self::Kill,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    Param {
        id: usize,
        value: f32,
    },
    Midi {
        status: u8,
        data1: u8,
        #[serde(default)]
        data2: u8,
    },
    Preset {
        index: usize,
    },
    Snapshot {
        values: Vec<f32>,
    },
    Learn {
        id: usize,
    },
    CancelLearn,
    Unlearn {
        id: usize,
    },
    Remap {
        cc: usize,
        id: usize,
    },
    ReverbBypass {
        enabled: bool,
        tail: Tail,
    },
    Panic,
    Trimpot {
        voice: usize,
        field: usize,
        value: f32,
    },
}

fn authorized(s: &AppState, h: &HeaderMap, q: &HashMap<String, String>) -> bool {
    s.token
        .as_ref()
        .map(|t| {
            h.get("authorization")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.strip_prefix("Bearer ") == Some(t.as_ref()))
                .unwrap_or(false)
                || q.get("token").map(|v| v == t.as_ref()).unwrap_or(false)
        })
        .unwrap_or(true)
}

async fn command(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(req): Json<Request>,
) -> Response {
    if !authorized(&s, &headers, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let c = match req {
        Request::Param { id, value }
            if id < NUM_PARAMS
                && value.is_finite()
                && value >= PARAM_SPECS[id].min
                && value <= PARAM_SPECS[id].max =>
        {
            Command::Param { id, value }
        }
        Request::Midi {
            status,
            data1,
            data2,
        } if (0x80..0xf0).contains(&status) && data1 < 128 && data2 < 128 => Command::Midi {
            status,
            data1,
            data2,
        },
        Request::Preset { index } if index < 10 => Command::Preset(index),
        Request::Snapshot { values }
            if values.len() == NUM_PARAMS
                && values.iter().enumerate().all(|(i, v)| {
                    v.is_finite() && *v >= PARAM_SPECS[i].min && *v <= PARAM_SPECS[i].max
                }) =>
        {
            Command::Snapshot(values.try_into().unwrap())
        }
        Request::Learn { id } if id < NUM_PARAMS => Command::Learn(id),
        Request::CancelLearn => Command::CancelLearn,
        Request::Unlearn { id } if id < NUM_PARAMS => Command::Unlearn(id),
        Request::Remap { cc, id }
            if id < NUM_PARAMS && prophet_params::AtomicParamBank::learnable_cc(cc) =>
        {
            Command::Remap { cc, id }
        }
        Request::ReverbBypass { enabled, tail } => Command::ReverbBypass {
            enabled,
            tail: tail.into(),
        },
        Request::Panic => Command::Panic,
        Request::Trimpot {
            voice,
            field,
            value,
        } if voice < 12 && field < 4 && value.is_finite() => Command::Trimpot {
            voice,
            field,
            value,
        },
        _ => return (StatusCode::BAD_REQUEST, "invalid command").into_response(),
    };
    let id = s.ids.fetch_add(1, Ordering::Relaxed);
    match s.tx.try_send(IngressCommand {
        command: c,
        id,
        received: Instant::now(),
    }) {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"acceptedCommandId":id})),
        )
            .into_response(),
        Err(_) => (StatusCode::TOO_MANY_REQUESTS, "control queue full").into_response(),
    }
}

async fn state(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if !authorized(&s, &h, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    (
        [("content-type", "application/json")],
        s.telemetry.borrow().clone(),
    )
        .into_response()
}

async fn ws(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&s, &h, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    upgrade.on_upgrade(move|mut socket|async move{
  let mut rx=s.telemetry.clone();
  loop {
   let message=rx.borrow_and_update().clone();
   if socket.send(Message::Text(message.into())).await.is_err(){break;}
   tokio::select!{
    _=rx.changed()=>{},
    incoming=socket.recv()=>match incoming {Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>continue}
   }
  }
  // Releasing notes after disconnect prevents a remote keyboard from leaving stuck gates.
  let _=s.tx.send(IngressCommand { command:Command::Panic, id:s.ids.fetch_add(1,Ordering::Relaxed), received:Instant::now() }).await;
 }).into_response()
}

async fn filter_response(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    Json(values): Json<Vec<f32>>,
) -> Response {
    if !authorized(&s, &h, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(arr): Result<[f32; NUM_PARAMS], _> = values.try_into() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if arr.iter().any(|v| !v.is_finite()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let token = s.filter_clock.next();
    let Ok(_permit) = s.filter_permits.acquire().await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if !token.is_current() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"superseded": true})),
        )
            .into_response();
    }
    let sample_rate = s.sample_rate;
    let result = tokio::task::spawn_blocking(move || {
        prophet_standalone::transfer::compute_filter_transfer(&arr, sample_rate, Some(&token))
    })
    .await;
    match result {
        Ok(Ok(data)) => Json(data).into_response(),
        Ok(Err(prophet_analysis::Error::Superseded)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"superseded": true})),
        )
            .into_response(),
        Ok(Err(_)) | Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn wire(t: &Telemetry, device_errors: u64) -> String {
    let envelope = |e: &prophet_dsp::env_cem3310::EnvelopeTelemetry| {
        serde_json::json!({
            "stage": e.stage,
            "elapsedSamples": e.elapsed_samples,
            "level": e.level,
            "progress": e.progress,
            "timed": e.timed
        })
    };
    let mut times = t.timing_ns[..t.timing_count].to_vec();
    times.sort_unstable();
    let q = |p: f64| {
        times
            .get(((times.len() as f64 * p).ceil() as usize).saturating_sub(1))
            .copied()
            .unwrap_or(0) as f64
            / 1000.
    };
    let mean = if times.is_empty() {
        0.
    } else {
        times.iter().map(|v| *v as f64).sum::<f64>() / times.len() as f64 / 1000.
    };
    serde_json::json!({
        "v":1,"ready":true,"sampleRate":t.sample_rate,"activeVoiceMask":t.kernel.active_voice_mask,
        "voiceVcaLevels":t.kernel.voice_vca_levels,"voiceFiltEnvLevels":t.kernel.voice_filt_env_levels,"voiceMidiNotes":t.kernel.voice_midi_notes,
        "ampEnvelopes":t.kernel.amp_envelopes.iter().map(envelope).collect::<Vec<_>>(),
        "filterEnvelopes":t.kernel.filter_envelopes.iter().map(envelope).collect::<Vec<_>>(),
        "scope":t.scope.as_slice(),"dspLoadPercent":t.cpu,"peakDspLoadPercent":t.peak_cpu,"deadlineMisses":t.deadline_misses,
        "underruns":device_errors,"deviceErrors":device_errors,"driverXruns":null,
        "frames":t.frames,"values":t.params.as_slice(),"trimpots":t.trimpots,"lateCommands":t.late_commands,"droppedTelemetry":t.dropped_telemetry,
        "revision":t.revision,"parameterCommandIds":t.parameter_command_ids.as_slice(),"sequence":t.sequence,"monotonicNs":t.monotonic_ns,
        "midiLearn":{"active":t.learn_active,"target":if (t.learn_target as usize)<NUM_PARAMS {Some(t.learn_target)}else{None},"ccMap":t.cc_map.iter().map(|v|if (*v as usize)<NUM_PARAMS{Some(*v)}else{None}).collect::<Vec<_>>()},
        "reverb":{"enabled":t.reverb_enabled,"tail":if t.reverb_kill{"kill"}else{"spill"},"dormant":t.reverb_dormant,"enableGain":t.reverb_enable_gain},
        "callback":{"frames":t.callback_frames,"processingUs":t.processing_ns as f64/1000.,"intervalUs":t.callback_interval_ns as f64/1000.,"meanUs":mean,"p50Us":q(0.5),"p95Us":q(0.95),"p99Us":q(0.99),"maxUs":q(1.),"window":times.len(),"backendBufferedUs":t.backend_buffered_ns.map(|n|n as f64/1000.),"invalid":t.invalid_callbacks},
        "control":{"lastAppliedCommandId":t.last_command_id,"appliedFrame":t.applied_frame,"waitUs":t.control_wait_us,"relayWaitUs":t.relay_wait_us,"rtQueueWaitUs":t.rt_queue_wait_us,"queueDepth":t.queue_depth,"peakQueueDepth":t.peak_queue_depth},
        "captureDroppedChunks":t.dropped_capture
    }).to_string()
}

async fn capture_ws(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if !authorized(&s, &h, &q) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut rx = s.capture.subscribe();
    upgrade.on_upgrade(move |mut socket|async move {
        loop {tokio::select! {
            chunk=rx.recv()=>match chunk {Ok(c)=>if socket.send(Message::Text(c.into())).await.is_err(){break;},Err(broadcast::error::RecvError::Lagged(_))=>continue,Err(_)=>break},
            msg=socket.recv()=>match msg{Some(Ok(Message::Close(_)))|None|Some(Err(_))=>break,_=>{}}
        }}
    }).into_response()
}

fn stream_for<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut p: AudioProcessor,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let errors = p.underruns.clone();
    device.build_output_stream(
        config,
        move |out: &mut [T], info| {
            let ts = info.timestamp();
            p.backend_buffered_ns = ts
                .playback
                .duration_since(&ts.callback)
                .map(|d| d.as_nanos() as u64);
            p.process(out, channels)
        },
        move |_| {
            errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}

fn stream_for_host<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut rt_slot: RtHostSlot,
    sr: u32,
    underruns: Arc<AtomicU64>,
    cpu_tenths: Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let errors = underruns.clone();
    device.build_output_stream(
        config,
        move |out: &mut [T], _info| {
            let start = Instant::now();
            if channels == 0 || out.is_empty() || out.len() % channels != 0 {
                out.fill(T::from_sample(0.0));
                return;
            }
            let total_frames = out.len() / channels;
            let mut l = [0.0f32; 1024];
            let mut r = [0.0f32; 1024];
            let xruns = underruns.load(Ordering::Relaxed).min(u32::MAX as u64) as u32;
            let cur_cpu = cpu_tenths.load(Ordering::Relaxed) as f32 / 10.0;
            let mut pos = 0usize;
            while pos < total_frames {
                let n = (total_frames - pos).min(1024);
                let _ = rt_slot.process_planar_f32(
                    &mut l[..n],
                    &mut r[..n],
                    &[],
                    Some(xruns),
                    Some(cur_cpu),
                    None,
                );
                for i in 0..n {
                    let li = if l[i].is_finite() { l[i] } else { 0.0 };
                    let ri = if r[i].is_finite() { r[i] } else { 0.0 };
                    if channels == 1 {
                        out[pos + i] = T::from_sample(0.5 * (li + ri));
                    } else {
                        let base = (pos + i) * channels;
                        for c in 0..channels {
                            out[base + c] = T::from_sample(match c {
                                0 => li,
                                1 => ri,
                                _ => 0.0,
                            });
                        }
                    }
                }
                pos += n;
            }
            let elapsed_ns = start.elapsed().as_nanos() as f64;
            let budget_ns = (total_frames as f64 / sr.max(8000) as f64) * 1e9;
            if budget_ns > 0.0 {
                let inst_pct = (elapsed_ns / budget_ns) * 100.0;
                let prev_pct = cpu_tenths.load(Ordering::Relaxed) as f64 / 10.0;
                let smooth = prev_pct * 0.9 + inst_pct * 0.1;
                cpu_tenths.store(
                    (smooth * 10.0).round().clamp(0.0, 10000.0) as u32,
                    Ordering::Relaxed,
                );
            }
        },
        move |_| {
            errors.fetch_add(1, Ordering::Relaxed);
        },
        None,
    )
}

fn resolve_ui_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    let candidates = [
        PathBuf::from("ui/dist"),
        PathBuf::from("ui"),
        PathBuf::from("../ui"),
        PathBuf::from("../ui/dist"),
    ];
    for candidate in &candidates {
        if candidate.join("index.html").is_file() {
            return candidate.clone();
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for rel in ["ui", "ui/dist", "../ui", "../ui/dist", "../../ui/dist"] {
                let candidate = dir.join(rel);
                if candidate.join("index.html").is_file() {
                    return candidate;
                }
            }
        }
    }
    PathBuf::from("ui/dist")
}

fn browser_open_url(addr: std::net::SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        if addr.is_ipv4() {
            format!("http://127.0.0.1:{}", addr.port())
        } else {
            format!("http://[::1]:{}", addr.port())
        }
    } else {
        format!("http://{addr}")
    }
}

fn launch_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let status = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(url).spawn();

    match status {
        Ok(_) => println!("Launched default browser: {url}"),
        Err(err) => eprintln!("Warning: could not launch browser for {url}: {err}"),
    }
}

fn should_use_http_dev_mode(args: &[String]) -> bool {
    args.iter().any(|v| v == "--http-dev" || v == "--bind")
}

fn format_device_capabilities(device: &cpal::Device) -> String {
    let Ok(ranges) = device.supported_output_configs() else {
        return "capabilities unavailable".to_string();
    };
    let mut channels = Vec::new();
    let mut min_sr = u32::MAX;
    let mut max_sr = 0u32;
    let mut formats = Vec::new();
    for r in ranges {
        let ch = r.channels();
        if !channels.contains(&ch) {
            channels.push(ch);
        }
        min_sr = min_sr.min(r.min_sample_rate().0);
        max_sr = max_sr.max(r.max_sample_rate().0);
        let fmt = format!("{:?}", r.sample_format());
        if !formats.contains(&fmt) {
            formats.push(fmt);
        }
    }
    if channels.is_empty() {
        return "no reported output stream configs".to_string();
    }
    channels.sort_unstable();
    let ch_str = channels
        .iter()
        .map(|c| format!("{c}ch"))
        .collect::<Vec<_>>()
        .join("/");
    let sr_str = if min_sr == max_sr {
        format!("{min_sr} Hz")
    } else {
        format!("{min_sr}..{max_sr} Hz")
    };
    format!("{ch_str} | {sr_str} | [{}]", formats.join(", "))
}

fn print_devices(host: &cpal::Host) -> Result<(), Box<dyn Error>> {
    println!("Audio host backend: {:?}", host.id());
    let default_name = host.default_output_device().and_then(|d| d.name().ok());
    println!("Output devices:");
    let mut count = 0usize;
    for (idx, d) in host.output_devices()?.enumerate() {
        count += 1;
        let name = d.name().unwrap_or_else(|_| "<unknown>".to_string());
        let default_tag = if default_name.as_ref() == Some(&name) {
            " (default)"
        } else {
            ""
        };
        let caps = format_device_capabilities(&d);
        println!("  [{idx}] {name}{default_tag} — {caps}");
    }
    if count == 0 {
        println!("  (no output audio devices found)");
    }
    println!("MIDI input ports:");
    match midir::MidiInput::new("PROPHET-XII") {
        Ok(input) => {
            let ports = input.ports();
            if ports.is_empty() {
                println!("  (no MIDI input ports found)");
            } else {
                for (idx, p) in ports.iter().enumerate() {
                    let pname = input
                        .port_name(p)
                        .unwrap_or_else(|_| "<unknown>".to_string());
                    println!("  [{idx}] {pname}");
                }
            }
        }
        Err(err) => println!("  (MIDI enumeration unavailable: {err})"),
    }
    Ok(())
}

fn print_help() {
    println!(
        concat!(
            "PROPHET-XII Standalone Synthesizer v{}\n",
            "Usage: prophet-standalone [OPTIONS]\n\n",
            "Default mode opens a native desktop GUI window using embedded UI assets\n",
            "with zero listening TCP sockets and zero external browser tabs.\n\n",
            "Options:\n",
            "  --headless            Run in native mode without opening a GUI window\n",
            "  --http-dev            Enable opt-in developer HTTP/WebSocket bridge on 127.0.0.1:8787\n",
            "  --bind ADDR           Enable developer HTTP/WebSocket bridge bound to ADDR\n",
            "  --ui DIR              Path to UI directory containing index.html (used with --http-dev/--bind)\n",
            "  --open                Open browser URL (only with --http-dev/--bind, skipped under --null)\n",
            "  --device NAME         Select output audio device by exact name (see --list-devices)\n",
            "  --list-devices        List audio host backend, output devices, capabilities, and MIDI input ports\n",
            "  --midi-port INDEX     Connect MIDI input port by index\n",
            "  --sample-rate HZ      Sample rate in 8000..192000 (default: 48000)\n",
            "  --buffer-frames N     Audio callback block size in frames (default: 256 when supported)\n",
            "  --demo                Trigger an initial demo chord on startup\n",
            "  --duration SECONDS    Run for SECONDS then exit cleanly\n",
            "  --null                Use software null-sink audio clock (explicit test mode, no audio device)\n",
            "  --help                Print this help message\n\n",
            "Security: Binding to a non-loopback address requires PROPHET_CONTROL_TOKEN (>= 16 chars)."
        ),
        env!("CARGO_PKG_VERSION")
    );
}

struct AudioSelection {
    device: Option<cpal::Device>,
    config: Option<cpal::SupportedStreamConfig>,
    sr: u32,
    requested_buffer: Option<u32>,
}

fn negotiate_audio(
    host: &cpal::Host,
    use_null: bool,
    device_name: Option<String>,
    requested_sr_arg: Option<u32>,
    requested_buffer: Option<u32>,
) -> Result<AudioSelection, Box<dyn Error>> {
    let device = if use_null {
        None
    } else if let Some(name) = device_name {
        Some(
            host.output_devices()?
                .find(|d| d.name().ok().as_ref() == Some(&name))
                .ok_or("audio device not found")?,
        )
    } else {
        Some(
            host.default_output_device()
                .ok_or("no audio output device; use --list-devices to diagnose")?,
        )
    };
    let mut config = device
        .as_ref()
        .map(|d| d.default_output_config())
        .transpose()?;
    let requested_sr = requested_sr_arg.unwrap_or(48000);
    if !(8000..=192000).contains(&requested_sr) {
        return Err("sample rate must be 8000..192000".into());
    }
    if let Some(d) = device.as_ref() {
        if let Ok(mut ranges) = d.supported_output_configs() {
            if let Some(range) = ranges.find(|r| {
                r.channels() == 2
                    && r.min_sample_rate().0 <= requested_sr
                    && r.max_sample_rate().0 >= requested_sr
                    && r.sample_format() == cpal::SampleFormat::F32
            }) {
                config = Some(range.with_sample_rate(cpal::SampleRate(requested_sr)));
            }
        }
    }
    let sr = config
        .as_ref()
        .map(|c| c.sample_rate().0)
        .unwrap_or(requested_sr);
    if requested_sr_arg.is_some() && sr != requested_sr {
        return Err("requested sample rate is not supported by selected device".into());
    }
    if use_null && requested_buffer.is_some_and(|n| n > 8192) {
        return Err("null buffer exceeds 8192 frames".into());
    }
    if requested_buffer == Some(0) {
        return Err("buffer frames must be nonzero".into());
    }
    Ok(AudioSelection {
        device,
        config,
        sr,
        requested_buffer,
    })
}

struct StandaloneHostSink {
    ctrl: Arc<Mutex<HostControlPlane>>,
    editor: OnceLock<Weak<EmbeddedEditorHandle>>,
}

impl HostBridgeSink for StandaloneHostSink {
    fn snapshot_json(&self) -> String {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        let snap = ctrl.snapshot();
        ui_snapshot_to_json(&snap)
    }

    fn capture_ui_state(&self) -> UiSnapshot {
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.snapshot()
    }

    fn dispatch_command(&self, cmd: UiCommand) -> UiCommandReply {
        if let UiCommand::HostControl {
            ref op,
            buffer_frames,
            ..
        } = cmd
        {
            if op == "setUiZoom" {
                if let Some(zp) = buffer_frames {
                    let clamped = zp.clamp(75, 200);
                    let scale = clamped as f32 / 100.0;
                    if let Some(weak) = self.editor.get() {
                        if let Some(ed) = weak.upgrade() {
                            ed.resize(
                                ((EDITOR_LOGICAL_WIDTH as f32) * scale).round() as u32,
                                ((EDITOR_LOGICAL_HEIGHT as f32) * scale).round() as u32,
                                1.0,
                            );
                        }
                    }
                }
                return UiCommandReply::ok("UI zoom updated");
            }
        }
        let mut ctrl = self.ctrl.lock().unwrap_or_else(|e| e.into_inner());
        ctrl.dispatch(cmd)
    }

    fn on_zoom_changed(&self, zoom: f32) {
        let scale = zoom.clamp(0.75, 2.0);
        if let Some(weak) = self.editor.get() {
            if let Some(ed) = weak.upgrade() {
                ed.resize(
                    ((EDITOR_LOGICAL_WIDTH as f32) * scale).round() as u32,
                    ((EDITOR_LOGICAL_HEIGHT as f32) * scale).round() as u32,
                    1.0,
                );
            }
        }
    }
}

fn run_native_desktop_mode(
    host: &cpal::Host,
    selection: AudioSelection,
    midi_port: Option<usize>,
    demo: bool,
    duration: Option<u64>,
    headless: bool,
    ignored_ui_flag: bool,
) -> Result<(), Box<dyn Error>> {
    if ignored_ui_flag {
        eprintln!("Note: --ui DIR only applies to --http-dev / --bind mode; native window serves embedded UI");
    }
    let sr = selection.sr;
    let init_block = selection.requested_buffer.unwrap_or(256).clamp(1, 1024) as u16;
    let (rt_slot, mut ctrl) = create_host_session(
        "standalone",
        vec![
            "engine-port-v1",
            "vector4",
            "reverb-algorithms",
            "music-sequencer",
            "vcf-validity",
            "native-window",
            "cpal-audio",
            "hardware-midi",
        ],
        Prepare {
            sample_rate: sr,
            max_frames: init_block,
            oversampling: 2,
        },
        true,
    )
    .map_err(|e| format!("failed to prepare host engine session: {:?}", e))?;

    ctrl.set_audio_running(true, Some(sr), Some(init_block));
    if demo {
        for n in [48u8, 55, 60, 64] {
            let _ = ctrl.dispatch(UiCommand::Midi {
                status: 0x90,
                data1: n,
                data2: 96,
                note_id: Some(u32::from(n)),
            });
        }
    }

    let underruns = Arc::new(AtomicU64::new(0));
    let cpu_tenths = Arc::new(AtomicU32::new(0));
    let midi_drops = Arc::new(AtomicU64::new(0));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let (mut midi_tx, mut midi_rx) = rtrb::RingBuffer::<[u8; 3]>::new(512);
    let _midi = if let Some(port_idx) = midi_port {
        let mut input = midir::MidiInput::new("PROPHET-XII")?;
        input.ignore(midir::Ignore::None);
        let ports = input.ports();
        let selected = ports
            .get(port_idx)
            .ok_or("MIDI port does not exist")?;
        let drops = midi_drops.clone();
        Some(input.connect(
            selected,
            "PROPHET-XII input",
            move |_, msg, _| {
                if msg.len() >= 2 {
                    let pkt = [msg[0], msg[1], *msg.get(2).unwrap_or(&0)];
                    if midi_tx.push(pkt).is_err() {
                        drops.fetch_add(1, Ordering::Relaxed);
                    }
                }
            },
            (),
        )?)
    } else {
        None
    };

    let _stream: Option<cpal::Stream>;
    let mut null_thread: Option<std::thread::JoinHandle<()>> = None;
    let audio_summary: String;

    if let Some(device) = selection.device {
        let supported = selection.config.unwrap();
        let mut cfg = supported.config();
        cfg.buffer_size = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                let frames = selection.requested_buffer.unwrap_or(256u32.clamp(*min, *max));
                if frames < *min || frames > *max {
                    return Err(format!("buffer {frames} outside supported {min}..{max}").into());
                }
                cpal::BufferSize::Fixed(frames)
            }
            cpal::SupportedBufferSize::Unknown => {
                if selection.requested_buffer.is_some() {
                    return Err("device cannot validate explicit buffer size; omit --buffer-frames to use reported default".into());
                }
                eprintln!("Device buffer range unknown; using backend default");
                cpal::BufferSize::Default
            }
        };
        let device_name = device.name()?;
        audio_summary = format!(
            "{:?} | {} | {} Hz | {:?} | {}ch | {:?}",
            host.id(),
            device_name,
            sr,
            cfg.buffer_size,
            cfg.channels,
            supported.sample_format()
        );
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream_for_host::<f32>(
                &device,
                &cfg,
                rt_slot,
                sr,
                underruns.clone(),
                cpu_tenths.clone(),
            )?,
            cpal::SampleFormat::I16 => stream_for_host::<i16>(
                &device,
                &cfg,
                rt_slot,
                sr,
                underruns.clone(),
                cpu_tenths.clone(),
            )?,
            cpal::SampleFormat::U16 => stream_for_host::<u16>(
                &device,
                &cfg,
                rt_slot,
                sr,
                underruns.clone(),
                cpu_tenths.clone(),
            )?,
            _ => return Err("unsupported device sample format".into()),
        };
        stream.play()?;
        _stream = Some(stream);
    } else {
        _stream = None;
        let block = selection.requested_buffer.unwrap_or(256) as usize;
        eprintln!("NULL TEST SINK: no playback device");
        audio_summary = format!("NULL TEST SINK | {sr} Hz | {block} frames");
        let null_stop = stop_flag.clone();
        let cpu_arc = cpu_tenths.clone();
        let ur_arc = underruns.clone();
        let mut slot = rt_slot;
        null_thread = Some(
            std::thread::Builder::new()
                .name("prophet-null-audio".to_owned())
                .spawn(move || {
                    // Allocate planar scratch once outside the render loop to keep audio loop allocation-free.
                    let mut l = vec![0.0f32; block];
                    let mut r = vec![0.0f32; block];
                    let mut deadline = Instant::now();
                    let block_dur = Duration::from_secs_f64(block as f64 / sr as f64);
                    while !null_stop.load(Ordering::Acquire) {
                        let start = Instant::now();
                        let xruns = ur_arc.load(Ordering::Relaxed).min(u32::MAX as u64) as u32;
                        let cur_cpu = cpu_arc.load(Ordering::Relaxed) as f32 / 10.0;
                        let _ = slot.process_planar_f32(
                            &mut l[..block],
                            &mut r[..block],
                            &[],
                            Some(xruns),
                            Some(cur_cpu),
                            None,
                        );
                        let elapsed_ns = start.elapsed().as_nanos() as f64;
                        let budget_ns = (block as f64 / sr as f64) * 1e9;
                        if budget_ns > 0.0 {
                            let inst_pct = (elapsed_ns / budget_ns) * 100.0;
                            let prev_pct = cpu_arc.load(Ordering::Relaxed) as f64 / 10.0;
                            let smooth = prev_pct * 0.9 + inst_pct * 0.1;
                            cpu_arc.store(
                                (smooth * 10.0).round().clamp(0.0, 10000.0) as u32,
                                Ordering::Relaxed,
                            );
                        }
                        deadline += block_dur;
                        let sleep_dur = deadline.saturating_duration_since(Instant::now());
                        if !sleep_dur.is_zero() {
                            std::thread::sleep(sleep_dur);
                        }
                    }
                })?,
        );
    }

    let ctrl_arc = Arc::new(Mutex::new(ctrl));
    let ctrl_worker_arc = ctrl_arc.clone();
    let worker_stop = stop_flag.clone();
    let midi_drops_worker = midi_drops.clone();
    let ctrl_thread = std::thread::Builder::new()
        .name("prophet-ctrl-worker".to_owned())
        .spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                if let Ok(mut c) = ctrl_worker_arc.lock() {
                    for _ in 0..64 {
                        let Ok(pkt) = midi_rx.pop() else { break; };
                        let _ = c.dispatch_hardware_midi(
                            PhysicalMidiSource::Native,
                            pkt[0],
                            pkt[1],
                            pkt[2],
                            None,
                            0,
                        );
                    }
                    c.poll();
                    let drops = midi_drops_worker.load(Ordering::Relaxed);
                    c.diagnostics = Some(serde_json::json!({
                        "mode": "native-desktop",
                        "httpServer": false,
                        "hardwareMidiDrops": drops,
                    }));
                }
                std::thread::sleep(Duration::from_millis(4));
            }
        })?;

    // Signal & optional --duration terminator (no listening socket opened).
    let sig_stop = stop_flag.clone();
    let _signal_thread = std::thread::Builder::new()
        .name("prophet-signal-waiter".to_owned())
        .spawn(move || {
            if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                rt.block_on(async move {
                    if let Some(secs) = duration {
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(secs)) => {}
                            _ = tokio::signal::ctrl_c() => {}
                        }
                    } else {
                        let _ = tokio::signal::ctrl_c().await;
                    }
                    sig_stop.store(true, Ordering::Release);
                });
            }
        });

    println!("PROPHET-XII v{} (Native Desktop Mode)", env!("CARGO_PKG_VERSION"));
    println!("  Audio:   {audio_summary}");
    println!("  Control: embedded prophet-host + native window (0 TCP ports)");

    if headless {
        println!("  UI:      --headless");
        while !stop_flag.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(20));
        }
    } else {
        let sink = Arc::new(StandaloneHostSink {
            ctrl: ctrl_arc.clone(),
            editor: OnceLock::new(),
        });
        let cfg = EditorViewConfig {
            title: "PROPHET-XII Virtual Analog Synthesizer".to_owned(),
            backend_tag: "standalone".to_owned(),
            logical_width: EDITOR_LOGICAL_WIDTH,
            logical_height: EDITOR_LOGICAL_HEIGHT,
            scale_factor: 1.0,
            resizable: true,
        };
        let editor = Arc::new(
            spawn_embedded_editor(ParentWindow::StandaloneWindow, cfg, sink.clone())
                .map_err(|e| format!("failed to launch native desktop window: {e}"))?,
        );
        let _ = sink.editor.set(Arc::downgrade(&editor));
        println!("  UI:      embedded {}", editor.backend_kind());
        editor.pump_until_closed(&stop_flag);
        stop_flag.store(true, Ordering::Release);
        drop(editor);
    }

    stop_flag.store(true, Ordering::Release);
    if let Ok(mut c) = ctrl_arc.lock() {
        let _ = c.dispatch(UiCommand::Panic { all_origins: true });
    }
    std::thread::sleep(Duration::from_millis(25));
    let _ = ctrl_thread.join();
    if let Some(nh) = null_thread {
        let _ = nh.join();
    }
    Ok(())
}

fn run_http_dev_mode(
    host: &cpal::Host,
    selection: AudioSelection,
    bind: String,
    ui_dir: Option<PathBuf>,
    open_browser: bool,
    is_null: bool,
    midi_port: Option<usize>,
    demo: bool,
    duration: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let addr: std::net::SocketAddr = bind.parse()?;
        let token = std::env::var("PROPHET_CONTROL_TOKEN")
            .ok()
            .filter(|t| t.len() >= 16)
            .map(Arc::<str>::from);
        if !addr.ip().is_loopback() && token.is_none() {
            return Err(
                "remote control requires PROPHET_CONTROL_TOKEN (at least 16 characters)".into(),
            );
        }

        let sr = selection.sr;
        let (mut producer, consumer) = rtrb::RingBuffer::new(1024);
        let ids = Arc::new(AtomicU64::new(1));
        let (capture_tx, mut capture_rx) = rtrb::RingBuffer::new(256);
        let (capture_wire, _) = broadcast::channel(8);
        let (telemetry_tx, mut telemetry_rx) = rtrb::RingBuffer::new(4);
        let underruns = Arc::new(AtomicU64::new(0));

        let mut processor =
            AudioProcessor::new_stamped(sr as f32, consumer, telemetry_tx, underruns.clone());
        processor.attach_capture(capture_tx);
        if demo {
            for n in [48, 55, 60, 64] {
                processor.command(Command::Midi {
                    status: 0x90,
                    data1: n,
                    data2: 96,
                })
            }
        }
        let (tx, mut rx) = mpsc::channel::<IngressCommand>(512);
        let (wire_tx, wire_rx) = watch::channel("{\"v\":1,\"ready\":false}".to_string());
        let error_count = underruns.clone();
        let midi_drops = Arc::new(AtomicU64::new(0));
        let midi_panic = Arc::new(AtomicBool::new(false));
        let midi_drops_relay = midi_drops.clone();
        let midi_panic_relay = midi_panic.clone();
        let capture_relay = capture_wire.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(5));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut overflowed = 0u64;
            let mut last_dropped = 0u64;
            let mut panic_pending = false;
            loop {
                tokio::select! {
                    Some(c)=rx.recv()=>{
                        let stamp=StampedCommand { command:c.command, id:c.id, received:c.received, queued:Instant::now() };
                        if producer.push(stamp).is_err(){overflowed+=1;last_dropped=c.id;panic_pending=true;}
                    }
                    _=interval.tick()=>{
                        panic_pending |= midi_panic_relay.swap(false,Ordering::Relaxed);
                        if panic_pending {
                            let now=Instant::now();
                            if producer.push(StampedCommand{command:Command::Panic,id:0,received:now,queued:now}).is_ok(){panic_pending=false;}
                        }
                        let mut latest=None;
                        while let Ok(t)=telemetry_rx.pop(){latest=Some(t);}
                        if let Some(t)=latest {
                            let mut message:serde_json::Value=serde_json::from_str(&wire(&t,error_count.load(Ordering::Relaxed))).unwrap();
                            message["control"]["ingressDrops"]=overflowed.into();
                            message["control"]["hardwareMidiDrops"]=midi_drops_relay.load(Ordering::Relaxed).into();
                            message["control"]["lastDroppedCommandId"]=last_dropped.into();
                            wire_tx.send_replace(message.to_string());
                        }
                        let mut blocks=Vec::new();
                        for _ in 0..256 {
                            let Ok(c)=capture_rx.pop() else {break;};
                            if capture_relay.receiver_count()>0 {blocks.push(serde_json::json!({"sequence":c.sequence,"firstFrame":c.first_frame,"sampleRate":c.sample_rate,"samples":&c.samples[..c.len]}));}
                        }
                        if !blocks.is_empty(){let _=capture_relay.send(serde_json::json!({"v":1,"blocks":blocks}).to_string());}
                    }
                }
            }
        });

        let _midi = if let Some(port) = midi_port {
            let mut input = midir::MidiInput::new("PROPHET-XII")?;
            input.ignore(midir::Ignore::None);
            let ports = input.ports();
            let selected = ports.get(port).ok_or("MIDI port does not exist")?;
            let tx = tx.clone();
            let midi_ids = ids.clone();
            Some(input.connect(
                selected,
                "PROPHET-XII input",
                move |_, msg, _| {
                    if msg.len() >= 2 {
                        if tx
                            .try_send(IngressCommand {
                                command: Command::Midi {
                                    status: msg[0],
                                    data1: msg[1],
                                    data2: *msg.get(2).unwrap_or(&0),
                                },
                                id: midi_ids.fetch_add(1, Ordering::Relaxed),
                                received: Instant::now(),
                            })
                            .is_err()
                        {
                            midi_drops.fetch_add(1, Ordering::Relaxed);
                            midi_panic.store(true, Ordering::Relaxed);
                        }
                    }
                },
                (),
            )?)
        } else {
            None
        };

        let _stream;
        let audio_summary: String;
        if let Some(device) = selection.device {
            let supported = selection.config.unwrap();
            let mut cfg = supported.config();
            cfg.buffer_size = match supported.buffer_size() {
                cpal::SupportedBufferSize::Range { min, max } => {
                    let frames = selection.requested_buffer.unwrap_or(256u32.clamp(*min, *max));
                    if frames < *min || frames > *max {
                        return Err(
                            format!("buffer {frames} outside supported {min}..{max}").into(),
                        );
                    }
                    cpal::BufferSize::Fixed(frames)
                }
                cpal::SupportedBufferSize::Unknown => {
                    if selection.requested_buffer.is_some() {
                        return Err("device cannot validate explicit buffer size; omit --buffer-frames to use reported default".into());
                    }
                    eprintln!("Device buffer range unknown; using backend default, inspect observed callback frames/timestamps");
                    cpal::BufferSize::Default
                }
            };
            let device_name = device.name()?;
            eprintln!(
                "Backend {:?}, requested buffer {:?}",
                host.id(),
                cfg.buffer_size
            );
            eprintln!(
                "Audio device: {device_name} / {sr} Hz / {} channels / {:?}",
                cfg.channels,
                supported.sample_format()
            );
            audio_summary = format!(
                "{:?} | {} | {} Hz | {:?} | {}ch | {:?}",
                host.id(),
                device_name,
                sr,
                cfg.buffer_size,
                cfg.channels,
                supported.sample_format()
            );
            _stream = Some(match supported.sample_format() {
                cpal::SampleFormat::F32 => stream_for::<f32>(&device, &cfg, processor)?,
                cpal::SampleFormat::I16 => stream_for::<i16>(&device, &cfg, processor)?,
                cpal::SampleFormat::U16 => stream_for::<u16>(&device, &cfg, processor)?,
                _ => return Err("unsupported device sample format".into()),
            });
            _stream.as_ref().unwrap().play()?;
        } else {
            _stream = None;
            let block = selection.requested_buffer.unwrap_or(256) as usize;
            eprintln!("NULL TEST SINK: no playback device");
            audio_summary = format!("NULL TEST SINK | {sr} Hz | {block} frames");
            std::thread::spawn(move || {
                if block > 8192 {
                    eprintln!("null buffer exceeds 8192 frames");
                    return;
                }
                let mut buf = vec![0.0f32; block * 2];
                let mut deadline = Instant::now();
                loop {
                    processor.process(&mut buf, 2);
                    deadline += Duration::from_secs_f64(block as f64 / sr as f64);
                    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                }
            });
        }

        let presets = prophet_params::presets::factory_presets();
        let mut factory = prophet_control::ProphetXiiPlugin::new(sr as f32);
        let preset_meta = presets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                factory.load_preset(i);
                let values: Vec<_> = PARAM_SPECS
                    .iter()
                    .map(|s| factory.ipc.params.get(s.id))
                    .collect();
                serde_json::json!({"id":p.id,"name":p.name,"category":p.category.as_str(),"values":values})
            })
            .collect::<Vec<_>>();
        let metadata = serde_json::json!({
            "v": 1,
            "extensions": ["envelope-stages","parameter-revisions","midi-learn-lifecycle","reverb-bypass","continuous-capture","callback-timing"],
            "parameters": PARAM_SPECS.iter().map(|s| serde_json::json!({"id":s.id as usize,"name":s.name,"min":s.min,"max":s.max,"integer":s.integer,"logarithmic":s.logarithmic,"group":s.group})).collect::<Vec<_>>(),
            "presets": preset_meta
        });
        let static_dir = resolve_ui_dir(ui_dir);
        let ui_ready = static_dir.join("index.html").is_file();
        if !ui_ready {
            eprintln!(
                "Note: UI index.html not found at '{}' (serving API-only; pass --ui PATH to serve web UI)",
                static_dir.display()
            );
        }
        let app = Router::new()
            .route("/api/v1/command", post(command))
            .route("/api/v1/state", get(state))
            .route("/api/v1/filter", post(filter_response))
            .route("/api/v1/ws", get(ws))
            .route("/api/v1/capture", get(capture_ws))
            .route(
                "/api/v1/meta",
                get(move || async move { Json(metadata.clone()) }),
            )
            .fallback_service(ServeDir::new(&static_dir))
            .with_state(AppState {
                tx: tx.clone(),
                ids: ids.clone(),
                capture: capture_wire,
                telemetry: wire_rx,
                token,
                sample_rate: sr as f64,
                filter_clock: prophet_analysis::RequestClock::default(),
                filter_permits: Arc::new(Semaphore::new(2)),
            });
        let listener = tokio::net::TcpListener::bind(&bind).await?;
        let local_addr = listener.local_addr()?;
        let open_url = browser_open_url(local_addr);
        println!(
            "PROPHET-XII v{} (Developer HTTP/WS Mode)",
            env!("CARGO_PKG_VERSION")
        );
        println!("  Audio:   {audio_summary}");
        println!(
            "  UI dir:  {} ({})",
            static_dir.display(),
            if ui_ready {
                "ready"
            } else {
                "API-only / missing index.html"
            }
        );
        println!("PROPHET-XII control: http://{local_addr}");
        if open_browser {
            if is_null {
                eprintln!("Note: --open skipped in --null test sink mode");
            } else {
                launch_browser(&open_url);
            }
        }
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                if let Some(seconds) = duration {
                    tokio::time::sleep(Duration::from_secs(seconds)).await;
                } else {
                    let _ = tokio::signal::ctrl_c().await;
                }
                let _ = tx
                    .send(IngressCommand {
                        command: Command::Panic,
                        id: ids.fetch_add(1, Ordering::Relaxed),
                        received: Instant::now(),
                    })
                    .await;
                tokio::time::sleep(Duration::from_millis(100)).await;
            })
            .await?;
        Ok(())
    })
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |k: &str| args.iter().any(|v| v == k);
    let val = |k: &str| args.windows(2).find(|v| v[0] == k).map(|v| v[1].clone());

    if has("--help") {
        print_help();
        return Ok(());
    }

    // CPAL's native Windows host is WASAPI; no ASIO dependency is enabled.
    let host = cpal::default_host();
    if has("--list-devices") {
        return print_devices(&host);
    }

    let is_null = has("--null");
    let requested_sr = val("--sample-rate").map(|v| v.parse::<u32>()).transpose()?;
    let requested_buffer = val("--buffer-frames").map(|v| v.parse::<u32>()).transpose()?;
    let midi_port = val("--midi-port").map(|v| v.parse::<usize>()).transpose()?;
    let duration = val("--duration").map(|v| v.parse::<u64>()).transpose()?;
    let selection = negotiate_audio(
        &host,
        is_null,
        val("--device"),
        requested_sr,
        requested_buffer,
    )?;

    if should_use_http_dev_mode(&args) {
        let bind = val("--bind").unwrap_or_else(|| "127.0.0.1:8787".to_owned());
        return run_http_dev_mode(
            &host,
            selection,
            bind,
            val("--ui").map(PathBuf::from),
            has("--open"),
            is_null,
            midi_port,
            has("--demo"),
            duration,
        );
    }

    if has("--open") {
        eprintln!("Note: --open only applies to --http-dev / --bind mode; native window launches directly");
    }

    run_native_desktop_mode(
        &host,
        selection,
        midi_port,
        has("--demo"),
        duration,
        has("--headless"),
        val("--ui").is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_open_url_rewrites_unspecified_loopback() {
        let ipv4: std::net::SocketAddr = "0.0.0.0:8787".parse().unwrap();
        assert_eq!(browser_open_url(ipv4), "http://127.0.0.1:8787");
        let ipv6: std::net::SocketAddr = "[::]:8787".parse().unwrap();
        assert_eq!(browser_open_url(ipv6), "http://[::1]:8787");
        let explicit: std::net::SocketAddr = "127.0.0.1:9001".parse().unwrap();
        assert_eq!(browser_open_url(explicit), "http://127.0.0.1:9001");
    }

    #[test]
    fn resolve_ui_dir_honors_explicit_path() {
        let custom = PathBuf::from("custom/ui/path");
        assert_eq!(resolve_ui_dir(Some(custom.clone())), custom);
    }

    #[test]
    fn native_desktop_mode_is_default_and_http_bridge_is_opt_in() {
        let empty: Vec<String> = Vec::new();
        assert!(!should_use_http_dev_mode(&empty));
        let headless = vec!["--null".to_owned(), "--headless".to_owned()];
        assert!(!should_use_http_dev_mode(&headless));
        let http_dev = vec!["--http-dev".to_owned()];
        assert!(should_use_http_dev_mode(&http_dev));
        let bind = vec!["--bind".to_owned(), "127.0.0.1:8787".to_owned()];
        assert!(should_use_http_dev_mode(&bind));
    }
}
