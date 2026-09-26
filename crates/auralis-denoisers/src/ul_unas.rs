use std::error::Error;
use std::ffi::CStr;
use std::fmt;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::time::Instant;

use auralis_core::{
    Denoiser, DenoiserError, DenoiserMetadata, FRAME_SAMPLES, FrameProcessor, SAMPLE_RATE_HZ,
};
use ort::session::{IoBinding, Session};
use ort::value::Tensor;
use ort::{AsPointer, Error as OrtError, ortsys};
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Fft, FixedSync, Resampler};
use sha2::{Digest, Sha256};

use crate::InferenceTimingHandle;

pub const UL_UNAS_MODEL_SHA256: &str =
    "f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b";
pub const UL_UNAS_CANDIDATE_ID: &str = "ul-unas-dns3-streaming-onnx";

const NATIVE_SAMPLE_RATE_HZ: usize = 16_000;
const FFT_SIZE: usize = 512;
const HOP_SIZE: usize = 256;
const SPECTRUM_BINS: usize = FFT_SIZE / 2 + 1;
const CONV_CACHE_SAMPLES: usize = 5_358;
const TFA_CACHE_SAMPLES: usize = 402;
const INTER_CACHE_SAMPLES: usize = 1_056;
const DOWNSAMPLED_FRAME_SAMPLES: usize = 160;
const UPSAMPLED_FRAME_SAMPLES: usize = FRAME_SAMPLES;
const NATIVE_FIFO_CAPACITY: usize = 1_024;

const MIX_NAME: &CStr = c"mix";
const CONV_CACHE_NAME: &CStr = c"conv_cache";
const TFA_CACHE_NAME: &CStr = c"tfa_cache";
const INTER_CACHE_NAME: &CStr = c"inter_cache";
const ENH_NAME: &CStr = c"enh";
const CONV_CACHE_OUT_NAME: &CStr = c"conv_cache_out";
const TFA_CACHE_OUT_NAME: &CStr = c"tfa_cache_out";
const INTER_CACHE_OUT_NAME: &CStr = c"inter_cache_out";

#[derive(Debug)]
pub enum UlUnasLoadError {
    ModelIo {
        path: PathBuf,
        source: io::Error,
    },
    ModelHashMismatch {
        expected: &'static str,
        actual: String,
    },
    Runtime(OrtError),
    Resampler(String),
}

impl fmt::Display for UlUnasLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelIo { path, source } => {
                write!(
                    formatter,
                    "failed to read model {}: {source}",
                    path.display()
                )
            }
            Self::ModelHashMismatch { expected, actual } => {
                write!(
                    formatter,
                    "model SHA-256 mismatch: expected {expected}, got {actual}"
                )
            }
            Self::Runtime(error) => {
                write!(formatter, "ONNX Runtime initialization failed: {error}")
            }
            Self::Resampler(error) => write!(formatter, "resampler initialization failed: {error}"),
        }
    }
}

impl Error for UlUnasLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ModelIo { source, .. } => Some(source),
            Self::Runtime(source) => Some(source),
            Self::ModelHashMismatch { .. } | Self::Resampler(_) => None,
        }
    }
}

impl From<OrtError> for UlUnasLoadError {
    fn from(error: OrtError) -> Self {
        Self::Runtime(error)
    }
}

/// Official UL-UNAS streaming ONNX graph plus its causal STFT/ISTFT frontend.
pub struct UlUnasDenoiser {
    session: Session,
    binding: IoBinding,
    mix: Tensor<f32>,
    conv_cache: Tensor<f32>,
    tfa_cache: Tensor<f32>,
    inter_cache: Tensor<f32>,
    enhanced: Tensor<f32>,
    conv_cache_out: Tensor<f32>,
    tfa_cache_out: Tensor<f32>,
    inter_cache_out: Tensor<f32>,
    fft_forward: Arc<dyn RealToComplex<f32>>,
    fft_inverse: Arc<dyn ComplexToReal<f32>>,
    fft_input: Vec<f32>,
    fft_spectrum: Vec<Complex32>,
    fft_output: Vec<f32>,
    fft_forward_scratch: Vec<Complex32>,
    fft_inverse_scratch: Vec<Complex32>,
    analysis_history: [f32; HOP_SIZE],
    synthesis_overlap: [f32; HOP_SIZE],
    window: [f32; FFT_SIZE],
    first_output_hop: bool,
    initialized: bool,
    timing: InferenceTimingHandle,
}

