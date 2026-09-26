//! Model-runtime adapters kept outside the platform-independent audio core.

#![forbid(unsafe_code)]

mod deepfilter;
mod profile;
mod rnnoise;
mod timing;
mod ul_unas;

pub use profile::{EnhancementEngine, EnhancementProfile, EnhancementProfileMetadata};
pub use rnnoise::{RNNOISE_CANDIDATE_ID, RnnoiseFrameProcessor, RnnoiseLoadError};
pub use timing::{InferenceTimingHandle, InferenceTimingSnapshot};
pub use ul_unas::{
    UL_UNAS_CANDIDATE_ID, UL_UNAS_MODEL_SHA256, UlUnasDenoiser, UlUnasFrameProcessor,
    UlUnasLoadError,
};

pub const ORT_CRATE_VERSION: &str = "2.0.0-rc.13";
pub const ONNX_RUNTIME_VERSION: &str = "1.28";

pub fn onnx_runtime_build_info() -> &'static str {
    ort::info()
}
pub use deepfilter::{
    DEEPFILTER_CANDIDATE_ID, DEEPFILTER_MODEL_SHA256, DEEPFILTER_SOURCE_REVISION,
    DeepFilterFrameProcessor, DeepFilterLoadError,
};
