use std::fs;
use std::path::Path;

use auralis_core::{FrameProcessor, SAMPLE_RATE_HZ};
use auralis_denoisers::{InferenceTimingHandle, InferenceTimingSnapshot};
use serde::Serialize;

use crate::metrics::{RuntimeMetrics, process_offline};
use crate::{DynError, wav};

#[derive(Debug, Serialize)]
struct EnhanceReport {
    schema_id: &'static str,
    processor: &'static str,
    input: wav::Asset,
    output: wav::Asset,
    sample_rate_hz: u32,
    input_samples: usize,
    algorithmic_latency_samples: usize,
    algorithmic_latency_ms: f64,
    runtime: RuntimeMetrics,
    inference: Option<InferenceTimingSnapshot>,
    limitations: [&'static str; 2],
}

pub(crate) fn run<P: FrameProcessor>(
    input_path: &Path,
    output_path: &Path,
    report_path: &Path,
    mut processor: P,
    timing: Option<InferenceTimingHandle>,
) -> Result<(), DynError> {
    if output_path.exists() || report_path.exists() {
        return Err("refusing to overwrite enhance-wav output or report".into());
    }
    let processor_name = processor.name();
    let algorithmic_latency_samples = processor.algorithmic_latency_samples();
    let input = wav::read(input_path)?;
    processor
        .prepare()
        .map_err(|error| format!("prepare {processor_name}: {error}"))?;
    let (output, runtime) = process_offline(&input, processor);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)?;
    }
    wav::write(output_path, &output)?;
    let input_root = input_path.parent().unwrap_or_else(|| Path::new("."));
    let output_root = output_path.parent().unwrap_or_else(|| Path::new("."));
    let report = EnhanceReport {
        schema_id: "auralis.offline-enhance.v1",
        processor: processor_name,
        input: wav::asset(
            input_root,
            input_path.file_name().unwrap().to_str().unwrap(),
        )?,
        output: wav::asset(
            output_root,
            output_path.file_name().unwrap().to_str().unwrap(),
        )?,
        sample_rate_hz: SAMPLE_RATE_HZ,
        input_samples: input.len(),
        algorithmic_latency_samples,
        algorithmic_latency_ms: algorithmic_latency_samples as f64 / SAMPLE_RATE_HZ as f64
            * 1_000.0,
        runtime,
        inference: timing.map(|handle| handle.snapshot()),
        limitations: [
            "Offline processing does not prove native realtime transport stability.",
            "Output retains declared structural latency; no signal-derived alignment is applied.",
        ],
    };
    wav::write_json(report_path, &report)?;
    Ok(())
}
