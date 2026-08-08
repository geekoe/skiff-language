# Router Rust Migration Batch 5 — W-model-actor + W-model-spawn Leaf（M-actor / M-spawn 模型侧 gate）

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_model_actor_spawn`
集成目标：`/root/router_rust_integration_b5`

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-5.md`（当前在
  `integration/router-rust-migration-batch-5` 分支，基线 main@85596193 尚未包含；
  本叶子按路径引用，集成合流后可用）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），重点
  §5.3（W-model-actor → M-actor；C-model-spawn → W-model-spawn → M-spawn →
  H-spawn-parent-cut；`callerKind = request | actorInvocation` 决策；不建旧 shape
  兼容 reader）、§5.4（C-spawn 在 H-spawn-parent-cut 后才解锁；typed parent
  namespace / resolver 边界）、§5.5（`ActorFrameSink` / `SpawnSubmitRouter`
  sink 不拥有 pending）。
- 冻结契约：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-actor-contract.md`
    （corpus：`runtime/transport/testdata/actor-wire/`，20 帧 + 22 场景）。
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-spawn-contract.md`
    （corpus：`runtime/transport/testdata/spawn-wire/`，5 帧 + 10 场景；
    `callerKind` 目标 wire 与 `legacyCut` 规则）。
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-spawn-contract.md`（resolver /
    `SpawnSubmitRouter` 模型边界，本节点只消费其 wire 前置，不实现 handler）。
  - 兄弟叶子：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-actor-leaf.md`
    （contracts 交付清单与 corpus 约定）。
- 先例：`doc/implementation/router-rust-migration/execution/router-rust-migration-w-model-leaf.md`（batch 4
  W-model 叶子：corpus + consumer gate 形态、写集与自验收模式）。
- 仓库约定：`AGENTS.md`（skiff repo）、`/Users/geek/workspace/AGENTS.md`
  （workspace，git 外）。
- Baseline：`main@85596193`（`git rev-parse 85596193` = 85596193df24…，与批次
  文档一致；worktree HEAD 即该 commit）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `85596193df24f1fb5d0745eabf049e7e1ebf5a79`；当前
   `skiff/` 主 checkout 在 `integration/router-rust-migration-batch-5`，比 main
   仅多 `doc/implementation/router-rust-migration/execution/router-rust-migration-batch-5.md`（本叶子按路径引用）。
2. actor wire 现状（`runtime/transport/src`，batch 4 已合入 main）：
   - `actor_method.rs`：`actor.method.invoke/return/error/cancel` 帧级 codec
     （payload presence、identity/deadline/cancellation/trace/testCase 校验，
     decode/encode 双侧）。
   - `actor_owner.rs`：`actor.owner.invoke/control/control.ack/failure` 帧级
     codec（fence/routeAuthority/transition/bootstrap/deadline/eviction 校验）。
   - `protocol/actor.rs`：`actor.getOrCreate/replace/find/remove` 控制族 typed
     DTO（getOrCreate 的 testCase 配对校验；`ActorKeyFrameMetadata` /
     `ActorRefFrameMetadata` 尚无 canonical base64 / sha256 hash / epoch 的
     typed-decode 形状校验——不在共享 typed Deserialize 上收紧：
     `control_mapper/tests.rs`（本节点写集外）用非 sha256 hash 构造并回读
     控制帧，收紧会破坏既有测试；§2.2 logical-ref 校验面已由
     `actor_method` / `actor_owner` 帧级 codec 承担）。
   - `runtime/transport/tests/actor_wire_corpus.rs` 已消费全部 20 帧并 byte-exact
     通过（预检实测 6 tests ok）。
