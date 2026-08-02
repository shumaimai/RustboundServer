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
| Phase A: Play Integration | 4 | 1 | — | 完了 |
| Phase B: Login SM+Compression | 6 | 3 | — | 完了 |
| Phase C: World Visibility | 3 | 1 | — | 完了 |
| Phase D: Multiplayer | 3 | 1 | — | 完了 |
| Phase E: Polish | 5 | 3 | — | 完了 |
| Phase F–I + Mod API design | — | — | — | F–I **完了** / Mod API 設計+façade 着地 (#101/#132) |
| **合計 (A–E まで)** | **46** | **24** | **~298** | **A–E 完了** |

テスト: conformance 28 + protocol 219 + server（persist/registry 追加）+ 1 ignored。
`cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` はクリーン想定。
残り Phase I は GitHub Issue **#134**（#127–#130, #132–#133）。全体マップは #102 / #134。

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

## Phase A-E: 機能拡張 (完了)

### Phase A: Play Integration (#52-#55)
- PR #77 — LoginStateMachine統合、Play状態遷移、conformance play probe
- Login状態機械をconnection handlerに統合、Play状態への遷移を自動化

### Phase B: Login SM + Compression + Join Sequence (#56-#62)
- PR #78 — LoginStateMachine統合、compression有効化、Client Information
- PR #79 — Joinシーケンスパケット (Plugin Message, Change Difficulty, Player Abilities, Set Held Item, Entity Event, Set Default Spawn Position, Set Center Chunk, Set Render/Simulation Distance, Game Event) とregistry codec fixture
- #60 (Online-mode encryption) は延期、#62 (play.rs分割) は不要と判断

### Phase C: World Visibility (#63-#65)
- PR #80 — Flat/voidチャンクジェネレーター、チャンクload/unload、初期チャンク送信
- Chunk::generate() でスーパーフラット世界生成、send_initial_chunks() で半径2のチャンク送信

### Phase D: Multiplayer (#66-#68)
- PR #81 — サーバーバウンド移動パケット転送、Player Info Update/Remove、Spawn Player/Remove Entities
- TickMessage::PlayerPositionUpdate にyaw/pitch/on_ground追加
- PlayerInfoUpdate (0x3A), PlayerInfoRemove (0x39), SpawnPlayer (0x03), RemoveEntities (0x3E) コーデック
- セッション参加/離脱時の全セッションへのブロードキャスト

### Phase E: Polish (#69-#73)
- PR #82 — 切断パス整理、ライブプレイヤー数表示、Ctrl+Cシャットダウン
- PR #83 — 掘削/設置パケットコーデック とクリエイティブブロック更新
- PR #84 — Play conformance probe拡張 とForge差分テストヘルパー

**新規パケット (Phase B-E):**
| パケット | 方向 | ID | Phase |
|---|---|---|---|
| Plugin Message (brand) | clientbound | 0x17 | B |
| Change Difficulty | clientbound | 0x0C | B |
| Player Abilities | clientbound | 0x34 | B |
| Set Held Item | clientbound | 0x4D | B |
| Entity Event | clientbound | 0x1C | B |
| Set Default Spawn Position | clientbound | 0x50 | B |
| Set Center Chunk | clientbound | 0x4E | B |
| Set Render Distance | clientbound | 0x4F | B |
| Set Simulation Distance | clientbound | 0x5C | B |
| Game Event | clientbound | 0x1F | B |
| Declare Commands | clientbound | 0x10 | B |
| Update Recipes | clientbound | 0x6D | B |
| Client Information | serverbound | 0x08 | B |
| Confirm Teleportation | serverbound | 0x00 | B |
| Player Info Update | clientbound | 0x3A | D |
| Player Info Remove | clientbound | 0x39 | D |
| Spawn Player | clientbound | 0x03 | D |
| Remove Entities | clientbound | 0x3E | D |
| Player Digging | serverbound | 0x1D | E |
| Use Item On | serverbound | 0x31 | E |
| Block Update | clientbound | 0x0A | E |
| Acknowledge Block Change | clientbound | 0x06 | E |

**新規モジュール:** registry_codec.rs, play_diff.rs

**テスト:** 299 (28 conformance + 219 protocol + 52 server, 1 ignored) — 1 ignoredはForgeオラクル差分テスト

---

## コミット履歴

```
a49471b Phase E: Extend Play conformance probe and Forge differential helpers (#71) (#84)
83206f3 Phase E: Basic dig/place packet codecs and Creative block updates (#69) (#83)
89fd93e Phase E: Disconnect cleanup, live player count, Ctrl+C shutdown (#70, #72, #73) (#82)
303e61f Phase D: Multiplayer - movement forwarding, Player Info, spawn/despawn (#66, #67, #68) (#81)
12faeb6 Phase C: Flat/void chunk generator, chunk load/unload, initial chunks (#63, #64, #65) (#80)
063377e Phase B: Join sequence packets and registry codec fixture (#58, #59) (#79)
9855353 Phase B: LoginStateMachine integration, compression, Client Information (#56, #57, #61) (#78)
fe276b9 Implement Phase A: Play integration (#52-#55) (#77)
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

## 既知のギャップ（Phase I 完了後）

- ~~Creative 設置が石固定~~ — **#123 Done**
- ~~`PlayerHandle` / peer gamemode 0 固定~~ — **#122 Done**
- ~~ブロック／プレイヤー永続化・autosave~~ — **#124–#126 Done**
- ~~最小 block/item ID registry~~ — **#131 Done**
- ~~チャンク Unload~~ — **#127 Done**
- ~~リスポーン後のチャンク／インベントリ再同期~~ — **#128 Done**
- ~~食料消費・戦闘死~~ — **#129 / #130 Done**（スタブ）
- ~~Survival dig progress~~ — **#133 Done**（均一 hardness）
- ~~Tick-owned mutation façade~~ — **#132 Done**
- ~~Mod API 設計ドキュメント + 型スケルトン~~ — **#101 Done**
- ~~ModHost を tick ループへ接続~~ — **Done**（init/tick/shutdown；本番 mods はまだ空）
- online mode 未実装 — #60
- 動的 mod ロード / 登録 UI — follow-up

## 今後の展望

Phases A–I 完了。ModHost は tick に配線済み。次は **#60 Online mode** または実 mod の登録経路。

1. ~~正しさ — #122 / #131 / #123~~
2. ~~永続化 — #124 / #125 / #126~~
3. ~~ストリーム／リスポーン — #127 / #128~~
4. ~~Survival 感 — #129 / #130 / #133~~
5. ~~Mod API 準備 — #132 / #101 / tick 配線~~
6. **Online mode** — #60
7. **実 mod 登録** — `Server` へ mods を渡す経路

---

*最終更新: ModHost tick 配線*
*テスト数: workspace 緑想定*
