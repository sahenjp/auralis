# Auralis architecture

## Product invariants

- Windows 11 is the primary product target.
- Audio never leaves the machine by default.
- The CPU path must sustain realtime operation on an ordinary modern desktop CPU.
- Every processing stage declares frame size, state, lookahead, supported sample rate, and worst observed processing time.
- Audio callbacks do only bounded conversion/copy, wait-free queue operations, and atomic metric updates.
- Quality and performance claims require a frozen corpus, machine-readable results, and blind listening evidence.

## Runtime topology

```text
physical microphone                         render/reference capture
        |                                             |
 capture callback (bounded)                 reference callback (bounded)
        |                                             |
 fixed-capacity SPSC                         fixed-capacity SPSC
        +--------------------+------------------------+
                             |
                    processing worker
       conditioning -> AEC -> enhancement/TSE -> AGC -> limiter
                             |
                    fixed-capacity SPSC
                             |
                     render/transport callback
                             |
             monitor output now; virtual capture endpoint later
```

The initial processor is identity passthrough. The interface is intentionally frame-oriented so RNNoise-style DSP, DeepFilterNet, ONNX models, and AEC3 adapters can be benchmarked without rewriting device I/O.

## Technology allocation

| Concern | Initial decision | Reason |
|---|---|---|
| Realtime core and orchestration | Rust | Native performance, ownership-enforced SPSC endpoints, safer concurrency, strong tests/tooling |
| Phase 1 device I/O | CPAL 0.18.2 | Current low-level library with WASAPI, Linux, device enumeration, fixed buffers, and timestamps |
| Windows release I/O | CPAL plus narrow WASAPI/MMCSS adapter | CPAL exposes required transport timing; native code supplies period inspection and measured worker scheduling only |
| Queue | `rtrb` 0.4.0 | Fixed allocation at construction; reads/writes documented lock-free and wait-free |
| AEC | WebRTC AEC3 C++ adapter candidate | Mature render-reference design; do not invent a full canceller first |
| Neural inference | Backend-neutral adapter; CPU required, WinML/ORT acceleration optional | Avoid coupling the pipeline to a model or GPU vendor |
| Model research | Python allowed outside realtime product | Training and objective metrics have mature Python ecosystems; exported artifacts run locally in the product |
| Virtual microphone | Signed WaveRT virtual capture endpoint, with user-mode engine transport | A normal system-wide selectable capture endpoint ultimately requires a driver-class endpoint; keep DSP out of kernel mode |

An all-C++ product would integrate directly with WDK and WebRTC but expands memory-safety and concurrency risk across the whole system. An all-Rust product would make WDK samples and AEC3 integration unnecessarily difficult. Python or a managed runtime is unsuitable inside the bounded callback/worker path, but remains useful for research.

## Audio contract

- Internal format: mono `f32`, nominal range `[-1, 1]`
- Sample rate: 48,000 Hz
- Frame: 480 samples / 10 ms
- Queues: four frames by default; capacity is a safety bound, not a target fill level

The 480-sample processing frame is not a hardware-period requirement. Capture
callbacks append arbitrary frame counts to a fixed-capacity frame adapter. It
publishes a complete 480-sample frame only when one is available. The render
adapter performs the inverse operation and can split or merge processed frames
to satisfy arbitrary render callback lengths. Both adapters own fixed storage
allocated before streaming starts.

```text
arbitrary capture callback frames
  -> bounded capture frame adapter
  -> 480-sample SPSC input queue
  -> processing worker
  -> bounded asynchronous resampler and fixed output adapter
  -> 480-sample SPSC output queue
  -> bounded render frame adapter
  -> arbitrary render callback frames
```

## Milestone 2 Windows backend boundary

CPAL 0.18.2 remains the audio transport. On Windows it provides stable endpoint
IDs, WASAPI capture/playback timestamps, stream clocks, callback frame counts,
buffer-size reporting, and xrun errors. A narrow `auralis-wasapi` crate reads
`IAudioClient3` period information from CPAL's existing `IMMDevice` and owns
the processing worker's MMCSS registration; it does not duplicate stream
ownership or move platform types into `auralis-core`.

```text
Audio device transport: CPAL
  +-- portable callback and stream-clock interface
  +-- Windows-only read-only IAudioClient3 period probe
  +-- Windows worker-only Pro Audio MMCSS registration

Processing engine: auralis-core
  +-- no CPAL, COM, WASAPI, or OS-specific types
```

The probe records default, fundamental, minimum, maximum, and current shared
engine periods. A requested CPAL buffer size is recorded separately and is not
reported as an `InitializeSharedAudioStream` period request. CPAL currently
uses the ordinary WASAPI shared-stream initialization path; Auralis therefore
reports `requested_iaudioclient3_period_frames` as absent.