3. spawn wire 现状：
   - `protocol/spawn.rs`：`SpawnSubmitRequestFrameHeader` 仍是旧 shape
     （无 `callerKind`，`callerRequestId: Option<String>`），无帧级 codec。
   - 旧 shape 被生产消费：`control_mapper.rs`（outbound 映射）、
     `runtime/request-contract` / `runtime/eval/src/spawn_ops.rs` /
     `runtime/host`（`SpawnSubmitControlRequest.caller_request_id`）。
     这些生产 consumer 的硬切归 H-spawn-parent-cut，本节点写集外。
   - `runtime/transport/tests/spawn_wire_corpus.rs` 用测试内 mirror
     （`TargetSpawnSubmitRequestFrameHeader`）验证目标 wire；契约 §6.1 指定
     W-model-spawn 交付 production canonical codec 后由真实 codec 接管同一
     corpus（frameHex 保持 C-model-spawn 生成值，不改 bytes）。
4. consumer gate 先例：`router/tests/w_model_registration_consumer.rs` /
   `runtime/tests/w_model_registration_consumer.rs`（batch 4 M-gate 形态：
   corpus 直读 + byte-exact roundtrip + scenario 名冻结）。
5. 设计空洞检查：
   - 本节点不实现 C-spawn 的 resolver / `SpawnSubmitRouter` handler（归
     W-actor）；`spawn-wire/scenarios` 参考模型测试保持 test-only。
   - canonical spawn codec 与旧 shape 并存是刻意的迁移姿态：旧 DTO 继续服务
     未 cut 的生产 outbound 构造；canonical decode 对旧 shape 一律拒绝
     （无兼容 reader），H-spawn-parent-cut 负责删除旧 shape 与 consumer 切换。
   - actor 控制族 metadata 的 canonical base64 / sha256 形状校验：不写入共享
     typed Deserialize；W-model 负例探针经 `actor.method.invoke` /
     `actor.owner.control` 的现有 canonical codec 校验面覆盖 logical ref 形状
     （canonical base64、sha256、epoch>0），getOrCreate key/hash 语义校验归
     W-actor admission。
   - 不新增 workspace crate / Cargo 依赖。

## 任务范围

1. W-model-actor：收敛 actor 族 wire 的 corpus gate——20 帧 corpus 全部经真实
   codec `encode(decode(hex)) == hex`，并补齐 canonical codec 负例探针
   （logical ref 形状、identity 前缀、argumentsEncodingVersion、
   testCase 配对、owner control operation 约束）。
2. W-model-spawn：新增 spawn canonical codec——closed enum
   `callerKind = request | actorInvocation`、required `callerRequestId`、
   closed `targetKind = function | actorMethod` 与 actorMethod 成对约束、
   required payload；`spawn.submit.request` 新 generation 帧级 encode/decode；
   response/error 帧级 codec（空 payload 强制）；旧 shape 无兼容 reader
   （decode 拒绝）。golden bytes 与 `spawn-wire/frames.json` 逐字节一致
   （frameHex 不重生成）。
3. M-actor / M-spawn 模型侧 gate：
   - transport corpus 测试（`w_model_actor_corpus.rs` / `w_model_spawn_corpus.rs`，
     `w_model_actor_*` / `w_model_spawn_*` 前缀）；
   - skiff-router 与 runtime crate 的 consumer 测试直接消费同一 corpus
     （`router/tests/w_model_actor_consumer.rs`、`runtime/tests/…`、
     `router/tests/w_model_spawn_consumer.rs`、`runtime/tests/…`）。
4. `spawn_wire_corpus.rs` 从测试内 mirror 切换到 production canonical codec
   （契约 §6.1 “真实 codec 接管同一 corpus”）。
5. 交付叶子任务文件（本文件）。

非目标：不实现 W-actor 的六个 model 面（catalog/ownership/broker/relay/scheduler）；
不实现 C-spawn resolver / `SpawnSubmitRouter`；不做 H-spawn-parent-cut（不写
TS Router / Rust Runtime production consumer）；不删旧 shape DTO；不改
`control_mapper.rs` / `runtime/eval` / `runtime/host` / `runtime/request-contract`
production；不写 skiff-router production（`router/src/`）。

## 写集（全部在 worktree `/Users/geek/workspace/wt-w-model-actor-spawn`）

production（`runtime/transport/src`，仅 actor/spawn owner；lib/protocol 只加
additive 声明）：

