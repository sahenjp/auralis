# ADR 0008: Bounded asynchronous drift correction

- Status: accepted after native 10-minute evidence
- Date: 2026-08-26

## Problem

The uncorrected 60-second native run was stable, but the 10-minute run measured
capture approximately 13.46 ppm faster than render, a positive steady-state
output-queue slope, movement to the four-frame limit, and one underrun during a
short scheduling excursion. Leaving this uncontrolled risks an eventual queue
boundary in a long duplex session.

## Candidates

| Candidate | Quality and latency | Realtime behavior | License / integration |
|---|---|---|---|
| libsamplerate | Mature band-limited sinc modes, including a highest-quality mode; stateful variable ratio | Processing API is stateful, but Auralis would need to audit a C wrapper and build/FFI path | BSD-2-Clause; native C dependency |
| SpeexDSP resampler | Quality levels tuned for speech and fractional rate changes | Stateful float API and fractional rate setter; C allocation and rate-change behavior require a wrapper audit | BSD-3-Clause; native C dependency |
| Rubato `Async` sinc | Windowed-sinc interpolation, explicit filter delay, continuously adjustable ratio | `process_into_buffer` is documented for realtime use with preallocated input/output buffers and no allocation or blocking | MIT OR Apache-2.0; pure library integration |
| Rubato `Slip` | No filter delay and very low CPU | Corrects by crossfaded single-frame insertion/deletion, so it is not a true resampler | Same license, but rejected because normal drift correction must not depend on sample slipping |

Rubato was not selected merely because it is Rust. Its differentiators for this
specific boundary are a true asynchronously adjustable sinc implementation,
an allocation-free buffer API, explicit delay reporting, and no new C build or
FFI ownership surface.

## Decision

Run a mono Rubato 5.0 `Async` windowed-sinc resampler on the processing worker,
after fixed-frame processing and before the output SPSC queue. Use a 128-sample
sinc, cubic interpolation, construction-time buffers, and the fixed-input mode.
Its variable output is repacked into fixed 480-sample queue frames outside the
audio callbacks.

Control the output/input ratio from total buffered output samples. Average all
100 worker observations in each one-second interval, low-pass the error with a
20-second time constant, and use a slow PI controller. The target is 3.5
480-sample frames for the current four-frame queue. Clamp ratio correction to
±250 ppm and ramp each accepted ratio change. Expose current/minimum/maximum
ratio, errors, and filter delay in machine-readable results.

If a repacked frame completes while the output ring is full, hold exactly one
completed frame in a fixed worker-side staging slot and retry it before newer
output. Count and drop the newest frame only if congestion persists while both
the ring and staging slot are full. Include ring and adapter state in total
fill metrics and expose the seven-frame effective bound separately from the
four-frame SPSC capacity.

## Consequences

Both callbacks retain the same bounded copy/SPSC/atomic-only work. Correction
adds worker CPU and a reported sinc filter delay. Six deterministic two-hour
simulations at ±10, ±50, and ±100 ppm remain within the four-frame bound and
converge near the inverse clock error. The accepted 30-minute native run
measured a 3.499-frame mean, 0.00510-frame² variance, +0.128 ppm equivalent
residual slope, and zero underrun, overrun, xrun, or correction error.

## References

- Rubato, [realtime and allocation guidance](https://docs.rs/rubato/latest/rubato/)
- Rubato, [`Resampler::process_into_buffer` and adjustable ratio](https://docs.rs/rubato/latest/rubato/trait.Resampler.html)
- libsamplerate, [full stateful API](https://libsndfile.github.io/libsamplerate/api_full.html)
- SpeexDSP, [resampler API](https://github.com/xiph/speexdsp/blob/master/include/speex/speex_resampler.h)
