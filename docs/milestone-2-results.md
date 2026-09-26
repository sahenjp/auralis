# Milestone 2: Windows realtime characterization

Date: 2026-08-26. These are development measurements from the Windows 11
machine attached to this repository, not product claims. WSLg results from
Milestone 1 are retained only as historical evidence and are not the Windows
production baseline.

## Native path and device

The `x86_64-pc-windows-msvc` release executable was built with the installed
MSVC/Windows SDK libraries and then executed as a Windows process. This is
native runtime verification, not a successful cross-compilation claim.

| Item | Measured value |
|---|---|
| Backend | CPAL 0.18.2 WASAPI |
| Input | `マイク (G435 Wireless Gaming Headset)` |
| Input ID | `wasapi:{0.0.1.00000000}.{36c1bc52-9517-489c-802b-c2e929c70a07}` |
| Output | `ヘッドセット イヤフォン (G435 Wireless Gaming Headset)` |
| Output ID | `wasapi:{0.0.0.00000000}.{37677885-f296-441b-8656-0bc88df3c95a}` |
| Requested input/output | 48 kHz, two device channels, `f32`, backend-default buffer |
| Actual stream format | 48 kHz, two device channels, `f32`, 480-frame stream buffer |
| Internal processing | 48 kHz, mono, `f32`, fixed 480-sample frame |

The device callbacks are stereo because that is the supported native endpoint
format selected by CPAL. Capture is downmixed to the mono internal contract and
render is duplicated to both output channels. This conversion is bounded and
occurs in callbacks without allocation.

## WASAPI shared-engine periods

The Windows-only probe queried `IAudioClient3` using the same endpoint selected
by CPAL. Both tested endpoints returned the following for the active 48 kHz
format:

| Endpoint | Default | Fundamental | Minimum | Maximum | Current | Observed steady callback |
|---|---:|---:|---:|---:|---:|---:|
| G435 capture | 480 | 480 | 480 | 480 | 480 | 480 frames |
| G435 render | 480 | 480 | 480 | 480 | 480 | 480 frames |

All period values are frames, equal to 10 ms on this device. This is a measured
device result, not an engine-wide assumption. The first render callback asked
for 1,056 frames; subsequent render callbacks asked for 480. The capture
histogram contained 480-frame callbacks. Auralis requested CPAL's default
buffer and did not call `InitializeSharedAudioStream`, so the machine report
records the requested `IAudioClient3` period as `null`.

CPAL already exposes stable endpoint IDs, callback/device timestamps, stream
clocks, actual stream buffer sizes, callback frame counts, and xrun errors.
Only shared-engine period queries were missing. ADR 0005 therefore retains CPAL
and limits native WASAPI code to a read-only period probe.

## Uncorrected characterization

The explicit startup pre-roll target was three processing frames because
`ceil(1056 / 480) = 3`. Pre-roll silence is counted separately and no longer
appears as a startup underrun.

| Metric | 60 seconds | 10 minutes |
|---|---:|---:|
| Measured duration | 60.000 s | 600.000 s |
| Relative timestamp-rate estimate | +0.64 ppm | +13.46 ppm |
| Estimated difference over run | +1.85 samples | +387.64 samples |
| Output queue max | 3 frames | 4 frames |
| Input/output overruns | 0 / 0 | 0 / 0 |
| Output underruns | 0 | 1 callback / 480 samples |
| Backend xruns | 0 | 0 |
| Software latency average / max | 24.007 / 25.960 ms | 31.837 / 55.011 ms |
| CPU average / max | 4.720% / 22.222% | 4.029% / 29.907% |
| Peak RSS | 13,594,624 bytes | 14,073,856 bytes |

The 60-second run alone would have suggested no meaningful drift. The
10-minute run moved between two, three, and four output frames, reached the
queue limit, and recorded a non-startup underrun. This was sufficient evidence
to implement bounded correction, but not proof that every excursion was caused
only by hardware clock mismatch. Scheduler and callback batching remain
possible contributors.

## Adaptive correction

