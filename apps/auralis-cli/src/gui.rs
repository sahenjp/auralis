//! Lightweight local control UI for a running Auralis duplex session.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auralis_audio_io::{
    DeviceSelection, DuplexRunHandle, DuplexRunOptions, DuplexRunState,
    start_characterization_with_processor,
};
use auralis_core::{FrameProcessor, Passthrough, PipelineConfig};
use auralis_denoisers::{
    DeepFilterFrameProcessor, InferenceTimingHandle, RnnoiseFrameProcessor, UlUnasFrameProcessor,
};
use auralis_diagnostics::ProcessResourceMonitor;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::DynError;

const GUI_HTML: &str = include_str!("../gui/index.html");
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const SESSION_DURATION: Duration = Duration::from_secs(86_400);
const SESSION_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuiProfile {
    Passthrough,
    LowLatency,
    Balanced,
    MaximumQuality,
}

impl GuiProfile {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "passthrough" => Some(Self::Passthrough),
            "low-latency" => Some(Self::LowLatency),
            "balanced" => Some(Self::Balanced),
            "maximum-quality" => Some(Self::MaximumQuality),
            _ => None,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::LowLatency => "low-latency",
            Self::Balanced => "balanced",
            Self::MaximumQuality => "maximum-quality",
        }
    }

    const fn engine(self) -> &'static str {
        match self {
            Self::Passthrough => "passthrough",
            Self::LowLatency => "rnnoise-main-official-model",
            Self::Balanced => "ul-unas-dns3-streaming-onnx",
            Self::MaximumQuality => "deepfilternet3-ll",
        }
    }

    const fn algorithmic_latency_samples(self) -> usize {
        match self {
            Self::Passthrough => 0,
            Self::LowLatency => 960,
            Self::Balanced => 2_208,
            Self::MaximumQuality => 480,
        }
    }
}

struct GuiArguments {
    bind: String,
    port: u16,
    profile: GuiProfile,
    model_path: String,
    library_path: String,
    no_open: bool,
}

struct GuiSession {
    handle: DuplexRunHandle,
    profile: GuiProfile,
    processor: &'static str,
    algorithmic_latency_samples: usize,
    timing: Option<InferenceTimingHandle>,
    started: Instant,
    monitor: Option<ProcessResourceMonitor>,
}

impl GuiSession {
    fn stop(self) -> Result<(), DynError> {
        let Self {
            handle, monitor, ..
        } = self;
        let result = handle.stop().map(|_| ());
        if let Some(monitor) = monitor {
            let _ = monitor.finish();
        }
        result
    }
}

struct GuiState {
    profile: GuiProfile,
    model_path: String,
    library_path: String,
    input_device_id: Option<String>,
    output_device_id: Option<String>,
    session: Option<GuiSession>,
    last_error: Option<String>,
}

impl GuiState {
    fn new(arguments: &GuiArguments) -> Self {
        Self {
            profile: arguments.profile,
            model_path: arguments.model_path.clone(),
            library_path: arguments.library_path.clone(),
            input_device_id: None,
            output_device_id: None,
            session: None,
            last_error: None,
        }
    }
}

type SharedState = Arc<Mutex<GuiState>>;

#[derive(Deserialize)]
struct StartRequest {
    profile: Option<String>,
    model_path: Option<String>,
    library_path: Option<String>,
    input_device_id: Option<String>,
    output_device_id: Option<String>,
}

struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

pub fn run(arguments: impl Iterator<Item = String>) -> Result<(), DynError> {
    let arguments = parse_arguments(arguments)?;
    let state = Arc::new(Mutex::new(GuiState::new(&arguments)));
    let listener = TcpListener::bind((arguments.bind.as_str(), arguments.port))?;
    let address = listener.local_addr()?;
    let host = match address.ip() {
        std::net::IpAddr::V4(ip) => ip.to_string(),
        std::net::IpAddr::V6(ip) => format!("[{ip}]"),
    };
    let url = format!("http://{host}:{}", address.port());
    println!("Auralis GUI listening at {url}");
    if !arguments.no_open {
        open_browser(&url);
    }

    for incoming in listener.incoming() {
        let stream = incoming?;
        let state = Arc::clone(&state);
        thread::Builder::new()
            .name("auralis-gui-http".to_owned())
            .spawn(move || {
                if let Err(error) = handle_client(stream, &state) {
                    eprintln!("auralis-gui: {error}");
                }
            })?;
    }
    Ok(())
}

