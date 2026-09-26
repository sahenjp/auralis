//! Slow, bounded output-rate correction for independently clocked duplex devices.

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{
    Adjustable, Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction,
};
use serde::Serialize;

use crate::{FRAME_SAMPLES, Metrics};

const CONTROL_INTERVAL_FRAMES: usize = 100;
const FILTER_ALPHA: f64 = 0.05;
const PROPORTIONAL_GAIN_PPM_PER_SAMPLE: f64 = 0.5;
const INTEGRAL_GAIN_PPM_PER_SAMPLE_SECOND: f64 = 0.002;
const SINC_LENGTH: usize = 128;

/// Construction-time policy for bounded asynchronous clock correction.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct DriftCorrectionConfig {
    /// Enable the worker-side resampler and fill controller.
    pub enabled: bool,
    /// Desired total buffered audio, including the render adapter's partial frame.
    pub target_fill_frames: f64,
    /// Absolute ratio correction limit in parts per million.
    pub maximum_correction_ppm: f64,
}

impl Default for DriftCorrectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_fill_frames: 3.5,
            maximum_correction_ppm: 250.0,
        }
    }
}

pub(crate) struct AdaptiveResampler {
    resampler: Async<f32>,
    input: Vec<Vec<f32>>,
    output: Vec<Vec<f32>>,
    controller: FillController,
    grace_frames: usize,
}

impl AdaptiveResampler {
    pub(crate) fn new(
        config: DriftCorrectionConfig,
        queue_capacity_frames: usize,
    ) -> Result<Self, String> {
        if !config.maximum_correction_ppm.is_finite()
            || !(1.0..=1_000.0).contains(&config.maximum_correction_ppm)
        {
            return Err("maximum drift correction must be in 1..=1000 ppm".to_owned());
        }
        if !config.target_fill_frames.is_finite() || config.target_fill_frames <= 0.0 {
            return Err("drift target fill must be positive".to_owned());
        }

        let maximum_target = queue_capacity_frames as f64;
        let target_samples = config.target_fill_frames.min(maximum_target) * FRAME_SAMPLES as f64;
        let maximum_relative_ratio = 1.0 / (1.0 - config.maximum_correction_ppm / 1_000_000.0);
        let parameters =
            SincInterpolationParameters::new(SINC_LENGTH, WindowFunction::BlackmanHarris2);
        let resampler = Async::new_sinc(
            1.0,
            maximum_relative_ratio,
            &parameters,
            FRAME_SAMPLES,
            1,
            FixedAsync::Input,
        )
        .map_err(|error| error.to_string())?;
        let input = vec![vec![0.0; FRAME_SAMPLES]];
        let output = vec![vec![0.0; resampler.output_frames_max()]];

        Ok(Self {
            resampler,
            input,
            output,
            controller: FillController::new(target_samples, config.maximum_correction_ppm),
            grace_frames: 0,
        })
    }

    pub(crate) fn output_delay_samples(&self) -> usize {
        self.resampler.output_delay()
    }

    pub(crate) fn process_frame(
        &mut self,
        samples: &[f32; FRAME_SAMPLES],
        buffered_samples: usize,
        pre_roll_complete: bool,
        metrics: &Metrics,
    ) -> &[f32] {
        self.input[0].copy_from_slice(samples);

        if pre_roll_complete {
            self.grace_frames = self.grace_frames.saturating_add(1);
            if self.grace_frames > CONTROL_INTERVAL_FRAMES * 2
                && let Some(ratio_ppm) = self.controller.update(buffered_samples as f64)
            {
                let ratio = 1.0 + ratio_ppm / 1_000_000.0;
                if self.resampler.set_resample_ratio(ratio, true).is_ok() {
                    metrics.drift_correction_ratio(ratio_ppm);
                } else {
                    metrics.drift_correction_error();
                }
            }
        }

        let input_adapter = SequentialSliceOfVecs::new(&self.input, 1, FRAME_SAMPLES)
            .expect("preallocated mono input adapter is valid");
        let output_capacity = self.output[0].len();
        let mut output_adapter =
            SequentialSliceOfVecs::new_mut(&mut self.output, 1, output_capacity)
                .expect("preallocated mono output adapter is valid");
        match self
            .resampler
            .process_into_buffer(&input_adapter, &mut output_adapter, None)
        {
            Ok((consumed, produced)) if consumed == FRAME_SAMPLES => &self.output[0][..produced],
            Ok(_) | Err(_) => {
                metrics.drift_correction_error();
                self.output[0][..FRAME_SAMPLES].copy_from_slice(samples);
                &self.output[0][..FRAME_SAMPLES]
            }
        }
    }
}

