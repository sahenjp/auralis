//! Auralis diagnostic command-line application.

#![forbid(unsafe_code)]

use std::error::Error;
use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use auralis_audio_io::{
    DuplexRunOptions, available_hosts, enumerate_devices, run_default_characterization,
    run_default_characterization_with_processor,
};
use auralis_core::{
    FRAME_DURATION_NS, FRAME_SAMPLES, FrameProcessor, MetricsSnapshot, Passthrough, PipelineConfig,
    SAMPLE_RATE_HZ, start_pipeline,
};
use auralis_denoisers::{
    DEEPFILTER_CANDIDATE_ID, DEEPFILTER_MODEL_SHA256, DEEPFILTER_SOURCE_REVISION,
    DeepFilterFrameProcessor, EnhancementProfile, EnhancementProfileMetadata,
    InferenceTimingSnapshot, ONNX_RUNTIME_VERSION, ORT_CRATE_VERSION, RNNOISE_CANDIDATE_ID,
    RnnoiseFrameProcessor, UL_UNAS_CANDIDATE_ID, UL_UNAS_MODEL_SHA256, UlUnasFrameProcessor,
    onnx_runtime_build_info,
};
use auralis_diagnostics::{ProcessResourceMonitor, ProcessResourceSnapshot};
use serde::Serialize;
use sha2::{Digest, Sha256};

mod gui;

type DynError = Box<dyn Error + Send + Sync>;

#[derive(Debug, Serialize)]
struct SimulationReport {
    backend: &'static str,
    duration_seconds: f64,
    sample_rate_hz: u32,
    frame_samples: usize,
    processor: &'static str,
    generated_frames: u64,
    output_energy: f64,
    non_silent_output_samples: u64,
    metrics: MetricsSnapshot,
}

