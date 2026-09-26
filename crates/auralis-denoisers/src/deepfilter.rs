use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Write as _};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use auralis_core::{
    Denoiser, DenoiserError, DenoiserMetadata, FRAME_SAMPLES, FrameProcessor, SAMPLE_RATE_HZ,
};
use df::tract::{DfParams, DfTract, ReduceMask, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};
use sha2::{Digest, Sha256};

use crate::InferenceTimingHandle;

pub const DEEPFILTER_MODEL_SHA256: &str =
    "5998e58e8ba0e09bb76986ef97b84afa065a571ef282d4a1222f341e3251cf3a";
pub const DEEPFILTER_SOURCE_REVISION: &str = "d375b2d8309e0935d165700c91da9de862a99c31";
pub const DEEPFILTER_CANDIDATE_ID: &str = "deepfilternet3-ll-official";

static NEXT_PROCESSOR_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static WORKER_ENGINES: RefCell<HashMap<u64, DfTract>> = RefCell::new(HashMap::new());
}

#[derive(Debug)]
pub struct DeepFilterLoadError(String);

impl fmt::Display for DeepFilterLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for DeepFilterLoadError {}

/// Worker-only adapter for the pinned official DeepFilterNet3 low-latency model.
pub struct DeepFilterFrameProcessor {
    id: u64,
    model: Arc<[u8]>,
    output: [f32; FRAME_SAMPLES],
    timing: InferenceTimingHandle,
}

impl DeepFilterFrameProcessor {
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, DeepFilterLoadError> {
        let model = fs::read(model_path.as_ref())
            .map_err(|error| DeepFilterLoadError(format!("read DeepFilterNet model: {error}")))?;
        let mut actual_hash = String::with_capacity(64);
        for byte in Sha256::digest(&model) {
            write!(&mut actual_hash, "{byte:02x}").expect("writing to String cannot fail");
        }
        if actual_hash != DEEPFILTER_MODEL_SHA256 {
            return Err(DeepFilterLoadError(format!(
                "DeepFilterNet model SHA-256 mismatch: expected {DEEPFILTER_MODEL_SHA256}, got {actual_hash}"
            )));
        }
        Ok(Self {
            id: NEXT_PROCESSOR_ID.fetch_add(1, Ordering::Relaxed),
            model: model.into(),
            output: [0.0; FRAME_SAMPLES],
            timing: InferenceTimingHandle::default(),
        })
    }

    pub fn timing_handle(&self) -> InferenceTimingHandle {
        self.timing.clone()
    }

    fn process_native(
        id: u64,
        model: &[u8],
        timing: &InferenceTimingHandle,
        input: &[f32; FRAME_SAMPLES],
        output: &mut [f32; FRAME_SAMPLES],
    ) -> Result<(), DenoiserError> {
        let started = Instant::now();
        let result = process_frame(id, model, input, output);
        timing.record_ns(elapsed_ns(started));
        result.map_err(|_| DenoiserError::RuntimeFailure("DeepFilterNet inference failed"))
    }

    fn reset_state(&mut self) {
        remove_engine(self.id);
        self.output.fill(0.0);
        self.timing.reset();
    }
}

impl Denoiser for DeepFilterFrameProcessor {
    fn metadata(&self) -> DenoiserMetadata {
        DenoiserMetadata {
            candidate_id: DEEPFILTER_CANDIDATE_ID,
            native_sample_rate_hz: SAMPLE_RATE_HZ,
            frame_size_samples: 960,
            hop_size_samples: FRAME_SAMPLES,
            lookahead_samples: 0,
            algorithmic_latency_samples: 480,
            stateful: true,
            reset_semantics: "remove worker-local engine and clear output buffer",
        }
    }

    fn initialize(&mut self) -> Result<(), DenoiserError> {
        self.metadata().validate()?;
        self.prepare()
            .map_err(|_| DenoiserError::RuntimeFailure("DeepFilterNet initialization failed"))
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), DenoiserError> {
        let input: &[f32; FRAME_SAMPLES] = input
            .try_into()
            .map_err(|_| DenoiserError::InvalidBufferLength)?;
        let output: &mut [f32; FRAME_SAMPLES] = output
            .try_into()
            .map_err(|_| DenoiserError::InvalidBufferLength)?;
        Self::process_native(self.id, &self.model, &self.timing, input, output)
    }

    fn reset(&mut self) -> Result<(), DenoiserError> {
        self.reset_state();
        Ok(())
    }
}

