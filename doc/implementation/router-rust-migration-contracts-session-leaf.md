# Router Rust Migration Batch 3 — contracts-session Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_contracts_session`
集成目标：`/root/router_rust_integration_b3`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-3.md`
  （contract pack freeze，session 链；baseline `main@1d442366`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5，
  2026-08-01），重点 §3.2（owner/invariant）、§3.4（identity/fence）、§3.5
  （真实 Runtime handshake 合同）、§3.6（disconnect 是 cancellation + barrier）、
  §5.3（C-model-registration lane）、§5.4（contract packs 必填项、
  C-session/C-process-lifecycle）、§5.5（demux 与 sink bundle）、§7
  （E-session/H-registration-cut 依赖）。冲突时以权威设计为准。
- 本叶子落盘的冻结契约：
  - `doc/implementation/router-rust-migration-c-model-registration-contract.md`
  - `doc/implementation/router-rust-migration-c-session-contract.md`
  - `doc/implementation/router-rust-migration-c-process-lifecycle-contract.md`

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@1d442366e63e17085c4a4ab0d306627c5f494e3a`
  （`git rev-parse main` 已验证；HEAD 与 worktree 均在该 commit）。
- 分支 / worktree：`feat/router-rust-contracts-session` /
  `/Users/geek/workspace/wt-contracts-session`（基线即上述 commit）。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-contracts-session/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定：`main` = `1d442366…`，与批次文档一致。
2. M0 后的 transport 现状（`runtime/transport`）：
   - `protocol/session.rs` 已有 `RouterBootstrapFrameHeader`、
     `RuntimeCapabilitiesFrameHeader`、`RuntimeHealthFrameHeader`、
     `RuntimeRegisteredFrameHeader`，以及 legacy `RuntimeRegisterFrameHeader`；
     bootstrap 有语义 decode（`decode_router_bootstrap_frame_header`）。
   - `assembly_activation.rs` 已有 `assembly.activation:Register` 的
     encode/decode 与方向校验（`AssemblyActivationControl::Register`）。
   - 尚无 `RuntimeConnectionEpoch` / `RuntimeSessionEpoch` /
     `RuntimeRegistrationDirectory` / `RuntimeRegistrationTransition` 类型
     （本 pack 冻结其契约，不写 production 类型）。
3. 当前 TS/Rust wire 与设计 §3.5 的偏差（记录在
   C-model-registration 契约 §4）：当前 inbound 注册仍走 legacy
   `runtime.register` 帧（`RuntimeRegisterFrameHeader`），而设计目标是
   `assembly.activation:Register`；当前 wire 没有 connection epoch / session
   epoch 绑定字段（epoch 是 router-local 状态，不要求 wire 字段）。本 pack
   按设计 §3.5 冻结目标 corpus 并记录差异；不需要改公共契约（corpus 使用
   现有 canonical codec 即可 byte-exact 表达目标序列）。

## 任务目标（contract pack：session 链）

1. 冻结 C-model-registration：byte-exact handshake sequence corpus（§3.5）：
   accept `RuntimeConnectionEpoch` → `router.bootstrap` →
   `runtime.capabilities` → bind `RuntimeSessionEpoch` / acquire
   installed-consumer permits → `assembly.activation:Register` →
   `RuntimeRegistrationTransition` 验证 committed epoch → publish routable
   revision → `runtime.registered` ACK → `runtime.health`。覆盖 wrong order、
   identity change、duplicate/stale register、ACK 丢失的严格 terminal；
   health 不能在 ACK 前被当作 registered observation。
2. 冻结 C-session：connection/session task 端口、
   `RuntimeRegistrationDirectory`（current_by_replica/sessions_by_epoch 双索引，
   replacement/cancel/barrier 语义 §3.2/§3.6）、pre-auth 上限与
   bootstrap/capabilities/register timeout、session cancellation token +
   reserved terminal + consumer manifest + ACK barrier + fail-stop 契约。
3. 冻结 C-process-lifecycle（§5.4）：stop public/control admission → stop new
   activation + reconcile in-flight durable decision → drain HTTP/client WS
   finalizers → terminal dispatcher/broker/actor pending → release Runtime
   generation leases → close Runtime sessions via barrier → join blocking
   loader/tasks/timers → close Mongo；每步总 deadline，超时非零退出/fail-stop。

## 交付清单

- 契约文档三份（见引用链）。
- Byte-exact corpus fixture 与其测试（`runtime/transport/testdata/` +
  `runtime/transport/tests/`）：
  - `registration-handshake/frames.json`（帧目录：每帧完整二进制 hex +
    typed header + decodeAs + direction）。
  - `registration-handshake/scenarios/*.json`（accept 与全部负例序列）。
  - `process-lifecycle/shutdown-sequence.json`（停机顺序 fixture）。
  - `tests/registration_handshake_corpus.rs`（byte-exact 校验 + 参考状态机）。
  - `tests/session_directory_contract.rs`（双索引/replacement/transition/
    barrier/pre-auth/timeout 参考模型测试）。
  - `tests/process_lifecycle_contract.rs`（停机顺序 fixture 校验）。

## 写入边界

可写：

- `doc/implementation/router-rust-migration-contracts-session-leaf.md` 及三份
  契约文档。
- `runtime/transport/testdata/registration-handshake/`、
  `runtime/transport/testdata/process-lifecycle/`（corpus fixtures）。
- `runtime/transport/tests/` 下三个测试文件（corpus/参考模型，test-only）。

禁止：

- `skiff-router` production、`runtime/transport/src` production 模块结构、
  deployment/artifact-model production 类型、AGENTS.md、scripts README、
  verify 注册表/selector graph/verify.yml、`skiff-instance.mjs`、
  `Cargo.toml`/`Cargo.lock`（本节点不需要新依赖）。
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
  `pnpm verify`。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| corpus 测试通过（含负例序列） | `CARGO_TARGET_DIR=<worktree>/target cargo test --package skiff-runtime-transport --test registration_handshake_corpus --test session_directory_contract --test process_lifecycle_contract` |
| 现有 transport 测试不回归 | `cargo test --package skiff-runtime-transport`（聚焦运行） |
| 契约文档覆盖 §5.4 必填项 | 三份文档均含 owner/invariant、typed inputs/outputs、capacity、queue full、timeout/disconnect/replacement/shutdown terminal、health fields、fake seam、real boundary probe |
| 无 production consumer 提前依赖 | `rg -n "contracts-session|c-model-registration|registration-handshake|RuntimeRegistrationDirectory" --glob '!doc/**' --glob '!runtime/transport/tests/**' --glob '!runtime/transport/testdata/**'`（无命中） |
| baseline/写集干净 | `git status` 仅上述新增文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b3` 报告 branch、worktree、implementation
commit/tree、实际写集、自验收矩阵，并通知 root（父 Agent）。