fn parse_arguments(mut arguments: impl Iterator<Item = String>) -> Result<GuiArguments, DynError> {
    let mut parsed = GuiArguments {
        bind: "127.0.0.1".to_owned(),
        port: 8_765,
        profile: GuiProfile::Balanced,
        model_path: default_model_path(),
        library_path: std::env::var("AURALIS_RNNOISE_LIBRARY").unwrap_or_default(),
        no_open: false,
    };
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--bind" => parsed.bind = arguments.next().ok_or("--bind requires an address")?,
            "--port" => {
                parsed.port = arguments
                    .next()
                    .ok_or("--port requires a number")?
                    .parse()?;
            }
            "--profile" => {
                let value = arguments.next().ok_or("--profile requires a value")?;
                parsed.profile = GuiProfile::parse(&value)
                    .ok_or_else(|| format!("unsupported GUI profile: {value}"))?;
            }
            "--model" => {
                parsed.model_path = arguments.next().ok_or("--model requires a path")?;
            }
            "--library" => {
                parsed.library_path = arguments.next().ok_or("--library requires a path")?;
            }
            "--no-open" => parsed.no_open = true,
            value => return Err(format!("unexpected GUI argument: {value}").into()),
        }
    }
    Ok(parsed)
}

fn default_model_path() -> String {
    let path = PathBuf::from("models/ulunas_stream_simple.onnx");
    if path.is_file() {
        path.display().to_string()
    } else {
        String::new()
    }
}

fn handle_client(mut stream: TcpStream, state: &SharedState) -> Result<(), DynError> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let request = read_request(&mut stream)?;
    let (status, content_type, body) = route(request, state);
    write_response(&mut stream, status, content_type, &body)?;
    Ok(())
}

fn route(request: HttpRequest, state: &SharedState) -> (u16, &'static str, String) {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => {
            (200, "text/html; charset=utf-8", GUI_HTML.to_owned())
        }
        ("GET", "/api/state") => (200, "application/json", state_json(state).to_string()),
        ("GET", "/api/devices") => match auralis_audio_io::enumerate_devices() {
            Ok(devices) => (
                200,
                "application/json",
                json!({ "schema_version": 1, "devices": devices }).to_string(),
            ),
            Err(error) => json_error(500, error.to_string()),
        },
        ("POST", "/api/session/start") => match serde_json::from_str::<StartRequest>(&request.body)
        {
            Ok(start) => match start_session(state, start) {
                Ok(value) => (200, "application/json", value.to_string()),
                Err(error) => json_error(400, error.to_string()),
            },
            Err(error) => json_error(400, format!("invalid start request: {error}")),
        },
        ("POST", "/api/session/stop") => match stop_session(state) {
            Ok(value) => (200, "application/json", value.to_string()),
            Err(error) => json_error(500, error.to_string()),
        },
        _ => json_error(404, "not found".to_owned()),
    }
}

fn start_session(state: &SharedState, request: StartRequest) -> Result<Value, DynError> {
    let old = {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("GUI state lock poisoned"))?;
        if let Some(session) = state.session.as_ref() {
            match session.handle.control().state() {
                DuplexRunState::Starting | DuplexRunState::Running => {
                    return Err("an audio session is already running".into());
                }
                DuplexRunState::Finished | DuplexRunState::Failed => state.session.take(),
            }
        } else {
            None
        }
    };
    if let Some(old) = old {
        let _ = old.stop();
    }
    let mut state = state
        .lock()
        .map_err(|_| io::Error::other("GUI state lock poisoned"))?;
    if state.session.is_some() {
        return Err("an audio session is already running".into());
    }

    let profile = request
        .profile
        .as_deref()
        .map(|value| {
            GuiProfile::parse(value).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported GUI profile: {value}"),
                )
            })
        })
        .transpose()?
        .unwrap_or(state.profile);
    let model_path = request
        .model_path
        .unwrap_or_else(|| state.model_path.clone());
    let library_path = request
        .library_path
        .unwrap_or_else(|| state.library_path.clone());
    let input_device_id = request
        .input_device_id
        .or_else(|| state.input_device_id.clone());
    let output_device_id = request
        .output_device_id
        .or_else(|| state.output_device_id.clone());
    let session = build_session(
        profile,
        &model_path,
        &library_path,
        DeviceSelection {
            input_id: input_device_id.clone(),
            output_id: output_device_id.clone(),
        },
    )?;
    state.profile = profile;
    state.model_path = model_path;
    state.library_path = library_path;
    state.input_device_id = input_device_id;
    state.output_device_id = output_device_id;
    state.last_error = None;
    state.session = Some(session);
    Ok(json!({ "ok": true, "state": "starting", "profile": profile.id() }))
}

