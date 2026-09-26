<div align="center">
  <img src="docs/assets/auralis-header.svg" alt="Auralis — voice enhancement, measured and local" width="100%" />
  <p><a href="README.md">日本語</a> · <strong>English</strong></p>
  <p>
    <a href="https://github.com/sahenjp/auralis/actions/workflows/ci.yml"><img src="https://github.com/sahenjp/auralis/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
    <img src="https://img.shields.io/badge/code-Apache--2.0-79bda0" alt="Apache-2.0 source license" />
    <img src="https://img.shields.io/badge/platform-Windows%2011-526b65" alt="Windows 11" />
  </p>
</div>

# Auralis

**Clearer voice, without sending it away.** Auralis is a local-first, real-time speech-enhancement prototype for Windows 11. It captures and renders audio through the existing Windows audio path, and keeps denoising on a worker thread outside the real-time device callbacks.

> **Measured prototype, not a product claim.** The results below are reproducible engineering measurements. They do not establish subjective preference, physical end-to-end latency, or superiority over commercial software.

## What has been measured

| Result | Measurement |
|---|---:|
| Noisy-speech SI-SDR, UL-UNAS | **7.025 dB mean** across 630 frozen cases |
| STOI change, UL-UNAS | **+0.0057 mean** across the same cases |
| Native Windows 11 stability | **30 min, zero** input/output losses, underruns, xruns, or deadline misses |
| UL-UNAS inference time | **1.35 ms p95** in the recorded 30-minute run |
| Accounted software pipeline | **89.3 ms average**; this is not acoustic/device end-to-end latency |

The selected quality engine operates at 16 kHz and attenuates speech energy above 8 kHz. The frozen measurements show a **−13.112 dB** mean change in clean-speech 8–20 kHz energy. That trade-off is part of the result, not hidden by the aggregate score.

More detail: [Milestone 3 measurements](docs/milestone-3-results.md) · [Latency definitions](docs/latency-budget.md) · [Evaluation method](docs/benchmark-methodology.md) · [Hugging Face model card](https://huggingface.co/sahenjp/auralis).

## Engines

| Profile | Engine | Status |
|---|---|---|
| `low-latency` | RNNoise | Provisional lightweight reference |
| `balanced` | UL-UNAS streaming ONNX | Provisional quality engine; strongest aggregate noisy-corpus result in this evaluation |
| `maximum-quality` | DeepFilterNet3-LL | Experimental; rejected for live use after Windows deadline misses |

GTCRN remains an offline comparison candidate. Profiles run one engine at a time; they do not cascade denoisers.

## Try it

Rust 1.97.1 is pinned in `rust-toolchain.toml`. On Ubuntu/WSL, install the audio headers first:

```bash
sudo apt-get install libasound2-dev libpulse-dev
```

Run the deterministic, device-free simulation:

```bash
cargo run -p auralis-cli -- simulate 2
```

On Windows 11, list devices and launch the local control GUI:

```powershell
cargo run --release -p auralis-cli -- devices
cargo run --release -p auralis-cli -- gui --profile balanced --model C:\Auralis\ulunas_stream_simple.onnx
```

The ONNX file is **not bundled**. Use only a model artifact you are authorized to use; the pinned upstream snapshot has a non-exclusive MIT license for its software, but no separate weight-specific notice was found in the model directory. Auralis does not host or redistribute that artifact.

The GUI starts a local session, selects input/output devices, and shows queue, underrun/xrun, CPU/RSS, inference, and software-latency diagnostics. It is not a virtual microphone.

## How the audio path is built

```text
WASAPI / CPAL capture → bounded frame queue → processing worker → bounded frame queue → render
```

The internal processing contract is 48 kHz mono `f32`, in fixed 10 ms frames. Device callbacks do only bounded format conversion, fixed-size copies, SPSC queue operations, and atomic metric updates. Model loading and inference stay outside the callbacks. Drift correction runs on the worker side.

## Reproduce the checks

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p auralis-cli -- simulate 2
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke
```

The pinned offline model-lab environment and bake-off commands are documented in [`tools/auralis-model-lab/README.md`](tools/auralis-model-lab/README.md). Audio corpora and generated WAVs are intentionally not checked in.

## Current limits

- No human listening responses have been collected; objective metrics do not measure naturalness or preference.
- Physical microphone-to-speaker latency has not been measured. The reported software latency excludes device and acoustic latency.
- No acoustic echo cancellation (AEC), target-speaker isolation, or virtual microphone is implemented.
- The selected model is limited to an 8 kHz output bandwidth and can reduce high-frequency speech detail.
- The model weights are not hosted here; their separate redistribution terms remain unconfirmed.

## Project map

| Path | Purpose |
|---|---|
| `crates/auralis-core` | Fixed frames, processing contract, bounded queues, metrics |
| `crates/auralis-audio-io` · `crates/auralis-wasapi` | Device and Windows audio adapters |
| `crates/auralis-denoisers` | Isolated denoiser implementations |
| `apps/auralis-cli` | Device tools, simulation, local control GUI |
| `tools/auralis-bench` · `tools/auralis-model-lab` | Deterministic and offline evaluation |
| `bench/` · `docs/` | Dataset manifests, methods, measurements, and design records |

## License

Auralis source code is licensed under [Apache-2.0](LICENSE). Third-party model weights and datasets have their own terms; see [`models/README.md`](models/README.md) and the pinned candidate records before using or redistributing them.
