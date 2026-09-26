//! Audio-device discovery and measured duplex stream adapters.

#![forbid(unsafe_code)]

use std::error::Error;
use std::io;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use auralis_core::{
    CallbackTiming, FRAME_SAMPLES, FrameProcessor, MetricsSnapshot, Passthrough, PipelineConfig,
    SAMPLE_RATE_HZ, start_pipeline,
};
#[cfg(target_os = "windows")]
use auralis_wasapi::register_current_thread_for_pro_audio;
use auralis_wasapi::{WasapiEnginePeriod, query_engine_period};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, Host, InputCallbackInfo, OutputCallbackInfo, SampleFormat,
    StreamConfig, StreamInstant, SupportedBufferSize, SupportedStreamConfigRange,
};
use serde::Serialize;

type DynError = Box<dyn Error + Send + Sync>;

const STREAM_BUILD_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const WORKER_SCHEDULING_SUCCEEDED: u8 = 1;
#[cfg(target_os = "windows")]
const WORKER_SCHEDULING_FAILED: u8 = 2;

#[derive(Clone, Debug, Serialize)]
pub struct WorkerSchedulingReport {
    pub requested_mmcss_task: Option<&'static str>,
    pub requested_mmcss_priority: Option<&'static str>,
    pub registration_attempted: bool,
    pub registration_succeeded: bool,
    pub error: Option<String>,
}

#[derive(Default)]
struct WorkerSchedulingStatus {
    state: AtomicU8,
    error: OnceLock<String>,
}

#[derive(Clone, Default)]
struct WorkerSchedulingState {
    status: Arc<WorkerSchedulingStatus>,
}

impl WorkerSchedulingState {
    fn report(&self) -> WorkerSchedulingReport {
        let state = self.status.state.load(Ordering::Relaxed);
        WorkerSchedulingReport {
            requested_mmcss_task: cfg!(target_os = "windows").then_some("Pro Audio"),
            requested_mmcss_priority: cfg!(target_os = "windows").then_some("normal"),
            registration_attempted: state != 0,
            registration_succeeded: state == WORKER_SCHEDULING_SUCCEEDED,
            error: self.status.error.get().cloned(),
        }
    }
}

struct PlatformProcessor<P> {
    inner: P,
    _scheduling: WorkerSchedulingState,
    #[cfg(target_os = "windows")]
    registration_attempted: bool,
}

impl<P: FrameProcessor> FrameProcessor for PlatformProcessor<P> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn algorithmic_latency_samples(&self) -> usize {
        self.inner.algorithmic_latency_samples()
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        #[cfg(target_os = "windows")]
        if !self.registration_attempted {
            self.registration_attempted = true;
            match register_current_thread_for_pro_audio() {
                Ok(()) => {
                    self._scheduling
                        .status
                        .state
                        .store(WORKER_SCHEDULING_SUCCEEDED, Ordering::Relaxed);
                }
                Err(error) => {
                    let _ = self._scheduling.status.error.set(error);
                    self._scheduling
                        .status
                        .state
                        .store(WORKER_SCHEDULING_FAILED, Ordering::Relaxed);
                }
            }
        }
        self.inner.process(samples);
    }
}

fn platform_processor<P: FrameProcessor>(
    inner: P,
) -> (PlatformProcessor<P>, WorkerSchedulingState) {
    let scheduling = WorkerSchedulingState::default();
    let processor = PlatformProcessor {
        inner,
        _scheduling: scheduling.clone(),
        #[cfg(target_os = "windows")]
        registration_attempted: false,
    };
    (processor, scheduling)
}