#[derive(Debug, Serialize)]
struct MeasuredRun<T> {
    run: T,
    process_resources: ProcessResourceSnapshot,
    measurement_started_unix_seconds: u64,
    benchmark_code_revision: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct DenoisedMeasuredRun<T> {
    run: T,
    process_resources: ProcessResourceSnapshot,
    denoiser_runtime: DenoiserRuntimeReport,
    software_latency: SoftwareLatencyAccounting,
    measurement_started_unix_seconds: u64,
    benchmark_code_revision: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct SoftwareLatencyAccounting {
    measured_transport_software_pipeline_average_ms: f64,
    measured_transport_software_pipeline_max_ms: f64,
    /// Structural candidate-path delay is reported separately from a paired
    /// transport delta; it is not an incremental measurement against the
    /// passthrough reference.
    structural_path_addition_samples: usize,
    structural_path_addition_ms: f64,
    incremental_software_pipeline_latency_samples: Option<usize>,
    incremental_software_pipeline_latency_ms: Option<f64>,
    incremental_software_pipeline_latency_definition: &'static str,
    transport_delta_vs_milestone_2_reference_average_ms: Option<f64>,
    transport_delta_vs_milestone_2_reference_max_ms: Option<f64>,
    total_software_pipeline_average_ms: f64,
    total_software_pipeline_max_ms: f64,
    physical_e2e_latency_measured: bool,
}

#[derive(Debug, Serialize)]
struct DenoiserRuntimeReport {
    schema_version: u32,
    host: HostRuntimeMetadata,
    enhancement_profile: Option<EnhancementProfileMetadata>,
    candidate_id: &'static str,
    model_sha256: String,
    model_size_bytes: u64,
    /// Weight-only memory is not isolated by the process-level RSS probe.
    model_memory_bytes: Option<u64>,
    /// Same-process load-envelope delta for model, runtime, and adapter state.
    model_load_envelope_bytes: Option<u64>,
    model_memory_measurement: Option<ModelMemoryMeasurement>,
    runtime_artifact_sha256: String,
    runtime_artifact_size_bytes: u64,
    source_revision: &'static str,
    runtime: &'static str,
    runtime_crate_version: &'static str,
    onnx_runtime_version: &'static str,
    runtime_build_info: &'static str,
    execution_provider: &'static str,
    requested_intra_op_threads: usize,
    requested_inter_op_threads: usize,
    native_sample_rate_hz: u32,
    required_frame_size_samples: usize,
    hop_size_samples: usize,
    model_algorithmic_latency_samples: usize,
    model_algorithmic_latency_ms: f64,
    downsampling_algorithmic_latency_samples_at_48_khz: usize,
    downsampling_algorithmic_latency_ms: f64,
    upsampling_algorithmic_latency_samples_at_48_khz: usize,
    upsampling_algorithmic_latency_ms: f64,
    additional_resampling_and_framing_latency_samples_at_48_khz: usize,
    additional_resampling_and_framing_latency_ms: f64,
    processor_algorithmic_latency_samples_at_48_khz: usize,
    processor_algorithmic_latency_ms: f64,
    model_inference_wall_time: InferenceTimingSnapshot,
}

#[derive(Debug, Serialize)]
struct ModelMemoryMeasurement {
    method: &'static str,
    baseline_resident_memory_bytes: u64,
    post_load_resident_memory_bytes: u64,
    delta_resident_memory_bytes: u64,
    interpretation: &'static str,
}

#[derive(Debug, Serialize)]
struct HostRuntimeMetadata {
    target_os: &'static str,
    target_family: &'static str,
    target_architecture: &'static str,
    pointer_width: &'static str,
    cpu_features: Vec<&'static str>,
    affinity_changed: bool,
    os_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct RestartReport<T> {
    cycles: usize,
    seconds_per_cycle: u64,
    runs: Vec<T>,
    process_resources: ProcessResourceSnapshot,
}

struct CharacterizeArguments {
    options: DuplexRunOptions,
    output: Option<PathBuf>,
    denoiser: CharacterizeDenoiser,
    profile: Option<EnhancementProfile>,
}

struct SimulateArguments {
    seconds: u64,
    denoiser: CharacterizeDenoiser,
    profile: Option<EnhancementProfile>,
}

enum CharacterizeDenoiser {
    Passthrough,
    UlUnas { model_path: PathBuf },
    DeepFilter { model_path: PathBuf },
    Rnnoise { library_path: PathBuf },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("auralis-cli: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), DynError> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("gui") => {
            gui::run(arguments)?;
        }
        Some("devices") => {
            reject_extra_arguments(arguments)?;
            print_json(
                &serde_json::json!({
                    "available_hosts": available_hosts(),
                    "devices": enumerate_devices()?,
                }),
                None,
            )?;
        }
        Some("run" | "characterize") => {
            let parsed = parse_characterize_arguments(arguments)?;
            let measurement_started_unix_seconds = unix_timestamp_seconds();
            let monitor = ProcessResourceMonitor::start()?;
            let model_memory_baseline =
                ProcessResourceMonitor::current_resident_memory_bytes().ok();
            let profile = parsed.profile;
            match parsed.denoiser {
                CharacterizeDenoiser::Passthrough => {
                    let report = run_default_characterization(parsed.options)?;
                    let measured = MeasuredRun {
                        run: report,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&measured, parsed.output)?;
                }
                CharacterizeDenoiser::UlUnas { model_path } => {
                    let model_size_bytes = model_path.metadata()?.len();
                    let processor = UlUnasFrameProcessor::load(&model_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        ul_unas_runtime_report(&processor, model_size_bytes, profile);
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let report =
                        run_default_characterization_with_processor(parsed.options, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &report.metrics,
                        report.processing_path_algorithmic_latency_samples,
                    );
                    let measured = DenoisedMeasuredRun {
                        run: report,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&measured, parsed.output)?;
                }
                CharacterizeDenoiser::Rnnoise { library_path } => {
                    let processor = RnnoiseFrameProcessor::load(&library_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        rnnoise_runtime_report(&processor, &library_path, profile)?;
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let report =
                        run_default_characterization_with_processor(parsed.options, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &report.metrics,
                        report.processing_path_algorithmic_latency_samples,
                    );
                    let measured = DenoisedMeasuredRun {
                        run: report,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&measured, parsed.output)?;
                }
                CharacterizeDenoiser::DeepFilter { model_path } => {
                    let model_size_bytes = model_path.metadata()?.len();
                    let processor = DeepFilterFrameProcessor::load(&model_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        deepfilter_runtime_report(&processor, model_size_bytes, profile);
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let report =
                        run_default_characterization_with_processor(parsed.options, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &report.metrics,
                        report.processing_path_algorithmic_latency_samples,
                    );
                    let measured = DenoisedMeasuredRun {
                        run: report,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&measured, parsed.output)?;
                }
            }
        }
        Some("restart") => {
            let cycles = parse_usize(arguments.next().as_deref(), 4, 1, 100, "cycles")?;
            let seconds = parse_u64(arguments.next().as_deref(), 2, 1, 3_600, "seconds")?;
            reject_extra_arguments(arguments)?;
            let monitor = ProcessResourceMonitor::start()?;
            let mut runs = Vec::with_capacity(cycles);
            for _ in 0..cycles {
                runs.push(run_default_characterization(
                    DuplexRunOptions::for_duration(Duration::from_secs(seconds)),
                )?);
            }
            print_json(
                &RestartReport {
                    cycles,
                    seconds_per_cycle: seconds,
                    runs,
                    process_resources: monitor.finish().map_err(|_| "resource monitor panicked")?,
                },
                None,
            )?;
        }
        Some("simulate") => {
            let parsed = parse_simulate_arguments(arguments)?;
            let measurement_started_unix_seconds = unix_timestamp_seconds();
            let monitor = ProcessResourceMonitor::start()?;
            let model_memory_baseline =
                ProcessResourceMonitor::current_resident_memory_bytes().ok();
            let profile = parsed.profile;
            match parsed.denoiser {
                CharacterizeDenoiser::Passthrough => {
                    let report = MeasuredRun {
                        run: simulate(parsed.seconds)?,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&report, None)?;
                }
                CharacterizeDenoiser::UlUnas { model_path } => {
                    let model_size_bytes = model_path.metadata()?.len();
                    let processor = UlUnasFrameProcessor::load(model_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        ul_unas_runtime_report(&processor, model_size_bytes, profile);
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let run = simulate_with_processor(parsed.seconds, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &run.metrics,
                        denoiser_runtime.processor_algorithmic_latency_samples_at_48_khz,
                    );
                    let report = DenoisedMeasuredRun {
                        run,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&report, None)?;
                }
                CharacterizeDenoiser::Rnnoise { library_path } => {
                    let processor = RnnoiseFrameProcessor::load(&library_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        rnnoise_runtime_report(&processor, &library_path, profile)?;
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let run = simulate_with_processor(parsed.seconds, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &run.metrics,
                        denoiser_runtime.processor_algorithmic_latency_samples_at_48_khz,
                    );
                    let report = DenoisedMeasuredRun {
                        run,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&report, None)?;
                }
                CharacterizeDenoiser::DeepFilter { model_path } => {
                    let model_size_bytes = model_path.metadata()?.len();
                    let processor = DeepFilterFrameProcessor::load(model_path)?;
                    let timing = processor.timing_handle();
                    let mut denoiser_runtime =
                        deepfilter_runtime_report(&processor, model_size_bytes, profile);
                    attach_model_memory_measurement(&mut denoiser_runtime, model_memory_baseline);
                    let run = simulate_with_processor(parsed.seconds, processor)?;
                    denoiser_runtime.model_inference_wall_time = timing.snapshot();
                    let software_latency = software_latency_accounting(
                        &run.metrics,
                        denoiser_runtime.processor_algorithmic_latency_samples_at_48_khz,
                    );
                    let report = DenoisedMeasuredRun {
                        run,
                        process_resources: monitor
                            .finish()
                            .map_err(|_| "resource monitor panicked")?,
                        denoiser_runtime,
                        software_latency,
                        measurement_started_unix_seconds,
                        benchmark_code_revision: option_env!("AURALIS_GIT_REVISION"),
                    };
                    print_json(&report, None)?;
                }
            }
        }
        None | Some("--help" | "-h") => print_help(),
        Some(command) => return Err(format!("unknown command: {command}").into()),
    }
    Ok(())
}

fn parse_characterize_arguments(
    mut arguments: impl Iterator<Item = String>,
) -> Result<CharacterizeArguments, DynError> {
    let mut seconds = 60;
    let mut sample_ms = 1_000;
    let mut requested_buffer_frames = None;
    let mut queue_capacity_frames = PipelineConfig::default().queue_capacity_frames;
    let mut startup_pre_roll_frames = PipelineConfig::default().startup_pre_roll_frames;
    let mut target_fill_frames = PipelineConfig::default()
        .drift_correction
        .target_fill_frames;
    let mut drift_correction_enabled = true;
    let mut mute_output = false;
    let mut output = None;
    let mut denoiser = None;
    let mut profile = None;
    let mut model_path = None;
    let mut library_path = None;
    let mut positional_consumed = false;

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--output" => {
                output = Some(arguments.next().ok_or("--output requires a path")?.into());
            }
            "--denoiser" => {
                denoiser = Some(arguments.next().ok_or("--denoiser requires a value")?);
            }
            "--profile" => {
                profile = Some(arguments.next().ok_or("--profile requires a value")?);
            }
            "--model" => {
                model_path = Some(PathBuf::from(
                    arguments.next().ok_or("--model requires a path")?,
                ));
            }
            "--library" => {
                library_path = Some(PathBuf::from(
                    arguments.next().ok_or("--library requires a path")?,
                ));
            }
            "--sample-ms" => {
                sample_ms =
                    parse_u64(arguments.next().as_deref(), 1_000, 100, 60_000, "sample-ms")?;
            }
            "--buffer-frames" => {
                let value = arguments
                    .next()
                    .ok_or("--buffer-frames requires default or a frame count")?;
                requested_buffer_frames = if value == "default" {
                    None
                } else {
                    Some(value.parse::<u32>()?)
                };
            }
            "--pre-roll-frames" => {
                startup_pre_roll_frames =
                    parse_usize(arguments.next().as_deref(), 1, 0, 64, "pre-roll-frames")?;
            }
            "--queue-capacity-frames" => {
                queue_capacity_frames = parse_usize(
                    arguments.next().as_deref(),
                    PipelineConfig::default().queue_capacity_frames,
                    1,
                    64,
                    "queue-capacity-frames",
                )?;
            }
            "--target-fill-frames" => {
                target_fill_frames = arguments
                    .next()
                    .ok_or("--target-fill-frames requires a value")?
                    .parse()?;
            }
            "--no-drift-correction" => drift_correction_enabled = false,
            "--mute-output" => mute_output = true,
            value if !value.starts_with('-') && !positional_consumed => {
                seconds = parse_u64(Some(value), 60, 1, 86_400, "seconds")?;
                positional_consumed = true;
            }
            _ => return Err(format!("unexpected argument: {argument}").into()),
        }
    }

    if startup_pre_roll_frames > queue_capacity_frames {
        return Err("pre-roll-frames must not exceed queue-capacity-frames".into());
    }
    if !target_fill_frames.is_finite()
        || target_fill_frames <= 0.0
        || target_fill_frames > queue_capacity_frames as f64
    {
        return Err(
            "target-fill-frames must be finite, greater than zero, and no larger than queue-capacity-frames"
                .into(),
        );
    }

    let (denoiser, profile) = parse_denoiser_selection(
        profile.as_deref(),
        denoiser.as_deref(),
        model_path,
        library_path,
    )?;

    Ok(CharacterizeArguments {
        options: DuplexRunOptions {
            duration: Duration::from_secs(seconds),
            sample_interval: Duration::from_millis(sample_ms),
            requested_buffer_frames,
            mute_output,
            pipeline: PipelineConfig {
                queue_capacity_frames,
                startup_pre_roll_frames,
                drift_correction: auralis_core::DriftCorrectionConfig {
                    enabled: drift_correction_enabled,
                    target_fill_frames,
                    ..Default::default()
                },
            },
        },
        output,
        denoiser,
        profile,
    })
}

fn parse_simulate_arguments(
    mut arguments: impl Iterator<Item = String>,
) -> Result<SimulateArguments, DynError> {
    let seconds = parse_u64(arguments.next().as_deref(), 2, 1, 86_400, "seconds")?;
    let mut denoiser = None;
    let mut profile = None;
    let mut model_path = None;
    let mut library_path = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--denoiser" => {
                denoiser = Some(arguments.next().ok_or("--denoiser requires a value")?);
            }
            "--profile" => {
                profile = Some(arguments.next().ok_or("--profile requires a value")?);
            }
            "--model" => {
                model_path = Some(PathBuf::from(
                    arguments.next().ok_or("--model requires a path")?,
                ));
            }
            "--library" => {
                library_path = Some(PathBuf::from(
                    arguments.next().ok_or("--library requires a path")?,
                ));
            }
            _ => return Err(format!("unexpected argument: {argument}").into()),
        }
    }
    let (denoiser, profile) = parse_denoiser_selection(
        profile.as_deref(),
        denoiser.as_deref(),
        model_path,
        library_path,
    )?;
    Ok(SimulateArguments {
        seconds,
        denoiser,
        profile,
    })
}

fn parse_denoiser_selection(
    profile: Option<&str>,
    denoiser: Option<&str>,
    model_path: Option<PathBuf>,
    library_path: Option<PathBuf>,
) -> Result<(CharacterizeDenoiser, Option<EnhancementProfile>), DynError> {
    if profile.is_some() && denoiser.is_some() {
        return Err("--profile and --denoiser are mutually exclusive".into());
    }
    let profile = profile
        .map(|value| {
            EnhancementProfile::parse(value)
                .ok_or_else(|| format!("unsupported enhancement profile: {value}"))
        })
        .transpose()?;
    let denoiser = denoiser.or_else(|| profile.map(EnhancementProfile::cli_denoiser));
    let selected = match denoiser {
        None | Some("passthrough") => {
            if model_path.is_some() || library_path.is_some() {
                return Err("--model/--library requires a matching --denoiser".into());
            }
            CharacterizeDenoiser::Passthrough
        }
        Some("ul-unas") => {
            if library_path.is_some() {
                return Err("--library is only valid with --denoiser rnnoise".into());
            }
            CharacterizeDenoiser::UlUnas {
                model_path: model_path.ok_or("--denoiser ul-unas requires --model PATH")?,
            }
        }
        Some("deepfilter") => {
            if library_path.is_some() {
                return Err("--library is only valid with --denoiser rnnoise".into());
            }
            CharacterizeDenoiser::DeepFilter {
                model_path: model_path.ok_or("--denoiser deepfilter requires --model PATH")?,
            }
        }
        Some("rnnoise") => {
            if model_path.is_some() {
                return Err("--model is only valid with --denoiser ul-unas/deepfilter".into());
            }
            CharacterizeDenoiser::Rnnoise {
                library_path: library_path.ok_or("--denoiser rnnoise requires --library PATH")?,
            }
        }
        Some(value) => return Err(format!("unsupported denoiser: {value}").into()),
    };
    Ok((selected, profile))
}

fn parse_u64(
    value: Option<&str>,
    default: u64,
    minimum: u64,
    maximum: u64,
    label: &str,
) -> Result<u64, DynError> {
    let parsed = value.map_or(Ok(default), str::parse::<u64>)?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(format!("{label} must be between {minimum} and {maximum}").into());
    }
    Ok(parsed)
}