impl UlUnasDenoiser {
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, UlUnasLoadError> {
        let model_path = model_path.as_ref();
        verify_model_hash(model_path)?;

        let session = Session::builder()?
            .with_intra_threads(1)
            .map_err(builder_error)?
            .with_inter_threads(1)
            .map_err(builder_error)?
            .commit_from_file(model_path)?;
        validate_model_contract(&session)?;

        let mut binding = session.create_binding()?;
        let mix = zero_tensor([1_usize, SPECTRUM_BINS, 1, 2])?;
        let conv_cache = zero_tensor([1_usize, CONV_CACHE_SAMPLES])?;
        let tfa_cache = zero_tensor([1_usize, TFA_CACHE_SAMPLES])?;
        let inter_cache = zero_tensor([1_usize, INTER_CACHE_SAMPLES])?;
        let enhanced = zero_tensor([1_usize, SPECTRUM_BINS, 1, 2])?;
        let conv_cache_out = zero_tensor([1_usize, CONV_CACHE_SAMPLES])?;
        let tfa_cache_out = zero_tensor([1_usize, TFA_CACHE_SAMPLES])?;
        let inter_cache_out = zero_tensor([1_usize, INTER_CACHE_SAMPLES])?;

        bind_input(&mut binding, MIX_NAME, &mix)?;
        bind_input(&mut binding, CONV_CACHE_NAME, &conv_cache)?;
        bind_input(&mut binding, TFA_CACHE_NAME, &tfa_cache)?;
        bind_input(&mut binding, INTER_CACHE_NAME, &inter_cache)?;
        bind_output(&mut binding, ENH_NAME, &enhanced)?;
        bind_output(&mut binding, CONV_CACHE_OUT_NAME, &conv_cache_out)?;
        bind_output(&mut binding, TFA_CACHE_OUT_NAME, &tfa_cache_out)?;
        bind_output(&mut binding, INTER_CACHE_OUT_NAME, &inter_cache_out)?;

        let mut planner = RealFftPlanner::<f32>::new();
        let fft_forward = planner.plan_fft_forward(FFT_SIZE);
        let fft_inverse = planner.plan_fft_inverse(FFT_SIZE);
        let fft_input = fft_forward.make_input_vec();
        let fft_spectrum = fft_forward.make_output_vec();
        let fft_output = fft_inverse.make_output_vec();
        let fft_forward_scratch = fft_forward.make_scratch_vec();
        let fft_inverse_scratch = fft_inverse.make_scratch_vec();

        let mut denoiser = Self {
            session,
            binding,
            mix,
            conv_cache,
            tfa_cache,
            inter_cache,
            enhanced,
            conv_cache_out,
            tfa_cache_out,
            inter_cache_out,
            fft_forward,
            fft_inverse,
            fft_input,
            fft_spectrum,
            fft_output,
            fft_forward_scratch,
            fft_inverse_scratch,
            analysis_history: [0.0; HOP_SIZE],
            synthesis_overlap: [0.0; HOP_SIZE],
            window: periodic_hann_window(),
            first_output_hop: true,
            initialized: true,
            timing: InferenceTimingHandle::default(),
        };
        denoiser.warm_up()?;
        denoiser.reset_state();
        denoiser.timing.reset();
        Ok(denoiser)
    }

    pub fn timing_handle(&self) -> InferenceTimingHandle {
        self.timing.clone()
    }

    fn warm_up(&mut self) -> Result<(), UlUnasLoadError> {
        self.run_model().map_err(UlUnasLoadError::Runtime)
    }

