<div align="center">
  <img src="docs/assets/auralis-header.svg" alt="Auralis — 音声を測定しながら、ローカルで整える" width="100%" />
  <p><strong>日本語</strong> · <a href="README.en.md">English</a></p>
  <p>
    <a href="https://github.com/sahenjp/auralis/actions/workflows/ci.yml"><img src="https://github.com/sahenjp/auralis/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
    <img src="https://img.shields.io/badge/code-Apache--2.0-79bda0" alt="Apache-2.0 source license" />
    <img src="https://img.shields.io/badge/platform-Windows%2011-526b65" alt="Windows 11" />
  </p>
  <p><a href="https://huggingface.co/j-llm/Auralis/resolve/main/Auralis-Windows-x64.zip"><img src="https://img.shields.io/badge/DOWNLOAD-WINDOWS%20X64-9bd9b4?style=for-the-badge" alt="Download Auralis for Windows x64" /></a></p>
</div>

# Auralis

**マイクの背景ノイズを抑えて、声を聞きやすく。音声はPC内で処理します。** AuralisはWindows 11向けのリアルタイム音声ノイズ抑制アプリです。

<p align="center">
  <img src="docs/assets/auralis-gui-preview.png" alt="AuralisのGUI。マイクと出力先、Balancedモードを選ぶ画面" width="94%" />
  <br /><sub>マイク音声を処理し、選択したヘッドホンでモニターできます。</sub>
</p>

## できること

- マイク音声の背景ノイズをリアルタイムに抑制
- 入力・出力デバイスと処理モードをGUIから選択
- CPU、推論時間、queue、underrun/xrunを確認
- 音声はPC内で処理し、外部へアップロードしません

## 今すぐ試す

1. [Windows x64版をダウンロード](https://huggingface.co/j-llm/Auralis/resolve/main/Auralis-Windows-x64.zip)して展開。
2. `start-auralis.cmd` をダブルクリック。初回は固定モデルを取得・検証します。
3. マイクとヘッドホンを選び、**Balanced → Start**。

Windows 11 x64とMicrosoft Visual C++ 2015–2022 x64 Runtimeが必要です。初回のモデル取得時だけインターネットを使います。ヘッドホンでの利用を推奨します。

## できないこと

- Discord/Teamsなどの入力に選べる**仮想マイク**（未実装）
- AEC、話者分離（未実装）
- 8 kHzを超える音声帯域の保持（モデルの出力帯域制限）

## 測定結果

<details>
<summary>評価とリアルタイム測定</summary>

- UL-UNAS: 雑音あり630ケースで平均 SI-SDR **7.025 dB**、STOI変化 **+0.0057**
- Windows 11実機: 30分で音声欠落/underrun/xrun 0件、推論時間 p95 **1.35 ms**
- ソフトウェア経路遅延の平均は **89.3 ms**。物理的なマイク–スピーカー間遅延ではありません。
- ブラインド聴取は未実施。計測は好みや自然さを示しません。

詳細: [測定レポート](docs/milestone-3-results.md) · [評価方法](docs/benchmark-methodology.md) · [遅延の定義](docs/latency-budget.md)

</details>

モデル重みはAuralisが学習したものではなく、[Xiaobin-Rong/UL-UNAS](https://github.com/Xiaobin-Rong/ul-unas)の固定版です。ライセンスとSHA-256は[モデルカード](https://huggingface.co/j-llm/Auralis)を参照してください。

Auralisのソースコード: [Apache-2.0](LICENSE)。