fn parse_usize(
    value: Option<&str>,
    default: usize,
    minimum: usize,
    maximum: usize,
    label: &str,
) -> Result<usize, DynError> {
    let parsed = value.map_or(Ok(default), str::parse::<usize>)?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(format!("{label} must be between {minimum} and {maximum}").into());
    }
    Ok(parsed)
}

fn reject_extra_arguments(mut arguments: impl Iterator<Item = String>) -> Result<(), DynError> {
    if let Some(extra) = arguments.next() {
        Err(format!("unexpected argument: {extra}").into())
    } else {
        Ok(())
    }
}

fn print_json<T: Serialize>(value: &T, output: Option<PathBuf>) -> Result<(), DynError> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(path) = output {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn print_help() {
    println!(
        r#"Auralis measured realtime transport

Usage:
  auralis-cli gui [--profile passthrough|low-latency|balanced|maximum-quality]
             [--model PATH] [--library PATH] [--bind ADDRESS] [--port N] [--no-open]
  auralis-cli devices
           auralis-cli characterize [seconds] [--sample-ms N] [--buffer-frames default|N]
               [--queue-capacity-frames N] [--target-fill-frames N]
               [--pre-roll-frames N] [--no-drift-correction] [--mute-output] [--output PATH]
               [--profile low-latency|balanced|maximum-quality]
               [--denoiser passthrough|ul-unas|deepfilter|rnnoise] [--model PATH] [--library PATH]
  auralis-cli run [same options as characterize]
  auralis-cli restart [cycles] [seconds-per-cycle]
  auralis-cli simulate [seconds] [--profile low-latency|balanced|maximum-quality]
               [--denoiser passthrough|ul-unas|deepfilter|rnnoise]
               [--model PATH] [--library PATH]"#
    );
}

fn simulate(seconds: u64) -> Result<SimulationReport, DynError> {
    simulate_with_processor(seconds, Passthrough)
}

fn simulate_with_processor<P: FrameProcessor>(
    seconds: u64,
    processor: P,
) -> Result<SimulationReport, DynError> {
    let total_frames = seconds.saturating_mul(100);
    let parts = start_pipeline(PipelineConfig::default(), processor)?;
    let auralis_core::PipelineParts {
        mut capture,
        mut render,
        mut worker,
        metrics,
    } = parts;
    let processor = worker.processor_name();
    let waveform = make_test_frame();

    let capture_thread = thread::Builder::new()
        .name("auralis-sim-capture".to_owned())
        .spawn(move || {
            let start = Instant::now();
            for frame_index in 0..total_frames {
                sleep_until(start, frame_index);
                capture.process_callback(&waveform, 1);
            }
        })?;
    wait_for_output_frame(&metrics, Duration::from_millis(250));
    let render_thread = thread::Builder::new()
        .name("auralis-sim-render".to_owned())
        .spawn({
            let render_metrics = metrics.clone();
            move || {
                let start = Instant::now();
                let mut output = [0.0_f32; FRAME_SAMPLES];
                let mut energy = 0.0_f64;
                let mut non_silent = 0_u64;
                for frame_index in 0..total_frames {
                    sleep_until(start, frame_index);
                    wait_for_processed_frame(
                        &render_metrics,
                        frame_index + 1,
                        Duration::from_secs(1),
                    );
                    render.process_callback(&mut output, 1);
                    for sample in output {
                        energy += f64::from(sample) * f64::from(sample);
                        non_silent += u64::from(sample != 0.0);
                    }
                }
                (energy, non_silent)
            }
        })?;

    capture_thread
        .join()
        .map_err(|_| "simulated capture thread panicked")?;
    let (output_energy, non_silent_output_samples) = render_thread
        .join()
        .map_err(|_| "simulated render thread panicked")?;
    worker
        .stop()
        .map_err(|_| "simulated processing worker panicked")?;

    Ok(SimulationReport {
        backend: "paced-simulation",
        duration_seconds: seconds as f64,
        sample_rate_hz: SAMPLE_RATE_HZ,
        frame_samples: FRAME_SAMPLES,
        processor,
        generated_frames: total_frames,
        output_energy,
        non_silent_output_samples,
        metrics: metrics.snapshot(),
    })
}

fn make_test_frame() -> [f32; FRAME_SAMPLES] {
    let mut frame = [0.0_f32; FRAME_SAMPLES];
    for (index, sample) in frame.iter_mut().enumerate() {
        let time = index as f32 / SAMPLE_RATE_HZ as f32;
        *sample = 0.2 * (TAU * 440.0 * time).sin() + 0.05 * (TAU * 1_700.0 * time).sin();
    }
    frame
}

fn sleep_until(start: Instant, frame_index: u64) {
    let offset_ns = frame_index.saturating_mul(FRAME_DURATION_NS);
    let deadline = start + Duration::from_nanos(offset_ns);
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        thread::sleep(remaining);
    }
}

