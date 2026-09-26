<div align="center">
  <img src="docs/assets/auralis-header.svg" alt="Auralis — 音声を測定しながら、ローカルで整える" width="100%" />
  <p><strong>日本語</strong> · <a href="README.en.md">English</a></p>
  <p>
    <a href="https://github.com/sahenjp/auralis/actions/workflows/ci.yml"><img src="https://github.com/sahenjp/auralis/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
    <img src="https://img.shields.io/badge/code-Apache--2.0-79bda0" alt="Apache-2.0 source license" />
    <img src="https://img.shields.io/badge/platform-Windows%2011-526b65" alt="Windows 11" />
  </p>
</div>

# Auralis

**声を、外へ送らずに聞き取りやすく。** Auralisは、Windows 11向けのローカル優先・リアルタイム音声強調プロトタイプです。既存のWindows音声経路で入出力し、ノイズ抑制はリアルタイム音声コールバックの外にある処理ワーカーで行います。

> **測定可能な試作段階です。** 以下は再現可能な技術測定であり、聴感上の好み、マイクからスピーカーまでの実遅延、商用製品に対する優位性を示すものではありません。

## いま測れていること

| 結果 | 測定値 |
|---|---:|
| UL-UNASの雑音下SI-SDR | 644ケース中、雑音あり630ケースの平均 **7.025 dB** |
| 同STOI変化 | 同じ630ケースで平均 **+0.0057** |
| Windows 11実機の安定性 | **30分、音声欠落・underrun・xrun・期限超過 0件** |
| UL-UNAS推論時間 | 30分計測で **p95 1.35 ms** |
| 計上したソフトウェア経路遅延 | 平均 **89.3 ms**。機器・音響を含む総遅延ではありません |

品質候補は16 kHz動作で、出力帯域は8 kHzまでです。クリーン音声の8–20 kHz成分は平均 **−13.112 dB** 変化しました。集計値だけでは見えなくなる高域のトレードオフも公開しています。

詳しくは [Milestone 3測定結果](docs/milestone-3-results.md) · [遅延の定義](docs/latency-budget.md) · [評価方法](docs/benchmark-methodology.md) · [Hugging Faceモデルカード](https://huggingface.co/sahenjp/auralis)。

## エンジン

| プロファイル | エンジン | 状態 |
|---|---|---|
| `low-latency` | RNNoise | 軽量な比較・参照用。暫定採用 |
| `balanced` | UL-UNAS streaming ONNX | 暫定の品質候補。この評価の雑音コーパス集計で最良 |
| `maximum-quality` | DeepFilterNet3-LL | 実験用。Windowsで期限超過が起きたためリアルタイム用途には不採用 |

GTCRNはオフライン比較候補です。各プロファイルは単一エンジンを使い、複数のノイズ抑制器を直列接続しません。

## 試す

Rust 1.97.1を `rust-toolchain.toml` で固定しています。Ubuntu/WSLでは先に音声ヘッダーを入れてください。

```bash
sudo apt-get install libasound2-dev libpulse-dev
```

音声デバイスなしで再現可能なシミュレーションを実行:

```bash
cargo run -p auralis-cli -- simulate 2
```

Windows 11でデバイス一覧とローカル操作GUIを起動:

```powershell
cargo run --release -p auralis-cli -- devices
cargo run --release -p auralis-cli -- gui --profile balanced --model C:\Auralis\ulunas_stream_simple.onnx
```

ONNXファイルは**同梱していません**。利用権のあるモデルを指定してください。固定した上流リポジトリのソフトウェアには非独占のMITライセンスがありますが、モデル重みに限定した別個の許諾表示は確認できていません。Auralisは重みをホスト・再配布しません。

GUIではローカルセッションの開始・停止、入出力デバイス選択、queue / underrun / xrun / CPU / RSS / 推論時間 / ソフトウェア遅延の確認ができます。仮想マイク機能ではありません。

## 音声処理の構成

```text
WASAPI / CPAL capture → 有界フレームキュー → 処理ワーカー → 有界フレームキュー → render
```

内部形式は48 kHz mono `f32`、固定10 msフレームです。デバイスコールバックは有界な形式変換、固定サイズコピー、SPSCキュー操作、Atomicメトリクス更新のみを行います。モデルのロードと推論はコールバック外で実行し、クロック差の補正もワーカー側に置いています。

## 検証コマンド

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p auralis-cli -- simulate 2
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke
```

固定環境でのオフライン評価と再実行方法は [`tools/auralis-model-lab/README.md`](tools/auralis-model-lab/README.md) にあります。コーパス音声と生成WAVはGitに含めていません。

## 既知の制約

- 人によるブラインド聴取評価は未実施です。客観指標は自然さや好みを測りません。
- マイクからスピーカーまでの物理的な総遅延は未測定です。ソフトウェア遅延の値には機器・音響遅延を含みません。
- AEC、話者分離、仮想マイクは未実装です。
- 選定モデルの出力帯域は8 kHzまでで、高域の音声成分が減る場合があります。
- モデル重みはこのリポジトリにありません。重み固有の再配布条件は未確認です。

## リポジトリ案内

| パス | 内容 |
|---|---|
| `crates/auralis-core` | 固定フレーム、処理契約、有界キュー、メトリクス |
| `crates/auralis-audio-io` · `crates/auralis-wasapi` | デバイス・Windows音声アダプター |
| `crates/auralis-denoisers` | 分離されたノイズ抑制エンジン |
| `apps/auralis-cli` | デバイス確認、シミュレーション、ローカルGUI |
| `tools/auralis-bench` · `tools/auralis-model-lab` | 決定論的・オフライン評価 |
| `bench/` · `docs/` | データセット定義、評価方法、測定、設計記録 |

## ライセンス

Auralisのソースコードは[Apache-2.0](LICENSE)です。サードパーティのモデル重みとデータセットにはそれぞれ別の条件があります。利用・再配布の前に [`models/README.md`](models/README.md) と候補ごとのライセンス記録を確認してください。
