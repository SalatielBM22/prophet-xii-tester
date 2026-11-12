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
use prophet_dsp::fx::solaris_reverb::ReverbTailPolicy;
use prophet_params::{NUM_PARAMS, PARAM_SPECS};
use prophet_standalone::{AudioProcessor, Command, StampedCommand, Telemetry};
use serde::Deserialize;
use std::time::Instant;
use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::{broadcast, mpsc, watch};
use tower_http::services::ServeDir;
#[derive(Clone)]
struct AppState {
    tx: mpsc::Sender<IngressCommand>,
    ids: Arc<AtomicU64>,
    capture: broadcast::Sender<String>,
    telemetry: watch::Receiver<String>,
    token: Option<Arc<str>>,
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
            if values.len() == NUM_PARAMS && values.iter().enumerate().all(|(i,v)| v.is_finite() && *v>=PARAM_SPECS[i].min && *v<=PARAM_SPECS[i].max) =>
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
    if values.len() != NUM_PARAMS || values.iter().any(|v| !v.is_finite()) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        use prophet_params::ParamId::*;
        let mut f = prophet_dsp::vcf_moog::DualVcfStage::new(48000.0, 1.0);
        f.set_params(
            values[FilterCutoff as usize],
            values[FilterResonance as usize],
            values[FilterDrive as usize],
            values[MoogBassComp as usize],
            values[FilterMorph as usize],
        );
        let mut impulse = [0.0f32; 4096];
        for (i, y) in impulse.iter_mut().enumerate() {
            *y = f.process_sample(if i == 0 { 0.0001 } else { 0.0 }) / 0.0001;
        }
        let mut hz = Vec::new();
        let mut db = Vec::new();
        let mut phase = Vec::new();
        for k in 0..256 {
            let freq = 20.0f64 * 1000.0f64.powf(k as f64 / 255.0);
            let mut re = 0.0;
            let mut im = 0.0;
            for (i, &v) in impulse.iter().enumerate() {
                let a = -std::f64::consts::TAU * freq * i as f64 / 48000.0;
                re += v as f64 * a.cos();
                im += v as f64 * a.sin();
            }
            hz.push(freq);
            db.push(20.0 * re.hypot(im).max(1e-9).log10());
            phase.push(im.atan2(re));
        }
        serde_json::json!({"hz":hz,"db":db,"phase":phase})
    })
    .await;
    match result {
        Ok(v) => Json(v).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
fn wire(t: &Telemetry, device_errors: u64) -> String {
    let envelope = |e: &prophet_dsp::env_cem3310::EnvelopeTelemetry| serde_json::json!({"stage":e.stage,"elapsedSamples":e.elapsed_samples,"level":e.level,"progress":e.progress,"timed":e.timed});
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
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |k: &str| args.iter().any(|v| v == k);
    let val = |k: &str| args.windows(2).find(|v| v[0] == k).map(|v| v[1].clone());
    if has("--help") {
        println!("prophet-standalone [--bind 127.0.0.1:8787] [--ui ui/dist] [--device NAME] [--midi-port 0] [--duration SECONDS] [--demo] [--null] [--buffer-frames N] [--sample-rate HZ]\nRemote bind requires PROPHET_CONTROL_TOKEN. --null is an explicit test sink, without audio device.");
        return Ok(());
    }
    // CPAL's native Windows host is WASAPI; no ASIO dependency is enabled.
    let host = cpal::default_host();
    if has("--list-devices") {
        for d in host.output_devices()? {
            println!("{}", d.name()?)
        }
        return Ok(());
    }
    let bind = val("--bind").unwrap_or("127.0.0.1:8787".into());
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
    let (mut producer, consumer) = rtrb::RingBuffer::new(1024);
    let ids = Arc::new(AtomicU64::new(1));
    let (capture_tx, mut capture_rx) = rtrb::RingBuffer::new(256);
    let (capture_wire, _) = broadcast::channel(8);
    let (telemetry_tx, mut telemetry_rx) = rtrb::RingBuffer::new(4);
    let underruns = Arc::new(AtomicU64::new(0));
    let device = if has("--null") {
        None
    } else if let Some(name) = val("--device") {
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
    let requested_sr = val("--sample-rate")
        .map(|v| v.parse::<u32>())
        .transpose()?
        .unwrap_or(48000);
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
    if val("--sample-rate").is_some() && sr != requested_sr {
        return Err("requested sample rate is not supported by selected device".into());
    }
    let requested_buffer = val("--buffer-frames")
        .map(|v| v.parse::<u32>())
        .transpose()?;
    if has("--null") && requested_buffer.is_some_and(|n| n > 8192) {
        return Err("null buffer exceeds 8192 frames".into());
    }
    if requested_buffer == Some(0) {
        return Err("buffer frames must be nonzero".into());
    }
    let mut processor =
        AudioProcessor::new_stamped(sr as f32, consumer, telemetry_tx, underruns.clone());
    processor.attach_capture(capture_tx);
    if has("--demo") {
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
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(5));
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
    let _midi = if let Some(port) = val("--midi-port") {
        let mut input = midir::MidiInput::new("PROPHET-XII")?;
        input.ignore(midir::Ignore::None);
        let ports = input.ports();
        let selected = ports
            .get(port.parse::<usize>()?)
            .ok_or("MIDI port does not exist")?;
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
    if let Some(device) = device {
        let supported = config.unwrap();
        let mut cfg = supported.config();
        cfg.buffer_size = match supported.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => {
                let frames = requested_buffer.unwrap_or(256u32.clamp(*min, *max));
                if frames < *min || frames > *max {
                    return Err(format!("buffer {frames} outside supported {min}..{max}").into());
                }
                cpal::BufferSize::Fixed(frames)
            }
            cpal::SupportedBufferSize::Unknown => {
                if requested_buffer.is_some() {
                    return Err("device cannot validate explicit buffer size; omit --buffer-frames to use reported default".into());
                }
                eprintln!("Device buffer range unknown; using backend default, inspect observed callback frames/timestamps");
                cpal::BufferSize::Default
            }
        };
        eprintln!(
            "Backend {:?}, requested buffer {:?}",
            host.id(),
            cfg.buffer_size
        );
        eprintln!(
            "Audio device: {} / {} Hz / {} channels / {:?}",
            device.name()?,
            sr,
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
        eprintln!("NULL TEST SINK: no playback device");
        std::thread::spawn(move || {
            let block = requested_buffer.unwrap_or(256) as usize;
            if block > 8192 {
                eprintln!("null buffer exceeds 8192 frames");
                return;
            }
            let mut buf = vec![0.0f32; block * 2];
            let mut deadline = std::time::Instant::now();
            loop {
                processor.process(&mut buf, 2);
                deadline += std::time::Duration::from_secs_f64(block as f64 / sr as f64);
                std::thread::sleep(deadline.saturating_duration_since(std::time::Instant::now()));
            }
        });
    }
    let presets = prophet_params::presets::factory_presets();
    let mut factory = prophet_control::ProphetXiiPlugin::new(sr as f32);
    let preset_meta=presets.iter().enumerate().map(|(i,p)|{
        factory.load_preset(i);
        let values:Vec<_>=PARAM_SPECS.iter().map(|s|factory.ipc.params.get(s.id)).collect();
        serde_json::json!({"id":p.id,"name":p.name,"category":p.category.as_str(),"values":values})
    }).collect::<Vec<_>>();
    let metadata = serde_json::json!({"v":1,"extensions":["envelope-stages","parameter-revisions","midi-learn-lifecycle","reverb-bypass","continuous-capture","callback-timing"],"parameters":PARAM_SPECS.iter().map(|s|serde_json::json!({"id":s.id as usize,"name":s.name,"min":s.min,"max":s.max,"integer":s.integer,"logarithmic":s.logarithmic,"group":s.group})).collect::<Vec<_>>(),"presets":preset_meta});
    let static_dir = PathBuf::from(val("--ui").unwrap_or("ui/dist".into()));
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
        .fallback_service(ServeDir::new(static_dir))
        .with_state(AppState {
            tx: tx.clone(),
            ids: ids.clone(),
            capture: capture_wire,
            telemetry: wire_rx,
            token,
        });
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("PROPHET-XII control: http://{bind}");
    let duration = val("--duration").map(|v| v.parse::<u64>()).transpose()?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            if let Some(seconds) = duration {
                tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
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
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        })
        .await?;
    Ok(())
}