fn wait_for_output_frame(metrics: &auralis_core::Metrics, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while metrics.snapshot().output_queue_depth_frames == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_micros(250));
    }
}

fn wait_for_processed_frame(metrics: &auralis_core::Metrics, expected: u64, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while metrics.snapshot().processed_frames < expected && Instant::now() < deadline {
        thread::sleep(Duration::from_micros(250));
    }
}

fn samples_to_ms(samples: usize, sample_rate_hz: u32) -> f64 {
    samples as f64 / sample_rate_hz as f64 * 1_000.0
}

fn unix_timestamp_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn software_latency_accounting(
    metrics: &MetricsSnapshot,
    structural_path_addition_samples: usize,
) -> SoftwareLatencyAccounting {
    let structural_path_addition_ms =
        samples_to_ms(structural_path_addition_samples, SAMPLE_RATE_HZ);
    SoftwareLatencyAccounting {
        measured_transport_software_pipeline_average_ms: metrics.software_latency_average_ms,
        measured_transport_software_pipeline_max_ms: metrics.software_latency_max_ms,
        structural_path_addition_samples,
        structural_path_addition_ms,
        incremental_software_pipeline_latency_samples: None,
        incremental_software_pipeline_latency_ms: None,
        incremental_software_pipeline_latency_definition: "paired baseline transport delta was not measured in this run; structural path addition is reported separately",
        transport_delta_vs_milestone_2_reference_average_ms: None,
        transport_delta_vs_milestone_2_reference_max_ms: None,
        total_software_pipeline_average_ms: metrics.software_latency_average_ms
            + structural_path_addition_ms,
        total_software_pipeline_max_ms: metrics.software_latency_max_ms
            + structural_path_addition_ms,
        physical_e2e_latency_measured: false,
    }
}