#[derive(Clone, Debug, Serialize)]
pub struct SupportedFormatDescriptor {
    pub channels: u16,
    pub sample_format: String,
    pub minimum_sample_rate_hz: u32,
    pub maximum_sample_rate_hz: u32,
    pub minimum_buffer_frames: Option<u32>,
    pub maximum_buffer_frames: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceDescriptor {
    pub host: String,
    pub id: String,
    pub name: String,
    pub direction: &'static str,
    pub is_default: bool,
    pub supported_formats: Vec<SupportedFormatDescriptor>,
    pub wasapi_engine_period: Option<WasapiEnginePeriod>,
    pub wasapi_probe_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamFormatReport {
    pub channels: u16,
    pub sample_rate_hz: u32,
    pub sample_format: &'static str,
    pub buffer_frames: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamClockReport {
    pub start_ns: u64,
    pub end_ns: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueueTimeSeriesPoint {
    pub elapsed_seconds: f64,
    pub input_queue_depth_frames: usize,
    pub output_queue_depth_frames: usize,
    pub output_buffered_samples: usize,
    pub input_callback_sample_frames: u64,
    pub output_callback_sample_frames: u64,
    pub input_overrun_frames: u64,
    pub output_overrun_frames: u64,
    pub output_underrun_callbacks: u64,
    pub stream_xruns: u64,
    pub startup_preroll_completed: bool,
    pub drift_correction_ratio_ppm: f64,
    pub input_callback_timestamp_ns: Option<u64>,
    pub input_device_timestamp_ns: Option<u64>,
    pub output_callback_timestamp_ns: Option<u64>,
    pub output_device_timestamp_ns: Option<u64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueueOccupancySummary {
    pub steady_state_only: bool,
    pub capacity_frames: usize,
    pub observations: usize,
    pub mean_frames: f64,
    pub variance_frames_squared: f64,
    pub minimum_frames: f64,
    pub maximum_frames: f64,
    pub slope_frames_per_second: f64,
    pub slope_samples_per_second: f64,
    pub slope_equivalent_ppm: f64,
    pub projected_seconds_to_boundary: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClockDriftEstimate {
    pub input_observed_rate_hz: Option<f64>,
    pub output_observed_rate_hz: Option<f64>,
    pub relative_clock_difference_ppm: Option<f64>,
    pub estimated_clock_difference_samples: Option<f64>,
    pub callback_sample_difference: i64,
    pub output_queue: QueueOccupancySummary,
    pub correction_enabled: bool,
    pub correction_ratio_ppm: f64,
    pub correction_min_ppm: f64,
    pub correction_max_ppm: f64,
    pub correction_errors: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct LatencyBreakdown {
    pub input_callback_duration_average_ms: f64,
    pub input_callback_duration_max_ms: f64,
    pub output_callback_duration_average_ms: f64,
    pub output_callback_duration_max_ms: f64,
    pub processing_duration_average_ms: f64,
    pub processing_duration_max_ms: f64,
    pub software_pipeline_average_ms: f64,
    pub software_pipeline_max_ms: f64,
    pub input_engine_period_ms: Option<f64>,
    pub output_engine_period_ms: Option<f64>,
    pub startup_preroll_ms: f64,
    pub drift_resampler_delay_ms: f64,
    pub end_to_end_latency_measured: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DuplexRunOptions {
    pub duration: Duration,
    pub sample_interval: Duration,
    pub requested_buffer_frames: Option<u32>,
    pub mute_output: bool,
    pub pipeline: PipelineConfig,
}

impl DuplexRunOptions {
    pub fn for_duration(duration: Duration) -> Self {
        let mut pipeline = PipelineConfig::default();
        pipeline.drift_correction.enabled = true;
        Self {
            duration,
            sample_interval: DEFAULT_SAMPLE_INTERVAL,
            requested_buffer_frames: None,
            mute_output: false,
            pipeline,
        }
    }
}

/// Optional endpoint IDs for a long-lived local control session.
#[derive(Clone, Debug, Default)]
pub struct DeviceSelection {
    pub input_id: Option<String>,
    pub output_id: Option<String>,
}

const SESSION_RUNNING: u8 = 1;
const SESSION_FINISHED: u8 = 2;
const SESSION_FAILED: u8 = 3;

struct SessionState {
    stop: AtomicBool,
    state: AtomicU8,
    metrics: OnceLock<auralis_core::Metrics>,
    error: OnceLock<String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            stop: AtomicBool::new(false),
            state: AtomicU8::new(0),
            metrics: OnceLock::new(),
            error: OnceLock::new(),
        }
    }
}

/// State and cancellation access for a running duplex session.
#[derive(Clone)]
pub struct DuplexRunControl {
    state: Arc<SessionState>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DuplexRunState {
    Starting,
    Running,
    Finished,
    Failed,
}

impl DuplexRunControl {
    pub fn request_stop(&self) {
        self.state.stop.store(true, Ordering::Relaxed);
    }

    pub fn state(&self) -> DuplexRunState {
        match self.state.state.load(Ordering::Relaxed) {
            SESSION_RUNNING => DuplexRunState::Running,
            SESSION_FINISHED => DuplexRunState::Finished,
            SESSION_FAILED => DuplexRunState::Failed,
            _ => DuplexRunState::Starting,
        }
    }

    pub fn metrics_snapshot(&self) -> Option<auralis_core::MetricsSnapshot> {
        self.state
            .metrics
            .get()
            .map(auralis_core::Metrics::snapshot)
    }

    pub fn error(&self) -> Option<String> {
        self.state.error.get().cloned()
    }
}

/// Owns a long-lived duplex session and joins it on shutdown.
pub struct DuplexRunHandle {
    control: DuplexRunControl,
    join: Option<thread::JoinHandle<Result<DuplexRunReport, DynError>>>,
}

impl DuplexRunHandle {
    pub fn control(&self) -> DuplexRunControl {
        self.control.clone()
    }

    pub fn stop(mut self) -> Result<DuplexRunReport, DynError> {
        self.control.request_stop();
        let join = self
            .join
            .take()
            .ok_or_else(|| io::Error::other("duplex session already joined"))?;
        join.join()
            .map_err(|_| io::Error::other("duplex session panicked"))?
    }
}

impl Drop for DuplexRunHandle {
    fn drop(&mut self) {
        self.control.request_stop();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DuplexRunReport {
    pub schema_version: u32,
    pub backend: String,
    pub input_device_id: String,
    pub input_device: String,
    pub output_device_id: String,
    pub output_device: String,
    pub output_muted: bool,
    pub requested_input_format: StreamFormatReport,
    pub requested_output_format: StreamFormatReport,
    pub actual_input_format: StreamFormatReport,
    pub actual_output_format: StreamFormatReport,
    pub input_wasapi_engine_period: Option<WasapiEnginePeriod>,
    pub input_wasapi_probe_error: Option<String>,
    pub output_wasapi_engine_period: Option<WasapiEnginePeriod>,
    pub output_wasapi_probe_error: Option<String>,
    pub requested_iaudioclient3_period_frames: Option<u32>,
    pub input_stream_clock: StreamClockReport,
    pub output_stream_clock: StreamClockReport,
    pub requested_duration_seconds: f64,
    pub measured_duration_seconds: f64,
    pub processor: &'static str,
    pub processor_algorithmic_latency_samples: usize,
    pub drift_resampler_algorithmic_latency_samples: usize,
    pub processing_path_algorithmic_latency_samples: usize,
    pub worker_scheduling: WorkerSchedulingReport,
    pub pipeline_config: PipelineConfig,
    pub queue_capacity_frames: usize,
    pub queue_time_series: Vec<QueueTimeSeriesPoint>,
    pub clock_drift: ClockDriftEstimate,
    pub latency: LatencyBreakdown,
    pub metrics: MetricsSnapshot,
}

pub fn available_hosts() -> Vec<String> {
    cpal::available_hosts()
        .iter()
        .map(|host| host.name().to_owned())
        .collect()
}

pub fn enumerate_devices() -> Result<Vec<DeviceDescriptor>, DynError> {
    let host = cpal::default_host();
    let host_name = host.id().name().to_owned();
    let default_input_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let default_output_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let mut result = Vec::new();

    for device in host.input_devices()? {
        let id = device.id()?.to_string();
        let supported_formats = supported_formats(&device, true).unwrap_or_default();
        let (wasapi_engine_period, wasapi_probe_error) = period_probe(&device);
        result.push(DeviceDescriptor {
            host: host_name.clone(),
            is_default: default_input_id.as_deref() == Some(id.as_str()),
            name: device.to_string(),
            id,
            direction: "input",
            supported_formats,
            wasapi_engine_period,
            wasapi_probe_error,
        });
    }

    for device in host.output_devices()? {
        let id = device.id()?.to_string();
        let supported_formats = supported_formats(&device, false).unwrap_or_default();
        let (wasapi_engine_period, wasapi_probe_error) = period_probe(&device);
        result.push(DeviceDescriptor {
            host: host_name.clone(),
            is_default: default_output_id.as_deref() == Some(id.as_str()),
            name: device.to_string(),
            id,
            direction: "output",
            supported_formats,
            wasapi_engine_period,
            wasapi_probe_error,
        });
    }
    Ok(result)
}

pub fn run_default_passthrough(duration: Duration) -> Result<DuplexRunReport, DynError> {
    run_default_characterization(DuplexRunOptions::for_duration(duration))
}

pub fn run_default_characterization(
    options: DuplexRunOptions,
) -> Result<DuplexRunReport, DynError> {
    run_default_characterization_with_processor(options, Passthrough)
}

pub fn run_default_characterization_with_processor<P: FrameProcessor>(
    options: DuplexRunOptions,
    processor: P,
) -> Result<DuplexRunReport, DynError> {
    if options.duration.is_zero() {
        return Err("duration must be greater than zero".into());
    }
    if options.sample_interval.is_zero() {
        return Err("sample interval must be greater than zero".into());
    }

    let host = cpal::default_host();
    let input_device = default_device(&host, true)?;
    let output_device = default_device(&host, false)?;
    run_characterization(&host, input_device, output_device, options, processor, None)
}

/// Start a cancellable duplex run for the local control UI.
pub fn start_characterization_with_processor<P: FrameProcessor>(
    options: DuplexRunOptions,
    processor: P,
    selection: DeviceSelection,
) -> Result<DuplexRunHandle, DynError> {
    if options.duration.is_zero() {
        return Err("duration must be greater than zero".into());
    }
    if options.sample_interval.is_zero() {
        return Err("sample interval must be greater than zero".into());
    }

    let state = Arc::new(SessionState::default());
    let thread_state = Arc::clone(&state);
    let join = thread::Builder::new()
        .name("auralis-duplex-session".to_owned())
        .spawn(move || {
            let result = (|| {
                let host = cpal::default_host();
                let input_device = selected_device(&host, true, selection.input_id.as_deref())?;
                let output_device = selected_device(&host, false, selection.output_id.as_deref())?;
                run_characterization(
                    &host,
                    input_device,
                    output_device,
                    options,
                    processor,
                    Some(Arc::clone(&thread_state)),
                )
            })();
            match &result {
                Ok(_) => {
                    thread_state
                        .state
                        .store(SESSION_FINISHED, Ordering::Relaxed);
                }
                Err(error) => {
                    let _ = thread_state.error.set(error.to_string());
                    thread_state.state.store(SESSION_FAILED, Ordering::Relaxed);
                }
            }
            result
        })?;

    Ok(DuplexRunHandle {
        control: DuplexRunControl { state },
        join: Some(join),
    })
}

fn default_device(host: &Host, input: bool) -> Result<Device, DynError> {
    if input {
        host.default_input_device().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no default input device").into()
        })
    } else {
        host.default_output_device().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no default output device").into()
        })
    }
}

fn selected_device(
    host: &Host,
    input: bool,
    requested_id: Option<&str>,
) -> Result<Device, DynError> {
    let Some(requested_id) = requested_id else {
        return default_device(host, input);
    };
    if input {
        for device in host.input_devices()? {
            if device.id()?.to_string() == requested_id {
                return Ok(device);
            }
        }
    } else {
        for device in host.output_devices()? {
            if device.id()?.to_string() == requested_id {
                return Ok(device);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "requested {} device was not found: {requested_id}",
            if input { "input" } else { "output" }
        ),
    )
    .into())
}

fn run_characterization<P: FrameProcessor>(
    host: &Host,
    input_device: Device,
    output_device: Device,
    options: DuplexRunOptions,
    processor: P,
    session: Option<Arc<SessionState>>,
) -> Result<DuplexRunReport, DynError> {
    let input_name = input_device.to_string();
    let input_id = input_device.id()?.to_string();
    let output_name = output_device.to_string();
    let output_id = output_device.id()?.to_string();
    let input_config = choose_input_config(&input_device, options.requested_buffer_frames)?;
    let output_config = choose_output_config(&output_device, options.requested_buffer_frames)?;
    let input_channels = input_config.channels;
    let output_channels = output_config.channels;
    let requested_input_format = format_report(&input_config, options.requested_buffer_frames);
    let requested_output_format = format_report(&output_config, options.requested_buffer_frames);

    let (processor, worker_scheduling_state) = platform_processor(processor);
    let parts = start_pipeline(options.pipeline, processor)?;
    let auralis_core::PipelineParts {
        mut capture,
        mut render,
        mut worker,
        metrics,
    } = parts;
    if let Some(session) = &session {
        let _ = session.metrics.set(metrics.clone());
    }

    let input_error_metrics = metrics.clone();
    let input_stream = input_device.build_input_stream::<f32, _, _>(
        input_config,
        move |data, info| {
            capture.process_callback_with_timing(
                data,
                usize::from(input_channels),
                input_callback_timing(info),
            );
        },
        move |error| input_error_metrics.stream_error(error.kind() == ErrorKind::Xrun),
        Some(STREAM_BUILD_TIMEOUT),
    )?;
    let output_error_metrics = metrics.clone();
    let output_stream = output_device.build_output_stream::<f32, _, _>(
        output_config,
        move |data, info| {
            render.process_callback_with_timing(
                data,
                usize::from(output_channels),
                output_callback_timing(info),
            );
            if options.mute_output {
                data.fill(0.0);
            }
        },
        move |error| output_error_metrics.stream_error(error.kind() == ErrorKind::Xrun),
        Some(STREAM_BUILD_TIMEOUT),
    )?;

    let actual_input_buffer_frames = input_stream.buffer_size().ok();
    let actual_output_buffer_frames = output_stream.buffer_size().ok();
    let actual_input_format = StreamFormatReport {
        channels: input_channels,
        sample_rate_hz: SAMPLE_RATE_HZ,
        sample_format: "f32",
        buffer_frames: actual_input_buffer_frames,
    };
    let actual_output_format = StreamFormatReport {
        channels: output_channels,
        sample_rate_hz: SAMPLE_RATE_HZ,
        sample_format: "f32",
        buffer_frames: actual_output_buffer_frames,
    };
    let processor = worker.processor_name();
    let processor_algorithmic_latency_samples = worker.processor_algorithmic_latency_samples();
    let drift_resampler_algorithmic_latency_samples =
        worker.drift_resampler_algorithmic_latency_samples();
    let processing_path_algorithmic_latency_samples =
        worker.processing_path_algorithmic_latency_samples();
    let input_clock_start = instant_ns(input_stream.now());
    let output_clock_start = instant_ns(output_stream.now());

    input_stream.play()?;
    output_stream.play()?;
    if let Some(session) = &session {
        session.state.store(SESSION_RUNNING, Ordering::Relaxed);
    }
    let (input_wasapi_engine_period, input_wasapi_probe_error) = period_probe(&input_device);
    let (output_wasapi_engine_period, output_wasapi_probe_error) = period_probe(&output_device);

    let started = Instant::now();
    let mut queue_time_series = Vec::new();
    queue_time_series.push(queue_point(started.elapsed(), &metrics.snapshot()));
    loop {
        if session
            .as_ref()
            .is_some_and(|session| session.stop.load(Ordering::Relaxed))
        {
            break;
        }
        let elapsed = started.elapsed();
        if elapsed >= options.duration {
            break;
        }
        thread::sleep(options.sample_interval.min(options.duration - elapsed));
        // ponytail: live history is capped at 256 points; raise only for longer UI trends.
        if session.is_some() && queue_time_series.len() >= 256 {
            queue_time_series.remove(0);
        }
        queue_time_series.push(queue_point(started.elapsed(), &metrics.snapshot()));
    }
    let measured_duration = started.elapsed();
    let input_clock_end = instant_ns(input_stream.now());
    let output_clock_end = instant_ns(output_stream.now());

    drop(output_stream);
    drop(input_stream);
    worker
        .stop()
        .map_err(|_| io::Error::other("processing worker panicked"))?;
    let worker_scheduling = worker_scheduling_state.report();
    let snapshot = metrics.snapshot();
    let clock_drift = analyze_clock_drift(
        &queue_time_series,
        &snapshot,
        measured_duration,
        options.pipeline.queue_capacity_frames,
    );
    let latency = latency_breakdown(
        &snapshot,
        input_wasapi_engine_period.as_ref(),
        output_wasapi_engine_period.as_ref(),
    );

    Ok(DuplexRunReport {
        schema_version: 6,
        backend: host.id().name().to_owned(),
        input_device_id: input_id,
        input_device: input_name,
        output_device_id: output_id,
        output_device: output_name,
        output_muted: options.mute_output,
        requested_input_format,
        requested_output_format,
        actual_input_format,
        actual_output_format,
        input_wasapi_engine_period,
        input_wasapi_probe_error,
        output_wasapi_engine_period,
        output_wasapi_probe_error,
        requested_iaudioclient3_period_frames: None,
        input_stream_clock: StreamClockReport {
            start_ns: input_clock_start,
            end_ns: input_clock_end,
        },
        output_stream_clock: StreamClockReport {
            start_ns: output_clock_start,
            end_ns: output_clock_end,
        },
        requested_duration_seconds: options.duration.as_secs_f64(),
        measured_duration_seconds: measured_duration.as_secs_f64(),
        processor,
        processor_algorithmic_latency_samples,
        drift_resampler_algorithmic_latency_samples,
        processing_path_algorithmic_latency_samples,
        worker_scheduling,
        pipeline_config: options.pipeline,
        queue_capacity_frames: options.pipeline.queue_capacity_frames,
        queue_time_series,
        clock_drift,
        latency,
        metrics: snapshot,
    })
}

fn supported_formats(
    device: &Device,
    input: bool,
) -> Result<Vec<SupportedFormatDescriptor>, DynError> {
    let ranges: Vec<_> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };
    Ok(ranges.iter().map(supported_format_descriptor).collect())
}

fn supported_format_descriptor(range: &SupportedStreamConfigRange) -> SupportedFormatDescriptor {
    let (minimum_buffer_frames, maximum_buffer_frames) = match range.buffer_size() {
        SupportedBufferSize::Range { min, max } => (Some(*min), Some(*max)),
        SupportedBufferSize::Unknown => (None, None),
    };
    SupportedFormatDescriptor {
        channels: range.channels(),
        sample_format: format!("{:?}", range.sample_format()),
        minimum_sample_rate_hz: range.min_sample_rate(),
        maximum_sample_rate_hz: range.max_sample_rate(),
        minimum_buffer_frames,
        maximum_buffer_frames,
    }
}

fn choose_input_config(
    device: &Device,
    requested_buffer_frames: Option<u32>,
) -> Result<StreamConfig, DynError> {
    choose_config(
        device.supported_input_configs()?,
        1,
        "input",
        requested_buffer_frames,
    )
}

fn choose_output_config(
    device: &Device,
    requested_buffer_frames: Option<u32>,
) -> Result<StreamConfig, DynError> {
    choose_config(
        device.supported_output_configs()?,
        2,
        "output",
        requested_buffer_frames,
    )
}

fn choose_config<I>(
    configs: I,
    preferred_channels: u16,
    direction: &'static str,
    requested_buffer_frames: Option<u32>,
) -> Result<StreamConfig, DynError>
where
    I: Iterator<Item = SupportedStreamConfigRange>,
{
    let selected = configs
        .filter(|range| {
            range.sample_format() == SampleFormat::F32
                && range.min_sample_rate() <= SAMPLE_RATE_HZ
                && range.max_sample_rate() >= SAMPLE_RATE_HZ
        })
        .min_by_key(|range| range.channels().abs_diff(preferred_channels))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("{direction} device has no 48 kHz f32 stream configuration"),
            )
        })?;

    if let (Some(requested), SupportedBufferSize::Range { min, max }) =
        (requested_buffer_frames, selected.buffer_size())
        && !(min..=max).contains(&&requested)
    {
        return Err(format!(
            "{direction} buffer request {requested} is outside supported range {min}..={max}"
        )
        .into());
    }

    Ok(StreamConfig {
        channels: selected.channels(),
        sample_rate: SAMPLE_RATE_HZ,
        buffer_size: requested_buffer_frames.map_or(BufferSize::Default, BufferSize::Fixed),
    })
}

fn format_report(
    config: &StreamConfig,
    requested_buffer_frames: Option<u32>,
) -> StreamFormatReport {
    StreamFormatReport {
        channels: config.channels,
        sample_rate_hz: config.sample_rate,
        sample_format: "f32",
        buffer_frames: requested_buffer_frames,
    }
}

fn period_probe(device: &Device) -> (Option<WasapiEnginePeriod>, Option<String>) {
    match query_engine_period(device) {
        Ok(period) => (period, None),
        Err(error) => (None, Some(error)),
    }
}

fn input_callback_timing(info: &InputCallbackInfo) -> CallbackTiming {
    let timestamp = info.timestamp();
    CallbackTiming {
        callback_ns: Some(instant_ns(timestamp.callback)),
        device_ns: Some(instant_ns(timestamp.capture)),
    }
}

fn output_callback_timing(info: &OutputCallbackInfo) -> CallbackTiming {
    let timestamp = info.timestamp();
    CallbackTiming {
        callback_ns: Some(instant_ns(timestamp.callback)),
        device_ns: Some(instant_ns(timestamp.playback)),
    }
}

fn instant_ns(instant: StreamInstant) -> u64 {
    u64::try_from(instant.as_nanos()).unwrap_or(u64::MAX)
}

fn queue_point(elapsed: Duration, metrics: &MetricsSnapshot) -> QueueTimeSeriesPoint {
    QueueTimeSeriesPoint {
        elapsed_seconds: elapsed.as_secs_f64(),
        input_queue_depth_frames: metrics.input_queue_depth_frames,
        output_queue_depth_frames: metrics.output_queue_depth_frames,
        output_buffered_samples: metrics.output_buffered_samples,
        input_callback_sample_frames: metrics.input_callback_sample_frames,
        output_callback_sample_frames: metrics.output_callback_sample_frames,
        input_overrun_frames: metrics.input_overrun_frames,
        output_overrun_frames: metrics.output_overrun_frames,
        output_underrun_callbacks: metrics.output_underrun_callbacks,
        stream_xruns: metrics.stream_xruns,
        startup_preroll_completed: metrics.startup_preroll_completed,
        drift_correction_ratio_ppm: metrics.drift_correction_ratio_ppm,
        input_callback_timestamp_ns: metrics.input_callback_timestamp_latest_ns,
        input_device_timestamp_ns: metrics.input_device_timestamp_latest_ns,
        output_callback_timestamp_ns: metrics.output_callback_timestamp_latest_ns,
        output_device_timestamp_ns: metrics.output_device_timestamp_latest_ns,
    }
}

fn analyze_clock_drift(
    series: &[QueueTimeSeriesPoint],
    metrics: &MetricsSnapshot,
    duration: Duration,
    queue_capacity_frames: usize,
) -> ClockDriftEstimate {
    let input_rate = observed_rate(
        metrics.input_callback_sample_frames,
        metrics.input_first_callback_frames,
        metrics.input_device_timestamp_first_ns,
        metrics.input_device_timestamp_latest_ns,
    );
    let output_rate = observed_rate(
        metrics.output_callback_sample_frames,
        metrics.output_first_callback_frames,
        metrics.output_callback_timestamp_first_ns,
        metrics.output_callback_timestamp_latest_ns,
    );
    let relative_clock_difference_ppm = input_rate
        .zip(output_rate)
        .and_then(|(input, output)| (output > 0.0).then_some((input / output - 1.0) * 1_000_000.0));
    let estimated_clock_difference_samples = input_rate
        .zip(output_rate)
        .map(|(input, output)| (input - output) * duration.as_secs_f64());

    ClockDriftEstimate {
        input_observed_rate_hz: input_rate,
        output_observed_rate_hz: output_rate,
        relative_clock_difference_ppm,
        estimated_clock_difference_samples,
        callback_sample_difference: signed_difference(
            metrics.input_callback_sample_frames,
            metrics.output_callback_sample_frames,
        ),
        output_queue: summarize_queue(
            series,
            queue_capacity_frames
                .saturating_add(1)
                .saturating_add(usize::from(metrics.drift_correction_enabled) * 2),
        ),
        correction_enabled: metrics.drift_correction_enabled,
        correction_ratio_ppm: metrics.drift_correction_ratio_ppm,
        correction_min_ppm: metrics.drift_correction_min_ppm,
        correction_max_ppm: metrics.drift_correction_max_ppm,
        correction_errors: metrics.drift_correction_errors,
    }
}

fn observed_rate(
    total_frames: u64,
    first_callback_frames: u64,
    first_timestamp_ns: Option<u64>,
    latest_timestamp_ns: Option<u64>,
) -> Option<f64> {
    let span_ns = latest_timestamp_ns?.checked_sub(first_timestamp_ns?)?;
    if span_ns == 0 || total_frames < first_callback_frames {
        return None;
    }
    Some((total_frames - first_callback_frames) as f64 * 1_000_000_000.0 / span_ns as f64)
}

fn signed_difference(left: u64, right: u64) -> i64 {
    let difference = i128::from(left) - i128::from(right);
    i64::try_from(difference).unwrap_or(if difference.is_negative() {
        i64::MIN
    } else {
        i64::MAX
    })
}

fn summarize_queue(
    series: &[QueueTimeSeriesPoint],
    queue_capacity_frames: usize,
) -> QueueOccupancySummary {
    let series = series
        .iter()
        .position(|sample| sample.startup_preroll_completed)
        .map_or(&[][..], |start| &series[start..]);
    if series.is_empty() {
        return QueueOccupancySummary {
            steady_state_only: true,
            capacity_frames: queue_capacity_frames,
            observations: 0,
            mean_frames: 0.0,
            variance_frames_squared: 0.0,
            minimum_frames: 0.0,
            maximum_frames: 0.0,
            slope_frames_per_second: 0.0,
            slope_samples_per_second: 0.0,
            slope_equivalent_ppm: 0.0,
            projected_seconds_to_boundary: None,
        };
    }

    let count = series.len() as f64;
    let mean_x = series
        .iter()
        .map(|sample| sample.elapsed_seconds)
        .sum::<f64>()
        / count;
    let mean_y = series.iter().map(output_fill_frames).sum::<f64>() / count;
    let covariance = series
        .iter()
        .map(|sample| (sample.elapsed_seconds - mean_x) * (output_fill_frames(sample) - mean_y))
        .sum::<f64>();
    let time_variance = series
        .iter()
        .map(|sample| (sample.elapsed_seconds - mean_x).powi(2))
        .sum::<f64>();
    let slope = if time_variance > f64::EPSILON {
        covariance / time_variance
    } else {
        0.0
    };
    let variance = series
        .iter()
        .map(|sample| (output_fill_frames(sample) - mean_y).powi(2))
        .sum::<f64>()
        / count;
    let minimum = series
        .iter()
        .map(output_fill_frames)
        .fold(f64::INFINITY, f64::min);
    let maximum = series
        .iter()
        .map(output_fill_frames)
        .fold(f64::NEG_INFINITY, f64::max);
    let last = series.last().map_or(0.0, output_fill_frames);
    let projected_seconds_to_boundary = if slope > f64::EPSILON {
        Some((queue_capacity_frames as f64 - last).max(0.0) / slope)
    } else if slope < -f64::EPSILON {
        Some(last / -slope)
    } else {
        None
    };
    let slope_samples_per_second = slope * FRAME_SAMPLES as f64;

    QueueOccupancySummary {
        steady_state_only: true,
        capacity_frames: queue_capacity_frames,
        observations: series.len(),
        mean_frames: mean_y,
        variance_frames_squared: variance,
        minimum_frames: minimum,
        maximum_frames: maximum,
        slope_frames_per_second: slope,
        slope_samples_per_second,
        slope_equivalent_ppm: slope_samples_per_second / SAMPLE_RATE_HZ as f64 * 1_000_000.0,
        projected_seconds_to_boundary,
    }
}

fn output_fill_frames(sample: &QueueTimeSeriesPoint) -> f64 {
    sample.output_buffered_samples as f64 / FRAME_SAMPLES as f64
}

fn latency_breakdown(
    metrics: &MetricsSnapshot,
    input_period: Option<&WasapiEnginePeriod>,
    output_period: Option<&WasapiEnginePeriod>,
) -> LatencyBreakdown {
    LatencyBreakdown {
        input_callback_duration_average_ms: metrics.input_callback_average_us / 1_000.0,
        input_callback_duration_max_ms: metrics.input_callback_max_us / 1_000.0,
        output_callback_duration_average_ms: metrics.output_callback_average_us / 1_000.0,
        output_callback_duration_max_ms: metrics.output_callback_max_us / 1_000.0,
        processing_duration_average_ms: metrics.processing_average_us / 1_000.0,
        processing_duration_max_ms: metrics.processing_max_us / 1_000.0,
        software_pipeline_average_ms: metrics.software_latency_average_ms,
        software_pipeline_max_ms: metrics.software_latency_max_ms,
        input_engine_period_ms: input_period.map(period_ms),
        output_engine_period_ms: output_period.map(period_ms),
        startup_preroll_ms: metrics.startup_preroll_target_frames as f64 * FRAME_SAMPLES as f64
            / SAMPLE_RATE_HZ as f64
            * 1_000.0,
        drift_resampler_delay_ms: metrics.drift_resampler_delay_samples as f64
            / SAMPLE_RATE_HZ as f64
            * 1_000.0,
        end_to_end_latency_measured: false,
    }
}

fn period_ms(period: &WasapiEnginePeriod) -> f64 {
    period.current_frames as f64 / period.format.sample_rate_hz as f64 * 1_000.0
}

#[cfg(test)]
mod tests {
    use super::{
        QueueTimeSeriesPoint, choose_config, observed_rate, signed_difference, summarize_queue,
    };
    use auralis_core::{FRAME_SAMPLES, SAMPLE_RATE_HZ};
    use cpal::{BufferSize, SampleFormat, SupportedBufferSize, SupportedStreamConfigRange};

