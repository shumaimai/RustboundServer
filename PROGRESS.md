# RustboundServer 進捗レポート

## プロジェクト概要

Minecraft Java Edition 1.20.1 (protocol 763) と互換性のあるピュアRustサーバーの実装。
クリーンルーム実装であり、公開ドキュメントとブラックボックス観察のみを使用。

- **対象プロトコル:** Minecraft 1.20.1 (protocol 763)
- **言語:** Rust (edition 2024, MSRV 1.85)
- **ライセンス:** MIT OR Apache-2.0
- **リポジトリ:** https://github.com/shumaimai/RustboundServer

---

## マイルストーン完了状況

| マイルストーン | Issue数 | PR数 | テスト数 | 状態 |
|---|---|---|---|---|
| M1: Status Conformance | 5 | 5 | 21 | 完了 |
| M2: Login | 7 | 4 | 129 | 完了 |
| M3: Play | 7 | 2 | 163 | 完了 |
| M4: Server Core | 6 | 4 | 33 | 完了 |
| **合計** | **25** | **15** | **240** | **全完了** |

全240テスト合格 (23 conformance + 184 protocol + 33 server)
`cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` クリーン

---

## ワークスペース構成

```
rustbound-protocol   — プロトコルコーデック (framing, handshake, status, login, play)
rustbound-conformance — ブラックボックス適合性クライアント (status, login, play)
rustbound-server      — サーバーコア (listener, connection, tick, world, config)
```

### ソースファイル一覧 (25ファイル)

#### rustbound-protocol (12ファイル)
- `primitives.rs` — VarInt, String, UUID, f32/f64, bool, i8/i16/i32/i64, byte array
- `framing.rs` — 長さプレフィックスフレーミング、エンコード/デコード
- `state.rs` — ProtocolState (Handshaking/Status/Login/Play/Closed)、NextState
- `handshake.rs` — Handshake packet (0x00)
- `status.rs` — Status Request/Response (0x00), Ping/Pong (0x01)
- `compression.rs` — Set Compression packet、zlib圧縮/展開レイヤー
- `login.rs` — Login Start (0x00), Disconnect (0x00), Encryption Request/Response (0x01/0x02), Login Success (0x02), Plugin Request/Response (0x04/0x02)
- `login_state_machine.rs` — Login状態機械オーケストレーション
- `play.rs` — Join Game (0x28), Keep Alive (0x23/0x12), Player Position (0x14), Position+Rotation (0x15), Rotation (0x16), Synchronize Player Position (0x3c), Chunk Data (0x24), Disconnect (0x1a)
- `play_state_machine.rs` — Play状態機械オーケストレーション
- `lib.rs` — モジュールエクスポート

#### rustbound-conformance (6ファイル)
- `client.rs` — Status conformanceクライアント (async/tokio)
- `login_client.rs` — Login conformanceクライアント (同期)
- `play_client.rs` — Play conformanceクライアント (同期)
- `snapshot.rs` — StatusSnapshot正規化
- `diff.rs` — 差分テスト比較
- `lib.rs` — モジュールエクスポート

#### rustbound-server (7ファイル)
- `listener.rs` — TCPリスナー、graceful shutdown
- `connection.rs` — コネクションハンドラー、状態ルーター
- `world.rs` — World, Chunk, PlayerHandle
- `tick.rs` — 20 TPSティックループ、チャンネルメッセージング
- `config.rs` — server.propertiesパース、CLI引数
- `server.rs` — サーバーオーケストレーション、統合テスト
- `main.rs` — エントリーポイント

---

## マイルストーン詳細

### M1: Status Conformance (完了)

**目標:** Minecraft 1.20.1のStatusプロトコル（サーバーリスト ping）を実装し、Forgeサーバーとの差分テストを可能にする。

**PRs:**
- PR #6 — プロトコルプリミティブ (VarInt, String, UUID等)
- PR #7 — パケットフレーミング (長さプレフィックス)
- PR #8 — Handshake packet と状態ルーティング
- PR #9 — Status Request/Response と Ping/Pong
- PR #10 — Status conformanceクライアント と正規化

**テスト:** 21 (conformance)

### M2: Login (完了)

**目標:** Loginプロトコル（オフラインモード）を完全実装し、Login状態機械とconformanceクライアントを構築する。

**PRs:**
- PR #18 — Compression レイヤー と Set Compression packet
- PR #19 — Login Start と Login Disconnect
- PR #20 — Encryption Request/Response
- PR #25 — Login Success, Plugin Request/Response, 状態機械, conformanceクライアント

**新規プリミティブ:** Uuid, encode_byte_array/decode_byte_array, InvalidBoolean codec error
**新規モジュール:** compression.rs, login.rs, login_state_machine.rs, login_client.rs

**テスト:** 129 (protocol) — 合計150

### M3: Play (完了)

**目標:** Play状態の主要パケットを実装し、Play状態機械とconformanceクライアントを構築する。