fn attach_model_memory_measurement(
    report: &mut DenoiserRuntimeReport,
    baseline_resident_memory_bytes: Option<u64>,
) {
    let Some(baseline_resident_memory_bytes) = baseline_resident_memory_bytes else {
        return;
    };
    let Ok(post_load_resident_memory_bytes) =
        ProcessResourceMonitor::current_resident_memory_bytes()
    else {
        return;
    };
    let measurement = ModelMemoryMeasurement {
        method: "same-process RSS delta before and after denoiser load",
        baseline_resident_memory_bytes,
        post_load_resident_memory_bytes,
        delta_resident_memory_bytes: post_load_resident_memory_bytes
            .saturating_sub(baseline_resident_memory_bytes),
        interpretation: "includes the model, inference runtime, and persistent adapter allocations; excludes stream processing after load; not a weight-only byte count",
    };
    report.model_load_envelope_bytes = Some(measurement.delta_resident_memory_bytes);
    report.model_memory_measurement = Some(measurement);
}

fn ul_unas_runtime_report(
    processor: &UlUnasFrameProcessor,
    model_size_bytes: u64,
    profile: Option<EnhancementProfile>,
) -> DenoiserRuntimeReport {
    let model_algorithmic_latency_samples =
        processor.model_algorithmic_latency_samples_at_native_rate();
    let downsampling_delay_samples_at_48_khz =
        processor.downsampler_delay_samples_at_native_rate() * 3;
    let upsampling_delay_samples_at_48_khz = processor.upsampler_delay_samples_at_auralis_rate();
    let processor_algorithmic_latency_samples = processor.algorithmic_latency_samples();
    let model_algorithmic_latency_samples_at_48_khz = model_algorithmic_latency_samples * 3;
    let additional_resampling_and_framing_latency_samples = processor_algorithmic_latency_samples
        .saturating_sub(model_algorithmic_latency_samples_at_48_khz);
    DenoiserRuntimeReport {
        schema_version: 2,
        host: host_runtime_metadata(),
        enhancement_profile: profile.map(EnhancementProfile::metadata),
        candidate_id: UL_UNAS_CANDIDATE_ID,
        model_sha256: UL_UNAS_MODEL_SHA256.to_owned(),
        model_size_bytes,
        model_memory_bytes: None,
        model_load_envelope_bytes: None,
        model_memory_measurement: None,
        runtime_artifact_sha256: UL_UNAS_MODEL_SHA256.to_owned(),
        runtime_artifact_size_bytes: model_size_bytes,
        source_revision: "00f7c700da43d38347f30a6ccebd86fcbc798e07",
        runtime: "ONNX Runtime CPU",
        runtime_crate_version: ORT_CRATE_VERSION,
        onnx_runtime_version: ONNX_RUNTIME_VERSION,
        runtime_build_info: onnx_runtime_build_info(),
        execution_provider: "CPUExecutionProvider",
        requested_intra_op_threads: 1,
        requested_inter_op_threads: 1,
        native_sample_rate_hz: 16_000,
        required_frame_size_samples: 512,
        hop_size_samples: 256,
        model_algorithmic_latency_samples,
        model_algorithmic_latency_ms: samples_to_ms(model_algorithmic_latency_samples, 16_000),
        downsampling_algorithmic_latency_samples_at_48_khz: downsampling_delay_samples_at_48_khz,
        downsampling_algorithmic_latency_ms: samples_to_ms(
            downsampling_delay_samples_at_48_khz,
            SAMPLE_RATE_HZ,
        ),
        upsampling_algorithmic_latency_samples_at_48_khz: upsampling_delay_samples_at_48_khz,
        upsampling_algorithmic_latency_ms: samples_to_ms(
            upsampling_delay_samples_at_48_khz,
            SAMPLE_RATE_HZ,
        ),
        additional_resampling_and_framing_latency_samples_at_48_khz:
            additional_resampling_and_framing_latency_samples,
        additional_resampling_and_framing_latency_ms: samples_to_ms(
            additional_resampling_and_framing_latency_samples,
            SAMPLE_RATE_HZ,
        ),
        processor_algorithmic_latency_samples_at_48_khz: processor_algorithmic_latency_samples,
        processor_algorithmic_latency_ms: samples_to_ms(
            processor_algorithmic_latency_samples,
            SAMPLE_RATE_HZ,
        ),
        model_inference_wall_time: processor.timing_handle().snapshot(),
    }
}