    #[test]
    fn chooses_48_khz_float_without_assuming_processing_period() {
        let ranges = vec![
            SupportedStreamConfigRange::new(
                8,
                44_100,
                96_000,
                SupportedBufferSize::Range { min: 64, max: 512 },
                SampleFormat::I16,
            ),
            SupportedStreamConfigRange::new(
                2,
                44_100,
                96_000,
                SupportedBufferSize::Range { min: 64, max: 512 },
                SampleFormat::F32,
            ),
        ];
        let config = choose_config(ranges.into_iter(), 2, "test", None).expect("valid config");
        assert_eq!(config.channels, 2);
        assert_eq!(config.sample_rate, SAMPLE_RATE_HZ);
        assert_eq!(config.buffer_size, BufferSize::Default);
    }

    #[test]
    fn rejects_out_of_range_explicit_buffer_request() {
        let ranges = vec![SupportedStreamConfigRange::new(
            2,
            48_000,
            48_000,
            SupportedBufferSize::Range { min: 64, max: 256 },
            SampleFormat::F32,
        )];
        assert!(choose_config(ranges.into_iter(), 2, "test", Some(480)).is_err());
    }

    #[test]
    fn queue_summary_reports_positive_and_negative_slopes() {
        let positive = summarize_queue(&queue_series(&[0, 1, 2, 3]), 4);
        assert!((positive.slope_frames_per_second - 1.0).abs() < 1e-12);
        assert_eq!(positive.minimum_frames, 0.0);
        assert_eq!(positive.maximum_frames, 3.0);

        let negative = summarize_queue(&queue_series(&[3, 2, 1, 0]), 4);
        assert!((negative.slope_frames_per_second + 1.0).abs() < 1e-12);
    }

