# Progress

箱庭（Hakoniwa）製品版として完了した状態を記録する。プロトコル基盤・Play ライフサイクル・箱庭ゲームループまで実装済み。

最終更新: 2026-08-04（完了ドキュメントの日本語化）

---

## 完了サマリ

| 領域 | 状態 |
|------|------|
| ログイン〜Play（プロトコル 763） | 完了 |
| 箱庭マップ・衝突・パック | 完了 |
| Overworld / Nether / End | 完了 |
| モブ・流体・チェスト | 完了 |
| ポータル転送・地形 ID 修正 | 完了 |
| 差分適合テスト | 28 件（`xtask`） |

Play 到達後の主経路は箱庭モードである。無限バニラ生成・Forge 互換は対象外。

---

## 箱庭実装履歴

| PR | 内容 |
|----|------|
| #160 | H0 — スコープ文書・CI ゲート |
| #161 | H1 — 衝突・落下・踏み固め |
| #162 | H2 — `.rbpk` マップパック |
| #167 | H3–H6 — 次元・モブ・流体・チェスト |
| #168 | パレット ID / 水同期 / 視点保持 |
| #169 | チェスト・ポータル到着・帰還条件 |

詳細仕様: [`docs/hakoniwa.md`](docs/hakoniwa.md)

---

## プロトコル・サーバ基盤（完了）

| 項目 | 状態 |
|------|------|
| Status / Login / Configuration / Play | 完了 |
| 圧縮・暗号化 | 完了 |
| Keep Alive / チャット / コマンド | 完了 |
| チャンク・ライト・エンティティ・インベントリ | 完了 |
| `server.properties` / オペレータ | 完了 |
| CI（`fmt` / `clippy` / `test` / `xtask check`） | 完了 |

---

## ディレクトリ

| パス | 役割 |
|------|------|
| `crates/rustbound-protocol/` | パケット・コーデック・暗号 |
| `crates/rustbound-server/` | 接続・tick・箱庭ワールド |
| `crates/rustbound-conformance/` | 差分適合テスト |
| `docs/` | 設計・箱庭仕様 |
| `data/hakoniwa/packs/` | 同梱マップパック |

参照用の Forge インストール・`素体データ/` はリポジトリに含めない。

---

## 残課題（製品スコープ外）

以下は箱庭完了後も意図的に未着手とする項目である。製品必須ではない。

| 項目 | 備考 |
|------|------|
| プラグイン／スクリプト API | 将来拡張 |
| マルチワールド同時稼働の運用面 | 単一ワールド想定で十分 |
| ハブ／マッチメイキング | 対象外 |
| リソースパック配信の拡張 | 現状は同梱パックで足りる |

旧ロードマップ上の Phase I（プラグイン）は着手していない。

---

## 検証コマンド

```bash
cargo test --workspace
cargo run -p xtask -- check
```

---

## 参照

- [`README.md`](README.md)
- [`docs/hakoniwa.md`](docs/hakoniwa.md)
- [`AGENTS.md`](AGENTS.md)
- [`docs/design.md`](docs/design.md)
