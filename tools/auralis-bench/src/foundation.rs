//! Deterministic controls that validate the offline benchmark before model use.

use std::fs;
use std::io;
use std::path::Path;

use auralis_core::{FRAME_SAMPLES, FrameProcessor, Passthrough, SAMPLE_RATE_HZ};
use serde::Serialize;

use crate::DynError;
use crate::fixtures::generate_clean;
use crate::metrics::{
    AlignedQualityMetrics, RuntimeMetrics, process_offline, quality_aligned, rms,
};
use crate::wav::{self, Asset};

const CONTROL_SAMPLE_COUNT: usize = SAMPLE_RATE_HZ as usize;
const GAIN: f32 = 0.5;
const DELAY_SAMPLES: usize = FRAME_SAMPLES * 2;

#[derive(Debug, Serialize)]
struct FoundationReport {
    schema_id: &'static str,
    purpose: &'static str,
    sample_rate_hz: u32,
    frame_samples: usize,
    source: Asset,
    timeline: TimelineValidation,
    controls: Vec<ControlResult>,
    all_validations_passed: bool,
}

#[derive(Debug, Serialize)]
struct TimelineValidation {
    timestamp_unit: &'static str,
    expected_frame_count: usize,
    frame_start_samples: Vec<usize>,
    strictly_increasing: bool,
    fixed_step_samples: usize,
    final_frame_end_sample: usize,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct ControlResult {
    system_id: &'static str,
    output: Asset,
    repeated_output: Asset,
    expected_sample_count: usize,
    actual_sample_count: usize,
    repeated_sample_count: usize,
    hashes_reproducible: bool,
    model_algorithmic_latency: LatencyValue,
    model_inference_wall_time: RuntimeMetrics,
    alignment: AlignedQualityMetrics,
    loudness: LoudnessValidation,
    validations: Vec<Validation>,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct LatencyValue {
    samples: usize,
    milliseconds: f64,
    processing_frames: f64,
}

#[derive(Debug, Serialize)]
struct LoudnessValidation {
    source_rms_dbfs: f64,
    output_rms_dbfs: f64,
    expected_gain_db: f64,
    measured_gain_db: f64,
    tolerance_db: f64,
    passed: bool,
}

#[derive(Debug, Serialize)]
struct Validation {
    name: &'static str,
    passed: bool,
    detail: String,
}

#[derive(Clone, Copy)]
struct GainProcessor {
    gain: f32,
}

impl FrameProcessor for GainProcessor {
    fn name(&self) -> &'static str {
        "known-gain-minus-6db"
    }

    fn algorithmic_latency_samples(&self) -> usize {
        0
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        for sample in samples {
            *sample *= self.gain;
        }
    }
}

#[derive(Clone)]
struct DelayProcessor {
    buffered: [f32; DELAY_SAMPLES],
}

impl Default for DelayProcessor {
    fn default() -> Self {
        Self {
            buffered: [0.0; DELAY_SAMPLES],
        }
    }
}

impl FrameProcessor for DelayProcessor {
    fn name(&self) -> &'static str {
        "known-delay-20ms"
    }

    fn algorithmic_latency_samples(&self) -> usize {
        DELAY_SAMPLES
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        let mut input = [0.0_f32; FRAME_SAMPLES];
        input.copy_from_slice(samples);
        samples.copy_from_slice(&self.buffered[..FRAME_SAMPLES]);
        self.buffered.copy_within(FRAME_SAMPLES.., 0);
        self.buffered[FRAME_SAMPLES..].copy_from_slice(&input);
    }
}