fn rnnoise_runtime_report(
    processor: &RnnoiseFrameProcessor,
    library_path: &std::path::Path,
    profile: Option<EnhancementProfile>,
) -> Result<DenoiserRuntimeReport, DynError> {
    let library_size_bytes = library_path.metadata()?.len();
    Ok(DenoiserRuntimeReport {
        schema_version: 2,
        host: host_runtime_metadata(),
        enhancement_profile: profile.map(EnhancementProfile::metadata),
        candidate_id: RNNOISE_CANDIDATE_ID,
        model_sha256: "0a8755f8e2d834eff6a54714ecc7d75f9932e845df35f8b59bc52a7cfe6e8b37".to_owned(),
        model_size_bytes: 58_603_099,
        model_memory_bytes: None,
        model_load_envelope_bytes: None,
        model_memory_measurement: None,
        runtime_artifact_sha256: sha256_file(library_path)?,
        runtime_artifact_size_bytes: library_size_bytes,
        source_revision: "70f1d256acd4b34a572f999a05c87bf00b67730d",
        runtime: "RNNoise native C dynamic library",
        runtime_crate_version: "local-pinned-source-build",
        onnx_runtime_version: "not-applicable",
        runtime_build_info: "external library; compiler flags recorded by build provenance",
        execution_provider: "native CPU scalar reference build",
        requested_intra_op_threads: 1,
        requested_inter_op_threads: 1,
        native_sample_rate_hz: SAMPLE_RATE_HZ,
        required_frame_size_samples: FRAME_SAMPLES,
        hop_size_samples: FRAME_SAMPLES,
        model_algorithmic_latency_samples: 960,
        model_algorithmic_latency_ms: 20.0,
        downsampling_algorithmic_latency_samples_at_48_khz: 0,
        downsampling_algorithmic_latency_ms: 0.0,
        upsampling_algorithmic_latency_samples_at_48_khz: 0,
        upsampling_algorithmic_latency_ms: 0.0,
        additional_resampling_and_framing_latency_samples_at_48_khz: 0,
        additional_resampling_and_framing_latency_ms: 0.0,
        processor_algorithmic_latency_samples_at_48_khz: processor.algorithmic_latency_samples(),
        processor_algorithmic_latency_ms: samples_to_ms(
            processor.algorithmic_latency_samples(),
            SAMPLE_RATE_HZ,
        ),
        model_inference_wall_time: processor.timing_handle().snapshot(),
    })
}