    fn run_model(&mut self) -> Result<(), OrtError> {
        let started = Instant::now();
        run_bound_session(&mut self.session, &self.binding)?;
        self.timing.record_ns(elapsed_ns(started));
        copy_tensor(&self.conv_cache_out, &mut self.conv_cache);
        copy_tensor(&self.tfa_cache_out, &mut self.tfa_cache);
        copy_tensor(&self.inter_cache_out, &mut self.inter_cache);
        Ok(())
    }

    fn reset_state(&mut self) {
        zero_tensor_data(&mut self.mix);
        zero_tensor_data(&mut self.conv_cache);
        zero_tensor_data(&mut self.tfa_cache);
        zero_tensor_data(&mut self.inter_cache);
        zero_tensor_data(&mut self.enhanced);
        zero_tensor_data(&mut self.conv_cache_out);
        zero_tensor_data(&mut self.tfa_cache_out);
        zero_tensor_data(&mut self.inter_cache_out);
        self.fft_input.fill(0.0);
        self.fft_spectrum.fill(Complex32::new(0.0, 0.0));
        self.fft_output.fill(0.0);
        self.fft_forward_scratch.fill(Complex32::new(0.0, 0.0));
        self.fft_inverse_scratch.fill(Complex32::new(0.0, 0.0));
        self.analysis_history.fill(0.0);
        self.synthesis_overlap.fill(0.0);
        self.first_output_hop = true;
    }
}

impl Denoiser for UlUnasDenoiser {
    fn metadata(&self) -> DenoiserMetadata {
        DenoiserMetadata {
            candidate_id: UL_UNAS_CANDIDATE_ID,
            native_sample_rate_hz: NATIVE_SAMPLE_RATE_HZ as u32,
            frame_size_samples: FFT_SIZE,
            hop_size_samples: HOP_SIZE,
            // The streaming graph is causal. The one-hop structural delay is
            // caused by the 512-sample analysis/synthesis framing, not future
            // input lookahead.
            lookahead_samples: 0,
            algorithmic_latency_samples: HOP_SIZE,
            stateful: true,
            reset_semantics: "zero recurrent caches, analysis history, and synthesis overlap",
        }
    }

    fn initialize(&mut self) -> Result<(), DenoiserError> {
        self.metadata().validate()?;
        self.initialized = true;
        self.reset_state();
        Ok(())
    }

    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), DenoiserError> {
        if !self.initialized {
            return Err(DenoiserError::NotInitialized);
        }
        if input.len() != HOP_SIZE || output.len() != HOP_SIZE {
            return Err(DenoiserError::InvalidBufferLength);
        }

        for (index, input_sample) in input.iter().enumerate() {
            self.fft_input[index] = self.analysis_history[index] * self.window[index];
            self.fft_input[index + HOP_SIZE] = input_sample * self.window[index + HOP_SIZE];
        }
        self.analysis_history.copy_from_slice(input);
        self.fft_forward
            .process_with_scratch(
                &mut self.fft_input,
                &mut self.fft_spectrum,
                &mut self.fft_forward_scratch,
            )
            .map_err(|_| DenoiserError::RuntimeFailure("forward FFT failed"))?;

        {
            let (_, mix) = self.mix.extract_tensor_mut();
            for (bin, complex) in self.fft_spectrum.iter().enumerate() {
                mix[bin * 2] = complex.re;
                mix[bin * 2 + 1] = complex.im;
            }
        }
        if self.run_model().is_err() {
            self.timing.record_failure();
            return Err(DenoiserError::RuntimeFailure(
                "ONNX Runtime inference failed",
            ));
        }

        {
            let (_, enhanced) = self.enhanced.extract_tensor();
            for (bin, complex) in self.fft_spectrum.iter_mut().enumerate() {
                complex.re = enhanced[bin * 2];
                complex.im = enhanced[bin * 2 + 1];
            }
        }
        self.fft_inverse
            .process_with_scratch(
                &mut self.fft_spectrum,
                &mut self.fft_output,
                &mut self.fft_inverse_scratch,
            )
            .map_err(|_| DenoiserError::RuntimeFailure("inverse FFT failed"))?;

        for (index, output_sample) in output.iter_mut().enumerate() {
            let left_window = self.window[index];
            let right_window = self.window[index + HOP_SIZE];
            let normalization = left_window * left_window + right_window * right_window;
            let current = self.fft_output[index] * left_window / FFT_SIZE as f32;
            *output_sample = if self.first_output_hop {
                0.0
            } else {
                (self.synthesis_overlap[index] + current) / normalization.max(f32::EPSILON)
            };
            self.synthesis_overlap[index] =
                self.fft_output[index + HOP_SIZE] * right_window / FFT_SIZE as f32;
        }
        self.first_output_hop = false;
        Ok(())
    }

    fn reset(&mut self) -> Result<(), DenoiserError> {
        if !self.initialized {
            return Err(DenoiserError::NotInitialized);
        }
        self.reset_state();
        Ok(())
    }
}