**PRs:**
- PR #33 — Join Game packet (0x28) と f32/f64/boolプリミティブ
- PR #38 — Keep Alive, Player Position/Rotation, Chunk Data, Disconnect, 状態機械, conformanceクライアント

**実装パケット (protocol 763):**
| パケット | 方向 | ID |
|---|---|---|
| Join Game | clientbound | 0x28 |
| Keep Alive | clientbound | 0x23 |
| Keep Alive | serverbound | 0x12 |
| Set Player Position | serverbound | 0x14 |
| Set Player Position and Rotation | serverbound | 0x15 |
| Set Player Rotation | serverbound | 0x16 |
| Synchronize Player Position | clientbound | 0x3c |
| Chunk Data and Update Light | clientbound | 0x24 |
| Disconnect (Play) | clientbound | 0x1a |

**新規モジュール:** play.rs, play_state_machine.rs, play_client.rs

**テスト:** 163 (protocol) — 合計184 protocol + 23 conformance = 207

### M4: Server Core (完了)

**目標:** プロトコルコーデック (M1-M3) を統合し、機能するサーバーを構築する。

**PRs:**
- PR #45 — TCPリスナー とコネクションアクセプター
- PR #46 — コネクションハンドラー と状態ルーター
- PR #47 — ティックループ とワールド管理 とプレイヤーセッション
- PR #48 — サーバー設定 とスタートアップ とconformance統合

**機能:**
- TCPリスナー (non-blocking, graceful shutdown, TCP_NODELAY)
- コネクションハンドラー (Handshaking -> Status/Login -> Play 状態ルーティング)
- 20 TPSティックループ (50ms/tick, シングル認証スレッド)
- ワールド管理 (チャンク読み込み/破棄, エンティティID割り当て)
- プレイヤーセッション (位置/回転/ゲームモード追跡)
- server.propertiesパース とCLI引数 (--config, --host, --port)
- 統合テスト: Status交換, Login conformance (オフラインモード)

**新規モジュール:** listener.rs, connection.rs, world.rs, tick.rs, config.rs, server.rs

**テスト:** 33 (server) — 合計240

---

## コミット履歴

```
ad9c663 Implement server configuration, startup, and conformance integration. (#48)
d33a4d7 Implement tick loop, world management, and player sessions. (#47)
d4dd604 Implement connection handler and state router. (#46)
e337955 Implement TCP listener and connection acceptor. (#45)
0de496d Implement remaining M3 Play packets: Keep Alive, position/rotation, chunk data, disconnect, state machine, conformance client. (#38)
867f6d9 Implement Join Game packet and add Play state primitives. (#33)
d4ba72d Implement remaining M2 Login packets: Login Success, Plugin Request/Response, state machine, and conformance client. (#25)
10ea866 Implement Encryption Request and Response packets. (#20)
bdaa604 Implement Login Start and Login Disconnect packets. (#19)
1ba1581 Implement compression layer and Set Compression packet. (#18)
02e8627 Add black-box status conformance client and normalizer. (#10)
0f8cb6d Implement Minecraft 1.20.1 status and ping exchange. (#9)
5a23ece Implement protocol 763 handshake and state routing. (#8)
d418d73 Match protocol primitive wire semantics. (#6)
c587232 Establish clean-room Rust workspace.
```

---

## 設計原則 (AGENTS.md準拠)

- ピュアRust実装 (Minecraft Java Edition 1.20.1互換)
- クリーンルーム実装 (公開ドキュメント + ブラックボックス観察のみ)
- Minecraft/Forge/mappings/逆コンパイル成果物の再配布・コミット禁止
- シングル認証ティックスレッド (並列化は測定で正当化されるまで保留)
- `unsafe` は隔離・文書化・監査済みモジュールのみ (現在: `unsafe_code = "forbid"`)
- グローバル `Arc<Mutex<_>>` 回避、明示的オーナーシップと狭い同期境界
- Forge 47.4.10をオラクルとした差分テスト (成果物はリポジトリに取り込まない)
- `素体データ/` は読み取り専用ローカル参照データ

---

## 今後の展望

M1-M4でMinecraft 1.20.1サーバーの基盤が完成。次のステップ候補:

1. **Play状態の完全統合** — コネクションハンドラーにPlay状態のパケット処理を統合 (Join Game送信、Keep Alive、位置更新)
2. **チャンク生成** — プロシージャルチャンク生成 (バニラワールド生成アルゴリズム)
3. **ブロック・エンティティシステム** — ブロック状態、エンティティ管理、AI
4. **インベントリ** — アイテムスロット、コンテナ、クラフト
5. **オンラインモード** — Mojang認証、暗号化、プロパティ署名
6. **マルチプレイヤー** — プレイヤー間ブロードキャスト、チャット、コマンド
7. **パフォーマンス最適化** — 並列チャンク生成、ネットワークI/O最適化

---

*最終更新: M4完了時点*
*テスト数: 240 (全合格)*
