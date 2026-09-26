//! Reproducible acoustic/loopback latency reference and correlation analysis.

use std::cmp::Ordering;
use std::path::Path;

use auralis_core::SAMPLE_RATE_HZ;
use serde::Serialize;

use crate::{DynError, wav};

const PULSE_SAMPLES: usize = 127;
const LEAD_IN_SECONDS: usize = 1;
const PULSE_AMPLITUDE: f32 = 0.5;
const DETECTION_THRESHOLD: f32 = 0.25;

#[derive(Debug, Serialize)]
pub(crate) struct ReferenceReport {
    schema_version: u32,
    method: &'static str,
    sample_rate_hz: u32,
    pulse_count: usize,
    pulse_samples: usize,
    interval_ms: u32,
    duration_samples: usize,
}

#[derive(Clone, Debug, Serialize)]
struct ProbeResult {
    reference_sample: usize,
    detected_sample: usize,
    latency_samples: usize,
    latency_ms: f64,
    normalized_correlation: f64,
}

#[derive(Debug, Serialize)]
pub(crate) struct LatencyReport {
    schema_version: u32,
    method: &'static str,
    reference_path: String,
    recording_path: String,
    sample_rate_hz: u32,
    max_lag_ms: u32,
    sample_count: usize,
    median_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    maximum_ms: f64,
    minimum_correlation: f64,
    probes: Vec<ProbeResult>,
}