1. `src/protocol/spawn.rs`：新增 `SpawnCallerKind` closed enum、
   `SpawnTargetKind` closed enum、canonical `SpawnSubmitRequestFrameHeaderV2`
   （字段顺序与 `spawn-wire/frames.json` mirror 一致）、
   `encode_spawn_submit_request_frame` / `decode_spawn_submit_request_frame`、
   `encode_spawn_submit_response_frame` / `decode_spawn_submit_response_frame`、
   `encode_spawn_submit_error_frame` / `decode_spawn_submit_error_frame`
   （schema/type/token/targetKind-actorMethod/identity 校验；response/error
   payload 必须为空）。
2. `src/actor_method.rs`：把 `validate_actor_ref` / `validate_owner` /
   `validate_identity` / `validate_token` 提升为 `pub(crate)` 供 spawn codec
   复用（不加新语义，不改既有校验行为）。
3. `src/protocol.rs`：re-export 新增 surface（additive）。

corpus / tests（`runtime/transport`）：

4. `tests/w_model_actor_corpus.rs`：新 corpus 测试（w_model_actor_* 前缀：
   20 帧 byte-exact + canonical codec 负例探针 + scenario 名冻结）。
5. `tests/w_model_spawn_corpus.rs`：新 corpus 测试（w_model_spawn_* 前缀：
   5 帧 byte-exact + legacy-cut 拒绝 + closed enum / targetKind 负例 +
   scenario 名冻结）。
6. `tests/spawn_wire_corpus.rs`：删除测试内 mirror，改用 production
   canonical codec 接管同一 corpus（保持 resolver/router 参考模型不变）。
7. `testdata/`：不改任何 bytes（actor-wire / spawn-wire 均冻结）。

consumer gates：

8. `router/tests/w_model_actor_consumer.rs`、`router/tests/w_model_spawn_consumer.rs`。
9. `runtime/tests/w_model_actor_consumer.rs`、`runtime/tests/w_model_spawn_consumer.rs`。

doc：

10. `doc/implementation/router-rust-migration/execution/router-rust-migration-w-model-actor-spawn-leaf.md`（本文件）。

禁止写：`runtime/transport/src` 的 activation/connection 模块、`control_mapper.rs`、
`router/src/`、`runtime/eval`、`runtime/host`、`runtime/request-contract`、
deployment、AGENTS.md、scripts README、verify 注册表/selector graph/verify.yml、
`scripts/skiff-instance.mjs`、`Cargo.toml` / `Cargo.lock`。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| transport 新 corpus 测试 | `cargo test -p skiff-runtime-transport --test w_model_actor_corpus --test w_model_spawn_corpus` |
| contracts corpus 不回归且真实 codec 接管 | `cargo test -p skiff-runtime-transport --test actor_wire_corpus --test spawn_wire_corpus` |
| transport 全量 | `cargo test -p skiff-runtime-transport`（含 unit tests，旧 shape 生产面不回归） |
| router consumer gate | `cargo test -p skiff-router --test w_model_actor_consumer --test w_model_spawn_consumer` |
| runtime consumer gate | `cargo test -p runtime --test w_model_actor_consumer --test w_model_spawn_consumer` |
| golden bytes 不变 | corpus 测试断言 `encode(decode(hex)) == hex`；`git diff` 审计不触碰
  `testdata/actor-wire` / `testdata/spawn-wire` |
| 旧 shape 无兼容 reader | canonical spawn decode 拒绝 `legacy-no-caller-kind` 帧；
  `callerKind=function` / 缺失字段均拒绝；rg 反向搜索无 fallback 默认逻辑 |
| 生产 consumer 未提前切 | `rg` 反向搜索：`SpawnSubmitRequestFrameHeaderV2` /
  `encode_spawn_submit_request_frame` 在 `runtime/host`、`runtime/eval`、
  `control_mapper.rs`、`router/src` 零命中 |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-model-actor-spawn/target`
（不与其他 worktree 共享）。

## 交接

完成后向 `/root/router_rust_integration_b5` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
