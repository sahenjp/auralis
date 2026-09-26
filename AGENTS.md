# AGENTS.md — Auralis

このリポジトリは新規の Auralis 実装である。過去の同名プロジェクトからコード、設計、依存関係を持ち込まない。

## 優先事項

1. 音声品質と再現可能な測定
2. リアルタイム安全性とレイテンシ上限
3. 安定性、保守性、Windows 11 互換性
4. 最小差分

「Krisp killer」は目標であり、比較可能な測定結果が揃うまで優越性を主張しない。

## リアルタイム規則

音声コールバック内では、次を禁止する。

- ヒープ確保と解放
- ロック、待機、sleep、I/O、ログ出力
- ニューラル推論、ファイルアクセス、ネットワークアクセス
- 長さに上限のないループ、予測不能なシステム呼び出し

コールバックは、形式変換、固定量のコピー、固定容量SPSCキュー操作、Atomicメトリクス更新だけを行う。重いDSPと推論は処理ワーカーへ置く。キュー容量を増やして性能問題を隠さず、オーバーラン時は新しいフレームを明示的に破棄して計数する。アンダーラン時は無音を出して計数する。

リアルタイム経路を変更したら、割り当て、ロック、ブロッキング、I/O、処理上限をコードレビューで再確認する。

## 構成境界

- `crates/auralis-core`: OS非依存の固定フレーム、処理インターフェース、キュー、メトリクス
- `crates/auralis-audio-io`: デバイス列挙とストリーム。OS固有コードを処理核へ漏らさない
- `crates/auralis-diagnostics`: コールバック外でのCPU・メモリ計測
- `apps/auralis-cli`: 手動実行と診断
- `tools/auralis-bench`: オフライン評価、混合生成、機械可読レポート
- `docs/adr`: 重要な設計判断
- `bench`: 凍結コーパスの配置規約。権利を確認できない音声を追加しない
- `models`: モデル本体は原則Git管理しない。由来、ライセンス、ハッシュを別途管理する

C/C++、Windows Driver Kit、WebRTC、推論ランタイムのコードを導入する場合は狭いアダプタへ隔離し、FFI所有権とスレッド規則を文書化する。カーネルモードでDSPや推論を行わない。

## 音声契約

- 内部基準: 48 kHz、mono、`f32`、10 ms（480 sample）フレーム
- フレーム処理は有界で、定常状態に割り当てを発生させない
- モデルのlookaheadと状態はメタデータとして公開し、レイテンシ計算へ含める
- AECはrender/reference信号なしに実装済みと扱わない
- サンプルレート差やクロックドリフトを無視しない。未対応時は起動時に明示的に失敗させる

## 検証

変更範囲に応じ、少なくとも次を実行する。

```bash
cargo fmt --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p auralis-cli -- simulate 2
cargo run -p auralis-bench -- smoke --out-dir bench/work/smoke
git diff --check
```

Windows音声I/O変更はWindows 11実機でデバイス列挙、start/stop、入力切断、出力切断、既定デバイス変更、長時間のunderrun/overrunも確認する。仮想ドライバーを無署名でインストールしない。

## セキュリティとデータ

マイク音声は機密データとして扱い、既定で完全ローカル処理とする。音声をテレメトリ、ログ、クラッシュレポート、外部APIへ送らない。診断へ残せるのは技術メトリクスだけである。モデル取得機能には許可済みURL、サイズ上限、暗号学的ハッシュ検証、原子的配置を要求する。

## Git

ユーザーの明示指示なしに commit、push、PR、Release、deploy、ドライバーインストールを行わない。未コミット変更を破棄しない。
