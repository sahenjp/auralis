//! Fixed audio-frame definition used across all realtime stages.

/// Internal processing sample rate.
pub const SAMPLE_RATE_HZ: u32 = 48_000;

/// Samples in one 10 ms mono processing frame.
pub const FRAME_SAMPLES: usize = 480;

/// Duration of one processing frame in nanoseconds.
pub const FRAME_DURATION_NS: u64 = 10_000_000;

/// A fixed-capacity audio frame plus transport metadata.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AudioFrame {
    pub(crate) samples: [f32; FRAME_SAMPLES],
    pub(crate) sequence: u64,
    pub(crate) captured_at_ns: u64,
}

impl AudioFrame {
    pub(crate) fn silence(sequence: u64) -> Self {
        Self {
            samples: [0.0; FRAME_SAMPLES],
            sequence,
            captured_at_ns: 0,
        }
    }
}
