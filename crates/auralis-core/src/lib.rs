//! Platform-independent realtime audio transport and processing contracts.

#![forbid(unsafe_code)]

mod denoiser;
mod drift;
mod frame;
mod metrics;
mod pipeline;

pub use denoiser::{Denoiser, DenoiserError, DenoiserMetadata};
pub use drift::DriftCorrectionConfig;
pub use frame::{FRAME_DURATION_NS, FRAME_SAMPLES, SAMPLE_RATE_HZ};
pub use metrics::{CadenceSummary, CallbackFrameCount, CallbackTiming, Metrics, MetricsSnapshot};
pub use pipeline::{
    CaptureEndpoint, FrameProcessor, Passthrough, PipelineConfig, PipelineParts, ProcessorWorker,
    RenderEndpoint, start_pipeline,
};
