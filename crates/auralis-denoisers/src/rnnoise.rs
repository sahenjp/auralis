use std::error::Error;
use std::fmt;
use std::path::Path;
use std::time::Instant;

use auralis_core::{
    Denoiser, DenoiserError, DenoiserMetadata, FRAME_SAMPLES, FrameProcessor, SAMPLE_RATE_HZ,
};
use auralis_rnnoise_sys::{LoadError as SysLoadError, RNNOISE_FRAME_SAMPLES, State};

use crate::InferenceTimingHandle;

const PCM_SCALE: f32 = 32_768.0;
pub const RNNOISE_CANDIDATE_ID: &str = "rnnoise-main-official-model";

#[derive(Debug)]
pub struct RnnoiseLoadError(SysLoadError);

impl fmt::Display for RnnoiseLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for RnnoiseLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<SysLoadError> for RnnoiseLoadError {
    fn from(error: SysLoadError) -> Self {
        Self(error)
    }
}

/// Worker-only adapter for a locally built, dynamically loaded RNNoise library.
pub struct RnnoiseFrameProcessor {
    state: State,
    input: [f32; RNNOISE_FRAME_SAMPLES],
    output: [f32; RNNOISE_FRAME_SAMPLES],
    timing: InferenceTimingHandle,
}

impl RnnoiseFrameProcessor {
    pub fn load(library_path: impl AsRef<Path>) -> Result<Self, RnnoiseLoadError> {
        if RNNOISE_FRAME_SAMPLES != FRAME_SAMPLES {
            return Err(RnnoiseLoadError(SysLoadError::UnexpectedFrameSize(
                RNNOISE_FRAME_SAMPLES as i32,
            )));
        }
        Ok(Self {
            state: State::load(library_path)?,
            input: [0.0; RNNOISE_FRAME_SAMPLES],
            output: [0.0; RNNOISE_FRAME_SAMPLES],
            timing: InferenceTimingHandle::default(),
        })
    }

    pub fn timing_handle(&self) -> InferenceTimingHandle {
        self.timing.clone()
    }

    fn process_native(
        &mut self,
        input: &[f32; RNNOISE_FRAME_SAMPLES],
        output: &mut [f32; RNNOISE_FRAME_SAMPLES],
    ) {
        for (destination, source) in self.input.iter_mut().zip(input.iter()) {
            *destination = *source * PCM_SCALE;
        }
        let started = Instant::now();
        self.state.process(&self.input, &mut self.output);
        self.timing.record_ns(elapsed_ns(started));
        for (destination, source) in output.iter_mut().zip(self.output.iter()) {
            *destination = *source / PCM_SCALE;
        }
    }

    fn reset_state(&mut self) -> Result<(), DenoiserError> {
        self.state
            .reset()
            .map_err(|_| DenoiserError::RuntimeFailure("RNNoise state reset failed"))?;
        self.input.fill(0.0);
        self.output.fill(0.0);
        self.timing.reset();
        Ok(())
    }
}

impl Denoiser for RnnoiseFrameProcessor {
    fn metadata(&self) -> DenoiserMetadata {
        DenoiserMetadata {
            candidate_id: RNNOISE_CANDIDATE_ID,
            native_sample_rate_hz: SAMPLE_RATE_HZ,
            frame_size_samples: 960,
            hop_size_samples: RNNOISE_FRAME_SAMPLES,
            lookahead_samples: 0,
            algorithmic_latency_samples: 960,
            stateful: true,
            reset_semantics: "replace native state and clear input/output buffers",
        }
    }

    fn initialize(&mut self) -> Result<(), DenoiserError> {
        self.metadata().validate()
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), DenoiserError> {
        let input: &[f32; RNNOISE_FRAME_SAMPLES] = input
            .try_into()
            .map_err(|_| DenoiserError::InvalidBufferLength)?;
        let output: &mut [f32; RNNOISE_FRAME_SAMPLES] = output
            .try_into()
            .map_err(|_| DenoiserError::InvalidBufferLength)?;
        self.process_native(input, output);
        Ok(())
    }

    fn reset(&mut self) -> Result<(), DenoiserError> {
        self.reset_state()
    }
}