fn stop_session(state: &SharedState) -> Result<Value, DynError> {
    let session = {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("GUI state lock poisoned"))?;
        state.session.take()
    };
    let Some(session) = session else {
        return Ok(json!({ "ok": true, "state": "idle" }));
    };
    let result = session.stop();
    if let Err(error) = &result {
        let mut state = state
            .lock()
            .map_err(|_| io::Error::other("GUI state lock poisoned"))?;
        state.last_error = Some(error.to_string());
    }
    result.map(|_| json!({ "ok": true, "state": "idle" }))
}

fn build_session(
    profile: GuiProfile,
    model_path: &str,
    library_path: &str,
    selection: DeviceSelection,
) -> Result<GuiSession, DynError> {
    let options = DuplexRunOptions {
        duration: SESSION_DURATION,
        sample_interval: SESSION_SAMPLE_INTERVAL,
        requested_buffer_frames: None,
        mute_output: false,
        pipeline: PipelineConfig::default(),
    };
    let (handle, timing, algorithmic_latency_samples, processor_name) = match profile {
        GuiProfile::Passthrough => {
            let processor = Passthrough;
            let latency = processor.algorithmic_latency_samples();
            let name = processor.name();
            (
                start_characterization_with_processor(options, processor, selection)?,
                None,
                latency,
                name,
            )
        }
        GuiProfile::LowLatency => {
            let path = required_path(library_path, "RNNoise library")?;
            let processor = RnnoiseFrameProcessor::load(path)?;
            let timing = processor.timing_handle();
            let latency = processor.algorithmic_latency_samples();
            let name = processor.name();
            (
                start_characterization_with_processor(options, processor, selection)?,
                Some(timing),
                latency,
                name,
            )
        }
        GuiProfile::Balanced => {
            let path = required_path(model_path, "UL-UNAS model")?;
            let processor = UlUnasFrameProcessor::load(path)?;
            let timing = processor.timing_handle();
            let latency = processor.algorithmic_latency_samples();
            let name = processor.name();
            (
                start_characterization_with_processor(options, processor, selection)?,
                Some(timing),
                latency,
                name,
            )
        }
        GuiProfile::MaximumQuality => {
            let path = required_path(model_path, "DeepFilterNet model")?;
            let processor = DeepFilterFrameProcessor::load(path)?;
            let timing = processor.timing_handle();
            let latency = processor.algorithmic_latency_samples();
            let name = processor.name();
            (
                start_characterization_with_processor(options, processor, selection)?,
                Some(timing),
                latency,
                name,
            )
        }
    };
    Ok(GuiSession {
        handle,
        profile,
        processor: processor_name,
        algorithmic_latency_samples,
        timing,
        started: Instant::now(),
        monitor: ProcessResourceMonitor::start().ok(),
    })
}

fn required_path(value: &str, label: &str) -> Result<PathBuf, DynError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} path is required for this profile").into());
    }
    Ok(PathBuf::from(value))
}

