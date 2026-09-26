---
language: [en, ja]
tags: [audio, speech-enhancement, denoising, ul-unas]
---

<div align="center">
  <img src="https://raw.githubusercontent.com/sahenjp/auralis/main/docs/assets/auralis-header.svg" alt="Auralis — voice enhancement, measured and local" width="100%" />
  <p><a href="https://github.com/sahenjp/auralis/blob/main/README.md">日本語</a> · <a href="https://github.com/sahenjp/auralis/blob/main/README.en.md">English</a></p>
</div>

# Auralis · UL-UNAS evaluation

**Local speech enhancement, measured in the open.** This page summarizes Auralis' evaluation of the pinned UL-UNAS streaming ONNX artifact. It is an evaluation card, not a release of the model weights.

## At a glance

| Measurement | Result |
|---|---:|
| Noisy-speech SI-SDR · 630 frozen cases | **7.025 dB mean** |
| STOI change · same cases | **+0.0057 mean** |
| Clean-speech 8–20 kHz energy change | **−13.112 dB mean** |
| Native Windows 11 stability run | **30 min · 0 losses / underruns / xruns / deadline misses** |
| UL-UNAS inference · same run | **1.35 ms p95** |

These are engineering measurements, not listening-test results or a commercial-product comparison. The 16 kHz model path is bandwidth-limited to 8 kHz and measurably reduces high-frequency speech energy. The reported 89.3 ms average is accounted software-pipeline latency; physical device/acoustic end-to-end latency remains unmeasured.

## Artifact and provenance

- **Model:** UL-UNAS DNS3 streaming ONNX
- **Upstream repository:** [Xiaobin-Rong/ul-unas](https://github.com/Xiaobin-Rong/ul-unas)
- **Pinned revision:** `00f7c700da43d38347f30a6ccebd86fcbc798e07`
- **Artifact path:** `ulunas_onnx/onnx_models/ulunas_stream_simple.onnx`
- **SHA-256:** `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`
- **Signal path:** 48 kHz mono → 16 kHz model → 48 kHz output; 8 kHz output bandwidth

### Model availability and terms

**No ONNX weights or evaluation audio are hosted in this repository.** The pinned upstream repository has a non-exclusive MIT license for its software. The frozen model directory has no separate weight-specific license notice, so this card records the repository license without making a separate claim that the ONNX artifact is cleared for redistribution. The file remains available from its [pinned upstream source](https://github.com/Xiaobin-Rong/ul-unas/blob/00f7c700da43d38347f30a6ccebd86fcbc798e07/ulunas_onnx/onnx_models/ulunas_stream_simple.onnx); check the upstream terms before use or redistribution.

## Evaluation notes

Auralis uses the official stateful ONNX graph with explicit caches and a 48/16/48 kHz processing path. Inference runs on a worker, outside the real-time audio callbacks. The frozen offline corpus contains Japanese and English speech with six recorded DEMAND noise environments plus deterministic combinations; the broader requested noise and microphone-distance coverage remains incomplete.

No blinded human listening responses have been collected. SI-SDR and STOI cannot establish naturalness, absence of artifacts, or listener preference. No acoustic echo cancellation, target-speaker extraction, or virtual microphone is included.

## Reproduce

The Auralis repository includes the adapter, pinned metadata, manifests, measurement methodology, and full results:

- [Source and setup](https://github.com/sahenjp/auralis)
- [Model-lab commands](https://github.com/sahenjp/auralis/blob/main/tools/auralis-model-lab/README.md)
- [Frozen artifact metadata](https://github.com/sahenjp/auralis/blob/main/bench/candidates/frozen-set-v1.json)
- [Benchmark methodology](https://github.com/sahenjp/auralis/blob/main/docs/benchmark-methodology.md)
- [Milestone 3 report](https://github.com/sahenjp/auralis/blob/main/docs/milestone-3-results.md)

---

# 日本語

**音声強調を、測定とともに公開します。** このページは、固定したUL-UNAS streaming ONNXをAuralisに組み込んで評価した結果です。モデル重みの配布ページではありません。

## 測定サマリー

| 指標 | 結果 |
|---|---:|
| 雑音下SI-SDR · 固定630ケース | **平均 7.025 dB** |
| STOI変化 · 同じケース | **平均 +0.0057** |
| クリーン音声の8–20 kHz成分変化 | **平均 −13.112 dB** |
| Windows 11実機安定性 | **30分 · 欠落 / underrun / xrun / 期限超過 0件** |
| 同テストのUL-UNAS推論時間 | **p95 1.35 ms** |

これは技術測定であり、聴取テストや商用製品との比較結果ではありません。16 kHzモデルの出力帯域は8 kHzまでで、高域音声成分の低下を実測しています。平均89.3 msは計上上のソフトウェア経路遅延であり、実機の音響・デバイスを含む総遅延は未測定です。

## モデルと由来

- **モデル:** UL-UNAS DNS3 streaming ONNX
- **上流リポジトリ:** [Xiaobin-Rong/ul-unas](https://github.com/Xiaobin-Rong/ul-unas)
- **固定リビジョン:** `00f7c700da43d38347f30a6ccebd86fcbc798e07`
- **ファイル:** `ulunas_onnx/onnx_models/ulunas_stream_simple.onnx`
- **SHA-256:** `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`
- **音声経路:** 48 kHz mono → 16 kHzモデル → 48 kHz出力（帯域上限8 kHz）

### 重みの公開・ライセンス

**このリポジトリにはONNX重みも評価用音声も置いていません。** 固定した上流リポジトリのソフトウェアには非独占のMITライセンスがあります。一方、モデルディレクトリ内には重みに限定した別のライセンス表記が見つかっていません。そのためこのページでは、ONNXの再配布許諾が確認済みとは扱っていません。ファイルは[固定した上流ソース](https://github.com/Xiaobin-Rong/ul-unas/blob/00f7c700da43d38347f30a6ccebd86fcbc798e07/ulunas_onnx/onnx_models/ulunas_stream_simple.onnx)で確認し、利用・再配布前に上流条件を確認してください。

## 評価の範囲と限界

Auralisは上流の状態付きONNXグラフと明示的なキャッシュを使い、48/16/48 kHz経路で評価しています。推論はリアルタイム音声コールバック外のワーカーで実行します。固定コーパスには日本語・英語音声、6種類の実録DEMAND雑音と決定論的な組み合わせを含みます。要求された全雑音条件やマイク距離条件はまだ網羅していません。

ブラインド聴取の回答はまだありません。SI-SDRやSTOIだけでは自然さ、ノイズ抑制アーティファクト、聴取者の好みは判断できません。AEC、話者抽出、仮想マイクも含みません。

## 再現方法

アダプター、固定モデル情報、コーパスマニフェスト、測定方法、全結果は[Auralisリポジトリ](https://github.com/sahenjp/auralis)で公開しています。

- [導入とモデルラボの実行方法](https://github.com/sahenjp/auralis/blob/main/tools/auralis-model-lab/README.md)
- [固定アーティファクト情報](https://github.com/sahenjp/auralis/blob/main/bench/candidates/frozen-set-v1.json)
- [ベンチマーク手法](https://github.com/sahenjp/auralis/blob/main/docs/benchmark-methodology.md)
- [Milestone 3測定レポート](https://github.com/sahenjp/auralis/blob/main/docs/milestone-3-results.md)