impl FrameProcessor for RnnoiseFrameProcessor {
    fn name(&self) -> &'static str {
        RNNOISE_CANDIDATE_ID
    }

    fn algorithmic_latency_samples(&self) -> usize {
        // The current implementation uses a 960-sample analysis window and a
        // delayed spectrum. Structural latency and waveform-scoring alignment
        // remain separate measurements even when both are currently 960 samples.
        960
    }

    fn reset(&mut self) {
        if self.reset_state().is_err() {
            self.timing.record_failure();
        }
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        let input = *samples;
        self.process_native(&input, samples);
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use auralis_core::{FRAME_SAMPLES, FrameProcessor};

    use super::RnnoiseFrameProcessor;

    #[test]
    #[ignore = "requires AURALIS_RNNOISE_LIBRARY pointing to a verified local RNNoise build"]
    fn local_library_runs_and_resets_deterministically() {
        let library = env::var_os("AURALIS_RNNOISE_LIBRARY").expect("library path is required");
        let mut processor = RnnoiseFrameProcessor::load(library).expect("load RNNoise library");
        let input = std::array::from_fn::<_, FRAME_SAMPLES, _>(|index| {
            0.12 * (index as f32 * 0.03125).sin() + 0.04 * (index as f32 * 0.117).cos()
        });
        let mut first = input;
        let mut second = input;
        processor.process(&mut first);
        processor.process(&mut second);
        assert!(first.iter().all(|sample| sample.is_finite()));
        assert!(second.iter().all(|sample| sample.is_finite()));

        processor.reset();
        let mut first_after_reset = input;
        let mut second_after_reset = input;
        processor.process(&mut first_after_reset);
        processor.process(&mut second_after_reset);
        assert_eq!(first, first_after_reset);
        assert_eq!(second, second_after_reset);
        assert_eq!(processor.timing_handle().snapshot().failure_count, 0);
    }

    #[test]
    #[ignore = "requires verified AURALIS_RNNOISE_LIBRARY and AURALIS_RNNOISE_ALIGNMENT_WAV paths"]
    fn waveform_alignment_is_distinct_from_structural_latency() {
        const WARMUP_SAMPLES: usize = 48_000;
        const MAX_LAG_SAMPLES: usize = 1_440;

        let library = env::var_os("AURALIS_RNNOISE_LIBRARY").expect("library path is required");
        let wav_path = PathBuf::from(
            env::var_os("AURALIS_RNNOISE_ALIGNMENT_WAV").expect("alignment WAV path is required"),
        );
        let mut processor = RnnoiseFrameProcessor::load(library).expect("load RNNoise library");
        assert_eq!(processor.algorithmic_latency_samples(), 960);
        let mut reader = hound::WavReader::open(&wav_path).expect("open alignment WAV");
        let spec = reader.spec();
        assert_eq!(spec.channels, 1, "alignment WAV must be mono");
        assert_eq!(spec.sample_rate, 48_000, "alignment WAV must be 48 kHz");
        assert_eq!(
            spec.sample_format,
            hound::SampleFormat::Float,
            "alignment WAV must contain float samples"
        );
        assert_eq!(
            spec.bits_per_sample, 32,
            "alignment WAV must use f32 samples"
        );
        let mut input = reader
            .samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .expect("read alignment WAV samples");
        let complete_samples = input.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        input.truncate(complete_samples);
        assert!(
            input.len() > WARMUP_SAMPLES + MAX_LAG_SAMPLES,
            "alignment WAV is too short"
        );
        let mut output = vec![0.0_f32; input.len()];
        for frame_index in 0..input.len() / FRAME_SAMPLES {
            let start = frame_index * FRAME_SAMPLES;
            let mut frame = [0.0_f32; FRAME_SAMPLES];
            frame.copy_from_slice(&input[start..start + FRAME_SAMPLES]);
            processor.process(&mut frame);
            output[start..start + FRAME_SAMPLES].copy_from_slice(&frame);
        }

        let search_end = input.len() - MAX_LAG_SAMPLES;
        let mut best_lag = 0;
        let mut best_correlation = f64::NEG_INFINITY;
        for lag in 0..=MAX_LAG_SAMPLES {
            let mut dot = 0.0_f64;
            let mut input_energy = 0.0_f64;
            let mut output_energy = 0.0_f64;
            for index in (WARMUP_SAMPLES..search_end).step_by(4) {
                let input_sample = f64::from(input[index]);
                let output_sample = f64::from(output[index + lag]);
                dot += input_sample * output_sample;
                input_energy += input_sample * input_sample;
                output_energy += output_sample * output_sample;
            }
            let correlation = dot / (input_energy * output_energy).sqrt();
            if correlation > best_correlation {
                best_correlation = correlation;
                best_lag = lag;
            }
        }
        eprintln!(
            "RNNoise waveform alignment: {best_lag} samples, coefficient {best_correlation:.6}"
        );
        assert!(best_correlation > 0.5);
        assert_eq!(best_lag, 960);
    }
}
