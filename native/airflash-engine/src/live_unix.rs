//! Linux system-audio capture: PipeWire sink-monitor loop, PulseAudio fallback.
//!
//! The default Linux audio server is PipeWire. Its sink nodes expose
//! `monitor_FL`/`monitor_FR` ports that carry the mixed speaker feed. The
//! capture stream is started with `--target 0` (linked to nothing) and then
//! explicitly linked to the chosen sink's monitor ports with `pw-link`, so a
//! similarly-named microphone source can never be picked by fuzzy matching.
//!
//! When `pw-record` is unavailable (PulseAudio-only hosts), `parec` records
//! the default sink monitor directly and no linking step is needed.
//!
//! Raw s16le stereo is resampled to the streaming rate through the same
//! bounded PCM queue contract as the WASAPI loopback and the file/simulated
//! inputs. Helper binaries resolve strictly from `AIRFLASH_HELPER_DIR` when it
//! is set so tests can inject fakes (or simulate absent helpers by omitting
//! them); `AIRFLASH_CAPTURE_NO_LINK=1` skips the link step for environments
//! where the graph is wired externally.
use crate::{clock::unix_ns, rtp::FRAMES};
use anyhow::{Context, Result, ensure};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    ffi::OsStr,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Native capture rate requested from the audio server. PipeWire resamples
/// internally; the fixed rubato stage below converts to the streaming rate.
const NATIVE_RATE: u32 = 48000;
const NATIVE_CHANNELS: usize = 2;

/// A warning-shaped state change raised by the capture thread.
/// The session owns the JSON framing because the capture thread has no emitter.
#[derive(Clone, Debug)]
pub struct Warning {
    pub code: &'static str,
    pub recovered: bool,
    pub message: String,
}

#[derive(Default, Clone, Serialize)]
pub struct Metrics {
    pub dropped_frames: u64,
    pub underrun_packets: u64,
    pub capture_frames: u64,
    pub discontinuities: u64,
    pub max_queue_age_ms: f64,
    pub capture_to_send_p95_ms: Option<f64>,
    pub last_audio_qpc_ns: u64,
    pub input_rate: u32,
}

#[derive(Clone, Copy)]
struct Frame {
    left: f32,
    right: f32,
    stamp: u64,
}

struct State {
    equalizer: crate::equalizer::Processor,
    frames: VecDeque<Frame>,
    capacity: usize,
    target: usize,
    error: Option<String>,
    metrics: Metrics,
    ages: VecDeque<f64>,
    warnings: Vec<Warning>,
}
impl State {
    fn push_frame(&mut self, frame: Frame) {
        if frame.left.abs().max(frame.right.abs()) > 0.004 {
            self.metrics.last_audio_qpc_ns = frame.stamp;
        }
        if self.frames.len() >= self.capacity {
            self.frames.pop_front();
            self.metrics.dropped_frames += 1;
        }
        self.frames.push_back(frame);
    }
}

fn helper_dir() -> Option<PathBuf> {
    std::env::var_os("AIRFLASH_HELPER_DIR").map(PathBuf::from)
}

fn helper(name: &str) -> PathBuf {
    // A helper override dir is strict: when set, helpers resolve only from
    // there, so tests can simulate absent binaries by omitting them.
    if let Some(dir) = helper_dir() {
        return dir.join(name);
    }
    PathBuf::from(name)
}

fn run_helper(name: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(helper(name))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("run {name}"))
}

/// A PipeWire node eligible for capture: a sink (through its monitor)
/// or a source (directly).
#[derive(Debug, Clone)]
pub(crate) struct Sink {
    id: u32,
    name: String,
    description: String,
}

/// Monitor ports of one sink plus the default sink name, parsed from pw-dump.
#[derive(Debug, Default)]
struct Graph {
    sinks: Vec<Sink>,
    sources: Vec<Sink>,
    default_sink: Option<String>,
    /// (node id, port name) for every monitor output port.
    monitors: Vec<(u32, String)>,
    /// (node id, port name) for every non-monitor output port (source capture ports).
    source_ports: Vec<(u32, String)>,
}

