# Router Rust Migration Batch 4 — W-model Leaf（W-model-registration + W-model-bootstrap-wire / M-registration / M-bootstrap-wire）

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_model`
集成目标：`/root/router_rust_integration_b4`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration-batch-4.md`（当前在
  `integration/router-rust-migration-batch-4` 分支，基线 main 尚未包含；本叶子按路径引用，
  集成合流后可用）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），重点
  §3.5（真实 Runtime handshake 合同）、§5.3（C-model-registration →
  W-model-registration → M-registration；C-model-bootstrap-wire →
  W-model-bootstrap-wire → M-bootstrap-wire；M-pack consumer gate）、§5.4、
  §5.5（demux / `RuntimeBootstrapProvider`）、§7（E-session 依赖 M-registration）。
- 冻结契约：
  - `doc/implementation/router-rust-migration-c-model-registration-contract.md`
    （corpus：`runtime/transport/testdata/registration-handshake/`）。
  - `doc/implementation/router-rust-migration-c-model-bootstrap-wire-contract.md`
    （corpus：`runtime/transport/testdata/router-rust-bootstrap-wire-corpus.json`；
    §6 明确 W-model-bootstrap-wire 交付义务，含 payload presence 强制与
    `currentEnforced` 翻转）。
  - 兄弟叶子：`router-rust-migration-contracts-session-leaf.md`、
    `router-rust-migration-contracts-bootstrap-leaf.md`。
- 仓库约定：`AGENTS.md`（skiff repo）、`/Users/geek/workspace/AGENTS.md`（workspace，git 外）。
- Baseline：`main@7683b7c8`（`git rev-parse main` 与 worktree HEAD 一致）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `7683b7c8`，worktree HEAD 相同；当前 checkout 的
   `integration/router-rust-migration-batch-4` 分支比 main 仅多
   `f5032f0b docs(router-rust): add batch 4 execution parent`（本叶子按路径引用该文档）。
2. transport 现状（`runtime/transport/src`）：
   - `protocol/frame.rs`：binary frame codec（`encode_binary_frame` /
     `decode_typed_binary_frame`，SKBF magic + version + JSON header + payload）。
   - `protocol/session.rs`：已有 `RouterBootstrapFrameHeader` +
     `decode_router_bootstrap_frame_header`（header 级严格校验，**不检查 payload**）、
     `RuntimeCapabilitiesFrameHeader`、`RuntimeHealthFrameHeader`、
     `RuntimeRegisteredFrameHeader`、legacy `RuntimeRegisterFrameHeader`。
   - `assembly_activation.rs`：已有 `assembly.activation:Register` 的
     encode/decode 与方向校验（`AssemblyActivationControl::Register`，
     `AssemblyActivationFrameDirection::RuntimeToRouter`）。
   - 尚无 frame 级 bootstrap/session 专用 codec（payload presence 强制）、
     `CapturedBootstrapEpoch`/`RouterBootstrapSource`/
     `RuntimeBootstrapProvider` 表面（契约只冻结，未实现）。
3. corpus 现状：
   - `runtime/transport/testdata/registration-handshake/frames.json`（12 帧，
     byte-exact hex）+ `scenarios/*.json`（19 个场景，含 accept 与全部负例）。
   - `runtime/transport/testdata/router-rust-bootstrap-wire-corpus.json`
     （ref/frame/family/payloadPresence；`payload-non-empty-rejected` 目前
     `currentEnforced: false`，契约 §6.2 指定由 W-model 翻转）。
   - 既有 corpus 测试：`runtime/transport/tests/registration_handshake_corpus.rs`、
     `bootstrap_wire_corpus.rs`（contracts-session/contracts-bootstrap 交付，已合入 main）。
4. consumer 测试位置：
   - `router/tests/contracts.rs`（M0-D4 Router consumer gate 先例）。
   - `runtime` crate（`runtime/Cargo.toml`，lib 在 `driver/lib.rs`）尚无 `runtime/tests/`
     集成测试目录；本叶子新增 `runtime/tests/w_model_*.rs`。
   - runtime 生产消费（`runtime/host`）已有 `router.bootstrap` /
     `runtime.registered` 的 typed decode，但本节点不写 `runtime/host`。
5. 依赖坐标（设计空洞检查）：
   - 冻结签名 `RuntimeBootstrapProvider::bootstrap_frame(&RoutingEpoch)` 依赖
     `RoutingEpoch` production 类型，该类型归 C-bootstrap/W-bootstrap 且本批次没有
     W-bootstrap 节点。本叶子在 transport 实现 wire 面 captured tuple
     （`RouterBootstrapSource` / `CapturedBootstrapEpoch`）与 provider seam，
     并在叶子中记录 W-bootstrap 后续把 `RoutingEpoch` 映射到
     `RouterBootstrapSource` 的适配点；这不是公共契约变化（C-model-bootstrap-wire
     把该实现义务交给 W-model/W-bootstrap，未冻结 production 类型落点）。
   - 不新增 workspace crate，不改 `Cargo.toml`（router 依赖已含 transport；
     runtime 依赖已含 transport + artifact-model）。

## 任务范围

