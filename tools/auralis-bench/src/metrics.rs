//! Intrusive signal metrics and offline processor timing.

use std::time::Instant;

use auralis_core::{FRAME_SAMPLES, FrameProcessor, SAMPLE_RATE_HZ};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RuntimeMetrics {
    pub frames: usize,
    pub total_ms: f64,
    pub average_frame_us: f64,
    pub p50_frame_us: f64,
    pub p95_frame_us: f64,
    pub p99_frame_us: f64,
    pub max_frame_us: f64,
    pub realtime_factor: f64,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct QualityMetrics {
    pub si_sdr_db: f64,
    pub sdr_db: f64,
    pub measured_input_snr_db: f64,
    pub noise_attenuation_db: f64,
    pub speech_projection_gain_db: f64,
    pub peak_absolute: f64,
    pub clipped_samples: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct AlignedQualityMetrics {
    pub raw_unaligned: QualityMetrics,
    pub deterministic_latency_compensated: QualityMetrics,
    pub offset_samples: usize,
    pub offset_ms: f64,
    pub offset_source: String,
    pub alignment_method: &'static str,
}

pub(crate) fn process_offline<P: FrameProcessor>(
    input: &[f32],
    mut processor: P,
) -> (Vec<f32>, RuntimeMetrics) {
    let mut output = Vec::with_capacity(input.len());
    let mut frame_times_ns = Vec::with_capacity(input.len().div_ceil(FRAME_SAMPLES));
    let total_started = Instant::now();
    for input_frame in input.chunks(FRAME_SAMPLES) {
        let mut frame = [0.0_f32; FRAME_SAMPLES];
        frame[..input_frame.len()].copy_from_slice(input_frame);
        let frame_started = Instant::now();
        processor.process(&mut frame);
        frame_times_ns.push(elapsed_ns(frame_started));
        output.extend_from_slice(&frame[..input_frame.len()]);
    }
    let total_ns = elapsed_ns(total_started);
    frame_times_ns.sort_unstable();
    let frames = frame_times_ns.len();
    let audio_ns = input.len() as f64 / SAMPLE_RATE_HZ as f64 * 1_000_000_000.0;
    let runtime = RuntimeMetrics {
        frames,
        total_ms: total_ns as f64 / 1_000_000.0,
        average_frame_us: frame_times_ns.iter().sum::<u64>() as f64 / frames as f64 / 1_000.0,
        p50_frame_us: percentile_ns(&frame_times_ns, 0.50) / 1_000.0,
        p95_frame_us: percentile_ns(&frame_times_ns, 0.95) / 1_000.0,
        p99_frame_us: percentile_ns(&frame_times_ns, 0.99) / 1_000.0,
        max_frame_us: *frame_times_ns.last().unwrap_or(&0) as f64 / 1_000.0,
        realtime_factor: total_ns as f64 / audio_ns,
    };
    (output, runtime)
}

pub(crate) fn quality(reference: &[f32], noise: &[f32], output: &[f32]) -> QualityMetrics {
    let residual: Vec<f32> = output
        .iter()
        .zip(reference)
        .map(|(&estimate, &speech)| estimate - speech)
        .collect();
    let projection = projection_gain(reference, output);
    QualityMetrics {
        si_sdr_db: si_sdr(reference, output),
        sdr_db: sdr(reference, output),
        measured_input_snr_db: snr_db_from_components(reference, noise),
        noise_attenuation_db: db_ratio(energy(noise), energy(&residual)),
        speech_projection_gain_db: 20.0 * projection.abs().max(f64::EPSILON).log10(),
        peak_absolute: output
            .iter()
            .map(|sample| f64::from(sample.abs()))
            .fold(0.0, f64::max),
        clipped_samples: output.iter().filter(|sample| sample.abs() >= 0.999).count(),
    }
}

pub(crate) fn quality_aligned(
    reference: &[f32],
    noise: &[f32],
    output: &[f32],
    deterministic_delay_samples: usize,
    offset_source: &str,
) -> AlignedQualityMetrics {
    let compared_len = reference
        .len()
        .min(noise.len())
        .min(output.len().saturating_sub(deterministic_delay_samples));
    let compensated_reference = &reference[..compared_len];
    let compensated_noise = &noise[..compared_len];
    let compensated_output =
        &output[deterministic_delay_samples..deterministic_delay_samples + compared_len];
    AlignedQualityMetrics {
        raw_unaligned: quality(reference, noise, output),
        deterministic_latency_compensated: quality(
            compensated_reference,
            compensated_noise,
            compensated_output,
        ),
        offset_samples: deterministic_delay_samples,
        offset_ms: deterministic_delay_samples as f64 / SAMPLE_RATE_HZ as f64 * 1_000.0,
        offset_source: offset_source.to_owned(),
        alignment_method: "trim output prefix by offset; truncate reference/noise tail; no signal-derived search",
    }
}

pub(crate) fn rms(samples: &[f32]) -> f64 {
    (energy(samples) / samples.len() as f64).sqrt()
}

pub(crate) fn snr_db_from_components(clean: &[f32], noise: &[f32]) -> f64 {
    db_ratio(energy(clean), energy(noise))
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn percentile_ns(sorted: &[u64], quantile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * quantile).ceil() as usize;
    sorted[index] as f64
}

fn energy(samples: &[f32]) -> f64 {
    samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum()
}

fn dot(left: &[f32], right: &[f32]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(&left, &right)| f64::from(left) * f64::from(right))
        .sum()
}

fn projection_gain(reference: &[f32], estimate: &[f32]) -> f64 {
    dot(reference, estimate) / energy(reference).max(f64::EPSILON)
}

fn si_sdr(reference: &[f32], estimate: &[f32]) -> f64 {
    let alpha = projection_gain(reference, estimate);
    let mut target_energy = 0.0_f64;
    let mut residual_energy = 0.0_f64;
    for (&reference_sample, &estimate_sample) in reference.iter().zip(estimate) {
        let target = alpha * f64::from(reference_sample);
        let residual = f64::from(estimate_sample) - target;
        target_energy += target * target;
        residual_energy += residual * residual;
    }
    db_ratio(target_energy, residual_energy)
}

fn sdr(reference: &[f32], estimate: &[f32]) -> f64 {
    let error_energy = reference
        .iter()
        .zip(estimate)
        .map(|(&reference, &estimate)| {
            let error = f64::from(reference) - f64::from(estimate);
            error * error
        })
        .sum();
    db_ratio(energy(reference), error_energy)
}

fn db_ratio(numerator: f64, denominator: f64) -> f64 {
    10.0 * (numerator.max(f64::EPSILON) / denominator.max(f64::EPSILON)).log10()
}

#[cfg(test)]
mod tests {
    use auralis_core::Passthrough;

    use super::{percentile_ns, process_offline, quality_aligned, si_sdr};
    use crate::fixtures::{generate_clean, generate_noise, mix_at_snr};

    #[test]
    fn offline_passthrough_is_sample_exact() {
        let input = generate_clean(1_001);
        let (output, runtime) = process_offline(&input, Passthrough);
        assert_eq!(input, output);
        assert_eq!(runtime.frames, 3);
    }

    #[test]
    fn perfect_estimate_has_higher_si_sdr_than_noisy_estimate() {
        let clean = generate_clean(4_800);
        let noise = generate_noise(4_800);
        let mixed = mix_at_snr(&clean, &noise, 0.0).expect("mix should succeed");
        assert!(
            si_sdr(&mixed.reference, &mixed.reference) > si_sdr(&mixed.reference, &mixed.mixture)
        );
    }

    #[test]
    fn percentile_uses_nearest_rank_upward() {
        let values = [1, 2, 3, 4, 5];
        assert_eq!(percentile_ns(&values, 0.50), 3.0);
        assert_eq!(percentile_ns(&values, 0.95), 5.0);
    }

    #[test]
    fn deterministic_delay_alignment_does_not_search_signal() {
        let reference = generate_clean(4_800);
        let noise = vec![0.0; reference.len()];
        let mut delayed = vec![0.0; reference.len()];
        delayed[480..].copy_from_slice(&reference[..reference.len() - 480]);
        let metrics = quality_aligned(&reference, &noise, &delayed, 480, "declared test delay");
        assert_eq!(metrics.offset_samples, 480);
        assert!(
            metrics.deterministic_latency_compensated.sdr_db > metrics.raw_unaligned.sdr_db + 40.0
        );
    }
}