/// Auralis 48 kHz/480-sample adapter around the native 16 kHz/256-sample model hop.
pub struct UlUnasFrameProcessor {
    denoiser: UlUnasDenoiser,
    downsampler: Fft<f32>,
    upsampler: Fft<f32>,
    downsample_input: Vec<Vec<f32>>,
    downsample_output: Vec<Vec<f32>>,
    upsample_input: Vec<Vec<f32>>,
    upsample_output: Vec<Vec<f32>>,
    model_input_fifo: FixedFifo<NATIVE_FIFO_CAPACITY>,
    model_output_fifo: FixedFifo<NATIVE_FIFO_CAPACITY>,
    native_input: [f32; HOP_SIZE],
    native_output: [f32; HOP_SIZE],
    output_armed: bool,
    output_started: bool,
    timing: InferenceTimingHandle,
}

impl UlUnasFrameProcessor {
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self, UlUnasLoadError> {
        let denoiser = UlUnasDenoiser::load(model_path)?;
        let timing = denoiser.timing_handle();
        let downsampler = Fft::new(
            SAMPLE_RATE_HZ as usize,
            NATIVE_SAMPLE_RATE_HZ,
            FRAME_SAMPLES,
            1,
            FixedSync::Both,
        )
        .map_err(|error| UlUnasLoadError::Resampler(error.to_string()))?;
        let upsampler = Fft::new(
            NATIVE_SAMPLE_RATE_HZ,
            SAMPLE_RATE_HZ as usize,
            DOWNSAMPLED_FRAME_SAMPLES,
            1,
            FixedSync::Both,
        )
        .map_err(|error| UlUnasLoadError::Resampler(error.to_string()))?;
        if downsampler.input_frames_next() != FRAME_SAMPLES
            || downsampler.output_frames_next() != DOWNSAMPLED_FRAME_SAMPLES
            || upsampler.input_frames_next() != DOWNSAMPLED_FRAME_SAMPLES
            || upsampler.output_frames_next() != UPSAMPLED_FRAME_SAMPLES
        {
            return Err(UlUnasLoadError::Resampler(
                "unexpected fixed resampler framing".to_owned(),
            ));
        }

        Ok(Self {
            denoiser,
            downsampler,
            upsampler,
            downsample_input: vec![vec![0.0; FRAME_SAMPLES]],
            downsample_output: vec![vec![0.0; DOWNSAMPLED_FRAME_SAMPLES]],
            upsample_input: vec![vec![0.0; DOWNSAMPLED_FRAME_SAMPLES]],
            upsample_output: vec![vec![0.0; UPSAMPLED_FRAME_SAMPLES]],
            model_input_fifo: FixedFifo::default(),
            model_output_fifo: FixedFifo::default(),
            native_input: [0.0; HOP_SIZE],
            native_output: [0.0; HOP_SIZE],
            output_armed: false,
            output_started: false,
            timing,
        })
    }

    pub fn timing_handle(&self) -> InferenceTimingHandle {
        self.timing.clone()
    }

    pub fn model_algorithmic_latency_samples_at_native_rate(&self) -> usize {
        self.denoiser.metadata().algorithmic_latency_samples
    }

