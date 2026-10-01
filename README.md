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

<div align="center">
  <img src="docs/assets/auralis-header.svg" alt="Auralis — ローカル音声強調" width="100%" />
  <p><strong>日本語</strong> · <a href="README.en.md">English</a></p>
</div>

**Windows 11向けのローカル音声ノイズ抑制プロトタイプ。** 音声はデバイス上で処理し、推論はリアルタイムコールバックの外で実行します。

> 測定結果に基づく開発中の試作です。聴感上の優位性や物理的な総遅延は未確認です。

## 測定スナップショット

| UL-UNAS評価 | 結果 |
|---|---:|
| SI-SDR（雑音あり630ケース平均） | **7.025 dB** |
| STOI変化（同じケース） | **+0.0057** |
| Windows 11連続動作 | **30分・欠落/underrun/xrun 0件** |
| 推論時間 | **p95 1.35 ms** |

8 kHzを超える音声帯域は保持されず、クリーン音声の8–20 kHz成分は平均−13.112 dB変化しました。ソフトウェア経路遅延89.3 msは実機のマイクからスピーカーまでの総遅延ではありません。

## 試す

```bash
cargo run -p auralis-cli -- simulate 2
```

Windows 11のローカルGUI（モデルファイルは別途必要）:

```powershell
cargo run --release -p auralis-cli -- gui --profile balanced --model C:\Auralis\ulunas_stream_simple.onnx
```

**UL-UNASの重みを[Hugging Face](https://huggingface.co/sahenjp/auralis)で公開しています**（非独占MITライセンス、SHA-256 `f2e804d54d6a88f4f82f44d86c9f1cf646db2509bfca935cfbfc5fcd8cbfac3b`）。ライセンス全文と上流出典はモデルカードに記載しています。

## 現在の範囲

- RNNoise: 軽量参照エンジン。UL-UNAS: 暫定品質エンジン。
- AEC、仮想マイク、話者分離、ブラインド聴取評価は未実装/未実施。
- [測定レポート](docs/milestone-3-results.md) · [評価方法](docs/benchmark-methodology.md) · [遅延の定義](docs/latency-budget.md) · [Hugging Face](https://huggingface.co/sahenjp/auralis)

ソースコード: [Apache-2.0](LICENSE)。コーパス音声とテスト音声はリポジトリに含めていません。