#[derive(Clone, Debug)]
struct FillController {
    target_samples: f64,
    maximum_correction_ppm: f64,
    filtered_error_samples: f64,
    integral_sample_seconds: f64,
    accumulated_fill_samples: f64,
    accumulated_frames: usize,
}

impl FillController {
    fn new(target_samples: f64, maximum_correction_ppm: f64) -> Self {
        Self {
            target_samples,
            maximum_correction_ppm,
            filtered_error_samples: 0.0,
            integral_sample_seconds: 0.0,
            accumulated_fill_samples: 0.0,
            accumulated_frames: 0,
        }
    }

    fn update(&mut self, fill_samples: f64) -> Option<f64> {
        self.accumulated_fill_samples += fill_samples;
        self.accumulated_frames += 1;
        if self.accumulated_frames < CONTROL_INTERVAL_FRAMES {
            return None;
        }
        let average_fill = self.accumulated_fill_samples / self.accumulated_frames as f64;
        self.accumulated_fill_samples = 0.0;
        self.accumulated_frames = 0;

        let error = average_fill - self.target_samples;
        self.filtered_error_samples += FILTER_ALPHA * (error - self.filtered_error_samples);
        self.integral_sample_seconds += self.filtered_error_samples;

        let integral_limit = self.maximum_correction_ppm / INTEGRAL_GAIN_PPM_PER_SAMPLE_SECOND;
        self.integral_sample_seconds = self
            .integral_sample_seconds
            .clamp(-integral_limit, integral_limit);
        let correction = -(PROPORTIONAL_GAIN_PPM_PER_SAMPLE * self.filtered_error_samples
            + INTEGRAL_GAIN_PPM_PER_SAMPLE_SECOND * self.integral_sample_seconds);
        Some(correction.clamp(-self.maximum_correction_ppm, self.maximum_correction_ppm))
    }
}

#[cfg(test)]
mod tests {
    use super::{AdaptiveResampler, DriftCorrectionConfig, FillController};
    use crate::{FRAME_SAMPLES, Metrics};

    const SAMPLE_RATE: f64 = 48_000.0;
    const STEP_SECONDS: f64 = FRAME_SAMPLES as f64 / SAMPLE_RATE;

    #[test]
    fn correction_is_clamped() {
        let mut controller = FillController::new(1_680.0, 250.0);
        let mut correction = None;
        for _ in 0..100 {
            correction = controller.update(100_000.0);
        }
        assert_eq!(correction, Some(-250.0));
    }

    #[test]
    fn positive_and_negative_clock_mismatch_converge_and_stay_bounded() {
        for ppm in [-100.0, -50.0, -10.0, 10.0, 50.0, 100.0] {
            let (minimum, maximum, correction) = simulate(ppm, 2 * 60 * 60);
            assert!(minimum > 0.0, "{ppm} ppm reached underrun: {minimum}");
            assert!(maximum < 1_920.0, "{ppm} ppm reached overrun: {maximum}");
            assert!(
                (correction + ppm).abs() < 2.0,
                "{ppm} ppm converged to {correction} ppm"
            );
        }
    }

    #[test]
    fn sinc_resampler_uses_preallocated_buffers_for_small_ratio_changes() {
        let mut resampler = AdaptiveResampler::new(DriftCorrectionConfig::default(), 4)
            .expect("construct resampler");
        let metrics = Metrics::default();
        let input = [0.25; FRAME_SAMPLES];
        let mut produced = 0;
        for _ in 0..1_000 {
            produced += resampler.process_frame(&input, 1_440, true, &metrics).len();
        }
        let delay = resampler.output_delay_samples();
        assert!(delay > 0);
        assert!(
            produced.abs_diff(1_000 * FRAME_SAMPLES) <= delay + FRAME_SAMPLES,
            "produced {produced} samples with {delay} samples of reported delay"
        );
    }

    fn simulate(source_ppm: f64, seconds: usize) -> (f64, f64, f64) {
        let target = 1_680.0;
        let mut controller = FillController::new(target, 250.0);
        let mut fill = target;
        let mut minimum = fill;
        let mut maximum = fill;
        let mut correction_ppm = 0.0;
        let steps = (seconds as f64 / STEP_SECONDS) as usize;
        let source_frames = FRAME_SAMPLES as f64 * (1.0 + source_ppm / 1_000_000.0);
        for _ in 0..steps {
            if let Some(correction) = controller.update(fill) {
                correction_ppm = correction;
            }
            fill += source_frames * (1.0 + correction_ppm / 1_000_000.0) - FRAME_SAMPLES as f64;
            minimum = minimum.min(fill);
            maximum = maximum.max(fill);
        }
        (minimum, maximum, correction_ppm)
    }
}