pub(crate) fn run(out_dir: &Path) -> Result<(), DynError> {
    ensure_empty_directory(out_dir)?;
    fs::create_dir_all(out_dir.join("source"))?;
    fs::create_dir_all(out_dir.join("outputs"))?;
    fs::create_dir_all(out_dir.join("results"))?;

    let source = generate_clean(CONTROL_SAMPLE_COUNT);
    let zero_noise = vec![0.0_f32; source.len()];
    wav::write(&out_dir.join("source/control.wav"), &source)?;

    let timeline = validate_timeline(source.len());
    let controls = vec![
        run_control(
            out_dir,
            "identity",
            Passthrough,
            Passthrough,
            &source,
            &zero_noise,
            0,
            0.0,
        )?,
        run_control(
            out_dir,
            "known-gain-minus-6db",
            GainProcessor { gain: GAIN },
            GainProcessor { gain: GAIN },
            &source,
            &zero_noise,
            0,
            20.0 * f64::from(GAIN).log10(),
        )?,
        run_control(
            out_dir,
            "known-delay-20ms",
            DelayProcessor::default(),
            DelayProcessor::default(),
            &source,
            &zero_noise,
            DELAY_SAMPLES,
            0.0,
        )?,
    ];
    let all_validations_passed = timeline.passed && controls.iter().all(|control| control.passed);
    if !all_validations_passed {
        return Err("benchmark foundation control validation failed".into());
    }

    let report = FoundationReport {
        schema_id: "auralis.benchmark-foundation.v1",
        purpose: "deterministic benchmark plumbing controls; not speech-quality evidence",
        sample_rate_hz: SAMPLE_RATE_HZ,
        frame_samples: FRAME_SAMPLES,
        source: wav::asset(out_dir, "source/control.wav")?,
        timeline,
        controls,
        all_validations_passed,
    };
    wav::write_json(&out_dir.join("results/foundation.json"), &report)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_control<P: FrameProcessor>(
    out_dir: &Path,
    system_id: &'static str,
    processor: P,
    repeated_processor: P,
    source: &[f32],
    noise: &[f32],
    delay_samples: usize,
    expected_gain_db: f64,
) -> Result<ControlResult, DynError> {
    let (output, runtime) = process_offline(source, processor);
    let (repeated, _) = process_offline(source, repeated_processor);
    let output_relative = format!("outputs/{system_id}.wav");
    let repeated_relative = format!("outputs/{system_id}-repeat.wav");
    wav::write(&out_dir.join(&output_relative), &output)?;
    wav::write(&out_dir.join(&repeated_relative), &repeated)?;
    let output_asset = wav::asset(out_dir, &output_relative)?;
    let repeated_asset = wav::asset(out_dir, &repeated_relative)?;
    let hashes_reproducible = output_asset.sha256 == repeated_asset.sha256;
    let alignment = quality_aligned(
        source,
        noise,
        &output,
        delay_samples,
        "declared deterministic fixture delay",
    );
    let source_rms_dbfs = amplitude_db(rms(source));
    let output_for_loudness = if delay_samples == 0 {
        output.as_slice()
    } else {
        &output[delay_samples..]
    };
    let measured_gain_db = amplitude_db(rms(output_for_loudness))
        - amplitude_db(rms(&source[..output_for_loudness.len()]));
    let tolerance_db = 0.000_1;
    let loudness_passed = (measured_gain_db - expected_gain_db).abs() <= tolerance_db;
    let sample_counts_passed = output.len() == source.len() && repeated.len() == source.len();
    let latency_passed = alignment.offset_samples == delay_samples
        && (alignment.offset_ms - delay_samples as f64 / SAMPLE_RATE_HZ as f64 * 1_000.0).abs()
            < f64::EPSILON;
    let alignment_passed = if delay_samples == 0 {
        true
    } else {
        alignment.deterministic_latency_compensated.si_sdr_db
            > alignment.raw_unaligned.si_sdr_db + 40.0
            && alignment.deterministic_latency_compensated.sdr_db > 100.0
    };
    let degradation_detected = match system_id {
        "identity" => alignment.raw_unaligned.sdr_db > 100.0,
        "known-gain-minus-6db" => alignment.raw_unaligned.sdr_db < 20.0,
        "known-delay-20ms" => alignment.raw_unaligned.sdr_db < 20.0,
        _ => false,
    };
    let validations = vec![
        Validation {
            name: "sample_counts",
            passed: sample_counts_passed,
            detail: format!(
                "expected={}, first={}, repeat={}",
                source.len(),
                output.len(),
                repeated.len()
            ),
        },
        Validation {
            name: "repeated_output_hash",
            passed: hashes_reproducible,
            detail: format!(
                "first={}, repeat={}",
                output_asset.sha256, repeated_asset.sha256
            ),
        },
        Validation {
            name: "latency_accounting",
            passed: latency_passed,
            detail: format!(
                "offset_samples={}, offset_ms={}",
                alignment.offset_samples, alignment.offset_ms
            ),
        },
        Validation {
            name: "deterministic_alignment",
            passed: alignment_passed,
            detail: format!(
                "raw_sdr_db={}, compensated_sdr_db={}",
                alignment.raw_unaligned.sdr_db, alignment.deterministic_latency_compensated.sdr_db
            ),
        },
        Validation {
            name: "intentional_degradation_detection",
            passed: degradation_detected,
            detail: format!("raw_sdr_db={}", alignment.raw_unaligned.sdr_db),
        },
        Validation {
            name: "loudness_gain",
            passed: loudness_passed,
            detail: format!(
                "expected_gain_db={expected_gain_db}, measured_gain_db={measured_gain_db}"
            ),
        },
    ];
    let passed = validations.iter().all(|validation| validation.passed);
    Ok(ControlResult {
        system_id,
        output: output_asset,
        repeated_output: repeated_asset,
        expected_sample_count: source.len(),
        actual_sample_count: output.len(),
        repeated_sample_count: repeated.len(),
        hashes_reproducible,
        model_algorithmic_latency: LatencyValue {
            samples: delay_samples,
            milliseconds: delay_samples as f64 / SAMPLE_RATE_HZ as f64 * 1_000.0,
            processing_frames: delay_samples as f64 / FRAME_SAMPLES as f64,
        },
        model_inference_wall_time: runtime,
        alignment,
        loudness: LoudnessValidation {
            source_rms_dbfs,
            output_rms_dbfs: amplitude_db(rms(output_for_loudness)),
            expected_gain_db,
            measured_gain_db,
            tolerance_db,
            passed: loudness_passed,
        },
        validations,
        passed,
    })
}

fn validate_timeline(sample_count: usize) -> TimelineValidation {
    let frame_start_samples: Vec<usize> = (0..sample_count).step_by(FRAME_SAMPLES).collect();
    let expected_frame_count = sample_count.div_ceil(FRAME_SAMPLES);
    let strictly_increasing = frame_start_samples.windows(2).all(|pair| pair[0] < pair[1]);
    let fixed_step = frame_start_samples
        .windows(2)
        .all(|pair| pair[1] - pair[0] == FRAME_SAMPLES);
    let final_frame_end_sample = frame_start_samples
        .last()
        .copied()
        .unwrap_or(0)
        .saturating_add(sample_count % FRAME_SAMPLES)
        .max(sample_count);
    let passed = frame_start_samples.len() == expected_frame_count
        && strictly_increasing
        && fixed_step
        && final_frame_end_sample == sample_count;
    TimelineValidation {
        timestamp_unit: "samples from stream start",
        expected_frame_count,
        frame_start_samples,
        strictly_increasing,
        fixed_step_samples: FRAME_SAMPLES,
        final_frame_end_sample,
        passed,
    }
}

fn amplitude_db(value: f64) -> f64 {
    20.0 * value.max(f64::EPSILON).log10()
}

fn ensure_empty_directory(path: &Path) -> io::Result<()> {
    if path.exists() {
        let mut entries = fs::read_dir(path)?;
        if entries.next().transpose()?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "refusing to overwrite non-empty directory: {}",
                    path.display()
                ),
            ));
        }
    }
    fs::create_dir_all(path)
}

#[cfg(test)]
mod tests {
    use auralis_core::{FRAME_SAMPLES, FrameProcessor};

    use super::{DELAY_SAMPLES, DelayProcessor, validate_timeline};

    #[test]
    fn delay_processor_has_exact_two_frame_delay() {
        let mut processor = DelayProcessor::default();
        let mut first = [1.0_f32; FRAME_SAMPLES];
        let mut second = [2.0_f32; FRAME_SAMPLES];
        let mut third = [3.0_f32; FRAME_SAMPLES];
        processor.process(&mut first);
        processor.process(&mut second);
        processor.process(&mut third);
        assert_eq!(DELAY_SAMPLES, FRAME_SAMPLES * 2);
        assert_eq!(first, [0.0; FRAME_SAMPLES]);
        assert_eq!(second, [0.0; FRAME_SAMPLES]);
        assert_eq!(third, [1.0; FRAME_SAMPLES]);
    }

    #[test]
    fn timeline_uses_processing_frame_sample_clock() {
        let timeline = validate_timeline(FRAME_SAMPLES * 3);
        assert!(timeline.passed);
        assert_eq!(
            timeline.frame_start_samples,
            [0, FRAME_SAMPLES, FRAME_SAMPLES * 2]
        );
    }
}