The MMCSS operation runs once on the processing worker's first frame, never in
a capture or render callback. A thread-local guard keeps the registration on
that same thread and reverts it when the worker exits. The machine report
records the requested class/priority, whether registration was attempted and
successful, and any error. This was added only after a native corrected trial
showed the ordinary-priority worker starved while the input queue filled and
the output queue drained.

Render begins in a waiting state. The first observed output callback determines
the minimal automatic pre-roll target as `ceil(callback_frames / 480)`, bounded
by queue capacity, unless the operator supplied an explicit target. The target
is latched for the stream lifetime. Waiting callbacks output silence but are
classified as pre-roll, not underruns. Once the target is reached, an ordinary
empty output queue is an underrun.

Clock drift is an observed property, not an assumed one. Diagnostics compare
capture device timestamps with render callback timestamps and regress
steady-state queue occupancy after pre-roll. An uncorrected 10-minute native
run demonstrated queue movement and a nonzero relative clock estimate, so the
worker now contains an evidence-gated asynchronous sinc resampler. A slow,
filtered controller keeps total buffered output near its target and clamps its
ratio to ±250 ppm. The resampler remains outside callbacks and its 64-sample
filter delay is included in latency accounting.

Variable resampler output is accumulated into fixed 480-sample frames. If a
frame completes while the output ring is full, one fixed worker-side slot
holds it and retries it before newer output. The total observable output bound
is therefore seven frames: four ring frames, one render partial frame, one
resampler partial frame, and one staged complete frame. This does not enlarge
the SPSC queue or hide its occupancy.

- Overrun policy: stage one completed output frame; if congestion persists with both bounded stores full, drop the newest frame and increment a counter
- Underrun policy: emit silence and increment callback/sample counters
- Device mismatch: reject unsupported 48 kHz paths until a measured asynchronous resampler and drift controller exist

## Processing order

The provisional order is input conditioning, AEC, enhancement/denoising, target-speaker extraction, AGC, limiter. AEC comes before nonlinear suppression so its adaptive filter sees a minimally altered microphone signal and an explicit render reference. Enhancement versus target-speaker extraction order is intentionally undecided: both orders will be evaluated for speaker leakage, false rejection, naturalness, and cost.

## Model gate

Milestone 3 provisionally selects UL-UNAS for the balanced engine and retains
RNNoise as the low-latency reference. This is a measured local integration
decision, not a redistribution approval; the model-weight license gate remains
open. New denoisers enter through the same gate:

1. confirmed code, weight, and dataset licenses;
2. causal/lookahead audit;
3. CPU p50/p95/p99 inference time and realtime factor;
4. objective scores on frozen synthetic and captured sets;
5. blinded naturalness, intelligibility, suppression, artifact, and overall preferences;
6. failure analysis for keyboards, impulsive noise, music, echo, and interfering speech.

## Windows endpoint plan

An APO is a user-mode DSP object associated with an audio endpoint/driver and is valuable for endpoint effects and Windows 11 AEC reference integration. It is not assumed to be the simplest independent `Auralis Microphone` distribution mechanism. Microsoft SysVAD demonstrates a WDM WaveRT virtual device but is sample code, requires WDK work, signing, packaging, HLK testing, and production hardening. The planned split is therefore:

- signed, minimal virtual endpoint/transport component;
- unprivileged user-mode Auralis engine containing all DSP and ML;
- versioned IPC with bounded buffers and fail-silent behavior.

No kernel driver will be installed during ordinary development.

## Evidence checked 2026-08-26

- Microsoft, [Windows Audio Architecture](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/windows-audio-architecture)
- Microsoft, [`IAudioClient3`](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-iaudioclient3)
- Microsoft, [Multimedia Class Scheduler Service](https://learn.microsoft.com/en-us/windows/win32/procthread/multimedia-class-scheduler-service)
- Microsoft, [Windows 11 APO APIs](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/windows-11-apis-for-audio-processing-objects)
- Microsoft, [SysVAD virtual audio device sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-driver-samples/sysvad-virtual-audio-device-driver-sample/)
- Microsoft, [Windows driver signing](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/windows-driver-signing-tutorial)
- WebRTC, [AEC3 source](https://webrtc.googlesource.com/src/+/main/modules/audio_processing/aec3/)
- RustAudio, [CPAL](https://github.com/RustAudio/cpal)
- `rtrb`, [realtime SPSC documentation](https://docs.rs/rtrb/latest/rtrb/)
- ONNX Runtime, [execution providers](https://onnxruntime.ai/docs/execution-providers/)
- ONNX Runtime, [DirectML provider status](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)
- Xiph.Org, [RNNoise](https://github.com/xiph/rnnoise)
- DeepFilterNet authors, [DeepFilterNet](https://github.com/Rikorose/DeepFilterNet)
- Microsoft, [DNS Challenge](https://github.com/microsoft/DNS-Challenge)
