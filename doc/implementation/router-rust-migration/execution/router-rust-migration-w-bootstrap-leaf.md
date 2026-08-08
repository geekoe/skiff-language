# Router Rust Migration Batch 5 — W-bootstrap Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_w_bootstrap`
集成目标：`/root/router_rust_integration_b5`

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

## 引用链

- 批次文档：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-5.md`
  （W-bootstrap 节点；baseline `main@85596193`）。
- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`（draft v5），
  重点 §3.3（`ActiveRoutingEpochStore` 单一 authority、atomic `Arc` replacement）、
  §3.8（boundedness）、§5.4（C-bootstrap 解锁 W-bootstrap）、§7 E-bootstrap
  （committed 只读、pending fail closed、missing/malformed/identity mismatch、
  loader saturation、shutdown fail closed）、§10（health counters）。
- 冻结契约：
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-bootstrap-contract.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-artifact-contract.md`
  - `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-bootstrap-wire-contract.md`
  - `doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-bootstrap-leaf.md`
- 兄弟交付（已合入 main@85596193）：
  - A3：`router/src/artifact/`（`ActorRoutingProjectionStore` /
    `ActorRoutingCatalog` / `ActorRoutingProjectionRef`）；
  - W-session：`router/src/session/` 的 `SessionLayer` seam
    （`CommittedEpoch = RegisteredAssemblyTuple`，等待 bootstrap lane 接 epoch
    source）；
  - W-activation-state：`router/src/activation/` 的
    `ActivationStateRepository` read 面（含 `MemoryActivationStateRepository`
    fake）。

冲突时以权威设计为准；本叶子只记录 W-bootstrap 实现决策，不改变冻结契约语义。

## 零 worktree 只读预检结论（锚定 main@85596193）

1. 基线：`git rev-parse main` =
   `85596193df24f1fb5d0745eabf049e7e1ebf5a79`；HEAD 仅比 main 多批次父文档。
2. W-session seam 已就绪：`SessionLayerOptions { committed_epoch, pending_epoch,
   ... }`、`SessionLayer::bootstrap_bytes()` 与 `epoch_context()` 是唯一消费
   committed tuple 的 seam；W-session 叶子明确等待本节点接 epoch source，
   不要求改 handshake/directory/task 内部逻辑。
3. `ActivationStateRepository` read 面：
   `read(environment) -> Result<EnvironmentActivationState, RepositoryError>`；
   `CasMismatch` = 缺失、`InvalidRecord` = malformed、`Transient`/`Closed` =
   基础设施失败；`MemoryActivationStateRepository` 可作 fake repository。
4. A3 artifact 交付：`ActorRoutingProjectionStore::load_catalog` 一次构造
   `ActorRoutingCatalog`；`ActorRoutingProjectionRef { record_path }` 仍是
   A1/contracts-bootstrap 对齐 seam（canonical 记录路径推导未冻结），
   本节点以 typed 输入继续消费，不猜测路径。
5. contracts-bootstrap corpus：
   `deployment/tests/fixtures/bootstrap-chain-corpus.json`
   （committedOnly / pendingPresent / missing / malformed /
   committedRefMissing / committedRefMismatch）与
   `router-rust-bootstrap-wire-corpus.json` 均为冻结形态；W-model 已交付
   payload-presence 强制与 `RuntimeBootstrapProvider` seam。
6. 依赖：`skiff-runtime-config-snapshot` 已在 workspace 与 Cargo.lock；
   `skiff-deployment::fixtures::empty_runtime_assembly_fixture`、
   `RuntimeConfigSnapshot::new`、A0 `ActorRoutingProjection::new` 均公开，
   测试可真实构造完整 epoch 输入。
7. 任务可闭合；无需要改 session 内部语义的设计空洞。不返回
   `TASK_SCOPE_EXPANDED` / `TASK_NOT_EXECUTABLE`。

## 实现决策（在冻结契约语义内）

1. **读端口消费 repository read 面**：`CommittedActivationBootstrapReader` 只读
   消费 `ActivationStateRepository::read`；durable 缺失 → `FailClosedMissing`，
   `InvalidRecord` → `FailClosedMalformed`，pending 存在 → `FailClosedPending`
   （不投影、不 stage），committed ref 校验失败 → `FailClosedIdentityMismatch`。
   `Transient`/`Closed` 是 repository 端口相对文件 adapter 的新失败面，映射为
   新增 outcome `FailClosedRepository { message }`（本叶子记录为 W-bootstrap
   对齐项；C-bootstrap §2.1 的五个 durable-state outcome 语义不变，全部 fail
   closed、无 epoch 发布）。
2. **committed ref 校验由 store 完成，port 不复制校验逻辑**：reader 经
   `CommittedRefValidator` seam 调用
   `CanonicalArtifactStore::read_runtime_assembly`（完整四重校验链），
   失败即 `FailClosedIdentityMismatch`；validator 实现为
   `CanonicalCommittedRefValidator`，与 strict loader 同源、不复制校验。
3. **`BootstrapStrictLoader`**：顺序冻结为 assembly strict read → snapshot
   strict read → snapshot `environment()` 与调用方 environment 精确一致 →
   A3 actor routing projection strict read → `ActorRoutingCatalog` 一次构造 →
   `RoutingEpoch`。任一失败 → `BootstrapLoadFailure`，不产生 partial epoch、
   不发布。`ActorRoutingProjectionRef` 作为调用方 typed 输入传入（A3 D2 seam；
   canonical 推导由 E-bootstrap/integration 合流时替换，reader/loader 校验链
   不变）。
4. **`RoutingEpoch`**：不可变，持有 strict-loaded `Arc<RuntimeAssembly>`（含
   ingress/deployment projection：`gateway_ingress` / `resolved_deployments`）
   与 `Arc<RuntimeConfigSnapshot>`、`Arc<ActorRoutingCatalog>`；顶层暴露
   environment / generation / assembly_identity / config_snapshot_id /
   `registered_tuple()`（W-session seam 映射）。构造期校验 environment 精确
   匹配与 generation/environment lexical 边界，失败即 `InvalidEpoch`。
5. **`ActiveRoutingEpochStore`**：单 slot + `Mutex<Option<Arc<RoutingEpoch>>>`
   整指针替换（语义上即 atomic `Arc` replacement：capture 只能拿到完整 epoch，
   永不混合 tuple；publish 不可失败、无回滚）+ `publishCount` 计数。store
   不拥有 pending / eligibility / cache / health history。
6. **`BlockingLoader`**：有界 `Semaphore`（默认并发 8）+ 每次 read deadline
   （默认 5s）+ shutdown 标志 + occupancy/saturated/deadlineAborts 计数；
   饱和 = `LoaderSaturated` fail closed（不排队无限等待，health `queued` 恒为
   0）；超时 = `Deadline` fail closed；shutdown 后新 load 拒绝、drain 等
   occupancy 归零（drain deadline 内），在飞 read 在各自 deadline 内完成或
   逻辑 abort。
7. **初始 bootstrap runner**：`read_committed → project → strict load →
   publish`；非 `StableCommitted` 一律 fail closed 且不发布（完整 cold
   recovery 归 E-activation）。
8. **SessionLayer seam 接线**：`SessionLayerOptions` 增加
   `epoch_store: Option<Arc<ActiveRoutingEpochStore>>`（默认 None）；`with_options`
   保存 store；`bootstrap_bytes()` / `epoch_context()` 经私有
   `current_tuple()` 优先从 store capture 映射 `RegisteredAssemblyTuple`，
   无 store 时回退 `committed_epoch`（既有 session 测试 seam 不变）。不改
   handshake/directory/task/consumer 内部逻辑。

## 写集

生产（仅本叶子）：

- `router/src/bootstrap/mod.rs`、`reader.rs`、`epoch.rs`、`loader.rs`、
  `strict_loader.rs`、`runner.rs`；
- `router/src/lib.rs`（仅 additive `pub mod bootstrap;` + 必要 re-export）；
- `router/src/session/layer.rs`（仅 epoch source seam 装配；
  `SessionLayerOptions.epoch_store` + `current_tuple()` 接线）；
- `router/Cargo.toml`（增加既有 workspace 依赖
  `skiff-runtime-config-snapshot`；Cargo.lock 对应 skiff-router 条目 additive）；

测试 / 文档：

- `router/tests/bootstrap_reader.rs`、`bootstrap_epoch.rs`、
  `bootstrap_loader.rs`、`bootstrap_strict_loader.rs`、
  `bootstrap_runner.rs`、`bootstrap_session_seam.rs`；
- `doc/implementation/router-rust-migration/execution/router-rust-migration-w-bootstrap-leaf.md`（本文件）。

禁止写：`router/src/artifact/`、`router/src/activation/`、`router/src/main.rs`、
`router/src/listener.rs`、`router/src/session/` 除 layer.rs seam 外的文件、
deployment、runtime crate、`runtime/transport/src`、verify 注册表/selector
graph、`verify.yml`、AGENTS.md、scripts README、`skiff-instance.mjs`；不操作
stable instance / Mongo / PM2 / 4004-4007；不跑全量 `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| bootstrap 序列测试 | `cargo test -p skiff-router bootstrap`（含 bootstrap_* 文件） |
| fail-closed 负例 | reader/runner 测试覆盖 missing/malformed/pending/identity mismatch/saturated/deadline/shutdown，均无 epoch 发布 |
| epoch 发布 | `bootstrap_epoch`：atomic capture/publish、旧 Arc 延续、publishCount、pending 不进入 |
| SessionLayer 接入 | `bootstrap_session_seam`：store 驱动 bootstrap bytes/epoch context；既有 session 测试不回归 |
| 全 crate 回归 | `cargo test --package skiff-router` |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust,router-rust-process-smoke` |
| 格式/clippy | `cargo fmt --check`（触碰文件）、`cargo clippy -p skiff-router --all-targets`（无新增 error） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b5` 报告 branch、worktree、
implementation commit/tree、实际写集、自验收矩阵与已知 seam
（`ActorRoutingProjectionRef` 仍为调用方 typed 输入；`FailClosedRepository`
为 repository 端口对齐项），并通知 root（父 Agent）。
