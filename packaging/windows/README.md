# Auralis for Windows x64

日本語 / English

## 日本語

1. ZIPを展開します。
2. `start-auralis.cmd` をダブルクリックします。

Windows 11 x64とMicrosoft Visual C++ 2015–2022 x64 Runtimeが必要です（未導入の場合は[Microsoftから取得](https://aka.ms/vs/17/release/vc_redist.x64.exe)）。初回起動時はUL-UNASモデルを取得し、サイズとSHA-256を検証します。音声はPC内で処理し、送信しません。初回起動にはインターネット接続が必要です。

モデル重みはAuralis製ではなく、Xiaobin-Rong/UL-UNASの固定版です。非独占MITライセンス全文は `UL-UNAS-MIT-LICENSE.txt` にあります。AuralisのソースコードはApache-2.0です。

## English

1. Extract the ZIP.
2. Double-click `start-auralis.cmd`.

Requires Windows 11 x64 and the Microsoft Visual C++ 2015–2022 x64 Runtime (install it from [Microsoft](https://aka.ms/vs/17/release/vc_redist.x64.exe) if missing). On first launch, the pinned UL-UNAS model is downloaded and checked against its expected size and SHA-256. Audio stays on the PC and is not uploaded. Internet access is needed for the first launch.

The model weights were created upstream by Xiaobin-Rong/UL-UNAS, not by Auralis. The non-exclusive MIT license is included as `UL-UNAS-MIT-LICENSE.txt`; Auralis source is Apache-2.0.
