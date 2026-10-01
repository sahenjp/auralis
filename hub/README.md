---
language: [en, ja]
tags: [audio, speech-enhancement, denoising, ul-unas]
license: mit
---

<div align="center">
  <img src="https://raw.githubusercontent.com/sahenjp/auralis/main/docs/assets/auralis-header.svg" alt="Auralis — voice enhancement, measured and local" width="100%" />
  <p><a href="https://github.com/sahenjp/auralis/blob/main/README.md">日本語</a> · <a href="https://github.com/sahenjp/auralis/blob/main/README.en.md">English</a></p>
</div>

# Auralis · UL-UNAS

<a href="https://github.com/sahenjp/auralis/blob/main/README.md">日本語</a> · <a href="https://github.com/sahenjp/auralis/blob/main/README.en.md">English</a>

**Local speech denoising for Windows 11.** These weights are the original upstream UL-UNAS release; Auralis did not train them. Auralis evaluates them in a 48/16/48 kHz pipeline.

## Download

- [Auralis Windows x64 GUI package](https://huggingface.co/j-llm/Auralis/resolve/main/Auralis-Windows-x64.zip)
- [ulunas_stream_simple.onnx](https://huggingface.co/j-llm/Auralis/resolve/d6fe7e57b4f3c3d2744bcd74bf0cbe37b9e1aa55/ulunas_stream_simple.onnx)
- SHA-256: `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`
- Source: [Xiaobin-Rong/ul-unas](https://github.com/Xiaobin-Rong/ul-unas/tree/00f7c700da43d38347f30a6ccebd86fcbc798e07)
- License: non-exclusive MIT; see [`LICENSE`](LICENSE)

The weights are the pinned upstream artifact. No Auralis test recordings or benchmark audio are included.

## Evaluation snapshot

| Measure | Result |
|---|---:|
| SI-SDR · 630 noisy cases | **7.025 dB mean** |
| STOI change | **+0.0057 mean** |
| Clean 8–20 kHz energy change | **−13.112 dB mean** |
| Windows 11 stability run | **30 min · zero losses/underruns/xruns/deadline misses** |
| Inference time | **1.35 ms p95** |

The model outputs up to 8 kHz. These are objective engineering measurements, not listening-test results or physical end-to-end latency.

## 日本語

Auralisが学習した重みではありません。上流UL-UNASの固定ONNXを、Auralisの48/16/48 kHz経路で評価しています。

- [モデルをダウンロード](https://huggingface.co/j-llm/Auralis/resolve/d6fe7e57b4f3c3d2744bcd74bf0cbe37b9e1aa55/ulunas_stream_simple.onnx)
- [Auralis Windows x64 GUIパッケージ](https://huggingface.co/j-llm/Auralis/resolve/main/Auralis-Windows-x64.zip)
- SHA-256: `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`
- ライセンス: 上流の非独占MIT。詳細は [`LICENSE`](LICENSE)

| 指標 | 結果 |
|---|---:|
| SI-SDR · 雑音あり630ケース | **平均 7.025 dB** |
| STOI変化 | **平均 +0.0057** |
| クリーン音声8–20 kHz成分変化 | **平均 −13.112 dB** |
| Windows 11実機 | **30分・欠落/underrun/xrun/期限超過 0件** |
| 推論時間 | **p95 1.35 ms** |

モデルの出力帯域は8 kHzまでです。指標は聴感評価や実機の物理的な総遅延を示すものではありません。評価音声は配布していません。

詳細なアダプター、測定方法、再現手順: [Auralis GitHub](https://github.com/sahenjp/auralis)。
