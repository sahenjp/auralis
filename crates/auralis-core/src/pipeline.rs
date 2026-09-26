//! Realtime-safe framing endpoints around a non-callback processing worker.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, PushError, RingBuffer};
use serde::Serialize;

use crate::drift::AdaptiveResampler;
use crate::frame::AudioFrame;
use crate::{CallbackTiming, DriftCorrectionConfig, FRAME_SAMPLES, Metrics};

// This is outside audio callbacks. A sub-millisecond poll bounds wake latency
// without the high idle CPU observed with a 100 us interval on WSL2.
const IDLE_POLL_INTERVAL: Duration = Duration::from_micros(500);

/// An in-place, fixed-frame processing stage.
///
/// Implementations run on the processing worker, never on an audio callback.
pub trait FrameProcessor: Send + 'static {
    /// Stable diagnostic name.
    fn name(&self) -> &'static str;

    /// Total deterministic algorithmic delay in samples, excluding transport queues.
    fn algorithmic_latency_samples(&self) -> usize;

    /// Reset deterministic processor state before a new stream or discontinuity.
    fn reset(&mut self) {}

    /// Perform worker-thread-affine initialization before callbacks can start.
    fn prepare(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Process one 48 kHz mono frame in place.
    fn process(&mut self, samples: &mut [f32; FRAME_SAMPLES]);
}

/// Identity processor used to prove the transport before adding enhancement.
#[derive(Debug, Default)]
pub struct Passthrough;

impl FrameProcessor for Passthrough {
    fn name(&self) -> &'static str {
        "passthrough"
    }

    fn algorithmic_latency_samples(&self) -> usize {
        0
    }

    fn process(&mut self, _samples: &mut [f32; FRAME_SAMPLES]) {}
}

/// Bounded transport and startup configuration.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PipelineConfig {
    /// Maximum complete frames in each SPSC queue.
    pub queue_capacity_frames: usize,
    /// Minimum processed frames required before render leaves startup silence.
    ///
    /// The render adapter raises this to cover its first observed callback when
    /// that callback is larger than one processing frame.
    pub startup_pre_roll_frames: usize,
    /// Worker-side asynchronous correction for measured capture/render clock drift.
    pub drift_correction: DriftCorrectionConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            queue_capacity_frames: 4,
            startup_pre_roll_frames: 1,
            drift_correction: DriftCorrectionConfig::default(),
        }
    }
}

/// The two callback framing adapters, worker owner, and shared measurements.
pub struct PipelineParts {
    pub capture: CaptureEndpoint,
    pub render: RenderEndpoint,
    pub worker: ProcessorWorker,
    pub metrics: Metrics,
}

/// Construct and start a processing pipeline.
pub fn start_pipeline<P: FrameProcessor>(
    config: PipelineConfig,
    processor: P,
) -> io::Result<PipelineParts> {
    if !(1..=64).contains(&config.queue_capacity_frames) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "queue_capacity_frames must be between 1 and 64",
        ));
    }
    if config.startup_pre_roll_frames > config.queue_capacity_frames {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "startup_pre_roll_frames must not exceed queue_capacity_frames",
        ));
    }

    let (capture_producer, processor_consumer) = RingBuffer::new(config.queue_capacity_frames);
    let (processor_producer, render_consumer) = RingBuffer::new(config.queue_capacity_frames);
    let metrics = Metrics::default();
    let clock = RealtimeClock::new();
    let stop = Arc::new(AtomicBool::new(false));
    let worker = ProcessorWorker::spawn(
        processor,
        processor_consumer,
        processor_producer,
        config.queue_capacity_frames,
        config.drift_correction,
        metrics.clone(),
        Arc::clone(&stop),
    )?;

    Ok(PipelineParts {
        capture: CaptureEndpoint::new(
            capture_producer,
            config.queue_capacity_frames,
            metrics.clone(),
            clock,
        ),
        render: RenderEndpoint::new(
            render_consumer,
            config.queue_capacity_frames,
            config.startup_pre_roll_frames,
            metrics.clone(),
            clock,
        ),
        worker,
        metrics,
    })
}