fn state_json(shared: &SharedState) -> Value {
    let state = match shared.lock() {
        Ok(state) => state,
        Err(_) => {
            return json!({ "schema_version": 1, "state": "failed", "error": "GUI state lock poisoned" });
        }
    };
    let configured_profile = state.profile;
    let configured = json!({
        "profile": configured_profile.id(),
        "engine": configured_profile.engine(),
        "model_path": non_empty(&state.model_path),
        "library_path": non_empty(&state.library_path),
        "input_device_id": state.input_device_id.clone(),
        "output_device_id": state.output_device_id.clone(),
    });
    let Some(session) = state.session.as_ref() else {
        return json!({
            "schema_version": 1,
            "state": "idle",
            "configured": configured,
            "profile": configured_profile.id(),
            "engine": configured_profile.engine(),
            "processor": Value::Null,
            "algorithmic_latency": latency_json(configured_profile.algorithmic_latency_samples()),
            "queue_capacity_frames": PipelineConfig::default().queue_capacity_frames,
            "metrics": Value::Null,
            "resources": Value::Null,
            "inference": Value::Null,
            "total_software_pipeline_latency_ms": Value::Null,
            "physical_e2e_latency_measured": false,
            "error": state.last_error,
        });
    };
    let control = session.handle.control();
    let metrics = control.metrics_snapshot();
    let resources = session
        .monitor
        .as_ref()
        .and_then(ProcessResourceMonitor::latest);
    let inference = session.timing.as_ref().map(InferenceTimingHandle::snapshot);
    let error = control.error().or_else(|| state.last_error.clone());
    json!({
        "schema_version": 1,
        "state": run_state_name(control.state()),
        "configured": configured,
        "profile": session.profile.id(),
        "engine": session.profile.engine(),
        "processor": session.processor,
        "algorithmic_latency": latency_json(session.algorithmic_latency_samples),
        "queue_capacity_frames": PipelineConfig::default().queue_capacity_frames,
        "metrics": metrics,
        "resources": resources,
        "inference": inference,
        "total_software_pipeline_latency_ms": metrics.as_ref().map(|metrics| metrics.software_latency_average_ms),
        "elapsed_seconds": session.started.elapsed().as_secs_f64(),
        "physical_e2e_latency_measured": false,
        "error": error,
    })
}

fn latency_json(samples: usize) -> Value {
    json!({ "samples": samples, "milliseconds": samples as f64 / 48.0, "frames": samples as f64 / 480.0 })
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn run_state_name(state: DuplexRunState) -> &'static str {
    match state {
        DuplexRunState::Starting => "starting",
        DuplexRunState::Running => "running",
        DuplexRunState::Finished => "finished",
        DuplexRunState::Failed => "failed",
    }
}

fn json_error(status: u16, message: String) -> (u16, &'static str, String) {
    (
        status,
        "application/json",
        json!({ "ok": false, "error": message }).to_string(),
    )
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, DynError> {
    let mut bytes = Vec::with_capacity(8_192);
    let mut buffer = [0_u8; 4_096];
    let mut expected_total = None;
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > MAX_REQUEST_BYTES {
            return Err("request is too large".into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if expected_total.is_none()
            && let Some(header_end) = find_header_end(&bytes)
        {
            let headers = std::str::from_utf8(&bytes[..header_end])?;
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            expected_total = Some(header_end + 4 + content_length.unwrap_or(0));
        }
        if expected_total.is_some_and(|total| bytes.len() >= total) {
            break;
        }
    }
    let header_end = find_header_end(&bytes).ok_or("incomplete HTTP request")?;
    let headers = std::str::from_utf8(&bytes[..header_end])?;
    let mut request_line = headers
        .lines()
        .next()
        .ok_or("missing HTTP request line")?
        .split_whitespace();
    let method = request_line.next().ok_or("missing HTTP method")?.to_owned();
    let target = request_line.next().ok_or("missing HTTP target")?;
    let path = target.split('?').next().unwrap_or(target).to_owned();
    let body_start = header_end + 4;
    let body = if body_start < bytes.len() {
        String::from_utf8(bytes[body_start..].to_vec())?
    } else {
        String::new()
    };
    Ok(HttpRequest { method, path, body })
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let result = Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        eprintln!("auralis-gui: could not open browser: {error}; open {url} manually");
    }
}

#[cfg(test)]
mod tests {
    use super::{GuiProfile, find_header_end, latency_json};

    #[test]
    fn profile_latency_metadata_is_explicit() {
        assert_eq!(GuiProfile::parse("balanced"), Some(GuiProfile::Balanced));
        assert_eq!(GuiProfile::Balanced.algorithmic_latency_samples(), 2_208);
        assert_eq!(latency_json(480)["milliseconds"], 10.0);
    }

    #[test]
    fn request_header_boundary_is_found_without_unbounded_scan() {
        assert_eq!(find_header_end(b"GET / HTTP/1.1\r\n\r\nbody"), Some(14));
        assert_eq!(find_header_end(b"GET / HTTP/1.1\n\n"), None);
    }
}
