# Rustbound

Minecraft Java Edition **1.20.1**（プロトコル **763**）向けのクリーンルーム実装サーバ。Rust で記述する。

公開ドキュメントとブラックボックス観測のみを根拠とする。Minecraft・Forge・mappings・逆コンパイル成果物などの参照用アーティファクトは、取り込み・再配布しない。

本プロジェクトは Mojang Studios、Microsoft、Forge プロジェクトとは無関係である。Minecraft および関連商標は、それぞれの権利者に帰属する。

## 状態: 完了（箱庭）

製品スコープは **箱庭（Hakoniwa）** である。固定サイズの庭であり、無限バニラ生成や Forge 互換は対象外とする。

| 項目 | 状態 |
|------|------|
| オフライン参加、Play セッション、20 TPS tick | 完了 |
| 固定箱庭（`tiny` / `small` / `medium`）とワールド境界 | 完了 |
| ブロック衝突、破壊／設置 | 完了 |
| マップパック（現世／ネザー／エンド） | 完了 |
| 次元移動（ポータル、`/dim`） | 完了 |
| 簡易 Mob | 完了 |
| 静的な水／溶岩 | 完了 |
| チェスト（最低限のコンテナ） | 完了 |
| `dist` プロファイルによる小型バイナリ | 完了 |
| オンラインモード | 任意（[#60](https://github.com/shumaimai/RustboundServer/issues/60)） |
| Forge／Bedrock／無限地形 | 対象外 |

仕様: [docs/hakoniwa.md](docs/hakoniwa.md)。経緯: [PROGRESS.md](PROGRESS.md)。

## 起動（オフライン）

```console
cp server.properties.example server.properties
cargo run -p rustbound-server --release -- --config server.properties
```

**1.20.1** のオフラインクライアントで `localhost:25565` に接続する。例示設定の既定は Creative と `hakoniwa-size=tiny` である。

配布用ビルド:

```console
cargo build -p rustbound-server --profile dist
```

スモーク確認:

```console
./scripts/smoke_offline_join.sh
cargo test -p rustbound-server --lib server_offline_playability_smoke
```

## ワークスペース

```
crates/
  rustbound-protocol/     # ワイヤコーデック、Login / Play 状態機械
  rustbound-server/       # リスナ、セッション、tick、ワールド、箱庭
  rustbound-conformance/  # ブラックボックス探査と差分適合
```

## ビルドとテスト

```console
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

## 設計上の制約

- 権威ある tick は単一スレッド（20 TPS）。セッション間はチャネルで連携する。
- 既存の `LoginStateMachine` / `PlayStateMachine` を優先して用いる。
- Forge はローカルでの oracle に限り利用可。成果物はコミットしない。
- `unsafe` はワークスペース既定では禁止。隔離・文書化・監査されたモジュールに限る。
- 貢献者は [AGENTS.md](AGENTS.md) に従う。

## ライセンス

MIT OR Apache-2.0（選択可）。