    #[test]
    fn callback_clock_rate_excludes_first_buffer() {
        let rate = observed_rate(48_480, 480, Some(1_000_000_000), Some(2_000_000_000));
        assert_eq!(rate, Some(48_000.0));
    }

    #[test]
    fn signed_sample_difference_saturates() {
        assert_eq!(signed_difference(100, 80), 20);
        assert_eq!(signed_difference(80, 100), -20);
    }

    fn queue_series(depths: &[usize]) -> Vec<QueueTimeSeriesPoint> {
        depths
            .iter()
            .enumerate()
            .map(|(index, depth)| QueueTimeSeriesPoint {
                elapsed_seconds: index as f64,
                input_queue_depth_frames: 0,
                output_queue_depth_frames: *depth,
                output_buffered_samples: *depth * FRAME_SAMPLES,
                input_callback_sample_frames: 0,
                output_callback_sample_frames: 0,
                input_overrun_frames: 0,
                output_overrun_frames: 0,
                output_underrun_callbacks: 0,
                stream_xruns: 0,
                startup_preroll_completed: true,
                drift_correction_ratio_ppm: 0.0,
                input_callback_timestamp_ns: None,
                input_device_timestamp_ns: None,
                output_callback_timestamp_ns: None,
                output_device_timestamp_ns: None,
            })
            .collect()
    }
}