Correction runs after passthrough processing on the worker, never in either
audio callback. Rubato 5.0 `Async` uses a construction-time preallocated
windowed-sinc resampler. Its reported filter delay is 64 samples / 1.333 ms.
The controller averages 100 worker observations over one second, applies a
20-second low-pass response, targets 3.5 buffered frames, and clamps ratio to
±250 ppm.

The first controller version sampled one instantaneous fill value per second.
In a 10-minute trial it reacted to a short phase excursion with −245 to +148
ppm, produced 40 rejected lower-bound ratio requests, and still recorded one
underrun. That result was rejected. The averaging/filtering policy above is the
revised controller used for acceptance tests.

The revised resampler initially exposed a second boundary case: a variable
output frame could complete while the four-frame output ring was momentarily
full. A fixed one-frame worker-side staging slot now preserves that completed
frame and retries it before newer output. Only congestion that persists while
both the ring and the staging slot are full is counted as an output overrun.
The staging slot is fixed at construction and does not allocate or block.

Deterministic two-hour simulations for +10, +50, +100, −10, −50, and −100 ppm
remain bounded and converge to the inverse correction. A separate test verifies
the ±250 ppm clamp. The resampler uses `process_into_buffer` with input/output
storage allocated before streaming.

## Worker scheduling evidence

A corrected 10-minute trial with an ordinary-priority processing worker had no
output overrun, but at 324.66 seconds it recorded one 480-sample underrun. At
that same sample, the output queue was empty and the input queue held all four
frames. The worker had not run for roughly four callback periods; this was
scheduler starvation, not a gradual clock-control failure.

The Windows worker now registers itself once with MMCSS task `Pro Audio` at
relative priority `normal`. It does not raise the whole process priority and
does not change registry or scheduler configuration. Registration and its
error are reported in JSON; a thread-local guard calls
`AvRevertMmThreadCharacteristics` when the worker exits. No MMCSS call occurs
in an audio callback.

## Accepted native runs

All accepted runs used the same schema-4 release executable, G435 endpoints,
CPAL default device buffers, three-frame startup pre-roll, bounded asynchronous
correction, and worker MMCSS registration. Queue statistics include the output
ring, render partial frame, resampler partial frame, and one staged completed
frame. The resulting total bound is seven frames; the SPSC ring itself remains
four frames.

| Metric | 60 seconds | 10 minutes | 30 minutes |
|---|---:|---:|---:|
| Measured duration | 60.000 s | 600.000 s | 1800.000 s |
| Captured frames | 5,998 | 60,001 | 180,010 |
| Relative clock estimate | −0.669 ppm | +0.162 ppm | −0.0355 ppm |
| Estimated difference | −1.93 samples | +4.67 samples | −3.06 samples |
| Total fill mean | 3.160 frames | 3.496 frames | 3.499 frames |
| Fill variance | 0.0164 | 0.0151 | 0.00510 frames² |
| Fill min / max | 2.996 / 3.390 | 2.996 / 3.598 | 2.525 / 3.598 frames |
| Fill slope | +73.33 ppm | +2.657 ppm | +0.128 ppm equivalent |
| Final correction | +69.904 ppm | −0.270 ppm | −0.022 ppm |
| Correction min / max | +6.074 / +86.732 | −6.462 / +86.725 | −6.462 / +86.725 ppm |
| Input / output overruns | 0 / 0 | 0 / 0 | 0 / 0 |
| Output underruns | 0 | 0 | 0 |
| Backend xruns / errors | 0 / 0 | 0 / 0 | 0 / 0 |
| Correction errors | 0 | 0 | 0 |

The 60-second slope contains controller startup and is not used as the
long-term estimate. Over 30 minutes the total fill remained centered on the
3.5-frame target and its residual slope was 0.00615 samples/s, or 0.128 ppm.
There was no normal sustained queue growth and no steady-state loss event.

The 30-minute callback histogram contained 180,010 capture callbacks of 480
frames. Render contained 180,011 callbacks of 480 frames, one 960-frame
callback, and the initial 1,056-frame callback. Mean capture/render cadence was
9.99925 / 9.99931 ms; observed maxima were 22.504 / 23.875 ms. This confirms
that callback size and cadence are not treated as the fixed processing frame.