    pub fn downsampler_delay_samples_at_native_rate(&self) -> usize {
        self.downsampler.output_delay()
    }

    pub fn upsampler_delay_samples_at_auralis_rate(&self) -> usize {
        self.upsampler.output_delay()
    }

    fn process_inner(&mut self, samples: &mut [f32; FRAME_SAMPLES]) -> Result<(), DenoiserError> {
        if self.output_armed && !self.output_started {
            self.output_started = self.model_output_fifo.len() >= HOP_SIZE;
        }
        self.downsample_input[0].copy_from_slice(samples);
        let input_adapter = SequentialSliceOfVecs::new(&self.downsample_input, 1, FRAME_SAMPLES)
            .map_err(|_| DenoiserError::RuntimeFailure("downsampler input adapter failed"))?;
        let mut output_adapter = SequentialSliceOfVecs::new_mut(
            &mut self.downsample_output,
            1,
            DOWNSAMPLED_FRAME_SAMPLES,
        )
        .map_err(|_| DenoiserError::RuntimeFailure("downsampler output adapter failed"))?;
        let (_, produced) = self
            .downsampler
            .process_into_buffer(&input_adapter, &mut output_adapter, None)
            .map_err(|_| DenoiserError::RuntimeFailure("downsampling failed"))?;
        if produced != DOWNSAMPLED_FRAME_SAMPLES
            || !self
                .model_input_fifo
                .push(&self.downsample_output[0][..produced])
        {
            return Err(DenoiserError::RuntimeFailure(
                "unexpected downsampler output framing",
            ));
        }

        if self.model_input_fifo.len() >= HOP_SIZE {
            if !self.model_input_fifo.pop(&mut self.native_input) {
                return Err(DenoiserError::RuntimeFailure("model input FIFO underflow"));
            }
            self.denoiser
                .process(&self.native_input, &mut self.native_output)?;
            if !self.model_output_fifo.push(&self.native_output) {
                return Err(DenoiserError::RuntimeFailure("model output FIFO overflow"));
            }
            self.output_armed = true;
        }

        self.upsample_input[0].fill(0.0);
        if self.output_started
            && !self
                .model_output_fifo
                .pop(&mut self.upsample_input[0][..DOWNSAMPLED_FRAME_SAMPLES])
        {
            return Err(DenoiserError::RuntimeFailure("model output FIFO underflow"));
        }

        let input_adapter =
            SequentialSliceOfVecs::new(&self.upsample_input, 1, DOWNSAMPLED_FRAME_SAMPLES)
                .map_err(|_| DenoiserError::RuntimeFailure("upsampler input adapter failed"))?;
        let mut output_adapter =
            SequentialSliceOfVecs::new_mut(&mut self.upsample_output, 1, UPSAMPLED_FRAME_SAMPLES)
                .map_err(|_| DenoiserError::RuntimeFailure("upsampler output adapter failed"))?;
        let (_, produced) = self
            .upsampler
            .process_into_buffer(&input_adapter, &mut output_adapter, None)
            .map_err(|_| DenoiserError::RuntimeFailure("upsampling failed"))?;
        if produced != FRAME_SAMPLES {
            return Err(DenoiserError::RuntimeFailure(
                "unexpected upsampler output framing",
            ));
        }
        samples.copy_from_slice(&self.upsample_output[0][..FRAME_SAMPLES]);
        Ok(())
    }
}

impl FrameProcessor for UlUnasFrameProcessor {
    fn name(&self) -> &'static str {
        UL_UNAS_CANDIDATE_ID
    }

    fn algorithmic_latency_samples(&self) -> usize {
        // Constrained correlation with the frozen model measures 2,208 samples:
        // one 16 ms model hop, 10 ms of resampler delay, and 20 ms from the
        // deterministic 48 kHz/256-hop phase adapter.
        2_208
    }

    fn reset(&mut self) {
        let _ = self.denoiser.reset();
        self.downsampler.reset();
        self.upsampler.reset();
        self.downsample_input[0].fill(0.0);
        self.downsample_output[0].fill(0.0);
        self.upsample_input[0].fill(0.0);
        self.upsample_output[0].fill(0.0);
        self.model_input_fifo.clear();
        self.model_output_fifo.clear();
        self.native_input.fill(0.0);
        self.native_output.fill(0.0);
        self.output_armed = false;
        self.output_started = false;
    }

    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]) {
        if self.process_inner(samples).is_err() {
            self.timing.record_failure();
            samples.fill(0.0);
        }
    }
}

