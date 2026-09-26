//! Replaceable streaming-denoiser contract independent of any model runtime.

use std::error::Error;
use std::fmt;

use serde::Serialize;

/// Immutable signal and state contract for one denoiser implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DenoiserMetadata {
    pub candidate_id: &'static str,
    pub native_sample_rate_hz: u32,
    pub frame_size_samples: usize,
    pub hop_size_samples: usize,
    /// Future input samples required before producing the corresponding hop.
    /// A causal model reports zero even when its analysis window uses history.
    pub lookahead_samples: usize,
    /// Structural delay at `native_sample_rate_hz`, including framing and
    /// stateful buffering but excluding transport queues and inference time.
    pub algorithmic_latency_samples: usize,
    pub stateful: bool,
    pub reset_semantics: &'static str,
}

impl DenoiserMetadata {
    /// Reject incomplete contracts before an audio stream starts.
    pub fn validate(self) -> Result<(), DenoiserError> {
        if self.candidate_id.is_empty() {
            return Err(DenoiserError::InvalidMetadata("candidate_id is empty"));
        }
        if self.native_sample_rate_hz == 0 {
            return Err(DenoiserError::InvalidMetadata(
                "native_sample_rate_hz must be non-zero",
            ));
        }
        if self.frame_size_samples == 0 || self.hop_size_samples == 0 {
            return Err(DenoiserError::InvalidMetadata(
                "frame and hop sizes must be non-zero",
            ));
        }
        if self.hop_size_samples > self.frame_size_samples {
            return Err(DenoiserError::InvalidMetadata(
                "hop size must not exceed frame size",
            ));
        }
        if self.reset_semantics.is_empty() {
            return Err(DenoiserError::InvalidMetadata(
                "reset semantics must be explicit",
            ));
        }
        Ok(())
    }
}

/// Stable, allocation-free error surface for processing adapters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DenoiserError {
    InvalidMetadata(&'static str),
    NotInitialized,
    InvalidBufferLength,
    RuntimeFailure(&'static str),
}

impl fmt::Display for DenoiserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMetadata(message) => write!(formatter, "invalid metadata: {message}"),
            Self::NotInitialized => formatter.write_str("denoiser is not initialized"),
            Self::InvalidBufferLength => formatter.write_str("invalid denoiser buffer length"),
            Self::RuntimeFailure(message) => {
                write!(formatter, "denoiser runtime failure: {message}")
            }
        }
    }
}

impl Error for DenoiserError {}

/// Bounded streaming inference behind a model-independent interface.
///
/// `initialize` runs before the live stream and may load/validate model data.
/// `process` and `reset` run only outside audio callbacks. Implementations must
/// use bounded memory and must not allocate, perform I/O, or block in steady
/// state. Sample-rate conversion and framing adapters remain outside the model
/// implementation so their cost and delay can be measured independently.
pub trait Denoiser: Send + 'static {
    fn metadata(&self) -> DenoiserMetadata;

    fn initialize(&mut self) -> Result<(), DenoiserError>;

    /// Consume one native hop and write one native output hop.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), DenoiserError>;

    /// Return recurrent, framing, and model state to the documented initial state.
    fn reset(&mut self) -> Result<(), DenoiserError>;
}

#[cfg(test)]
mod tests {
    use super::{DenoiserError, DenoiserMetadata};

    #[test]
    fn valid_streaming_contract_is_accepted() {
        let metadata = DenoiserMetadata {
            candidate_id: "fixture",
            native_sample_rate_hz: 48_000,
            frame_size_samples: 960,
            hop_size_samples: 480,
            lookahead_samples: 0,
            algorithmic_latency_samples: 480,
            stateful: true,
            reset_semantics: "clear all history",
        };
        assert_eq!(metadata.validate(), Ok(()));
    }

    #[test]
    fn incomplete_contract_is_rejected() {
        let metadata = DenoiserMetadata {
            candidate_id: "fixture",
            native_sample_rate_hz: 16_000,
            frame_size_samples: 256,
            hop_size_samples: 512,
            lookahead_samples: 0,
            algorithmic_latency_samples: 0,
            stateful: true,
            reset_semantics: "clear caches",
        };
        assert_eq!(
            metadata.validate(),
            Err(DenoiserError::InvalidMetadata(
                "hop size must not exceed frame size"
            ))
        );
    }
}