#[derive(Clone, Copy)]
struct RealtimeClock {
    epoch: Instant,
}

impl RealtimeClock {
    fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }

    fn now_ns(self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Capture-side framing adapter and sole producer of the input queue.
pub struct CaptureEndpoint {
    producer: Producer<AudioFrame>,
    queue_capacity: usize,
    pending: AudioFrame,
    pending_samples: usize,
    next_sequence: u64,
    metrics: Metrics,
    clock: RealtimeClock,
}

impl CaptureEndpoint {
    fn new(
        producer: Producer<AudioFrame>,
        queue_capacity: usize,
        metrics: Metrics,
        clock: RealtimeClock,
    ) -> Self {
        Self {
            producer,
            queue_capacity,
            pending: AudioFrame::silence(0),
            pending_samples: 0,
            next_sequence: 0,
            metrics,
            clock,
        }
    }

    /// Downmix arbitrary interleaved `f32` callback data into fixed frames.
    pub fn process_callback(&mut self, interleaved: &[f32], channels: usize) {
        self.process_callback_with_timing(interleaved, channels, CallbackTiming::default());
    }

    /// Process callback data while recording backend capture timing.
    ///
    /// This method performs no allocation, I/O, locking, or waiting.
    pub fn process_callback_with_timing(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        timing: CallbackTiming,
    ) {
        let started = Instant::now();
        if channels == 0 {
            self.metrics.input_callback(elapsed_ns(started), 0, timing);
            return;
        }

        let sample_frames = interleaved.len() / channels;
        let callback_timestamp = self.clock.now_ns();
        for channel_frame in interleaved.chunks_exact(channels) {
            let mono = channel_frame.iter().copied().sum::<f32>() / channels as f32;
            if self.pending_samples == 0 {
                self.pending.sequence = self.next_sequence;
                self.pending.captured_at_ns = callback_timestamp;
            }
            self.pending.samples[self.pending_samples] = mono;
            self.pending_samples += 1;

            if self.pending_samples == FRAME_SAMPLES {
                self.enqueue_pending_frame();
            }
        }
        self.metrics
            .input_callback(elapsed_ns(started), sample_frames, timing);
    }

    fn enqueue_pending_frame(&mut self) {
        let next_frame = AudioFrame::silence(self.next_sequence.saturating_add(1));
        let completed = std::mem::replace(&mut self.pending, next_frame);
        self.pending_samples = 0;
        self.next_sequence = self.next_sequence.saturating_add(1);

        match self.producer.push(completed) {
            Ok(()) => {
                self.metrics.captured_frame();
                let depth = self.queue_capacity.saturating_sub(self.producer.slots());
                self.metrics.input_queue_depth(depth);
            }
            Err(PushError::Full(_)) => {
                self.metrics.input_overrun();
                self.metrics.input_queue_depth(self.queue_capacity);
            }
        }
    }
}

/// Render-side framing adapter and sole consumer of the output queue.
pub struct RenderEndpoint {
    consumer: Consumer<AudioFrame>,
    queue_capacity: usize,
    configured_pre_roll_frames: usize,
    effective_pre_roll_frames: usize,
    pre_roll_complete: bool,
    current: Option<AudioFrame>,
    current_sample: usize,
    metrics: Metrics,
    clock: RealtimeClock,
}

impl RenderEndpoint {
    fn new(
        consumer: Consumer<AudioFrame>,
        queue_capacity: usize,
        configured_pre_roll_frames: usize,
        metrics: Metrics,
        clock: RealtimeClock,
    ) -> Self {
        Self {
            consumer,
            queue_capacity,
            configured_pre_roll_frames,
            effective_pre_roll_frames: 0,
            pre_roll_complete: false,
            current: None,
            current_sample: 0,
            metrics,
            clock,
        }
    }

    /// Fill arbitrary interleaved `f32` output, duplicating mono to all channels.
    pub fn process_callback(&mut self, interleaved: &mut [f32], channels: usize) {
        self.process_callback_with_timing(interleaved, channels, CallbackTiming::default());
    }

    /// Fill callback data while recording backend render timing.
    ///
    /// This method performs no allocation, I/O, locking, or waiting. Missing
    /// steady-state samples are explicitly zeroed and counted.
    pub fn process_callback_with_timing(
        &mut self,
        interleaved: &mut [f32],
        channels: usize,
        timing: CallbackTiming,
    ) {
        let started = Instant::now();
        interleaved.fill(0.0);
        if channels == 0 {
            self.metrics.output_callback(elapsed_ns(started), 0, timing);
            return;
        }

        let callback_frames = interleaved.len() / channels;
        if !self.pre_roll_complete {
            if self.effective_pre_roll_frames == 0 {
                let callback_processing_frames = callback_frames.div_ceil(FRAME_SAMPLES);
                self.effective_pre_roll_frames = self
                    .configured_pre_roll_frames
                    .max(callback_processing_frames)
                    .min(self.queue_capacity);
            }
            let target = self.effective_pre_roll_frames;
            self.metrics.startup_preroll_target(target);
            self.metrics.output_queue_depth(self.consumer.slots());
            self.metrics
                .set_output_buffered_samples(self.consumer.slots() * FRAME_SAMPLES);
            if self.consumer.slots() < target {
                self.metrics.startup_preroll_wait(callback_frames);
                self.metrics
                    .output_callback(elapsed_ns(started), callback_frames, timing);
                return;
            }
            self.pre_roll_complete = true;
            self.metrics.startup_preroll_complete();
        }

        let callback_timestamp = self.clock.now_ns();
        let mut underrun_samples = 0_u64;
        for channel_frame in interleaved.chunks_exact_mut(channels) {
            if self.current.is_none() {
                match self.consumer.pop() {
                    Ok(frame) => {
                        self.metrics.output_queue_depth(self.consumer.slots());
                        self.metrics.software_latency(
                            callback_timestamp.saturating_sub(frame.captured_at_ns),
                        );
                        self.current = Some(frame);
                        self.current_sample = 0;
                    }
                    Err(_) => {
                        underrun_samples = underrun_samples.saturating_add(1);
                        continue;
                    }
                }
            }

            let Some(frame) = self.current.as_ref() else {
                continue;
            };
            channel_frame.fill(frame.samples[self.current_sample]);
            self.current_sample += 1;
            if self.current_sample == FRAME_SAMPLES {
                self.current = None;
                self.current_sample = 0;
                self.metrics.rendered_frame();
            }
        }

        if underrun_samples > 0 {
            self.metrics.output_underrun(underrun_samples);
        }
        let current_samples = self
            .current
            .as_ref()
            .map_or(0, |_| FRAME_SAMPLES.saturating_sub(self.current_sample));
        self.metrics
            .set_output_buffered_samples(self.consumer.slots() * FRAME_SAMPLES + current_samples);
        self.metrics
            .output_callback(elapsed_ns(started), callback_frames, timing);
    }
}

struct WorkerOutputAdapter {
    producer: Producer<AudioFrame>,
    ready: Option<AudioFrame>,
    pending: AudioFrame,
    pending_samples: usize,
    next_sequence: u64,
    metrics: Metrics,
    adaptive_resampler: Option<AdaptiveResampler>,
}

impl WorkerOutputAdapter {
    fn new(
        producer: Producer<AudioFrame>,
        _capacity: usize,
        metrics: Metrics,
        adaptive_resampler: Option<AdaptiveResampler>,
    ) -> Self {
        Self {
            producer,
            ready: None,
            pending: AudioFrame::silence(0),
            pending_samples: 0,
            next_sequence: 0,
            metrics,
            adaptive_resampler,
        }
    }

    fn slots(&self) -> usize {
        self.producer.slots()
    }

    fn push(&mut self, frame: AudioFrame) -> bool {
        if self.adaptive_resampler.is_none() {
            return self.producer.push(frame).is_ok();
        }
        if !flush_ready_frame(&mut self.producer, &mut self.ready) {
            self.update_worker_buffered_samples();
            return false;
        }
        self.update_worker_buffered_samples();

        let buffered_samples = self.metrics.output_buffered_samples();
        let samples = self
            .adaptive_resampler
            .as_mut()
            .expect("resampler presence checked above")
            .process_frame(
                &frame.samples,
                buffered_samples,
                self.metrics.startup_preroll_is_complete(),
                &self.metrics,
            );
        let captured_at_ns = frame.captured_at_ns;
        let mut overrun = false;
        let mut offset = 0;
        while offset < samples.len() {
            if self.pending_samples == 0 {
                self.pending.sequence = self.next_sequence;
                self.pending.captured_at_ns = captured_at_ns;
            }
            let copied = (FRAME_SAMPLES - self.pending_samples).min(samples.len() - offset);
            self.pending.samples[self.pending_samples..self.pending_samples + copied]
                .copy_from_slice(&samples[offset..offset + copied]);
            self.pending_samples += copied;
            offset += copied;

            if self.pending_samples == FRAME_SAMPLES {
                let next = AudioFrame::silence(self.next_sequence.saturating_add(1));
                let completed = std::mem::replace(&mut self.pending, next);
                self.pending_samples = 0;
                self.next_sequence = self.next_sequence.saturating_add(1);
                overrun |= !enqueue_completed_frame(&mut self.producer, &mut self.ready, completed);
            }
        }
        self.update_worker_buffered_samples();
        !overrun
    }

    fn update_worker_buffered_samples(&self) {
        let ready_samples = usize::from(self.ready.is_some()) * FRAME_SAMPLES;
        self.metrics
            .set_output_worker_buffered_samples(self.pending_samples.saturating_add(ready_samples));
    }
}

fn flush_ready_frame(producer: &mut Producer<AudioFrame>, ready: &mut Option<AudioFrame>) -> bool {
    let Some(frame) = ready.take() else {
        return true;
    };
    match producer.push(frame) {
        Ok(()) => true,
        Err(PushError::Full(frame)) => {
            *ready = Some(frame);
            false
        }
    }
}

fn enqueue_completed_frame(
    producer: &mut Producer<AudioFrame>,
    ready: &mut Option<AudioFrame>,
    frame: AudioFrame,
) -> bool {
    if !flush_ready_frame(producer, ready) {
        return false;
    }
    match producer.push(frame) {
        Ok(()) => true,
        Err(PushError::Full(frame)) => {
            *ready = Some(frame);
            true
        }
    }
}

/// Owns the non-callback processing thread and guarantees a joined shutdown.
pub struct ProcessorWorker {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    processor_name: &'static str,
    processor_algorithmic_latency_samples: usize,
    drift_resampler_algorithmic_latency_samples: usize,
}

impl ProcessorWorker {
    fn spawn<P: FrameProcessor>(
        mut processor: P,
        mut input: Consumer<AudioFrame>,
        output: Producer<AudioFrame>,
        output_capacity: usize,
        drift_config: DriftCorrectionConfig,
        metrics: Metrics,
        stop: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let processor_name = processor.name();
        processor.reset();
        let processor_algorithmic_latency_samples = processor.algorithmic_latency_samples();
        let mut drift_resampler_algorithmic_latency_samples = 0;
        let adaptive_resampler = if drift_config.enabled {
            let resampler = AdaptiveResampler::new(drift_config, output_capacity)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            let delay = resampler.output_delay_samples();
            metrics.configure_drift_correction(delay);
            drift_resampler_algorithmic_latency_samples = delay;
            Some(resampler)
        } else {
            None
        };
        let mut output =
            WorkerOutputAdapter::new(output, output_capacity, metrics.clone(), adaptive_resampler);
        let thread_stop = Arc::clone(&stop);
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let join = thread::Builder::new()
            .name("auralis-processing".to_owned())
            .spawn(move || {
                if let Err(error) = processor.prepare() {
                    let _ = ready_sender.send(Err(error));
                    return;
                }
                if ready_sender.send(Ok(())).is_err() {
                    return;
                }
                while !thread_stop.load(Ordering::Relaxed) {
                    match input.pop() {
                        Ok(mut frame) => {
                            metrics.input_queue_depth(input.slots());
                            let started = Instant::now();
                            processor.process(&mut frame.samples);
                            if output.push(frame) {
                                let depth = output_capacity.saturating_sub(output.slots());
                                metrics.output_queue_depth(depth);
                            } else {
                                metrics.output_overrun();
                            }
                            metrics.processed_frame(elapsed_ns(started));
                        }
                        Err(_) => thread::sleep(IDLE_POLL_INTERVAL),
                    }
                }
            })?;
        match ready_receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let _ = join.join();
                return Err(io::Error::other(format!(
                    "processing worker initialization failed: {error}"
                )));
            }
            Err(error) => {
                let _ = join.join();
                return Err(io::Error::other(format!(
                    "processing worker initialization channel failed: {error}"
                )));
            }
        }

        Ok(Self {
            stop,
            join: Some(join),
            processor_name,
            processor_algorithmic_latency_samples,
            drift_resampler_algorithmic_latency_samples,
        })
    }

    /// Processor diagnostic name.
    pub fn processor_name(&self) -> &'static str {
        self.processor_name
    }

    /// Processor/model algorithmic latency, excluding transport and drift resampling.
    pub fn processor_algorithmic_latency_samples(&self) -> usize {
        self.processor_algorithmic_latency_samples
    }

    /// Drift-resampler filter delay, kept separate from processor/model latency.
    pub fn drift_resampler_algorithmic_latency_samples(&self) -> usize {
        self.drift_resampler_algorithmic_latency_samples
    }

    /// Combined processing-path algorithmic latency, excluding transport queues.
    pub fn processing_path_algorithmic_latency_samples(&self) -> usize {
        self.processor_algorithmic_latency_samples
            .saturating_add(self.drift_resampler_algorithmic_latency_samples)
    }

    /// Stop immediately at a frame boundary and join the worker.
    pub fn stop(&mut self) -> thread::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().map_or(Ok(()), |join| join.join())
    }
}