fn deepfilter_runtime_report(
    processor: &DeepFilterFrameProcessor,
    model_size_bytes: u64,
    profile: Option<EnhancementProfile>,
) -> DenoiserRuntimeReport {
    DenoiserRuntimeReport {
        schema_version: 2,
        host: host_runtime_metadata(),
        enhancement_profile: profile.map(EnhancementProfile::metadata),
        candidate_id: DEEPFILTER_CANDIDATE_ID,
        model_sha256: DEEPFILTER_MODEL_SHA256.to_owned(),
        model_size_bytes,
        model_memory_bytes: None,
        model_load_envelope_bytes: None,
        model_memory_measurement: None,
        runtime_artifact_sha256: DEEPFILTER_MODEL_SHA256.to_owned(),
        runtime_artifact_size_bytes: model_size_bytes,
        source_revision: DEEPFILTER_SOURCE_REVISION,
        runtime: "official libDF Rust runtime",
        runtime_crate_version: "deep_filter 0.5.7-pre / tract 0.21.4",
        onnx_runtime_version: "not-applicable",
        runtime_build_info: "official split ONNX graphs executed by Tract CPU",
        execution_provider: "Tract CPU",
        requested_intra_op_threads: 1,
        requested_inter_op_threads: 1,
        native_sample_rate_hz: SAMPLE_RATE_HZ,
        required_frame_size_samples: 960,
        hop_size_samples: FRAME_SAMPLES,
        model_algorithmic_latency_samples: 480,
        model_algorithmic_latency_ms: 10.0,
        downsampling_algorithmic_latency_samples_at_48_khz: 0,
        downsampling_algorithmic_latency_ms: 0.0,
        upsampling_algorithmic_latency_samples_at_48_khz: 0,
        upsampling_algorithmic_latency_ms: 0.0,
        additional_resampling_and_framing_latency_samples_at_48_khz: 0,
        additional_resampling_and_framing_latency_ms: 0.0,
        processor_algorithmic_latency_samples_at_48_khz: processor.algorithmic_latency_samples(),
        processor_algorithmic_latency_ms: samples_to_ms(
            processor.algorithmic_latency_samples(),
            SAMPLE_RATE_HZ,
        ),
        model_inference_wall_time: processor.timing_handle().snapshot(),
    }
}

fn sha256_file(path: &std::path::Path) -> Result<String, DynError> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut result = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut result, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(result)
}