fn parse_graph(json: &str) -> Result<Graph> {
    let value: serde_json::Value =
        serde_json::from_str(json).context("parse pw-dump output")?;
    let mut graph = Graph::default();
    let objects = value.as_array().context("pw-dump output is not an array")?;
    for object in objects {
        let object_type = object
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if object_type == "PipeWire:Interface:Metadata"
            && object
                .pointer("/props/metadata.name")
                .and_then(|v| v.as_str())
                == Some("default")
        {
            if let Some(entries) = object.get("metadata").and_then(|v| v.as_array()) {
                for entry in entries {
                    if entry.pointer("/key").and_then(|v| v.as_str())
                        == Some("default.audio.sink")
                    {
                        graph.default_sink = entry
                            .pointer("/value/name")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned);
                    }
                }
            }
        }
        if !object_type.ends_with("Node") {
            continue;
        }
        let id = object.get("id").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let props = object.pointer("/info/props");
        let name = props
            .and_then(|p| p.get("node.name"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let media_class = props
            .and_then(|p| p.get("media.class"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if media_class == "Audio/Sink" || media_class.starts_with("Audio/Source") {
            let node = Sink {
                id,
                name: name.to_owned(),
                description: props
                    .and_then(|p| p.get("node.description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            };
            if media_class == "Audio/Sink" {
                graph.sinks.push(node);
            } else {
                graph.sources.push(node);
            }
        }
    }
    // Second pass: ports reference their node by id.
    for object in objects {
        if !object
            .get("type")
            .and_then(|v| v.as_str())
            .is_some_and(|t| t.ends_with("Port"))
        {
            continue;
        }
        let node_id = object
            .pointer("/info/props/node.id")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let direction = object
            .pointer("/info/props/port.direction")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let port = object
            .pointer("/info/props/port.name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if direction == "out" && port.starts_with("monitor_") {
            graph.monitors.push((node_id, port.to_owned()));
        } else if direction == "out" {
            graph.source_ports.push((node_id, port.to_owned()));
        }
    }
    Ok(graph)
}

/// Resolve the sink to capture: explicit endpoint (node name, id or
/// description) or the default sink, falling back to the first sink.
fn load_graph() -> Result<Graph> {
    let output = run_helper("pw-dump", &[])?;
    ensure!(output.status.success(), "pw-dump failed to list the audio graph");
    parse_graph(&String::from_utf8_lossy(&output.stdout))
}

fn resolve_monitor_in(graph: &Graph, endpoint: Option<&str>) -> Result<(String, String, String)> {
    ensure!(!graph.sinks.is_empty(), "no PipeWire audio sink found");
    let sink = match endpoint.filter(|s| !s.is_empty()) {
        Some(want) => graph
            .sinks
            .iter()
            .find(|s| s.name == want || s.description == want || s.id.to_string() == want)
            .with_context(|| format!("capture endpoint unavailable: {want}"))?
            .clone(),
        None => graph
            .default_sink
            .as_deref()
            .and_then(|name| graph.sinks.iter().find(|s| s.name == name))
            .or_else(|| graph.sinks.first())
            .context("no PipeWire audio sink found")?
            .clone(),
    };
    let mut ports: Vec<&str> = graph
        .monitors
        .iter()
        .filter(|(id, _)| *id == sink.id)
        .map(|(_, name)| name.as_str())
        .collect();
    ports.sort_unstable();
    let left = ports
        .iter()
        .find(|p| p.ends_with("_FL"))
        .context(format!(
            "sink '{}' has no monitor ports; is PipeWire running?",
            sink.name
        ))?;
    let right = ports
        .iter()
        .find(|p| p.ends_with("_FR"))
        .context(format!("sink '{}' has no stereo monitor ports", sink.name))?;
    // Fully qualified `node:port` names: bare monitor names are ambiguous
    // when several sinks exist, and pw-link rejects them.
    Ok((
        sink.name.clone(),
        format!("{}:{left}", sink.name),
        format!("{}:{right}", sink.name),
    ))
}

/// What a capture endpoint resolved to: a sink's monitor ports, or a
/// source node captured directly (microphone, USB audio input, Bluetooth
/// A2DP source, virtual source, ...). Source ports are fully qualified
/// `node:port` names when the source exposes named stereo/mono ports.
#[derive(Debug)]
pub(crate) enum CaptureTarget {
    Monitor {
        sink: String,
        left: String,
        right: String,
    },
    Source {
        node: Sink,
        left: Option<String>,
        right: Option<String>,
    },
}

/// Resolve a capture endpoint (node name, id or description). Sinks keep
/// the historical monitor behavior; anything that names a source node —
/// and only that — takes the direct source path. No endpoint (or an empty
/// one) means the default sink's monitor.
pub(crate) fn resolve_target(endpoint: Option<&str>) -> Result<CaptureTarget> {
    let graph = load_graph()?;
    if let Some(want) = endpoint.filter(|s| !s.is_empty()) {
        let names_sink = graph
            .sinks
            .iter()
            .any(|s| s.name == want || s.description == want || s.id.to_string() == want);
        if !names_sink {
            if let Some(source) = graph
                .sources
                .iter()
                .find(|s| s.name == want || s.description == want || s.id.to_string() == want)
            {
                let mut ports: Vec<&str> = graph
                    .source_ports
                    .iter()
                    .filter(|(id, _)| *id == source.id)
                    .map(|(_, name)| name.as_str())
                    .collect();
                ports.sort_unstable();
                let pick = |suffix: &str| {
                    ports
                        .iter()
                        .find(|p| p.ends_with(suffix))
                        .map(|p| format!("{}:{p}", source.name))
                };
                let mono = pick("_MONO");
                let left = pick("_FL").or_else(|| mono.clone());
                let right = pick("_FR").or(mono);
                return Ok(CaptureTarget::Source {
                    node: source.clone(),
                    left,
                    right,
                });
            }
        }
    }
    let (sink, left, right) = resolve_monitor_in(&graph, endpoint)?;
    Ok(CaptureTarget::Monitor { sink, left, right })
}

/// One capturable node in the PipeWire graph, for device pickers.
#[derive(Debug, Clone)]
pub struct CaptureDevice {
    pub kind: &'static str,
    pub name: String,
    pub description: String,
}

/// List every capturable node: sinks (captured through their monitor) and
/// sources (captured directly). Used by the CLI and the settings picker.
pub fn list_capture_devices() -> Result<Vec<CaptureDevice>> {
    let graph = load_graph()?;
    Ok(graph
        .sinks
        .iter()
        .map(|s| CaptureDevice {
            kind: "sink",
            name: s.name.clone(),
            description: s.description.clone(),
        })
        .chain(graph.sources.iter().map(|s| CaptureDevice {
            kind: "source",
            name: s.name.clone(),
            description: s.description.clone(),
        }))
        .collect())
}

fn node_present(node: &str) -> bool {
    let Ok(output) = run_helper("pw-dump", &[]) else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    String::from_utf8_lossy(&output.stdout).contains(node)
}

fn link_ports(outputs: &[&str], node: &str) -> Result<()> {
    if std::env::var_os("AIRFLASH_CAPTURE_NO_LINK").is_some_and(|v| v == "1") {
        return Ok(());
    }
    // The capture node appears asynchronously after spawn; poll briefly.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if node_present(node) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    ensure!(node_present(node), "capture node '{node}' never appeared");
    for (output, channel) in outputs.iter().zip(["input_FL", "input_FR"]) {
        let target = format!("{node}:{channel}");
        let status = run_helper("pw-link", &[output, &target])?;
        ensure!(
            status.status.success(),
            "cannot link {output} to {target}: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        );
    }
    Ok(())
}

enum Backend {
    PipeWire,
    Pulse,
}

fn spawn_capture(endpoint: Option<String>, node: String) -> Result<(Child, Backend)> {
    // Prefer PipeWire; its monitor ports follow the default sink.
    let probe = Command::new(helper("pw-record"))
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match probe {
        Ok(status) if status.success() => {
            let target = resolve_target(endpoint.as_deref())?;
            // Monitor capture and named-port source capture spawn unlinked
            // (--target 0) and link explicitly; a source without named
            // ports is targeted directly and the session manager links it.
            let (target_arg, links): (String, Vec<String>) = match &target {
                CaptureTarget::Monitor { left, right, .. } => {
                    ("0".to_string(), vec![left.clone(), right.clone()])
                }
                CaptureTarget::Source { node, left, right } => match (left, right) {
                    (Some(l), Some(r)) => ("0".to_string(), vec![l.clone(), r.clone()]),
                    _ => (node.id.to_string(), Vec::new()),
                },
            };
            let mut child = Command::new(helper("pw-record"))
                .args([
                    OsStr::new("--target"),
                    OsStr::new(&target_arg),
                    OsStr::new("--format"),
                    OsStr::new("s16"),
                    OsStr::new("--rate"),
                    OsStr::new(&NATIVE_RATE.to_string()),
                    OsStr::new("--channels"),
                    OsStr::new("2"),
                    OsStr::new("--latency"),
                    OsStr::new("50ms"),
                    OsStr::new("-P"),
                    OsStr::new(&format!("{{node.name={node}}}")),
                    OsStr::new("-"),
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .context("spawn pw-record")?;
            // A child that exits immediately (missing daemon, bad args) is a
            // configuration error, not a silent stream.
            thread::sleep(Duration::from_millis(300));
            if let Ok(Some(status)) = child.try_wait() {
                anyhow::bail!("pw-record exited immediately (status {status})");
            }
            if !links.is_empty() {
                let ports: Vec<&str> = links.iter().map(String::as_str).collect();
                link_ports(&ports, &node).inspect_err(|_| {
                    let _ = child.kill();
                })?;
            }
            Ok((child, Backend::PipeWire))
        }
        _ => spawn_parec(endpoint),
    }
}

fn spawn_parec(endpoint: Option<String>) -> Result<(Child, Backend)> {
    let device = match endpoint.filter(|s| !s.is_empty()) {
        Some(want) => match resolve_target(Some(&want)) {
            Ok(CaptureTarget::Monitor { sink, .. }) => format!("{sink}.monitor"),
            Ok(CaptureTarget::Source { node, .. }) => node.name,
            // No PipeWire graph (PulseAudio-only host): the endpoint is
            // already a Pulse source name.
            Err(_) => want,
        },
        None => {
            let output = run_helper("pactl", &["get-default-sink"])?;
            ensure!(output.status.success(), "pactl cannot find the default sink");
            format!("{}.monitor", String::from_utf8_lossy(&output.stdout).trim())
        }
    };
    let child = Command::new(helper("parec"))
        .args([
            OsStr::new("--device"),
            OsStr::new(&device),
            OsStr::new("--format=s16le"),
            OsStr::new(&format!("--rate={NATIVE_RATE}")),
            OsStr::new("--channels=2"),
            OsStr::new("--latency-msec=50"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context(
            "no system-audio helper found (need pw-record from PipeWire or parec from PulseAudio)",
        )?;
    Ok((child, Backend::Pulse))
}

fn feed_native(
    reader: &mut dyn std::io::Read,
    rate: u32,
    stop: &AtomicBool,
    state: &Arc<Mutex<State>>,
    ready: &std::sync::mpsc::SyncSender<std::result::Result<(), String>>,
) -> Result<()> {
    let chunk = (NATIVE_RATE / 100).max(16) as usize;
    let mut resampler = SincFixedIn::<f32>::new(
        rate as f64 / f64::from(NATIVE_RATE),
        1.001,
        SincInterpolationParameters {
            sinc_len: 64,
            f_cutoff: 0.9,
            interpolation: SincInterpolationType::Cubic,
            oversampling_factor: 128,
            window: WindowFunction::BlackmanHarris2,
        },
        chunk,
        2,
    )?;
    let mut inputs = [VecDeque::<f32>::new(), VecDeque::<f32>::new()];
    state.lock().unwrap().metrics.input_rate = NATIVE_RATE;
    let frame_ns = 1_000_000_000u64 / u64::from(rate);
    let mut signalled = false;
    let mut produced = 0u64;
    let mut raw = vec![0u8; chunk * NATIVE_CHANNELS * 2];
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Err(error) = read_exact_timeout(reader, &mut raw) {
            anyhow::bail!("capture stream ended: {error:#}");
        }
        let start = unix_ns();
        for pair in raw.chunks_exact(4) {
            let left = i16::from_le_bytes([pair[0], pair[1]]) as f32 / 32768.0;
            let right = i16::from_le_bytes([pair[2], pair[3]]) as f32 / 32768.0;
            inputs[0].push_back(left);
            inputs[1].push_back(right);
        }
        while inputs[0].len() >= chunk {
            let input: Vec<Vec<f32>> =
                inputs.iter_mut().map(|q| q.drain(..chunk).collect()).collect();
            let delay = if NATIVE_RATE == rate {
                0
            } else {
                resampler.output_delay() as u64 * frame_ns
            };
            let output = if NATIVE_RATE == rate {
                input
            } else {
                // The audio server clock and the send schedule drift apart;
                // steer the ratio by queue level instead of hard-dropping.
                let (have, target) = {
                    let s = state.lock().unwrap();
                    (s.frames.len(), s.target)
                };
                let correction =
                    ((target as f64 - have as f64) / rate as f64 * 0.01).clamp(-0.0005, 0.0005);
                resampler.set_resample_ratio_relative(1.0 + correction, true)?;
                resampler.process(&input, None)?
            };
            {
                let mut shared = state.lock().unwrap();
                for (i, (&left, &right)) in output[0].iter().zip(&output[1]).enumerate() {
                    shared.push_frame(Frame {
                        left,
                        right,
                        stamp: start.saturating_sub(delay) + (produced + i as u64) * frame_ns,
                    });
                }
                shared.metrics.capture_frames += output[0].len() as u64;
            }
            produced += output[0].len() as u64;
        }
        if !signalled {
            let _ = ready.send(Ok(()));
            signalled = true;
        }
    }
}

/// Blocking read with interrupt retries so a slow server cannot wedge the
/// capture thread past shutdown (the reader thread is joined on drop, and a
/// dead helper surfaces as end-of-stream).
fn read_exact_timeout(reader: &mut dyn std::io::Read, buf: &mut [u8]) -> Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match std::io::Read::read(reader, &mut buf[filled..]) {
            Ok(0) => anyhow::bail!("end of capture stream"),
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e).context("read capture stream"),
        }
    }
    Ok(())
}

pub struct Loopback {
    equalizer_control: crate::equalizer::Control,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    child: Option<Child>,
}

impl Loopback {
    pub fn start(
        endpoint: Option<String>,
        rate: u32,
        equalizer: crate::equalizer::Control,
    ) -> Result<Self> {
        ensure!(
            crate::rtp::SUPPORTED_RATES.contains(&rate),
            "unsupported streaming rate {rate}"
        );
        let capacity = rate as usize * 60 / 1000;
        let target = rate as usize * 20 / 1000;
        let state = Arc::new(Mutex::new(State {
            equalizer: crate::equalizer::Processor::new(equalizer.initial(), rate),
            frames: VecDeque::with_capacity(capacity),
            capacity,
            target,
            error: None,
            metrics: Metrics::default(),
            ages: VecDeque::with_capacity(4000),
            warnings: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let node = format!("airflash-capture-{}", std::process::id());
        let (mut child, _backend) = spawn_capture(endpoint, node.clone())
            .context("start system-audio capture (needs pw-record/PipeWire or parec/PulseAudio)")?;
        let stdout = child.stdout.take().context("capture helper has no stdout")?;
        let child_stop = stop.clone();
        let child_state = state.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("linux-capture".into())
            .spawn(move || {
                let mut reader = std::io::BufReader::with_capacity(65536, stdout);
                if let Err(e) = feed_native(&mut reader, rate, &child_stop, &child_state, &tx) {
                    let message = format!("{e:#}");
                    child_state.lock().unwrap().error = Some(message.clone());
                    let _ = tx.try_send(Err(message));
                }
            })?;
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(Self {
                equalizer_control: equalizer,
                state,
                stop,
                worker: Some(worker),
                child: Some(child),
            }),
            result => {
                stop.store(true, Ordering::Release);
                let _ = child.kill();
                let _ = worker.join();
                anyhow::bail!("system-audio capture initialization: {result:?}");
            }
        }
    }

    /// Per-application capture is a Windows WASAPI feature; Linux captures
    /// the whole system mix through the sink monitor instead.
    pub fn start_application(
        _application: crate::applications::Application,
        _pinned: bool,
        _rate: u32,
        _equalizer: crate::equalizer::Control,
    ) -> Result<Self> {
        anyhow::bail!("application capture is not supported on Linux")
    }

    /// True once the capture stream has ended (helper exit or read error).
    pub fn ended(&self) -> bool {
        self.state.lock().unwrap().error.is_some()
    }

    /// Capture-side warnings queued since the last call.
    pub fn take_warnings(&self) -> Vec<Warning> {
        std::mem::take(&mut self.state.lock().unwrap().warnings)
    }

    pub fn last_audio(&self) -> u64 {
        self.state.lock().unwrap().metrics.last_audio_qpc_ns
    }

    pub fn sample_age_ms(&self) -> f64 {
        let s = self.state.lock().unwrap();
        s.frames
            .front()
            .map_or(0.0, |f| unix_ns().saturating_sub(f.stamp) as f64 / 1e6)
    }

    /// `(queued, capacity)` in source frames.
    pub fn depth(&self) -> (usize, usize) {
        let s = self.state.lock().unwrap();
        (s.frames.len(), s.capacity)
    }

    /// The Unix capture path has no rate servo (PipeWire resamples to the
    /// native rate and the rubato stage is fixed), so counters stay zero.
    pub fn servo_metrics(&self) -> crate::servo::ServoCounters {
        crate::servo::ServoCounters {
            samples: 0,
            steps: 0,
            engagements: 0,
            releases: 0,
            saturated: 0,
            filtered_ms: 0.0,
            correction_ppm: 0.0,
        }
    }

    /// Captured drift estimate in parts per billion; not measured on Unix.
    pub fn drift_ppb(&self) -> f64 {
        0.0
    }

    /// Lateness samples feed the Windows rate servo; no-op on Unix.
    pub fn report_lateness(&self, _late_ms: f64) {}

    /// One packet of silence without consuming the capture queue (the
    /// standby fade path); mirrors the Windows packet_inner(force_silence).
    pub fn silence_packet(&self, _gain: f32) -> Result<(Vec<u8>, Option<u64>)> {
        let s = self.state.lock().unwrap();
        if let Some(e) = &s.error {
            anyhow::bail!("capture stopped: {e}");
        }
        Ok((vec![0; FRAMES * 4], None))
    }

    pub fn ready(&self) -> Result<bool> {
        let s = self.state.lock().unwrap();
        if let Some(e) = &s.error {
            anyhow::bail!("{e}");
        }
        Ok(s.frames.len() >= s.target)
    }

    pub fn packet(&self, gain: f32) -> Result<(Vec<u8>, Option<u64>)> {
        let mut s = self.state.lock().unwrap();
        if let Some((sequence, prepared)) = self.equalizer_control.latest() {
            s.equalizer.update(sequence, prepared);
        }
        if let Some(e) = &s.error {
            anyhow::bail!("capture stopped: {e}");
        }
        let mut out = Vec::with_capacity(FRAMES * 4);
        let mut marker = None;
        let now = unix_ns();
        let mut underrun = false;
        for _ in 0..FRAMES {
            let frame = if let Some(f) = s.frames.pop_front() {
                f
            } else {
                underrun = true;
                Frame {
                    left: 0.0,
                    right: 0.0,
                    stamp: now,
                }
            };
            if frame.left.abs().max(frame.right.abs()) > 0.004 {
                marker.get_or_insert(frame.stamp);
            }
            let age = now.saturating_sub(frame.stamp) as f64 / 1e6;
            s.metrics.max_queue_age_ms = s.metrics.max_queue_age_ms.max(age);
            if s.ages.len() == 4000 {
                s.ages.pop_front();
            }
            s.ages.push_back(age);
            for sample in s.equalizer.frame([frame.left, frame.right]) {
                let value = (sample * gain).clamp(-1.0, 1.0);
                out.extend(((value * 32767.0).round() as i16).to_be_bytes());
            }
        }
        if underrun {
            s.metrics.underrun_packets += 1;
        }
        Ok((out, marker))
    }

    /// Restore the capture queue to its normal target after a scheduler stall.
    pub fn discard_stale(&self) {
        let mut s = self.state.lock().unwrap();
        let cutoff = unix_ns().saturating_sub(20_000_000);
        while s.frames.front().is_some_and(|f| f.stamp < cutoff) || s.frames.len() > s.target {
            s.frames.pop_front();
            s.metrics.dropped_frames += 1;
        }
    }

    pub fn metrics(&self) -> Metrics {
        let s = self.state.lock().unwrap();
        let mut metrics = s.metrics.clone();
        let mut ages: Vec<_> = s.ages.iter().copied().collect();
        ages.sort_by(f64::total_cmp);
        metrics.capture_to_send_p95_ms = ages.get(ages.len() * 95 / 100).copied();
        metrics
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Locate a helper binary the way `spawn_capture` does (test seam).
#[cfg(test)]
pub(crate) fn helper_path(name: &str) -> PathBuf {
    helper(name)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::{Mutex as StdMutex, OnceLock};

    /// A capture source with no helper process, for session tests: frames
    /// are pushed directly with feed(), like the Windows test source.
    pub(crate) fn source() -> Loopback {
        let equalizer_control =
            crate::equalizer::Control::new(crate::equalizer::Settings::default(), crate::rtp::RATE)
                .unwrap();
        Loopback {
            state: Arc::new(Mutex::new(State {
                equalizer: crate::equalizer::Processor::new(
                    equalizer_control.initial(),
                    crate::rtp::RATE,
                ),
                frames: VecDeque::new(),
                capacity: crate::rtp::RATE as usize * 60 / 1000,
                target: crate::rtp::RATE as usize * 20 / 1000,
                error: None,
                metrics: Metrics::default(),
                ages: VecDeque::with_capacity(4000),
                warnings: Vec::new(),
            })),
            child: None,
            worker: None,
            stop: Arc::new(AtomicBool::new(false)),
            equalizer_control,
        }
    }

    pub(crate) fn feed(source: &Loopback, amplitude: f32) {
        let mut state = source.state.lock().unwrap();
        for _ in 0..FRAMES {
            state.push_frame(Frame {
                left: amplitude,
                right: amplitude,
                stamp: unix_ns(),
            });
        }
    }

    /// Helper-binary tests mutate process-global env; serialize them.
    fn env_lock() -> &'static StdMutex<()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
    }
    /// A poisoned lock means a sibling test already failed; continue so every
    /// failure is reported instead of cascading.
    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        env_lock().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Fake `pw-*`/`parec` helpers: a sine-wave recorder plus canned graph.
    fn fake_bin(label: &str, with_pw_record: bool, with_parec: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("airflash-helpers-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let graph = serde_json::json!([
            {"id": 36, "type": "PipeWire:Interface:Metadata",
             "props": {"metadata.name": "default"},
             "metadata": [{"key": "default.audio.sink", "value": {"name": "fake-sink"}}]},
            {"id": 7, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "fake-sink", "media.class": "Audio/Sink",
                                "node.description": "Fake Sink"}}},
            {"id": 8, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "other-sink", "media.class": "Audio/Sink",
                                "node.description": "Other Sink"}}},
            {"id": 71, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 7, "port.direction": "out", "port.name": "monitor_FL"}}},
            {"id": 72, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 7, "port.direction": "out", "port.name": "monitor_FR"}}},
            {"id": 73, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 8, "port.direction": "out", "port.name": "monitor_FL"}}},
            {"id": 74, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 8, "port.direction": "out", "port.name": "monitor_FR"}}},
            {"id": 9, "type": "PipeWire:Interface:Node",
             "info": {"props": {"node.name": "fake-mic", "media.class": "Audio/Source",
                                "node.description": "Fake Microphone"}}},
            {"id": 75, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 9, "port.direction": "out", "port.name": "capture_FL"}}},
            {"id": 76, "type": "PipeWire:Interface:Port",
             "info": {"props": {"node.id": 9, "port.direction": "out", "port.name": "capture_FR"}}},
        ]);
        std::fs::write(dir.join("pw-dump"), format!("#!/bin/sh\ncat <<'EOF'\n{graph}\nEOF\n")).unwrap();
        std::fs::write(
            dir.join("pw-link"),
            "#!/bin/sh\n# fake link: succeed only for known monitor ports\ncase \"$1\" in *monitor_FL|*monitor_FR) exit 0;; *) exit 1;; esac\n",
        )
        .unwrap();
        // Endless 440 Hz stereo s16le at 48 kHz on stdout.
        std::fs::write(
            dir.join("pw-record"),
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo fake; exit 0; fi\npython3 -c \"import math,struct,sys; f=sys.stdout.buffer; n=0\nwhile True:\n v=int(12000*math.sin(2*math.pi*440*n/48000)); f.write(struct.pack('<hh',v,v)); n+=1; f.flush() if n%4800==0 else None\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("parec"),
            "#!/bin/sh\npython3 -c \"import math,struct,sys; f=sys.stdout.buffer; n=0\nwhile True:\n v=int(12000*math.sin(2*math.pi*330*n/48000)); f.write(struct.pack('<hh',v,v)); n+=1; f.flush() if n%4800==0 else None\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("pactl"), "#!/bin/sh\necho fake-sink\n").unwrap();
        for name in ["pw-dump", "pw-link", "pw-record", "parec", "pactl"] {
            let path = dir.join(name);
            if (!with_pw_record && name == "pw-record") || (!with_parec && name == "parec") {
                std::fs::remove_file(&path).ok();
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        dir
    }

    fn control() -> crate::equalizer::Control {
        crate::equalizer::Control::new(
            crate::equalizer::Settings::default(),
            crate::rtp::RATE,
        )
        .unwrap()
    }

    #[test]
    fn graph_parsing_selects_default_and_explicit_sinks() {
        let _guard = env_guard();
        let dir = fake_bin("graph", true, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        let graph = load_graph().unwrap();
        let (sink, left, right) = resolve_monitor_in(&graph, None).unwrap();
        assert_eq!(
            (sink.as_str(), left.as_str(), right.as_str()),
            ("fake-sink", "fake-sink:monitor_FL", "fake-sink:monitor_FR")
        );
        let (sink, _, _) = resolve_monitor_in(&graph, Some("Other Sink")).unwrap();
        assert_eq!(sink, "other-sink");
        assert!(resolve_monitor_in(&graph, Some("missing")).is_err());
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn target_resolution_distinguishes_sinks_and_sources() {
        let _guard = env_guard();
        let dir = fake_bin("targets", true, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        match resolve_target(Some("Fake Microphone")).unwrap() {
            CaptureTarget::Source { node, left, right } => {
                assert_eq!(node.name, "fake-mic");
                assert_eq!(left.as_deref(), Some("fake-mic:capture_FL"));
                assert_eq!(right.as_deref(), Some("fake-mic:capture_FR"));
            }
            other => panic!("expected a source target, got {other:?}"),
        }
        match resolve_target(Some("Other Sink")).unwrap() {
            CaptureTarget::Monitor { sink, .. } => assert_eq!(sink, "other-sink"),
            other => panic!("expected a monitor target, got {other:?}"),
        }
        assert!(resolve_target(Some("missing")).is_err());
        let devices = list_capture_devices().unwrap();
        assert_eq!(devices.len(), 3);
        assert!(devices.iter().any(|d| d.kind == "source" && d.name == "fake-mic"));
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipewire_source_capture_streams_non_silent_audio() {
        let _guard = env_guard();
        let dir = fake_bin("srccapture", true, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        let source = Loopback::start(Some("fake-mic".to_string()), crate::rtp::RATE, control()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !source.ready().unwrap() {
            assert!(Instant::now() < deadline, "source capture never became ready");
            thread::sleep(Duration::from_millis(5));
        }
        let (pcm, _) = source.packet(1.0).unwrap();
        assert!(pcm.iter().any(|b| *b != 0), "source capture emitted silence");
        drop(source);
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pulse_source_capture_streams_non_silent_audio() {
        let _guard = env_guard();
        let dir = fake_bin("pulsesrc", false, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        let source = Loopback::start(Some("Fake Microphone".to_string()), crate::rtp::RATE, control()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !source.ready().unwrap() {
            assert!(Instant::now() < deadline, "pulse source capture never became ready");
            thread::sleep(Duration::from_millis(5));
        }
        let (pcm, _) = source.packet(1.0).unwrap();
        assert!(pcm.iter().any(|b| *b != 0), "pulse source capture emitted silence");
        drop(source);
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipewire_capture_streams_non_silent_audio() {
        let _guard = env_guard();
        let dir = fake_bin("capture", true, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        let source = Loopback::start(None, crate::rtp::RATE, control()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !source.ready().unwrap() {
            assert!(Instant::now() < deadline, "capture never became ready");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(source.metrics().input_rate, NATIVE_RATE);
        let (pcm, marker) = source.packet(1.0).unwrap();
        assert_eq!(pcm.len(), FRAMES * 4);
        assert!(pcm.iter().any(|b| *b != 0), "capture emitted silence");
        assert!(marker.is_some());
        assert!(source.metrics().capture_frames > 0);
        drop(source);
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pulse_fallback_captures_when_pipewire_is_absent() {
        let _guard = env_guard();
        let dir = fake_bin("pulse", false, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        let source = Loopback::start(None, crate::rtp::RATE, control()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !source.ready().unwrap() {
            assert!(Instant::now() < deadline, "pulse capture never became ready");
            thread::sleep(Duration::from_millis(5));
        }
        let (pcm, _) = source.packet(1.0).unwrap();
        assert!(pcm.iter().any(|b| *b != 0), "pulse capture emitted silence");
        drop(source);
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_helpers_fail_with_guidance() {
        let _guard = env_guard();
        let dir = std::env::temp_dir().join(format!("airflash-helpers-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        unsafe { std::env::set_var("AIRFLASH_CAPTURE_NO_LINK", "1") };
        // Point PATH at the empty dir only so neither pw-record nor parec resolve.
        let old_path = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", &dir) };
    let error = match Loopback::start(None, crate::rtp::RATE, control()) {
        Ok(_) => panic!("capture unexpectedly started without helpers"),
        Err(e) => e.to_string(),
    };
        assert!(error.contains("pw-record"), "unexpected error: {error}");
        if let Some(path) = old_path {
            unsafe { std::env::set_var("PATH", path) };
        }
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        unsafe { std::env::remove_var("AIRFLASH_CAPTURE_NO_LINK") };
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejected_rates_are_not_started() {
        assert!(Loopback::start(None, 96000, control()).is_err());
    }

    #[test]
    fn helper_path_prefers_override_dir() {
        let _guard = env_guard();
        let dir = fake_bin("which", true, true);
        unsafe { std::env::set_var("AIRFLASH_HELPER_DIR", &dir) };
        assert_eq!(helper_path("pw-record"), dir.join("pw-record"));
        unsafe { std::env::remove_var("AIRFLASH_HELPER_DIR") };
        std::fs::remove_dir_all(&dir).ok();
    }
}