1. W-model-registration：在 transport 收敛目标 handshake 的 frame 级
   DTO/codec（`assembly.activation:Register` 复用现有 activation codec；
   新增 `runtime.capabilities` / `runtime.registered` / `runtime.health` 的
   专用 frame codec，统一 schema/type/payload-presence/direction 校验）；
   golden bytes 与 `registration-handshake` corpus 逐字节一致。
2. W-model-bootstrap-wire：新增 `router.bootstrap` frame 级
   encode/decode（`decode_router_bootstrap_frame` 强制空 payload，翻转
   `currentEnforced`）；`CapturedBootstrapEpoch` / `RouterBootstrapSource` /
   `RuntimeBootstrapProvider` seam；golden bytes 与
   `router-rust-bootstrap-wire-corpus.json` / cross-system corpus 一致。
3. M-registration / M-bootstrap-wire Rust consumer gate：skiff-router 与
   runtime crate 的 consumer 测试直接消费同一 corpus
   （`runtime/transport/testdata/` 两个 corpus），不复制 fixture。
4. 交付叶子任务文件（本文件）。

非目标：不实现 W-session 状态机/目录（W-session 节点）；不删除 legacy
`runtime.register`（H-registration-cut 节点）；不写 skiff-router production；
不写 `runtime/host` / deployment / artifact-model production；不改 cross-system
corpus bytes。

## 写集（全部在 worktree `/Users/geek/workspace/wt-w-model`）

production（`runtime/transport/src`，仅 W-model owner）：

1. `src/protocol/session.rs`：新增 frame 级 codec 与 bootstrap provider seam
   （`encode_router_bootstrap_frame` / `decode_router_bootstrap_frame`、
   `encode_runtime_capabilities_frame` / `decode_runtime_capabilities_frame`、
   `encode_runtime_registered_frame` / `decode_runtime_registered_frame`、
   `encode_runtime_health_frame` / `decode_runtime_health_frame`、
   `CapturedBootstrapEpoch`、`RouterBootstrapSource`、`RuntimeBootstrapProvider`、
   `StatelessRuntimeBootstrapProvider`）。
2. `src/protocol.rs`：re-export 新增表面。
3. `src/protocol/session.rs` 内 `#[cfg(test)]`：codec roundtrip/payload/direction
   单元测试（不改变既有 golden bytes）。

corpus / tests（`runtime/transport`，W-model owner）：

4. `testdata/router-rust-bootstrap-wire-corpus.json`：`payload-non-empty-rejected`
   的 `currentEnforced` 翻转为 `true`（契约 §6.2 指定动作）。
5. `tests/bootstrap_wire_corpus.rs`：翻转 `currentEnforced` 断言（最小必要修改；
   真实 payload presence 强制探针在 `tests/w_model_bootstrap_wire_corpus.rs`）。
6. `tests/w_model_registration_corpus.rs`：新 corpus 测试（w_model_* 前缀）。
7. `tests/w_model_bootstrap_wire_corpus.rs`：新 corpus 测试（w_model_* 前缀）。

consumer gates：

8. `router/tests/w_model_registration_consumer.rs`、`router/tests/w_model_bootstrap_wire_consumer.rs`。
9. `runtime/tests/w_model_registration_consumer.rs`、`runtime/tests/w_model_bootstrap_wire_consumer.rs`。

doc：

10. `doc/implementation/router-rust-migration-w-model-leaf.md`（本文件）。

禁止写：skiff-router production（`router/src/`）、`runtime/host` production、
deployment、artifact-model production、verify 注册表 / selector graph / verify.yml、
AGENTS.md、scripts README、`scripts/skiff-instance.mjs`、`Cargo.toml` /
`Cargo.lock`（本节点不需要新依赖）。

## 自验收矩阵

| 项 | 证据 |
| --- | --- |
| transport 新 corpus 测试 | `cargo test -p skiff-runtime-transport --test w_model_registration_corpus --test w_model_bootstrap_wire_corpus` |
| transport 既有 corpus 不回归 | `cargo test -p skiff-runtime-transport`（含 registration_handshake / bootstrap_wire / 全部 unit tests） |
| router consumer gate | `cargo test -p skiff-router --test w_model_registration_consumer --test w_model_bootstrap_wire_consumer` |
| runtime consumer gate | `cargo test -p runtime --test w_model_registration_consumer --test w_model_bootstrap_wire_consumer` |
| golden bytes 不变 | corpus 测试断言 `encode(decode(hex)) == hex`；`git diff` 审计不触碰
  `registration-handshake/frames.json` 与 cross-system fixture |
| payload presence 已强制 | `bootstrap_wire_corpus.rs` + `w_model_bootstrap_wire_corpus.rs` 断言
  `currentEnforced == true` 且真实 codec 拒绝非空 payload |
| 无 production 提前依赖 | `rg` 反向搜索：无 router/src、runtime/host 等 production 引用新增 corpus/类型 |
| 写集干净 | `git status` 仅本叶子写集；`git diff main...HEAD` 聚焦 |

不跑全量 `pnpm verify`；不操作 stable instance/Mongo/PM2/4004-4007。
`CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-model/target`（不与其他 worktree 共享）。

## 交接

完成后向 `/root/router_rust_integration_b4` 报告 branch、worktree、implementation
commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。
