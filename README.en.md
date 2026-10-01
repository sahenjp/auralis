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

**A local-first speech-denoising prototype for Windows 11.** Audio stays on the device; inference runs outside the real-time callbacks.

> A measured work in progress. These results do not establish listener preference, commercial superiority, or physical end-to-end latency.

## Snapshot

| UL-UNAS evaluation | Result |
|---|---:|
| SI-SDR · 630 noisy cases | **7.025 dB mean** |
| STOI change · same cases | **+0.0057 mean** |
| Native Windows 11 run | **30 min · zero losses/underruns/xruns** |
| Inference time | **1.35 ms p95** |

The model path is limited to 8 kHz bandwidth. Clean-speech 8–20 kHz energy changed by −13.112 dB mean. The 89.3 ms software-path figure is not physical microphone-to-speaker latency.

## Start

Download the [prebuilt Windows x64 package](https://huggingface.co/j-llm/Auralis/resolve/main/Auralis-Windows-x64.zip), extract it, and **double-click `start-auralis.cmd`**. First launch downloads and verifies the pinned model.

To run from source, install Rust and double-click `start-auralis.cmd` in the repository. It builds the GUI on first launch.

The weights were not trained by Auralis. They are the pinned upstream release from [Xiaobin-Rong/ul-unas](https://github.com/Xiaobin-Rong/ul-unas), shared under its non-exclusive MIT license on [Hugging Face](https://huggingface.co/j-llm/Auralis). SHA-256: `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`.

For a device-free check:

```bash
cargo run -p auralis-cli -- simulate 2
```

## Scope

- RNNoise: lightweight reference. UL-UNAS: provisional quality engine.
- AEC, virtual microphone, target-speaker extraction, and blind listening are not implemented/completed.
- [Measurements](docs/milestone-3-results.md) · [Methodology](docs/benchmark-methodology.md) · [Latency](docs/latency-budget.md) · [Hugging Face card](https://huggingface.co/j-llm/Auralis)

Source code: [Apache-2.0](LICENSE). Audio corpora and test recordings are not included.