impl Drop for ProcessorWorker {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use rtrb::RingBuffer;

    use super::{
        Passthrough, PipelineConfig, enqueue_completed_frame, flush_ready_frame, start_pipeline,
    };
    use crate::FRAME_SAMPLES;
    use crate::frame::AudioFrame;

    #[test]
    fn passthrough_preserves_downmixed_frame() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        let mut stereo = [0.0_f32; FRAME_SAMPLES * 2];
        for (index, sample) in stereo.chunks_exact_mut(2).enumerate() {
            sample[0] = index as f32;
            sample[1] = -sample[0] * 0.5;
        }

        parts.capture.process_callback(&stereo, 2);
        wait_for_processed_frames(&parts, 1);
        let mut output = [0.0_f32; FRAME_SAMPLES];
        parts.render.process_callback(&mut output, 1);

        for (index, actual) in output.iter().copied().enumerate() {
            let left = index as f32;
            let expected = (left + (-left * 0.5)) / 2.0;
            assert!((actual - expected).abs() < f32::EPSILON);
        }
        parts.worker.stop().expect("worker should stop");
        let snapshot = parts.metrics.snapshot();
        assert_eq!(snapshot.captured_frames, 1);
        assert_eq!(snapshot.processed_frames, 1);
        assert_eq!(snapshot.rendered_frames, 1);
        assert_eq!(snapshot.output_underrun_callbacks, 0);
    }

    #[test]
    fn split_capture_and_render_frames_are_lossless() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        let source = sequence(FRAME_SAMPLES);
        parts.capture.process_callback(&source[..200], 1);
        parts.capture.process_callback(&source[200..], 1);
        wait_for_processed_frames(&parts, 1);

        let mut first = [0.0_f32; 137];
        let mut second = [0.0_f32; FRAME_SAMPLES - 137];
        parts.render.process_callback(&mut first, 1);
        parts.render.process_callback(&mut second, 1);

        assert_eq!([first.as_slice(), second.as_slice()].concat(), source);
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn merged_callbacks_produce_multiple_processing_frames() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        let source = sequence(FRAME_SAMPLES * 2);
        parts.capture.process_callback(&source, 1);
        wait_for_processed_frames(&parts, 2);

        let mut output = vec![0.0_f32; FRAME_SAMPLES * 2];
        parts.render.process_callback(&mut output, 1);
        assert_eq!(output, source);
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn irregular_callback_sequence_preserves_sample_order() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        let source = sequence(FRAME_SAMPLES * 3);
        let mut offset = 0;
        for size in [73, 511, 2, 854] {
            parts
                .capture
                .process_callback(&source[offset..offset + size], 1);
            offset += size;
        }
        wait_for_processed_frames(&parts, 3);

        let mut output = vec![0.0_f32; source.len()];
        let mut offset = 0;
        for size in [17, 700, 723] {
            parts
                .render
                .process_callback(&mut output[offset..offset + size], 1);
            offset += size;
        }
        assert_eq!(output, source);
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn ring_wraparound_remains_ordered() {
        let mut parts = start_pipeline(
            PipelineConfig {
                queue_capacity_frames: 2,
                startup_pre_roll_frames: 1,
                ..PipelineConfig::default()
            },
            Passthrough,
        )
        .expect("pipeline should start");

        for frame_index in 0..20 {
            let input = [frame_index as f32; FRAME_SAMPLES];
            parts.capture.process_callback(&input, 1);
            wait_for_processed_frames(&parts, frame_index + 1);
            let mut output = [0.0_f32; FRAME_SAMPLES];
            parts.render.process_callback(&mut output, 1);
            assert!(output.iter().all(|sample| *sample == frame_index as f32));
        }
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn startup_preroll_covers_first_large_render_callback() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        parts
            .capture
            .process_callback(&[0.25_f32; FRAME_SAMPLES], 1);
        wait_for_processed_frames(&parts, 1);

        let mut output = [1.0_f32; FRAME_SAMPLES * 2];
        parts.render.process_callback(&mut output, 1);
        assert!(output.iter().all(|sample| *sample == 0.0));
        assert_eq!(parts.metrics.snapshot().output_underrun_callbacks, 0);
        let mut smaller_output = [1.0_f32; FRAME_SAMPLES];
        parts.render.process_callback(&mut smaller_output, 1);
        assert!(smaller_output.iter().all(|sample| *sample == 0.0));

        parts.capture.process_callback(&[0.5_f32; FRAME_SAMPLES], 1);
        wait_for_processed_frames(&parts, 2);
        parts.render.process_callback(&mut output, 1);
        assert!(output[..FRAME_SAMPLES].iter().all(|sample| *sample == 0.25));
        assert!(output[FRAME_SAMPLES..].iter().all(|sample| *sample == 0.5));

        let snapshot = parts.metrics.snapshot();
        assert_eq!(snapshot.startup_preroll_target_frames, 2);
        assert_eq!(snapshot.startup_preroll_callbacks, 2);
        assert!(snapshot.startup_preroll_completed);
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn steady_state_underrun_outputs_silence_and_is_counted() {
        let mut parts =
            start_pipeline(PipelineConfig::default(), Passthrough).expect("pipeline should start");
        parts.capture.process_callback(&[0.5; FRAME_SAMPLES], 1);
        wait_for_processed_frames(&parts, 1);
        let mut output = [0.0_f32; FRAME_SAMPLES];
        parts.render.process_callback(&mut output, 1);

        output.fill(1.0);
        parts.render.process_callback(&mut output, 1);
        assert!(output.iter().all(|sample| *sample == 0.0));
        let snapshot = parts.metrics.snapshot();
        assert_eq!(snapshot.output_underrun_callbacks, 1);
        assert_eq!(snapshot.output_underrun_samples, FRAME_SAMPLES as u64);
        parts.worker.stop().expect("worker should stop");
    }

    #[test]
    fn repeated_start_stop_is_deterministic() {
        for _ in 0..20 {
            let mut parts = start_pipeline(PipelineConfig::default(), Passthrough)
                .expect("pipeline should start");
            parts.worker.stop().expect("worker should stop");
        }
    }

    #[test]
    fn worker_output_stages_one_completed_frame_until_a_slot_is_free() {
        let (mut producer, mut consumer) = RingBuffer::new(1);
        let mut ready = None;

        assert!(enqueue_completed_frame(
            &mut producer,
            &mut ready,
            AudioFrame::silence(0)
        ));
        assert!(enqueue_completed_frame(
            &mut producer,
            &mut ready,
            AudioFrame::silence(1)
        ));
        assert_eq!(ready.as_ref().map(|frame| frame.sequence), Some(1));
        assert!(!enqueue_completed_frame(
            &mut producer,
            &mut ready,
            AudioFrame::silence(99)
        ));
        assert_eq!(ready.as_ref().map(|frame| frame.sequence), Some(1));

        assert_eq!(consumer.pop().expect("first queued frame").sequence, 0);
        assert!(enqueue_completed_frame(
            &mut producer,
            &mut ready,
            AudioFrame::silence(2)
        ));
        assert_eq!(consumer.pop().expect("older staged frame").sequence, 1);
        assert_eq!(ready.as_ref().map(|frame| frame.sequence), Some(2));

        assert!(flush_ready_frame(&mut producer, &mut ready));
        assert!(ready.is_none());
        assert_eq!(consumer.pop().expect("last staged frame").sequence, 2);
    }

    #[test]
    fn invalid_queue_or_preroll_is_rejected() {
        let zero_capacity = match start_pipeline(
            PipelineConfig {
                queue_capacity_frames: 0,
                startup_pre_roll_frames: 0,
                ..PipelineConfig::default()
            },
            Passthrough,
        ) {
            Ok(_) => panic!("zero capacity should fail"),
            Err(error) => error,
        };
        assert_eq!(zero_capacity.kind(), std::io::ErrorKind::InvalidInput);

        let excessive_preroll = match start_pipeline(
            PipelineConfig {
                queue_capacity_frames: 2,
                startup_pre_roll_frames: 3,
                ..PipelineConfig::default()
            },
            Passthrough,
        ) {
            Ok(_) => panic!("excessive preroll should fail"),
            Err(error) => error,
        };
        assert_eq!(excessive_preroll.kind(), std::io::ErrorKind::InvalidInput);
    }

    fn sequence(samples: usize) -> Vec<f32> {
        (0..samples).map(|index| index as f32).collect()
    }

    fn wait_for_processed_frames(parts: &super::PipelineParts, expected: u64) {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            let snapshot = parts.metrics.snapshot();
            let delivered = snapshot
                .rendered_frames
                .saturating_add(snapshot.output_queue_depth_frames as u64);
            if snapshot.processed_frames >= expected && delivered >= expected {
                break;
            }
            assert!(Instant::now() < deadline, "worker did not process frame");
            thread::yield_now();
        }
    }
}