#[derive(Clone)]
struct FixedFifo<const CAPACITY: usize> {
    samples: [f32; CAPACITY],
    head: usize,
    len: usize,
}

impl<const CAPACITY: usize> Default for FixedFifo<CAPACITY> {
    fn default() -> Self {
        Self {
            samples: [0.0; CAPACITY],
            head: 0,
            len: 0,
        }
    }
}

impl<const CAPACITY: usize> FixedFifo<CAPACITY> {
    fn len(&self) -> usize {
        self.len
    }

    fn push(&mut self, input: &[f32]) -> bool {
        if input.len() > CAPACITY.saturating_sub(self.len) {
            return false;
        }
        for sample in input {
            let index = (self.head + self.len) % CAPACITY;
            self.samples[index] = *sample;
            self.len += 1;
        }
        true
    }

    fn pop(&mut self, output: &mut [f32]) -> bool {
        if output.len() > self.len {
            return false;
        }
        for sample in output {
            *sample = self.samples[self.head];
            self.head = (self.head + 1) % CAPACITY;
            self.len -= 1;
        }
        true
    }

    fn clear(&mut self) {
        self.samples.fill(0.0);
        self.head = 0;
        self.len = 0;
    }
}

fn validate_model_contract(session: &Session) -> Result<(), UlUnasLoadError> {
    let input_names: Vec<_> = session.inputs().iter().map(|input| input.name()).collect();
    let output_names: Vec<_> = session
        .outputs()
        .iter()
        .map(|output| output.name())
        .collect();
    if input_names != ["mix", "conv_cache", "tfa_cache", "inter_cache"]
        || output_names != ["enh", "conv_cache_out", "tfa_cache_out", "inter_cache_out"]
    {
        return Err(UlUnasLoadError::Runtime(OrtError::new(
            "UL-UNAS ONNX input/output names do not match the frozen contract",
        )));
    }
    Ok(())
}

fn zero_tensor<const DIMENSIONS: usize>(
    shape: [usize; DIMENSIONS],
) -> Result<Tensor<f32>, OrtError> {
    let samples = shape.iter().product();
    Tensor::from_array((shape, vec![0.0_f32; samples].into_boxed_slice()))
}

fn bind_input(binding: &mut IoBinding, name: &CStr, value: &Tensor<f32>) -> Result<(), OrtError> {
    ortsys![unsafe BindInput(binding.ptr_mut(), name.as_ptr(), value.ptr())?];
    Ok(())
}

fn bind_output(binding: &mut IoBinding, name: &CStr, value: &Tensor<f32>) -> Result<(), OrtError> {
    ortsys![unsafe BindOutput(binding.ptr_mut(), name.as_ptr(), value.ptr())?];
    Ok(())
}

fn run_bound_session(session: &mut Session, binding: &IoBinding) -> Result<(), OrtError> {
    ortsys![unsafe RunWithBinding(session.ptr_mut(), ptr::null(), binding.ptr())?];
    Ok(())
}

fn copy_tensor(source: &Tensor<f32>, destination: &mut Tensor<f32>) {
    let (_, source) = source.extract_tensor();
    let (_, destination) = destination.extract_tensor_mut();
    destination.copy_from_slice(source);
}

fn zero_tensor_data(tensor: &mut Tensor<f32>) {
    tensor.extract_tensor_mut().1.fill(0.0);
}

fn periodic_hann_window() -> [f32; FFT_SIZE] {
    std::array::from_fn(|index| {
        0.5 - 0.5 * (std::f32::consts::TAU * index as f32 / FFT_SIZE as f32).cos()
    })
}

