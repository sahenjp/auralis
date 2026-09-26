//! Reproducible offline benchmark harness.

#![forbid(unsafe_code)]

mod enhance;
mod fixtures;
mod foundation;
mod latency;
mod metrics;
mod score;
mod smoke;
mod wav;

use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use auralis_core::Passthrough;
use auralis_denoisers::{DeepFilterFrameProcessor, RnnoiseFrameProcessor, UlUnasFrameProcessor};

type DynError = Box<dyn Error + Send + Sync>;

const DEFAULT_SECONDS: u32 = 2;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("auralis-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), DynError> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("foundation") => {
            let mut out_dir = PathBuf::from("bench/work/foundation");
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--out-dir" => {
                        out_dir = arguments.next().ok_or("--out-dir requires a path")?.into();
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            foundation::run(&out_dir)?;
            println!("{}", out_dir.display());
        }
        Some("smoke") => {
            let mut out_dir = PathBuf::from("bench/work/smoke");
            let mut seconds = DEFAULT_SECONDS;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--out-dir" => {
                        out_dir = arguments.next().ok_or("--out-dir requires a path")?.into();
                    }
                    "--seconds" => {
                        seconds = arguments
                            .next()
                            .ok_or("--seconds requires a value")?
                            .parse()?;
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            if !(1..=60).contains(&seconds) {
                return Err("--seconds must be between 1 and 60".into());
            }
            smoke::run(&out_dir, seconds)?;
            println!("{}", out_dir.display());
        }
        Some("score") => {
            let mut reference: Option<PathBuf> = None;
            let mut noise: Option<PathBuf> = None;
            let mut output: Option<PathBuf> = None;
            let mut report: Option<PathBuf> = None;
            let mut system_id = None;
            let mut offset_samples = 0;
            let mut offset_source = String::from("declared candidate metadata");
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--reference" => {
                        reference =
                            Some(arguments.next().ok_or("--reference requires path")?.into());
                    }
                    "--noise" => {
                        noise = Some(arguments.next().ok_or("--noise requires path")?.into());
                    }
                    "--output" => {
                        output = Some(arguments.next().ok_or("--output requires path")?.into());
                    }
                    "--report" => {
                        report = Some(arguments.next().ok_or("--report requires path")?.into());
                    }
                    "--system-id" => {
                        system_id = Some(arguments.next().ok_or("--system-id requires value")?);
                    }
                    "--offset-samples" => {
                        offset_samples = arguments
                            .next()
                            .ok_or("--offset-samples requires value")?
                            .parse()?;
                    }
                    "--offset-source" => {
                        offset_source = arguments.next().ok_or("--offset-source requires value")?;
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            score::run(
                &reference.ok_or("score requires --reference PATH")?,
                &noise.ok_or("score requires --noise PATH")?,
                &output.ok_or("score requires --output PATH")?,
                &report.ok_or("score requires --report PATH")?,
                &system_id.ok_or("score requires --system-id VALUE")?,
                offset_samples,
                &offset_source,
            )?;
        }
        Some("enhance-wav") => {
            let mut denoiser = None;
            let mut model = None;
            let mut library = None;
            let mut input = None;
            let mut output = None;
            let mut report = None;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--denoiser" => {
                        denoiser = Some(arguments.next().ok_or("--denoiser requires value")?);
                    }
                    "--model" => {
                        model = Some(PathBuf::from(
                            arguments.next().ok_or("--model requires path")?,
                        ));
                    }
                    "--library" => {
                        library = Some(PathBuf::from(
                            arguments.next().ok_or("--library requires path")?,
                        ));
                    }
                    "--input" => {
                        input = Some(PathBuf::from(
                            arguments.next().ok_or("--input requires path")?,
                        ));
                    }
                    "--output" => {
                        output = Some(PathBuf::from(
                            arguments.next().ok_or("--output requires path")?,
                        ));
                    }
                    "--report" => {
                        report = Some(PathBuf::from(
                            arguments.next().ok_or("--report requires path")?,
                        ));
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            let input = input.ok_or("enhance-wav requires --input PATH")?;
            let output = output.ok_or("enhance-wav requires --output PATH")?;
            let report = report.ok_or("enhance-wav requires --report PATH")?;
            match denoiser.as_deref().unwrap_or("passthrough") {
                "passthrough" => {
                    if model.is_some() || library.is_some() {
                        return Err("passthrough does not accept --model/--library".into());
                    }
                    enhance::run(&input, &output, &report, Passthrough, None)?;
                }
                "ul-unas" => {
                    if library.is_some() {
                        return Err("ul-unas does not accept --library".into());
                    }
                    let processor =
                        UlUnasFrameProcessor::load(model.ok_or("ul-unas requires --model PATH")?)?;
                    let timing = processor.timing_handle();
                    enhance::run(&input, &output, &report, processor, Some(timing))?;
                }
                "deepfilter" => {
                    if library.is_some() {
                        return Err("deepfilter does not accept --library".into());
                    }
                    let processor = DeepFilterFrameProcessor::load(
                        model.ok_or("deepfilter requires --model PATH")?,
                    )?;
                    let timing = processor.timing_handle();
                    enhance::run(&input, &output, &report, processor, Some(timing))?;
                }
                "rnnoise" => {
                    if model.is_some() {
                        return Err("rnnoise does not accept --model".into());
                    }
                    let processor = RnnoiseFrameProcessor::load(
                        library.ok_or("rnnoise requires --library PATH")?,
                    )?;
                    let timing = processor.timing_handle();
                    enhance::run(&input, &output, &report, processor, Some(timing))?;
                }
                value => return Err(format!("unsupported denoiser: {value}").into()),
            }
            println!("{}", output.display());
        }
        Some("score-bakeoff") => {
            let mut corpus_manifest: Option<PathBuf> = None;
            let mut output_manifest: Option<PathBuf> = None;
            let mut report: Option<PathBuf> = None;
            let mut system_id = None;
            let mut offset_samples = 0;
            let mut offset_source = String::from("declared candidate metadata");
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--corpus-manifest" => {
                        corpus_manifest = Some(
                            arguments
                                .next()
                                .ok_or("--corpus-manifest requires path")?
                                .into(),
                        );
                    }
                    "--output-manifest" => {
                        output_manifest = Some(
                            arguments
                                .next()
                                .ok_or("--output-manifest requires path")?
                                .into(),
                        );
                    }
                    "--report" => {
                        report = Some(arguments.next().ok_or("--report requires path")?.into());
                    }
                    "--system-id" => {
                        system_id = Some(arguments.next().ok_or("--system-id requires value")?);
                    }
                    "--offset-samples" => {
                        offset_samples = arguments
                            .next()
                            .ok_or("--offset-samples requires value")?
                            .parse()?;
                    }
                    "--offset-source" => {
                        offset_source = arguments.next().ok_or("--offset-source requires value")?;
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            score::run_batch(
                &corpus_manifest.ok_or("score-bakeoff requires --corpus-manifest PATH")?,
                &output_manifest.ok_or("score-bakeoff requires --output-manifest PATH")?,
                &report.ok_or("score-bakeoff requires --report PATH")?,
                &system_id.ok_or("score-bakeoff requires --system-id VALUE")?,
                offset_samples,
                &offset_source,
            )?;
        }
        Some("latency-reference") => {
            let mut out = None;
            let mut count = 20;
            let mut interval_ms = 500;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--out" => out = Some(arguments.next().ok_or("--out requires path")?.into()),
                    "--count" => {
                        count = arguments.next().ok_or("--count requires value")?.parse()?;
                    }
                    "--interval-ms" => {
                        interval_ms = arguments
                            .next()
                            .ok_or("--interval-ms requires value")?
                            .parse()?;
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            let out: PathBuf = out.ok_or("latency-reference requires --out PATH")?;
            let report = latency::write_reference(&out, count, interval_ms)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Some("latency-analyze") => {
            let mut reference = None;
            let mut recording = None;
            let mut out = None;
            let mut max_lag_ms = 250;
            while let Some(argument) = arguments.next() {
                match argument.as_str() {
                    "--reference" => {
                        reference =
                            Some(arguments.next().ok_or("--reference requires path")?.into());
                    }
                    "--recording" => {
                        recording =
                            Some(arguments.next().ok_or("--recording requires path")?.into());
                    }
                    "--out" => out = Some(arguments.next().ok_or("--out requires path")?.into()),
                    "--max-lag-ms" => {
                        max_lag_ms = arguments
                            .next()
                            .ok_or("--max-lag-ms requires value")?
                            .parse()?;
                    }
                    _ => return Err(format!("unexpected argument: {argument}").into()),
                }
            }
            let reference: PathBuf =
                reference.ok_or("latency-analyze requires --reference PATH")?;
            let recording: PathBuf =
                recording.ok_or("latency-analyze requires --recording PATH")?;
            let out: PathBuf = out.ok_or("latency-analyze requires --out PATH")?;
            if out.exists() {
                return Err(format!("refusing to overwrite {}", out.display()).into());
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let report = latency::analyze(&reference, &recording, max_lag_ms)?;
            wav::write_json(&out, &report)?;
            println!("{}", out.display());
        }
        Some("help" | "--help" | "-h") | None => print_help(),
        Some(command) => return Err(format!("unknown command: {command}").into()),
    }
    Ok(())
}

fn print_help() {
    println!(
        "Auralis benchmark harness\n\n\
         Usage:\n\
           auralis-bench foundation [--out-dir PATH]\n\
           auralis-bench smoke [--out-dir PATH] [--seconds 1..60]\n\
           auralis-bench enhance-wav --denoiser passthrough|ul-unas|deepfilter|rnnoise\n\
               [--model PATH] [--library PATH] --input PATH --output PATH --report PATH\n\
           auralis-bench score --reference PATH --noise PATH --output PATH --report PATH\n\
               --system-id ID [--offset-samples N] [--offset-source TEXT]\n\
           auralis-bench score-bakeoff --corpus-manifest PATH --output-manifest PATH\n\
               --report PATH --system-id ID [--offset-samples N] [--offset-source TEXT]\n\
           auralis-bench latency-reference --out PATH [--count N] [--interval-ms N]\n\
           auralis-bench latency-analyze --reference PATH --recording PATH --out PATH [--max-lag-ms N]\n\n\
         The output directory must be absent or empty; existing results are never overwritten."
    );
}