impl FrameProcessor for DeepFilterFrameProcessor {
    fn name(&self) -> &'static str {
        DEEPFILTER_CANDIDATE_ID
    }

    fn algorithmic_latency_samples(&self) -> usize {
        480
    }

    fn reset(&mut self) {
        self.reset_state();
    }

    fn prepare(&mut self) -> Result<(), String> {
        ensure_engine(self.id, &self.model)
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        let input = *samples;
        if Self::process_native(self.id, &self.model, &self.timing, &input, &mut self.output)
            .is_ok()
        {
            samples.copy_from_slice(&self.output);
        } else {
            self.timing.record_failure();
            samples.fill(0.0);
        }
    }
}

impl Drop for DeepFilterFrameProcessor {
    fn drop(&mut self) {
        remove_engine(self.id);
    }
}

fn ensure_engine(id: u64, model: &[u8]) -> Result<(), String> {
    WORKER_ENGINES.with(|engines| {
        let mut engines = engines.borrow_mut();
        if engines.contains_key(&id) {
            return Ok(());
        }
        let params = DfParams::from_bytes(model)
            .map_err(|error| format!("load DeepFilterNet model: {error}"))?;
        // Match the official deep-filter CLI defaults used for the frozen bake-off.
        let runtime = RuntimeParams::default()
            .with_atten_lim(100.0)
            .with_thresholds(-15.0, 35.0, 35.0)
            .with_mask_reduce(ReduceMask::MAX);
        let engine = DfTract::new(params, &runtime)
            .map_err(|error| format!("initialize DeepFilterNet: {error}"))?;
        if engine.sr != SAMPLE_RATE_HZ as usize
            || engine.hop_size != FRAME_SAMPLES
            || engine.fft_size != 960
            || engine.lookahead != 0
        {
            return Err(format!(
                "unexpected DeepFilterNet contract: sample_rate={} hop={} fft={} lookahead={}",
                engine.sr, engine.hop_size, engine.fft_size, engine.lookahead
            ));
        }
        engines.insert(id, engine);
        Ok(())
    })
}

fn process_frame(
    id: u64,
    model: &[u8],
    samples: &[f32; FRAME_SAMPLES],
    output: &mut [f32; FRAME_SAMPLES],
) -> Result<(), String> {
    ensure_engine(id, model)?;
    WORKER_ENGINES.with(|engines| {
        let mut engines = engines.borrow_mut();
        let engine = engines
            .get_mut(&id)
            .ok_or_else(|| "DeepFilterNet worker engine is missing".to_owned())?;
        process_engine(engine, samples, output)
    })
}

fn process_engine(
    engine: &mut DfTract,
    samples: &[f32; FRAME_SAMPLES],
    output: &mut [f32; FRAME_SAMPLES],
) -> Result<(), String> {
    let input = ArrayView2::from_shape((1, FRAME_SAMPLES), samples.as_slice())
        .map_err(|error| format!("DeepFilterNet input shape: {error}"))?;
    let output = ArrayViewMut2::from_shape((1, FRAME_SAMPLES), output.as_mut_slice())
        .map_err(|error| format!("DeepFilterNet output shape: {error}"))?;
    engine
        .process(input, output)
        .map(|_| ())
        .map_err(|error| format!("DeepFilterNet inference: {error}"))
}

fn remove_engine(id: u64) {
    WORKER_ENGINES.with(|engines| {
        engines.borrow_mut().remove(&id);
    });
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::env;

    use auralis_core::{FRAME_SAMPLES, FrameProcessor};

    use super::DeepFilterFrameProcessor;

    #[test]
    #[ignore = "requires AURALIS_DEEPFILTER_MODEL pointing to the pinned local model archive"]
    fn official_model_runs_and_resets_deterministically() {
        let model = env::var_os("AURALIS_DEEPFILTER_MODEL").expect("model path is required");
        let mut processor = DeepFilterFrameProcessor::load(model).expect("load DeepFilterNet");
        assert_eq!(processor.algorithmic_latency_samples(), 480);
        let input = std::array::from_fn::<_, FRAME_SAMPLES, _>(|index| {
            0.12 * (index as f32 * 0.03125).sin() + 0.04 * (index as f32 * 0.117).cos()
        });
        let mut first = input;
        let mut second = input;
        processor.process(&mut first);
        processor.process(&mut second);
        processor.reset();
        let mut first_after_reset = input;
        let mut second_after_reset = input;
        processor.process(&mut first_after_reset);
        processor.process(&mut second_after_reset);
        assert_eq!(first, first_after_reset);
        assert_eq!(second, second_after_reset);
        assert_eq!(processor.timing_handle().snapshot().failure_count, 0);
    }
}