Four consecutive two-second start/stop cycles also completed with MMCSS
registration successful and zero underruns, overruns, stream errors, or xruns
in every cycle.

## Latency accounting

Machine reports keep these values separate:

- capture/render callback duration;
- fixed-frame processor plus resampler execution duration;
- software capture-to-output-dequeue latency;
- input and output WASAPI engine periods;
- startup pre-roll;
- asynchronous resampler filter delay;
- physical or loopback end-to-end latency.

No physical end-to-end value has been measured yet. The JSON correctly reports
`end_to_end_latency_measured: false`. The `auralis-bench latency-reference` and
`latency-analyze` commands implement a reproducible pulse/correlation method
and report median, p95, p99, maximum, and sample count. The deterministic test
recovers a synthetic 240-sample delay as 5.0 ms. The required Windows physical
or documented loopback procedure is in `docs/end-to-end-latency.md`.

The accepted 30-minute software measurements were:

| Category | Average | Maximum |
|---|---:|---:|
| Capture callback duration | 0.00171 ms | 0.1637 ms |
| Render callback duration | 0.00171 ms | 0.1723 ms |
| Passthrough + resampler processing | 0.0293 ms | 1.251 ms |
| Software capture-to-render dequeue | 45.964 ms | 59.611 ms |

The separate measured device engine periods were 10 ms input and 10 ms output;
startup pre-roll was 30 ms and resampler filter delay was 1.333 ms. These
values are not added and labelled physical end-to-end latency because the
device and acoustic/cable path has not been correlated. Average/max process CPU
was 4.894% / 29.907%; peak RSS was 18,239,488 bytes.

## Reproduction

```powershell
auralis-cli.exe devices > bench\work\native-windows-devices.json
auralis-cli.exe characterize 60 --sample-ms 100 `
  --output bench\work\native-windows-60s.json
auralis-cli.exe characterize 600 --sample-ms 100 `
  --output bench\work\native-windows-10m.json
auralis-cli.exe characterize 1800 --sample-ms 100 `
  --output bench\work\native-windows-30m.json
auralis-cli.exe restart 4 2 > bench\work\native-windows-restart.json
```

Output files use schema version 4 and include formats, periods, callback
histograms/cadence, timestamps, stream clocks, queue time series, effective
buffer bound, clock estimates, correction ratio, worker scheduling, pre-roll,
latency breakdown, CPU/RSS, and xrun counts. `--no-drift-correction` reproduces
an uncorrected control run.

## Milestone 2 acceptance

The native default Windows duplex path meets the Milestone 2 baseline: device
period and processing frame are decoupled, drift is quantified and corrected
with bounded slow feedback, the 30-minute queue remains bounded, all accepted
runs have zero steady-state underrun/overrun/xrun, and repeated startup is
deterministic. Denoiser integration remains out of scope and has not started.

## Remaining risks

- The G435 input and output may share hardware clocking; a separate physical
  capture/render pair has not been soaked.
- Device disconnect/reconnect and default-device changes require an intentional
  manual Windows test and were not triggered during unattended runs.
- Physical end-to-end latency and transparent-speech listening evidence remain
  unmeasured; software queue timing is not a substitute.
- The accepted 30-minute software pipeline average was 45.964 ms. It proves
  stability but does not meet the future Balanced latency target; lowering the
  conservative fill/pre-roll without reintroducing xruns remains follow-up.
- A two-hour native soak is supported by the command but was not run; the
  longest native evidence is 30 minutes.
- No denoiser, inference runtime, virtual microphone, or audio telemetry was
  introduced in Milestone 2.

## External record synchronization

This repository document remains the measurement source of truth. Obsidian
sync was not performed because the configured vault path
`C:\Users\danda\File\Latte-Notes` was not present in the current WSL mount;
the next action is to mount or restore that vault and link this report from the
existing Auralis note. Notion sync was not performed because no Notion
connector is available in this environment; the next action is to connect the
workspace and link this report rather than duplicate its full contents.
