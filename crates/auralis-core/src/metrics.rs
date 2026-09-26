//! Allocation-free atomic metrics written from realtime callbacks.

use std::array;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};

use serde::Serialize;

use crate::FRAME_DURATION_NS;

const CALLBACK_HISTOGRAM_SLOTS: usize = 32;

struct CallbackHistogram {
    frame_counts: [AtomicUsize; CALLBACK_HISTOGRAM_SLOTS],
    observations: [AtomicU64; CALLBACK_HISTOGRAM_SLOTS],
    overflow_observations: AtomicU64,
}

impl Default for CallbackHistogram {
    fn default() -> Self {
        Self {
            frame_counts: array::from_fn(|_| AtomicUsize::new(0)),
            observations: array::from_fn(|_| AtomicU64::new(0)),
            overflow_observations: AtomicU64::new(0),
        }
    }
}

impl CallbackHistogram {
    fn record(&self, frames: usize) {
        if frames == 0 {
            return;
        }

        let start = frames % CALLBACK_HISTOGRAM_SLOTS;
        for offset in 0..CALLBACK_HISTOGRAM_SLOTS {
            let slot = (start + offset) % CALLBACK_HISTOGRAM_SLOTS;
            let key = &self.frame_counts[slot];
            let existing = key.load(Ordering::Relaxed);
            if existing == frames
                || (existing == 0
                    && key
                        .compare_exchange(0, frames, Ordering::Relaxed, Ordering::Relaxed)
                        .is_ok())
            {
                self.observations[slot].fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        self.overflow_observations.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> (Vec<CallbackFrameCount>, u64) {
        let mut values = Vec::with_capacity(CALLBACK_HISTOGRAM_SLOTS);
        for slot in 0..CALLBACK_HISTOGRAM_SLOTS {
            let frames = self.frame_counts[slot].load(Ordering::Relaxed);
            let callbacks = self.observations[slot].load(Ordering::Relaxed);
            if frames != 0 && callbacks != 0 {
                values.push(CallbackFrameCount { frames, callbacks });
            }
        }
        values.sort_unstable_by_key(|value| value.frames);
        (values, self.overflow_observations.load(Ordering::Relaxed))
    }
}

struct Counters {
    captured_frames: AtomicU64,
    processed_frames: AtomicU64,
    rendered_frames: AtomicU64,
    input_overrun_frames: AtomicU64,
    output_overrun_frames: AtomicU64,
    output_underrun_callbacks: AtomicU64,
    output_underrun_samples: AtomicU64,
    stream_errors: AtomicU64,
    stream_xruns: AtomicU64,
    input_callback_calls: AtomicU64,
    input_callback_total_ns: AtomicU64,
    input_callback_max_ns: AtomicU64,
    input_callback_sample_frames: AtomicU64,
    input_first_callback_frames: AtomicU64,
    output_callback_calls: AtomicU64,
    output_callback_total_ns: AtomicU64,
    output_callback_max_ns: AtomicU64,
    output_callback_sample_frames: AtomicU64,
    output_first_callback_frames: AtomicU64,
    input_callback_timestamp_first_ns: AtomicU64,
    input_callback_timestamp_latest_ns: AtomicU64,
    input_device_timestamp_first_ns: AtomicU64,
    input_device_timestamp_latest_ns: AtomicU64,
    output_callback_timestamp_first_ns: AtomicU64,
    output_callback_timestamp_latest_ns: AtomicU64,
    output_device_timestamp_first_ns: AtomicU64,
    output_device_timestamp_latest_ns: AtomicU64,
    input_cadence_observations: AtomicU64,
    input_cadence_total_ns: AtomicU64,
    input_cadence_min_ns: AtomicU64,
    input_cadence_max_ns: AtomicU64,
    output_cadence_observations: AtomicU64,
    output_cadence_total_ns: AtomicU64,
    output_cadence_min_ns: AtomicU64,
    output_cadence_max_ns: AtomicU64,
    processing_total_ns: AtomicU64,
    processing_max_ns: AtomicU64,
    processing_deadline_misses: AtomicU64,
    input_queue_depth: AtomicUsize,
    input_queue_max_depth: AtomicUsize,
    output_queue_depth: AtomicUsize,
    output_queue_max_depth: AtomicUsize,
    output_buffered_samples: AtomicUsize,
    output_worker_buffered_samples: AtomicUsize,
    software_latency_observations: AtomicU64,
    software_latency_total_ns: AtomicU64,
    software_latency_max_ns: AtomicU64,
    startup_preroll_target_frames: AtomicUsize,
    startup_preroll_callbacks: AtomicU64,
    startup_preroll_samples: AtomicU64,
    startup_preroll_completed: AtomicU64,
    drift_correction_enabled: AtomicU64,
    drift_correction_ratio_milli_ppm: AtomicI64,
    drift_correction_min_milli_ppm: AtomicI64,
    drift_correction_max_milli_ppm: AtomicI64,
    drift_correction_errors: AtomicU64,
    drift_resampler_delay_samples: AtomicUsize,
    input_callback_histogram: CallbackHistogram,
    output_callback_histogram: CallbackHistogram,
}

impl Default for Counters {
    fn default() -> Self {
        Self {
            captured_frames: AtomicU64::new(0),
            processed_frames: AtomicU64::new(0),
            rendered_frames: AtomicU64::new(0),
            input_overrun_frames: AtomicU64::new(0),
            output_overrun_frames: AtomicU64::new(0),
            output_underrun_callbacks: AtomicU64::new(0),
            output_underrun_samples: AtomicU64::new(0),
            stream_errors: AtomicU64::new(0),
            stream_xruns: AtomicU64::new(0),
            input_callback_calls: AtomicU64::new(0),
            input_callback_total_ns: AtomicU64::new(0),
            input_callback_max_ns: AtomicU64::new(0),
            input_callback_sample_frames: AtomicU64::new(0),
            input_first_callback_frames: AtomicU64::new(0),
            output_callback_calls: AtomicU64::new(0),
            output_callback_total_ns: AtomicU64::new(0),
            output_callback_max_ns: AtomicU64::new(0),
            output_callback_sample_frames: AtomicU64::new(0),
            output_first_callback_frames: AtomicU64::new(0),
            input_callback_timestamp_first_ns: AtomicU64::new(0),
            input_callback_timestamp_latest_ns: AtomicU64::new(0),
            input_device_timestamp_first_ns: AtomicU64::new(0),
            input_device_timestamp_latest_ns: AtomicU64::new(0),
            output_callback_timestamp_first_ns: AtomicU64::new(0),
            output_callback_timestamp_latest_ns: AtomicU64::new(0),
            output_device_timestamp_first_ns: AtomicU64::new(0),
            output_device_timestamp_latest_ns: AtomicU64::new(0),
            input_cadence_observations: AtomicU64::new(0),
            input_cadence_total_ns: AtomicU64::new(0),
            input_cadence_min_ns: AtomicU64::new(u64::MAX),
            input_cadence_max_ns: AtomicU64::new(0),
            output_cadence_observations: AtomicU64::new(0),
            output_cadence_total_ns: AtomicU64::new(0),
            output_cadence_min_ns: AtomicU64::new(u64::MAX),
            output_cadence_max_ns: AtomicU64::new(0),
            processing_total_ns: AtomicU64::new(0),
            processing_max_ns: AtomicU64::new(0),
            processing_deadline_misses: AtomicU64::new(0),
            input_queue_depth: AtomicUsize::new(0),
            input_queue_max_depth: AtomicUsize::new(0),
            output_queue_depth: AtomicUsize::new(0),
            output_queue_max_depth: AtomicUsize::new(0),
            output_buffered_samples: AtomicUsize::new(0),
            output_worker_buffered_samples: AtomicUsize::new(0),
            software_latency_observations: AtomicU64::new(0),
            software_latency_total_ns: AtomicU64::new(0),
            software_latency_max_ns: AtomicU64::new(0),
            startup_preroll_target_frames: AtomicUsize::new(0),
            startup_preroll_callbacks: AtomicU64::new(0),
            startup_preroll_samples: AtomicU64::new(0),
            startup_preroll_completed: AtomicU64::new(0),
            drift_correction_enabled: AtomicU64::new(0),
            drift_correction_ratio_milli_ppm: AtomicI64::new(0),
            drift_correction_min_milli_ppm: AtomicI64::new(i64::MAX),
            drift_correction_max_milli_ppm: AtomicI64::new(i64::MIN),
            drift_correction_errors: AtomicU64::new(0),
            drift_resampler_delay_samples: AtomicUsize::new(0),
            input_callback_histogram: CallbackHistogram::default(),
            output_callback_histogram: CallbackHistogram::default(),
        }
    }
}

/// Timing supplied by an audio backend for one data callback.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallbackTiming {
    /// When the callback was invoked on the backend's monotonic clock.
    pub callback_ns: Option<u64>,
    /// Capture-at-ADC or playback-at-DAC estimate on the same stream clock.
    pub device_ns: Option<u64>,
}

/// Shared metrics handle. Updating it performs relaxed atomic operations only.
#[derive(Clone, Default)]
pub struct Metrics {
    counters: Arc<Counters>,
}

impl Metrics {
    pub(crate) fn captured_frame(&self) {
        self.counters
            .captured_frames
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn processed_frame(&self, elapsed_ns: u64) {
        self.counters
            .processed_frames
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .processing_total_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
        self.counters
            .processing_max_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
        if elapsed_ns > FRAME_DURATION_NS {
            self.counters
                .processing_deadline_misses
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn rendered_frame(&self) {
        self.counters
            .rendered_frames
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn input_overrun(&self) {
        self.counters
            .input_overrun_frames
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn output_overrun(&self) {
        self.counters
            .output_overrun_frames
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn output_underrun(&self, samples: u64) {
        self.counters
            .output_underrun_callbacks
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .output_underrun_samples
            .fetch_add(samples, Ordering::Relaxed);
    }

    /// Record one audio-backend stream failure.
    pub fn stream_error(&self, xrun: bool) {
        self.counters.stream_errors.fetch_add(1, Ordering::Relaxed);
        if xrun {
            self.counters.stream_xruns.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn input_callback(
        &self,
        elapsed_ns: u64,
        sample_frames: usize,
        timing: CallbackTiming,
    ) {
        self.counters
            .input_callback_calls
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .input_callback_total_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
        self.counters
            .input_callback_max_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
        self.counters
            .input_callback_sample_frames
            .fetch_add(sample_frames as u64, Ordering::Relaxed);
        set_first(
            &self.counters.input_first_callback_frames,
            sample_frames as u64,
        );
        self.counters.input_callback_histogram.record(sample_frames);
        record_timing(
            timing,
            &self.counters.input_callback_timestamp_first_ns,
            &self.counters.input_callback_timestamp_latest_ns,
            &self.counters.input_device_timestamp_first_ns,
            &self.counters.input_device_timestamp_latest_ns,
            &self.counters.input_cadence_observations,
            &self.counters.input_cadence_total_ns,
            &self.counters.input_cadence_min_ns,
            &self.counters.input_cadence_max_ns,
        );
    }

    pub(crate) fn output_callback(
        &self,
        elapsed_ns: u64,
        sample_frames: usize,
        timing: CallbackTiming,
    ) {
        self.counters
            .output_callback_calls
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .output_callback_total_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
        self.counters
            .output_callback_max_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
        self.counters
            .output_callback_sample_frames
            .fetch_add(sample_frames as u64, Ordering::Relaxed);
        set_first(
            &self.counters.output_first_callback_frames,
            sample_frames as u64,
        );
        self.counters
            .output_callback_histogram
            .record(sample_frames);
        record_timing(
            timing,
            &self.counters.output_callback_timestamp_first_ns,
            &self.counters.output_callback_timestamp_latest_ns,
            &self.counters.output_device_timestamp_first_ns,
            &self.counters.output_device_timestamp_latest_ns,
            &self.counters.output_cadence_observations,
            &self.counters.output_cadence_total_ns,
            &self.counters.output_cadence_min_ns,
            &self.counters.output_cadence_max_ns,
        );
    }

    pub(crate) fn input_queue_depth(&self, depth: usize) {
        self.counters
            .input_queue_depth
            .store(depth, Ordering::Relaxed);
        self.counters
            .input_queue_max_depth
            .fetch_max(depth, Ordering::Relaxed);
    }

    pub(crate) fn output_queue_depth(&self, depth: usize) {
        self.counters
            .output_queue_depth
            .store(depth, Ordering::Relaxed);
        self.counters
            .output_queue_max_depth
            .fetch_max(depth, Ordering::Relaxed);
    }

    pub(crate) fn output_buffered_samples(&self) -> usize {
        self.counters
            .output_buffered_samples
            .load(Ordering::Relaxed)
            .saturating_add(
                self.counters
                    .output_worker_buffered_samples
                    .load(Ordering::Relaxed),
            )
    }

    pub(crate) fn set_output_buffered_samples(&self, samples: usize) {
        self.counters
            .output_buffered_samples
            .store(samples, Ordering::Relaxed);
    }

    pub(crate) fn set_output_worker_buffered_samples(&self, samples: usize) {
        self.counters
            .output_worker_buffered_samples
            .store(samples, Ordering::Relaxed);
    }

    pub(crate) fn startup_preroll_is_complete(&self) -> bool {
        self.counters
            .startup_preroll_completed
            .load(Ordering::Relaxed)
            != 0
    }

    pub(crate) fn configure_drift_correction(&self, delay_samples: usize) {
        self.counters
            .drift_correction_enabled
            .store(1, Ordering::Relaxed);
        self.counters
            .drift_resampler_delay_samples
            .store(delay_samples, Ordering::Relaxed);
    }

    pub(crate) fn drift_correction_ratio(&self, ppm: f64) {
        let milli_ppm = (ppm * 1_000.0).round() as i64;
        self.counters
            .drift_correction_ratio_milli_ppm
            .store(milli_ppm, Ordering::Relaxed);
        self.counters
            .drift_correction_min_milli_ppm
            .fetch_min(milli_ppm, Ordering::Relaxed);
        self.counters
            .drift_correction_max_milli_ppm
            .fetch_max(milli_ppm, Ordering::Relaxed);
    }

    pub(crate) fn drift_correction_error(&self) {
        self.counters
            .drift_correction_errors
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn software_latency(&self, elapsed_ns: u64) {
        self.counters
            .software_latency_observations
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .software_latency_total_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
        self.counters
            .software_latency_max_ns
            .fetch_max(elapsed_ns, Ordering::Relaxed);
    }

    pub(crate) fn startup_preroll_target(&self, frames: usize) {
        self.counters
            .startup_preroll_target_frames
            .fetch_max(frames, Ordering::Relaxed);
    }

    pub(crate) fn startup_preroll_wait(&self, samples: usize) {
        self.counters
            .startup_preroll_callbacks
            .fetch_add(1, Ordering::Relaxed);
        self.counters
            .startup_preroll_samples
            .fetch_add(samples as u64, Ordering::Relaxed);
    }

    pub(crate) fn startup_preroll_complete(&self) {
        self.counters
            .startup_preroll_completed
            .store(1, Ordering::Relaxed);
    }

    /// Take a consistent-enough diagnostic snapshot without blocking writers.
    pub fn snapshot(&self) -> MetricsSnapshot {
        let load_u64 = |value: &AtomicU64| value.load(Ordering::Relaxed);
        let load_usize = |value: &AtomicUsize| value.load(Ordering::Relaxed);
        let processed_frames = load_u64(&self.counters.processed_frames);
        let processing_total_ns = load_u64(&self.counters.processing_total_ns);
        let input_callback_calls = load_u64(&self.counters.input_callback_calls);
        let output_callback_calls = load_u64(&self.counters.output_callback_calls);
        let software_latency_observations = load_u64(&self.counters.software_latency_observations);
        let input_cadence_observations = load_u64(&self.counters.input_cadence_observations);
        let output_cadence_observations = load_u64(&self.counters.output_cadence_observations);
        let (input_callback_frame_histogram, input_histogram_overflow_observations) =
            self.counters.input_callback_histogram.snapshot();
        let (output_callback_frame_histogram, output_histogram_overflow_observations) =
            self.counters.output_callback_histogram.snapshot();

        MetricsSnapshot {
            captured_frames: load_u64(&self.counters.captured_frames),
            processed_frames,
            rendered_frames: load_u64(&self.counters.rendered_frames),
            input_overrun_frames: load_u64(&self.counters.input_overrun_frames),
            output_overrun_frames: load_u64(&self.counters.output_overrun_frames),
            output_underrun_callbacks: load_u64(&self.counters.output_underrun_callbacks),
            output_underrun_samples: load_u64(&self.counters.output_underrun_samples),
            stream_errors: load_u64(&self.counters.stream_errors),
            stream_xruns: load_u64(&self.counters.stream_xruns),
            input_callback_calls,
            input_callback_sample_frames: load_u64(&self.counters.input_callback_sample_frames),
            input_first_callback_frames: load_u64(&self.counters.input_first_callback_frames),
            input_callback_average_us: average_ns(
                load_u64(&self.counters.input_callback_total_ns),
                input_callback_calls,
            ) / 1_000.0,
            input_callback_max_us: ns_to_us(load_u64(&self.counters.input_callback_max_ns)),
            output_callback_calls,
            output_callback_sample_frames: load_u64(&self.counters.output_callback_sample_frames),
            output_first_callback_frames: load_u64(&self.counters.output_first_callback_frames),
            output_callback_average_us: average_ns(
                load_u64(&self.counters.output_callback_total_ns),
                output_callback_calls,
            ) / 1_000.0,
            output_callback_max_us: ns_to_us(load_u64(&self.counters.output_callback_max_ns)),
            input_callback_frame_histogram,
            input_histogram_overflow_observations,
            output_callback_frame_histogram,
            output_histogram_overflow_observations,
            input_callback_cadence: cadence_snapshot(
                input_cadence_observations,
                load_u64(&self.counters.input_cadence_total_ns),
                load_u64(&self.counters.input_cadence_min_ns),
                load_u64(&self.counters.input_cadence_max_ns),
            ),
            output_callback_cadence: cadence_snapshot(
                output_cadence_observations,
                load_u64(&self.counters.output_cadence_total_ns),
                load_u64(&self.counters.output_cadence_min_ns),
                load_u64(&self.counters.output_cadence_max_ns),
            ),
            input_callback_timestamp_first_ns: optional_timestamp(load_u64(
                &self.counters.input_callback_timestamp_first_ns,
            )),
            input_callback_timestamp_latest_ns: optional_timestamp(load_u64(
                &self.counters.input_callback_timestamp_latest_ns,
            )),
            input_device_timestamp_first_ns: optional_timestamp(load_u64(
                &self.counters.input_device_timestamp_first_ns,
            )),
            input_device_timestamp_latest_ns: optional_timestamp(load_u64(
                &self.counters.input_device_timestamp_latest_ns,
            )),
            output_callback_timestamp_first_ns: optional_timestamp(load_u64(
                &self.counters.output_callback_timestamp_first_ns,
            )),
            output_callback_timestamp_latest_ns: optional_timestamp(load_u64(
                &self.counters.output_callback_timestamp_latest_ns,
            )),
            output_device_timestamp_first_ns: optional_timestamp(load_u64(
                &self.counters.output_device_timestamp_first_ns,
            )),
            output_device_timestamp_latest_ns: optional_timestamp(load_u64(
                &self.counters.output_device_timestamp_latest_ns,
            )),
            processing_average_us: average_ns(processing_total_ns, processed_frames) / 1_000.0,
            processing_max_us: ns_to_us(load_u64(&self.counters.processing_max_ns)),
            processing_realtime_factor: if processed_frames == 0 {
                0.0
            } else {
                processing_total_ns as f64 / (processed_frames as f64 * FRAME_DURATION_NS as f64)
            },
            processing_deadline_misses: load_u64(&self.counters.processing_deadline_misses),
            input_queue_depth_frames: load_usize(&self.counters.input_queue_depth),
            input_queue_max_depth_frames: load_usize(&self.counters.input_queue_max_depth),
            output_queue_depth_frames: load_usize(&self.counters.output_queue_depth),
            output_queue_max_depth_frames: load_usize(&self.counters.output_queue_max_depth),
            output_buffered_samples: load_usize(&self.counters.output_buffered_samples)
                .saturating_add(load_usize(&self.counters.output_worker_buffered_samples)),
            software_latency_observations,
            software_latency_average_ms: average_ns(
                load_u64(&self.counters.software_latency_total_ns),
                software_latency_observations,
            ) / 1_000_000.0,
            software_latency_max_ms: ns_to_ms(load_u64(&self.counters.software_latency_max_ns)),
            startup_preroll_target_frames: load_usize(&self.counters.startup_preroll_target_frames),
            startup_preroll_callbacks: load_u64(&self.counters.startup_preroll_callbacks),
            startup_preroll_samples: load_u64(&self.counters.startup_preroll_samples),
            startup_preroll_completed: load_u64(&self.counters.startup_preroll_completed) != 0,
            drift_correction_enabled: load_u64(&self.counters.drift_correction_enabled) != 0,
            drift_correction_ratio_ppm: self
                .counters
                .drift_correction_ratio_milli_ppm
                .load(Ordering::Relaxed) as f64
                / 1_000.0,
            drift_correction_min_ppm: signed_metric_or_zero(
                self.counters
                    .drift_correction_min_milli_ppm
                    .load(Ordering::Relaxed),
                i64::MAX,
            ) / 1_000.0,
            drift_correction_max_ppm: signed_metric_or_zero(
                self.counters
                    .drift_correction_max_milli_ppm
                    .load(Ordering::Relaxed),
                i64::MIN,
            ) / 1_000.0,
            drift_correction_errors: load_u64(&self.counters.drift_correction_errors),
            drift_resampler_delay_samples: load_usize(&self.counters.drift_resampler_delay_samples),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_timing(
    timing: CallbackTiming,
    callback_first: &AtomicU64,
    callback_latest: &AtomicU64,
    device_first: &AtomicU64,
    device_latest: &AtomicU64,
    cadence_observations: &AtomicU64,
    cadence_total_ns: &AtomicU64,
    cadence_min_ns: &AtomicU64,
    cadence_max_ns: &AtomicU64,
) {
    if let Some(callback_ns) = timing.callback_ns {
        set_first(callback_first, callback_ns);
        let previous = callback_latest.swap(callback_ns, Ordering::Relaxed);
        if previous != 0 && callback_ns >= previous {
            let cadence = callback_ns - previous;
            cadence_observations.fetch_add(1, Ordering::Relaxed);
            cadence_total_ns.fetch_add(cadence, Ordering::Relaxed);
            cadence_min_ns.fetch_min(cadence, Ordering::Relaxed);
            cadence_max_ns.fetch_max(cadence, Ordering::Relaxed);
        }
    }
    if let Some(device_ns) = timing.device_ns {
        set_first(device_first, device_ns);
        device_latest.store(device_ns, Ordering::Relaxed);
    }
}

fn set_first(target: &AtomicU64, value: u64) {
    if value != 0 {
        let _ = target.compare_exchange(0, value, Ordering::Relaxed, Ordering::Relaxed);
    }
}

fn signed_metric_or_zero(value: i64, unset: i64) -> f64 {
    if value == unset { 0.0 } else { value as f64 }
}

fn optional_timestamp(value: u64) -> Option<u64> {
    (value != 0).then_some(value)
}

fn cadence_snapshot(observations: u64, total_ns: u64, min_ns: u64, max_ns: u64) -> CadenceSummary {
    CadenceSummary {
        observations,
        average_ms: average_ns(total_ns, observations) / 1_000_000.0,
        min_ms: if observations == 0 {
            0.0
        } else {
            ns_to_ms(min_ns)
        },
        max_ms: ns_to_ms(max_ns),
    }
}

fn average_ns(total: u64, count: u64) -> f64 {
    if count == 0 {
        0.0
    } else {
        total as f64 / count as f64
    }
}

fn ns_to_us(ns: u64) -> f64 {
    ns as f64 / 1_000.0
}
fn ns_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

/// One exact device-callback frame count and its observation count.
#[derive(Clone, Debug, Serialize)]
pub struct CallbackFrameCount {
    pub frames: usize,
    pub callbacks: u64,
}

/// Summary of the interval between consecutive backend callback timestamps.
#[derive(Clone, Debug, Serialize)]
pub struct CadenceSummary {
    pub observations: u64,
    pub average_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

/// Serializable point-in-time engine measurements.
#[derive(Clone, Debug, Serialize)]
pub struct MetricsSnapshot {
    pub captured_frames: u64,
    pub processed_frames: u64,
    pub rendered_frames: u64,
    pub input_overrun_frames: u64,
    pub output_overrun_frames: u64,
    pub output_underrun_callbacks: u64,
    pub output_underrun_samples: u64,
    pub stream_errors: u64,
    pub stream_xruns: u64,
    pub input_callback_calls: u64,
    pub input_callback_sample_frames: u64,
    pub input_first_callback_frames: u64,
    pub input_callback_average_us: f64,
    pub input_callback_max_us: f64,
    pub output_callback_calls: u64,
    pub output_callback_sample_frames: u64,
    pub output_first_callback_frames: u64,
    pub output_callback_average_us: f64,
    pub output_callback_max_us: f64,
    pub input_callback_frame_histogram: Vec<CallbackFrameCount>,
    pub input_histogram_overflow_observations: u64,
    pub output_callback_frame_histogram: Vec<CallbackFrameCount>,
    pub output_histogram_overflow_observations: u64,
    pub input_callback_cadence: CadenceSummary,
    pub output_callback_cadence: CadenceSummary,
    pub input_callback_timestamp_first_ns: Option<u64>,
    pub input_callback_timestamp_latest_ns: Option<u64>,
    pub input_device_timestamp_first_ns: Option<u64>,
    pub input_device_timestamp_latest_ns: Option<u64>,
    pub output_callback_timestamp_first_ns: Option<u64>,
    pub output_callback_timestamp_latest_ns: Option<u64>,
    pub output_device_timestamp_first_ns: Option<u64>,
    pub output_device_timestamp_latest_ns: Option<u64>,
    pub processing_average_us: f64,
    pub processing_max_us: f64,
    pub processing_realtime_factor: f64,
    pub processing_deadline_misses: u64,
    pub input_queue_depth_frames: usize,
    pub input_queue_max_depth_frames: usize,
    pub output_queue_depth_frames: usize,
    pub output_queue_max_depth_frames: usize,
    pub output_buffered_samples: usize,
    pub software_latency_observations: u64,
    pub software_latency_average_ms: f64,
    pub software_latency_max_ms: f64,
    pub startup_preroll_target_frames: usize,
    pub startup_preroll_callbacks: u64,
    pub startup_preroll_samples: u64,
    pub startup_preroll_completed: bool,
    pub drift_correction_enabled: bool,
    pub drift_correction_ratio_ppm: f64,
    pub drift_correction_min_ppm: f64,
    pub drift_correction_max_ppm: f64,
    pub drift_correction_errors: u64,
    pub drift_resampler_delay_samples: usize,
}

#[cfg(test)]
mod tests {
    use super::{CallbackTiming, Metrics};

    #[test]
    fn snapshot_derives_averages_peaks_histograms_and_cadence() {
        let metrics = Metrics::default();
        metrics.input_callback(
            1_000,
            128,
            CallbackTiming {
                callback_ns: Some(10_000_000),
                device_ns: Some(9_000_000),
            },
        );
        metrics.input_callback(
            3_000,
            480,
            CallbackTiming {
                callback_ns: Some(20_000_000),
                device_ns: Some(19_000_000),
            },
        );
        metrics.processed_frame(5_000_000);
        metrics.software_latency(20_000_000);

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.input_callback_average_us, 2.0);
        assert_eq!(snapshot.input_callback_max_us, 3.0);
        assert_eq!(snapshot.input_callback_sample_frames, 608);
        assert_eq!(snapshot.input_callback_frame_histogram.len(), 2);
        assert_eq!(snapshot.input_callback_cadence.average_ms, 10.0);
        assert_eq!(snapshot.processing_realtime_factor, 0.5);
        assert_eq!(snapshot.software_latency_average_ms, 20.0);
        assert_eq!(snapshot.input_device_timestamp_latest_ns, Some(19_000_000));
    }
}