fn host_runtime_metadata() -> HostRuntimeMetadata {
    let mut cpu_features = Vec::new();
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            cpu_features.push("sse2");
        }
        if std::is_x86_feature_detected!("avx") {
            cpu_features.push("avx");
        }
        if std::is_x86_feature_detected!("avx2") {
            cpu_features.push("avx2");
        }
        if std::is_x86_feature_detected!("fma") {
            cpu_features.push("fma");
        }
    }
    HostRuntimeMetadata {
        target_os: std::env::consts::OS,
        target_family: std::env::consts::FAMILY,
        target_architecture: std::env::consts::ARCH,
        pointer_width: if cfg!(target_pointer_width = "64") {
            "64"
        } else if cfg!(target_pointer_width = "32") {
            "32"
        } else {
            "unknown"
        },
        cpu_features,
        affinity_changed: false,
        os_version: ProcessResourceMonitor::current_os_version(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CharacterizeDenoiser, EnhancementProfile, parse_characterize_arguments,
        parse_simulate_arguments, software_latency_accounting,
    };
    use auralis_core::Metrics;

    #[test]
    fn stable_profile_defaults_are_unchanged() {
        let parsed = parse_characterize_arguments(Vec::<String>::new().into_iter())
            .expect("defaults should parse");
        assert_eq!(parsed.options.pipeline.queue_capacity_frames, 4);
        assert_eq!(parsed.options.pipeline.startup_pre_roll_frames, 1);
        assert!(!parsed.options.mute_output);
        assert_eq!(
            parsed.options.pipeline.drift_correction.target_fill_frames,
            3.5
        );
    }

    #[test]
    fn latency_accounting_keeps_paired_delta_unmeasured() {
        let metrics = Metrics::default().snapshot();
        let accounting = software_latency_accounting(&metrics, 2_208);
        assert_eq!(accounting.structural_path_addition_samples, 2_208);
        assert_eq!(
            accounting.incremental_software_pipeline_latency_samples,
            None
        );
        assert_eq!(
            accounting.transport_delta_vs_milestone_2_reference_average_ms,
            None
        );
    }

    #[test]
    fn output_muting_is_explicit() {
        let parsed = parse_characterize_arguments(["--mute-output"].map(str::to_owned).into_iter())
            .expect("muted output should parse");
        assert!(parsed.options.mute_output);
    }

    #[test]
    fn lower_latency_operating_point_is_explicit() {
        let parsed = parse_characterize_arguments(
            [
                "--queue-capacity-frames",
                "3",
                "--target-fill-frames",
                "2.0",
                "--pre-roll-frames",
                "1",
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .expect("lower-latency profile should parse");
        assert_eq!(parsed.options.pipeline.queue_capacity_frames, 3);
        assert_eq!(parsed.options.pipeline.startup_pre_roll_frames, 1);
        assert_eq!(
            parsed.options.pipeline.drift_correction.target_fill_frames,
            2.0
        );
    }

    #[test]
    fn target_fill_cannot_exceed_capacity() {
        let result = parse_characterize_arguments(
            [
                "--queue-capacity-frames",
                "2",
                "--target-fill-frames",
                "2.5",
            ]
            .map(str::to_owned)
            .into_iter(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn ul_unas_requires_a_model_path() {
        let result = parse_simulate_arguments(
            ["10", "--denoiser", "ul-unas"]
                .map(str::to_owned)
                .into_iter(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn model_path_is_rejected_for_passthrough() {
        let result =
            parse_simulate_arguments(["--model", "model.onnx"].map(str::to_owned).into_iter());
        assert!(result.is_err());
    }

    #[test]
    fn ul_unas_model_path_is_parsed() {
        let parsed = parse_simulate_arguments(
            ["10", "--denoiser", "ul-unas", "--model", "model.onnx"]
                .map(str::to_owned)
                .into_iter(),
        )
        .expect("UL-UNAS selection should parse");
        assert_eq!(parsed.seconds, 10);
        match parsed.denoiser {
            CharacterizeDenoiser::UlUnas { model_path } => {
                assert_eq!(model_path, std::path::PathBuf::from("model.onnx"));
            }
            CharacterizeDenoiser::Passthrough
            | CharacterizeDenoiser::DeepFilter { .. }
            | CharacterizeDenoiser::Rnnoise { .. } => {
                panic!("expected UL-UNAS selection")
            }
        }
    }

    #[test]
    fn rnnoise_library_path_is_parsed() {
        let parsed = parse_simulate_arguments(
            ["10", "--denoiser", "rnnoise", "--library", "rnnoise.dll"]
                .map(str::to_owned)
                .into_iter(),
        )
        .expect("RNNoise selection should parse");
        match parsed.denoiser {
            CharacterizeDenoiser::Rnnoise { library_path } => {
                assert_eq!(library_path, std::path::PathBuf::from("rnnoise.dll"));
            }
            CharacterizeDenoiser::Passthrough
            | CharacterizeDenoiser::UlUnas { .. }
            | CharacterizeDenoiser::DeepFilter { .. } => {
                panic!("expected RNNoise selection")
            }
        }
    }

    #[test]
    fn deepfilter_model_path_is_parsed() {
        let parsed = parse_simulate_arguments(
            [
                "10",
                "--denoiser",
                "deepfilter",
                "--model",
                "DeepFilterNet3_ll_onnx.tar.gz",
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .expect("DeepFilterNet selection should parse");
        match parsed.denoiser {
            CharacterizeDenoiser::DeepFilter { model_path } => {
                assert_eq!(
                    model_path,
                    std::path::PathBuf::from("DeepFilterNet3_ll_onnx.tar.gz")
                );
            }
            CharacterizeDenoiser::Passthrough
            | CharacterizeDenoiser::UlUnas { .. }
            | CharacterizeDenoiser::Rnnoise { .. } => {
                panic!("expected DeepFilterNet selection")
            }
        }
    }

    #[test]
    fn deepfilter_requires_a_model_path() {
        let result =
            parse_simulate_arguments(["--denoiser", "deepfilter"].map(str::to_owned).into_iter());
        assert!(result.is_err());
    }

    #[test]
    fn rnnoise_requires_a_library_path() {
        let result =
            parse_simulate_arguments(["--denoiser", "rnnoise"].map(str::to_owned).into_iter());
        assert!(result.is_err());
    }

    #[test]
    fn unsupported_denoiser_is_rejected() {
        let result =
            parse_simulate_arguments(["--denoiser", "unknown"].map(str::to_owned).into_iter());
        assert!(result.is_err());
    }

    #[test]
    fn balanced_profile_selects_ul_unas() {
        let parsed = parse_simulate_arguments(
            ["10", "--profile", "balanced", "--model", "model.onnx"]
                .map(str::to_owned)
                .into_iter(),
        )
        .expect("balanced profile should parse");
        assert_eq!(parsed.profile, Some(EnhancementProfile::Balanced));
        assert!(matches!(
            parsed.denoiser,
            CharacterizeDenoiser::UlUnas { .. }
        ));
    }

    #[test]
    fn low_latency_profile_selects_rnnoise() {
        let parsed = parse_simulate_arguments(
            ["10", "--profile", "low-latency", "--library", "rnnoise.dll"]
                .map(str::to_owned)
                .into_iter(),
        )
        .expect("low-latency profile should parse");
        assert_eq!(parsed.profile, Some(EnhancementProfile::LowLatency));
        assert!(matches!(
            parsed.denoiser,
            CharacterizeDenoiser::Rnnoise { .. }
        ));
    }

    #[test]
    fn maximum_quality_profile_selects_deepfilter() {
        let parsed = parse_simulate_arguments(
            [
                "10",
                "--profile",
                "maximum-quality",
                "--model",
                "DeepFilterNet3_ll_onnx.tar.gz",
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .expect("maximum-quality profile should parse");
        assert_eq!(parsed.profile, Some(EnhancementProfile::MaximumQuality));
        assert!(matches!(
            parsed.denoiser,
            CharacterizeDenoiser::DeepFilter { .. }
        ));
    }

    #[test]
    fn profile_and_raw_denoiser_cannot_be_combined() {
        let result = parse_simulate_arguments(
            [
                "10",
                "--profile",
                "balanced",
                "--denoiser",
                "ul-unas",
                "--model",
                "model.onnx",
            ]
            .map(str::to_owned)
            .into_iter(),
        );
        assert!(result.is_err());
    }
}