fn verify_model_hash(path: &Path) -> Result<(), UlUnasLoadError> {
    let mut file = File::open(path).map_err(|source| UlUnasLoadError::ModelIo {
        path: path.to_owned(),
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| UlUnasLoadError::ModelIo {
                path: path.to_owned(),
                source,
            })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let mut actual = String::with_capacity(64);
    for byte in digest.finalize() {
        write!(&mut actual, "{byte:02x}").expect("writing to String cannot fail");
    }
    if actual != UL_UNAS_MODEL_SHA256 {
        return Err(UlUnasLoadError::ModelHashMismatch {
            expected: UL_UNAS_MODEL_SHA256,
            actual,
        });
    }
    Ok(())
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn builder_error<R>(error: ort::Error<R>) -> UlUnasLoadError {
    UlUnasLoadError::Runtime(OrtError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::env;

    use auralis_core::{Denoiser, FRAME_SAMPLES, FrameProcessor, SAMPLE_RATE_HZ};

    use super::{
        FFT_SIZE, FixedFifo, HOP_SIZE, UlUnasDenoiser, UlUnasFrameProcessor, periodic_hann_window,
    };

    #[test]
    fn fixed_fifo_wraps_without_allocation() {
        let mut fifo = FixedFifo::<8>::default();
        assert!(fifo.push(&[1.0, 2.0, 3.0, 4.0, 5.0]));
        let mut first = [0.0; 3];
        assert!(fifo.pop(&mut first));
        assert_eq!(first, [1.0, 2.0, 3.0]);
        assert!(fifo.push(&[6.0, 7.0, 8.0, 9.0]));
        let mut second = [0.0; 6];
        assert!(fifo.pop(&mut second));
        assert_eq!(second, [4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn periodic_hann_matches_expected_endpoints() {
        let window = periodic_hann_window();
        assert_eq!(window[0], 0.0);
        assert!((window[FFT_SIZE / 2] - 1.0).abs() < 1.0e-7);
        assert!(window[FFT_SIZE - 1] > 0.0);
    }

    #[test]
    #[ignore = "requires AURALIS_UL_UNAS_MODEL pointing to the frozen official ONNX artifact"]
    fn official_model_runs_and_resets_deterministically() {
        let model_path = env::var_os("AURALIS_UL_UNAS_MODEL").expect("model path is required");
        let mut denoiser = UlUnasDenoiser::load(model_path).expect("load frozen model");
        let input = std::array::from_fn::<_, HOP_SIZE, _>(|index| {
            ((index as f32 * 0.03125).sin() + (index as f32 * 0.117).cos()) * 0.1
        });
        let mut first = [0.0; HOP_SIZE];
        let mut second = [0.0; HOP_SIZE];
        denoiser
            .process(&input, &mut first)
            .expect("first inference");
        denoiser
            .process(&input, &mut second)
            .expect("second inference");
        assert!(first.iter().all(|sample| *sample == 0.0));
        assert!(second.iter().all(|sample| sample.is_finite()));
        assert!(second.iter().any(|sample| sample.abs() > 1.0e-7));

        denoiser.reset().expect("reset model state");
        let mut first_after_reset = [0.0; HOP_SIZE];
        let mut second_after_reset = [0.0; HOP_SIZE];
        denoiser
            .process(&input, &mut first_after_reset)
            .expect("first inference after reset");
        denoiser
            .process(&input, &mut second_after_reset)
            .expect("second inference after reset");
        assert_eq!(first, first_after_reset);
        assert_eq!(second, second_after_reset);
    }

    #[test]
    #[ignore = "requires AURALIS_UL_UNAS_MODEL pointing to the frozen official ONNX artifact"]
    fn frame_adapter_runs_without_fifo_or_inference_failure() {
        let model_path = env::var_os("AURALIS_UL_UNAS_MODEL").expect("model path is required");
        let mut processor = UlUnasFrameProcessor::load(model_path).expect("load frozen model");
        let timing = processor.timing_handle();
        for frame_index in 0..100 {
            let mut frame = std::array::from_fn::<_, FRAME_SAMPLES, _>(|sample_index| {
                let index = frame_index * FRAME_SAMPLES + sample_index;
                (index as f32 * 0.013).sin() * 0.1
            });
            processor.process(&mut frame);
            assert!(frame.iter().all(|sample| sample.is_finite()));
        }
        let snapshot = timing.snapshot();
        assert!(snapshot.count >= 60);
        assert_eq!(snapshot.failure_count, 0);
    }

    #[test]
    #[ignore = "requires AURALIS_UL_UNAS_MODEL pointing to the frozen official ONNX artifact"]
    fn frame_adapter_latency_matches_reported_algorithmic_delay() {
        const FRAMES: usize = 300;
        const WARMUP_SAMPLES: usize = 48_000;
        const MAX_LAG_SAMPLES: usize = 4_800;

        let model_path = env::var_os("AURALIS_UL_UNAS_MODEL").expect("model path is required");
        let mut processor = UlUnasFrameProcessor::load(model_path).expect("load frozen model");
        let expected_lag = processor.algorithmic_latency_samples();
        let mut input = vec![0.0_f32; FRAMES * FRAME_SAMPLES];
        let mut output = vec![0.0_f32; FRAMES * FRAME_SAMPLES];

        for (index, sample) in input.iter_mut().enumerate() {
            let time = index as f32 / SAMPLE_RATE_HZ as f32;
            let envelope = 0.55 + 0.45 * (std::f32::consts::TAU * 3.7 * time).sin();
            *sample = envelope
                * (0.12 * (std::f32::consts::TAU * 173.0 * time).sin()
                    + 0.08 * (std::f32::consts::TAU * 311.0 * time).sin()
                    + 0.04 * (std::f32::consts::TAU * 997.0 * time).sin());
        }
        for frame_index in 0..FRAMES {
            let start = frame_index * FRAME_SAMPLES;
            let mut frame = [0.0_f32; FRAME_SAMPLES];
            frame.copy_from_slice(&input[start..start + FRAME_SAMPLES]);
            processor.process(&mut frame);
            output[start..start + FRAME_SAMPLES].copy_from_slice(&frame);
        }

        let search_end = input.len() - MAX_LAG_SAMPLES;
        let correlation_at = |lag: usize, stride: usize| {
            let mut dot = 0.0_f64;
            let mut input_energy = 0.0_f64;
            let mut output_energy = 0.0_f64;
            for index in (WARMUP_SAMPLES..search_end).step_by(stride) {
                let input_sample = f64::from(input[index]);
                let output_sample = f64::from(output[index + lag]);
                dot += input_sample * output_sample;
                input_energy += input_sample * input_sample;
                output_energy += output_sample * output_sample;
            }
            dot / (input_energy * output_energy).sqrt()
        };
        let mut coarse_lag = 0;
        let mut coarse_correlation = f64::NEG_INFINITY;
        for lag in (0..=MAX_LAG_SAMPLES).step_by(4) {
            let correlation = correlation_at(lag, 8);
            if correlation > coarse_correlation {
                coarse_correlation = correlation;
                coarse_lag = lag;
            }
        }
        let refine_start = coarse_lag.saturating_sub(8);
        let refine_end = (coarse_lag + 8).min(MAX_LAG_SAMPLES);
        let mut best_lag = refine_start;
        let mut best_correlation = f64::NEG_INFINITY;
        for lag in refine_start..=refine_end {
            let correlation = correlation_at(lag, 1);
            if correlation > best_correlation {
                best_correlation = correlation;
                best_lag = lag;
            }
        }

        eprintln!(
            "UL-UNAS constrained correlation lag: {best_lag} samples ({:.3} ms), coefficient {best_correlation:.6}",
            best_lag as f64 / SAMPLE_RATE_HZ as f64 * 1_000.0,
        );
        assert!(
            best_correlation > 0.5,
            "correlation is too weak to validate lag"
        );
        assert_eq!(best_lag, expected_lag);
    }
}