pub(crate) fn write_reference(
    path: &Path,
    pulse_count: usize,
    interval_ms: u32,
) -> Result<ReferenceReport, DynError> {
    if pulse_count == 0 || pulse_count > 10_000 {
        return Err("--count must be in 1..=10000".into());
    }
    let interval_samples = milliseconds_to_samples(interval_ms)?;
    if interval_samples <= PULSE_SAMPLES {
        return Err("--interval-ms must be longer than the correlation pulse".into());
    }
    if path.exists() {
        return Err(format!("refusing to overwrite {}", path.display()).into());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let lead_in = LEAD_IN_SECONDS * SAMPLE_RATE_HZ as usize;
    let duration_samples = lead_in
        .checked_add(
            pulse_count
                .checked_mul(interval_samples)
                .ok_or("reference duration overflow")?,
        )
        .ok_or("reference duration overflow")?;
    let mut samples = vec![0.0_f32; duration_samples];
    let pulse = correlation_pulse();
    for index in 0..pulse_count {
        let start = lead_in + index * interval_samples;
        samples[start..start + pulse.len()].copy_from_slice(&pulse);
    }
    wav::write(path, &samples)?;

    Ok(ReferenceReport {
        schema_version: 1,
        method: "physical-or-loopback-pulse-correlation",
        sample_rate_hz: SAMPLE_RATE_HZ,
        pulse_count,
        pulse_samples: PULSE_SAMPLES,
        interval_ms,
        duration_samples,
    })
}

pub(crate) fn analyze(
    reference_path: &Path,
    recording_path: &Path,
    max_lag_ms: u32,
) -> Result<LatencyReport, DynError> {
    let reference = wav::read(reference_path)?;
    let recording = wav::read(recording_path)?;
    let starts = find_pulse_starts(&reference);
    if starts.is_empty() {
        return Err("reference contains no detectable pulses".into());
    }
    let max_lag_samples = milliseconds_to_samples(max_lag_ms)?;
    if max_lag_samples == 0 {
        return Err("--max-lag-ms must be greater than zero".into());
    }

    let pulse = correlation_pulse();
    let mut probes = Vec::with_capacity(starts.len());
    for reference_sample in starts {
        let search_end = reference_sample
            .checked_add(max_lag_samples)
            .ok_or("search range overflow")?;
        if search_end + pulse.len() > recording.len() {
            return Err(format!(
                "recording is too short to search pulse at sample {reference_sample}"
            )
            .into());
        }

        let mut best_sample = reference_sample;
        let mut best_correlation = f64::NEG_INFINITY;
        for candidate in reference_sample..=search_end {
            let correlation =
                normalized_correlation(&pulse, &recording[candidate..candidate + pulse.len()]);
            if correlation > best_correlation {
                best_correlation = correlation;
                best_sample = candidate;
            }
        }
        let latency_samples = best_sample - reference_sample;
        probes.push(ProbeResult {
            reference_sample,
            detected_sample: best_sample,
            latency_samples,
            latency_ms: samples_to_milliseconds(latency_samples),
            normalized_correlation: best_correlation,
        });
    }

    let mut latencies: Vec<f64> = probes.iter().map(|probe| probe.latency_ms).collect();
    latencies.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let minimum_correlation = probes
        .iter()
        .map(|probe| probe.normalized_correlation)
        .fold(f64::INFINITY, f64::min);

    Ok(LatencyReport {
        schema_version: 1,
        method: "physical-or-loopback-pulse-correlation",
        reference_path: reference_path.display().to_string(),
        recording_path: recording_path.display().to_string(),
        sample_rate_hz: SAMPLE_RATE_HZ,
        max_lag_ms,
        sample_count: latencies.len(),
        median_ms: percentile(&latencies, 0.50),
        p95_ms: percentile(&latencies, 0.95),
        p99_ms: percentile(&latencies, 0.99),
        maximum_ms: latencies.last().copied().unwrap_or(0.0),
        minimum_correlation,
        probes,
    })
}

fn correlation_pulse() -> [f32; PULSE_SAMPLES] {
    let mut pulse = [0.0; PULSE_SAMPLES];
    let mut state = 0x5D_u8;
    for sample in &mut pulse {
        let feedback = ((state >> 6) ^ (state >> 5)) & 1;
        state = ((state << 1) | feedback) & 0x7F;
        *sample = if state & 1 == 0 {
            -PULSE_AMPLITUDE
        } else {
            PULSE_AMPLITUDE
        };
    }
    pulse
}

fn find_pulse_starts(reference: &[f32]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index < reference.len() {
        if reference[index].abs() >= DETECTION_THRESHOLD
            && (index == 0 || reference[index - 1].abs() < DETECTION_THRESHOLD)
        {
            starts.push(index);
            index = index.saturating_add(PULSE_SAMPLES);
        } else {
            index += 1;
        }
    }
    starts
}

fn milliseconds_to_samples(milliseconds: u32) -> Result<usize, DynError> {
    let samples = u64::from(milliseconds)
        .checked_mul(u64::from(SAMPLE_RATE_HZ))
        .ok_or("sample count overflow")?
        / 1_000;
    usize::try_from(samples).map_err(Into::into)
}

fn samples_to_milliseconds(samples: usize) -> f64 {
    samples as f64 * 1_000.0 / f64::from(SAMPLE_RATE_HZ)
}

fn normalized_correlation(left: &[f32], right: &[f32]) -> f64 {
    let mut dot = 0.0;
    let mut left_energy = 0.0;
    let mut right_energy = 0.0;
    for (&left, &right) in left.iter().zip(right) {
        let left = f64::from(left);
        let right = f64::from(right);
        dot += left * right;
        left_energy += left * left;
        right_energy += right * right;
    }
    dot / (left_energy * right_energy).sqrt().max(f64::EPSILON)
}

fn percentile(sorted: &[f64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::{PULSE_SAMPLES, analyze, correlation_pulse, find_pulse_starts};
    use crate::wav;

    #[test]
    fn finds_each_separated_pulse() {
        let pulse = correlation_pulse();
        let mut samples = vec![0.0; 1_000];
        samples[100..100 + PULSE_SAMPLES].copy_from_slice(&pulse);
        samples[500..500 + PULSE_SAMPLES].copy_from_slice(&pulse);
        assert_eq!(find_pulse_starts(&samples), [100, 500]);
    }

    #[test]
    fn correlation_recovers_known_delay() {
        let directory =
            std::env::temp_dir().join(format!("auralis-latency-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let reference_path = directory.join("reference.wav");
        let recording_path = directory.join("recording.wav");
        let pulse = correlation_pulse();
        let mut reference = vec![0.0; 20_000];
        reference[1_000..1_000 + PULSE_SAMPLES].copy_from_slice(&pulse);
        reference[10_000..10_000 + PULSE_SAMPLES].copy_from_slice(&pulse);
        let delay = 240;
        let mut recording = vec![0.0; 21_000];
        recording[1_000 + delay..1_000 + delay + PULSE_SAMPLES].copy_from_slice(&pulse);
        recording[10_000 + delay..10_000 + delay + PULSE_SAMPLES].copy_from_slice(&pulse);
        wav::write(&reference_path, &reference).expect("write reference");
        wav::write(&recording_path, &recording).expect("write recording");

        let report = analyze(&reference_path, &recording_path, 20).expect("analyze");
        assert_eq!(report.sample_count, 2);
        assert_eq!(report.median_ms, 5.0);
        assert_eq!(report.maximum_ms, 5.0);
        assert!(report.minimum_correlation > 0.999);

        std::fs::remove_file(reference_path).expect("remove reference");
        std::fs::remove_file(recording_path).expect("remove recording");
        std::fs::remove_dir(directory).expect("remove temporary directory");
    }
}
