# Auralis

Auralis is a new, fully local real-time voice enhancement system for Windows 11. The project starts with a measured audio transport and benchmark foundation; AI denoising is deliberately not part of the first milestone.

“Krisp killer” is a target, not a claim. Auralis will make comparative claims only from reproducible, like-for-like measurements and blinded listening tests.

## Milestone 0/1 status

- 48 kHz mono internal contract with fixed 10 ms frames
- bounded, wait-free SPSC queues around a processing worker
- passthrough processor behind a replaceable `FrameProcessor` interface
- device enumeration and duplex capture/render through CPAL (WASAPI on Windows)
- deterministic paced simulation for machines without audio devices
- atomic callback, processing, queue, latency, underrun, and overrun metrics
- non-realtime sampling of process CPU and resident memory
- reproducible smoke benchmark with JSON and CSV output

## Milestone 2 status

- arbitrary device callback frame counts are adapted to fixed 480-sample
  processing frames without callback allocation
- native Windows reports include stable device IDs, formats, callback cadence
  and histograms, callback/device timestamps, CPAL stream clocks, WASAPI shared
  engine periods, queue time series, xruns, CPU, RSS, drift, and latency
- CPAL remains the stream backend; a narrow Windows-only adapter performs
  read-only `IAudioClient3` period queries
- startup uses an explicit minimal pre-roll target instead of counting expected
  startup silence as an underrun
- measured native long-run drift is corrected by a bounded worker-side
  asynchronous sinc resampler; no denoiser is enabled

## Product-quality prototype status

Auralis now exposes one primary engine per product profile; profiles never
cascade neural denoisers:

| Profile | Engine | Current acceptance |
|---|---|---|
| `low-latency` | RNNoise | provisional low-CPU fallback; weight clearance pending |
| `balanced` | UL-UNAS | provisional quality engine; native Windows stable; weight clearance pending |
| `maximum-quality` | DeepFilterNet3-LL | experimental; Windows first-inference deadline miss blocks realtime acceptance |

DeepFilterNet3-LL uses the pinned official 48 kHz model and produces the same
offline signal as the official runner within PCM16 quantization, but its first
native Windows inference measured hundreds of milliseconds. It is not silently
promoted past that failure. GTCRN remains an offline comparison candidate, not
a production engine.

The local blind quality gate contains 16 difficult clean, noisy, and transient
cases. It compares RAW, RNNoise, UL-UNAS, GTCRN, and DeepFilterNet anonymously
with keyboard playback and append-only JSONL ratings. See
`tools/auralis-model-lab/README.md`.

Auralis is still not a virtual microphone. AEC3 and target-speaker isolation
are not implemented yet.

The local control GUI is available through the CLI and runs as a loopback-only
browser window. It starts and stops the real duplex session, selects input and
output devices, and displays live engine diagnostics; it does not add work to
the audio callbacks.

## Build

Rust 1.97.1 is pinned by `rust-toolchain.toml`.

Ubuntu/WSL build prerequisites:

```bash
sudo apt-get install libasound2-dev libpulse-dev
cargo build --workspace --all-targets --all-features
```

Windows uses the CPAL WASAPI backend and does not require the Linux packages.

The current workspace also retains a local Windows artifact at
`target/windows-release/auralis-cli.exe` with its `DirectML.dll` sidecar. Copy
both files to the same Windows directory, keep the separately licensed model
file outside the repository, and run for example:

```powershell
.\auralis-cli.exe characterize 60 --profile balanced --model C:\Auralis\ulunas_stream_simple.onnx
```

This artifact has been link-checked as a Windows PE executable; native device
enumeration and capture/render execution still require running it on Windows
11. Do not redistribute the model or this build until the recorded weight and
third-party runtime terms are cleared.

## Run

```bash
# Works without an audio device; emits a JSON measurement report.
cargo run -p auralis-cli -- simulate 2

# Enumerate inputs and outputs.
cargo run -p auralis-cli -- devices

# Open the local control GUI (the pinned model path is required for Balanced).
cargo run --release -p auralis-cli -- gui \
  --profile balanced --model /path/to/ulunas_stream_simple.onnx

# Characterize the default native duplex path for 60 s and write JSON.
cargo run --release -p auralis-cli -- \
  characterize 60 --sample-ms 1000 --output bench/work/native-windows-60s.json

# Run the provisional balanced profile. The pinned UL-UNAS model is required.
cargo run --release -p auralis-cli -- \
  characterize 60 --profile balanced --model /path/to/ulunas_stream_simple.onnx

# Set AURALIS_GIT_REVISION to retain the source revision in the JSON report.

# Repeated deterministic stream lifecycle exercise.
cargo run --release -p auralis-cli -- restart 4 2

# Generate, process, and score deterministic smoke fixtures at five SNRs.
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke

# Generate a reference for physical or loopback latency measurement.
cargo run --release -p auralis-bench -- \
  latency-reference --out bench/work/latency-reference.wav
```

The WSL environment used to create the repository exposes WSLg PulseAudio sockets, but no hardware PCM device. `simulate` is therefore the local realtime verification path; the real duplex path must additionally be run from Windows 11.

## Documentation

- [Architecture](docs/architecture.md)
- [Latency budget](docs/latency-budget.md)
- [Benchmark methodology](docs/benchmark-methodology.md)
- [Observed development environment](docs/environment.md)
- [Milestone 1 measurements](docs/milestone-1-results.md)
- [Milestone 2 measurements](docs/milestone-2-results.md)
- [Milestone 3 denoiser bake-off and decision](docs/milestone-3-results.md)
- [End-to-end latency procedure](docs/end-to-end-latency.md)
- [Architecture decisions](docs/adr/)

## Repository layout

```text
apps/auralis-cli/          diagnostics and manual realtime runner
crates/auralis-core/       platform-independent realtime processing core
crates/auralis-audio-io/   device and stream adapters
tools/auralis-bench/       corpus generation and offline evaluation
bench/                     frozen-corpus contract and generated outputs
docs/adr/                  architecture decision records
models/                    model manifests later; model binaries are ignored
```

## Licensing

The Auralis source code is licensed under Apache-2.0. Third-party model weights have separate, unresolved terms and are not included or cleared for redistribution. Dataset audio is not included in this repository; consult each dataset's terms before fetching or redistributing it. The workspace remains marked `publish = false` for Rust package publication.
