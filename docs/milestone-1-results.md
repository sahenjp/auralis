# Milestone 1 measured baseline

Date: 2026-08-26. These are development measurements, not release claims.

## Paced transport simulation

Command:

```bash
cargo build --release -p auralis-cli
target/release/auralis-cli simulate 2
```

On the WSL2 environment recorded in `docs/environment.md`:

| Metric | Result |
|---|---:|
| Frames captured / processed / rendered | 200 / 200 / 200 |
| Input / output overruns | 0 / 0 |
| Output underruns | 0 callbacks, 0 samples |
| Input callback average / max | 1.602 / 3.406 µs |
| Output callback average / max | 1.226 / 21.850 µs |
| Passthrough processing average / max | 0.030 / 0.050 µs |
| Processing realtime factor | 0.00000299 |
| Software transport latency average / max | 0.717 / 0.750 ms |
| Maximum input / output queue depth | 1 / 1 frame |
| Process CPU average / max | 0.984% / 4.959% (100% = one logical CPU) |
| Peak resident memory | 4,079,616 bytes |

The idle worker poll was changed from 100 µs to 500 µs after the first measured run: average sampled CPU fell from 4.97% to 0.98%, while simulated software transport latency rose by about 0.35 ms and remained below 0.75 ms maximum.

## Real WSLg duplex path

Command:

```bash
target/release/auralis-cli run 2
```

This exercised the WSLg PulseAudio `RDP Source` to `RDP Sink` path at 48 kHz, mono capture, stereo output, requested 480-frame buffers:

| Metric | Result |
|---|---:|
| Frames captured / processed / rendered | 204 / 204 / 199 |
| Input / output overruns | 0 / 0 |
| Output underruns | 1 callback, 960 samples |
| Stream errors / backend xruns | 0 / 0 |
| Input callback average / max | 1.398 / 2.996 µs |
| Output callback average / max | 1.303 / 4.077 µs |
| Processing average / max | 0.031 / 0.070 µs |
| Software latency average / max | 31.610 / 42.525 ms |
| Maximum input / output queue depth | 1 / 4 frames |
| Process CPU average / max | 3.693% / 10.084% |
| Peak resident memory | 5,423,104 bytes |

The initial PulseAudio callback requested more samples than the one-frame prefill, producing the counted startup underrun. Capture produced five more complete frames than render consumed and the output queue reached its four-frame safety bound. This is evidence that stream batching and/or clock drift must be controlled; it is not a reason to increase queue capacity. An asynchronous resampler/drift controller and long-duration Windows test are required.

The software-latency timestamp begins in the capture callback and ends when a processed frame is dequeued. It excludes physical ADC/DAC, Windows/WSLg transport before callback delivery, and virtual-endpoint buffering. Only a hardware loopback correlation test can establish physical end-to-end latency.

## Benchmark smoke harness

Command:

```bash
cargo run --release -p auralis-bench -- smoke --out-dir bench/work/smoke
```

Results:

- five requested SNRs measured within approximately 0.000000003 dB;
- raw and Auralis passthrough output hashes matched for every case;
- zero samples reached the 0.999 clipping threshold;
- passthrough offline realtime factor ranged from 0.0000079 to 0.0001065 across the five two-second cases;
- a second run produced byte-identical manifest and mixture WAV files;
- rerunning into a non-empty output directory failed without overwriting data.

The fixtures are deterministic voiced tones plus synthetic fan, white-noise, and impulse components. They validate harness mechanics only and cannot support a speech-quality or competitor claim.

## Remaining proof before Phase 3

- Run the native WASAPI binary on Windows 11 hardware for at least one hour.
- Add device invalidation/default-device-change recovery tests.
- Implement and measure clock-drift control without expanding the latency bound.
- Add physical loopback end-to-end latency measurement.
- Freeze licensed clean speech, real noise, echo, and interfering-speaker corpora.
- Pin and audit STOI, PESQ, and DNSMOS implementations before enabling those metrics.

