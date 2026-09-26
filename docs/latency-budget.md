# Latency budget

Latency is reported as a distribution, not a single best-case number. The release target is below 35 ms p95 application latency in Balanced mode and below 60 ms p99 under declared supported load. Physical microphone-to-virtual-endpoint latency must later be measured with loopback hardware; software timestamps are not a substitute.

## Balanced target at 48 kHz

| Component | Target | Hard design allowance |
|---|---:|---:|
| Capture/audio-engine period | 2.5 ms | 5 ms |
| Accumulation to 10 ms frame | 5 ms average | 10 ms |
| Capture-to-worker queue | under 1 ms | 10 ms / one frame |
| DSP plus CPU inference | 4 ms p95 | 8 ms p99 |
| Model lookahead | 0–5 ms | 10 ms |
| Worker-to-output queue | under 1 ms | 10 ms / one frame |
| Virtual endpoint/render period | 2.5 ms | 5 ms |
| **Expected total** | **15–25 ms** | **53 ms** |

The four-frame SPSC queue capacity is an overload containment bound, not the
whole adapter capacity. Milestone 2 stability runs target 3.5 frames of total
output audio across the ring, render partial frame, resampler partial frame,
and bounded staging slot. This conservative stability target is not the final
latency target. Sustained fill slope toward a boundary is a defect even when
the mean remains within bounds. Presets may change model complexity or
lookahead only when the measured tradeoff is recorded.

## Measurements

- callback calls, total time, and maximum time for capture and output;
- frame processing p50/p95/p99 later, total/max in the initial engine;
- current and maximum input/output queue depth;
- capture-first-sample to output-dequeue software latency;
- model lookahead and frame accumulation added analytically;
- underrun callbacks/samples, input overruns, processor-output overruns, stream xruns;
- processing realtime factor;
- process CPU and resident memory in the product diagnostics phase;
- physical end-to-end latency from injected impulse/correlation in the Windows hardware lab.

The Milestone 2 report uses distinct categories:

- callback duration: time spent inside one capture or render callback;
- processing duration: time spent processing one fixed 480-sample frame;
- software pipeline latency: capture-frame timestamp to processed-frame dequeue;
- device buffering: separately reported WASAPI input/output engine periods;
- end-to-end latency: only a physical or documented loopback correlation result.

Startup pre-roll is reported separately and added analytically to the latency
breakdown. It is not hidden inside queue capacity. See
[`end-to-end-latency.md`](end-to-end-latency.md) for the pulse and correlation
procedure.

Increasing queue or device buffers requires an ADR with before/after dropout and latency evidence.
