# Router Rust Migration Batch 6 — W-routing-query Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界会话）
Agent：`/root/dev_w_routing_query`
集成目标：`/root/router_rust_integration_b6`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-6.md`
  （W-routing-query 节点；baseline `main@8cabf352`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（`RuntimeRegistrationDirectory` 双索引、exact registered
  tuple、registration revision）、§3.3（active routing 单一 authority、
  capture 一次 `Arc<RoutingEpoch>` → `RuntimeCandidateQuery` →
  `RegisteredSessionLease`；heartbeat 不参与 admission）、§3.4
  （identity/fence）、§5.4（C-routing-query → W-routing-query；
  W-dispatch/W-activation 共同消费）、§7（E-dispatch/E-activation 依赖
  W-routing-query）。冲突时以权威设计为准。
- 冻结契约：`doc/implementation/router-rust-migration-c-routing-query-contract.md`
  （本节点直接消费）、`doc/implementation/router-rust-migration-c-session-contract.md`
  （directory/identity/cancellation）、`doc/implementation/router-rust-migration-c-dispatch-contract.md`
  （lease 消费方）。
- 兄弟叶子：`doc/implementation/router-rust-migration-contracts-request-leaf.md`
  （corpus fixtures owner）。
- 上游实现：W-bootstrap（`router/src/bootstrap/epoch.rs`：
  `RoutingEpoch`/`ActiveRoutingEpochStore`）、W-session
  （`router/src/session/directory.rs`：`RuntimeRegistrationDirectory`；
  `router/src/session/identity.rs`：`RegisteredAssemblyTuple`/
  `RuntimeSessionEpoch`）。

## 基线与环境

- 仓库：`/Users/geek/workspace/skiff`。
- 精确 baseline：`main@8cabf352`（`git rev-parse` 已验证为
  `8cabf35289e87a610c0940b6aa10af3a0e67d64e`；worktree 即该 commit）。
- 分支 / worktree：`feat/router-rust-w-routing-query` /
  `/Users/geek/workspace/wt-w-routing-query`。
- `CARGO_TARGET_DIR=/Users/geek/workspace/wt-w-routing-query/target`
  （不与其他 worktree 共享）。

## 零 worktree 只读预检结论

1. baseline 锚定正确；并行 worktree `wt-w-dispatch`、`wt-w-model-request`
   已存在且都锚定同一 commit，本节点写入范围不与其重叠。
2. W-bootstrap 交付 `RoutingEpoch`（environment/assembly_generation/
   assembly_identity/config_snapshot_id + `registered_tuple()` +
   `deployment_projection()`）与 `ActiveRoutingEpochStore::capture()`
   （`Option<Arc<RoutingEpoch>>`，whole-pointer 原子发布）。
3. W-session 交付 `RuntimeRegistrationDirectory`：`current_by_replica()`、
   `record()`（`registered_tuple`/`registration_revision`/`routable`/
   `cancelled`）；`candidates()` 是 directory 自身的最小 session 级投影，
   不含 revision 视图标记，也不含 dispatch capabilities。
4. C-routing-query 契约的 `directory_view` 是独立 typed 视图
   （`SessionRecord` 含 `session_epoch`/`registered_tuple`/
   `registration_revision`/`cancelled`/`capabilities`），不是
   W-session `SessionRecord` 原样；capabilities（unary/serverStream）
   wire 来源是 `runtime.capabilities.capabilities.dispatchModes`
   （`RuntimeDispatchModeCapability`），W-session 尚未把
   dispatch_modes 保留进 directory。因此 W-routing-query 的查询输入是
   typed 视图，directory 对接由本模块的锁内快照 seam 提供，
   capabilities 由调用方从 capabilities binding 注入（见“设计决策”）。
5. corpus fixtures（9 个场景）位于
   `runtime/transport/testdata/routing-query/scenarios/`，reference
   测试 `runtime/transport/tests/routing_query_corpus.rs` 已合入；
   routing-query 目录下没有独立 reject-cases 文件，“负例”由本节点
   `router/tests/routing_query_negative.rs` 落盘（fixtures 不修改）。
6. `verify --only router-rust` 展开为
   `cargo test --no-fail-fast --package skiff-router`；本节点新增测试
   自动纳入。
7. `skiff-router` 依赖已含 `skiff-artifact-model`、
   `skiff-runtime-transport`、`skiff-runtime-config-snapshot`、
   `skiff-deployment`、`tokio`（sync）与 serde，本节点不需要新依赖。

## 任务目标

实现 stateless `RuntimeCandidateQuery`（C-routing-query §3/§5）：

1. 捕获一次当前 `Arc<RoutingEpoch>`（whole-epoch lease；调用方从
   `ActiveRoutingEpochStore::capture()` 获得，查询只使用该 epoch）。
2. 读取 typed directory view 的 exact registered tuple / registration
   revision / cancellation / dispatch capabilities；拒绝 cancelled
   session；heartbeat freshness 不进入类型与投影（`RuntimeHealthLedger`
   只服务 health projection）。
3. 返回 `Vec<RegisteredSessionLease>`（empty = fail closed 信号）：
   `{ session_epoch, registration_revision, exact_registered_tuple,
   cancellation, capabilities }`。
4. 消费 C-routing-query 全部 9 个场景（同一 fixtures，router 侧
   `routing_query_corpus.rs`），负例在 router 侧落盘，供 W-dispatch /
   W-activation 后续共用同一 corpus 与同一投影实现。
5. 与 `RuntimeRegistrationDirectory` 对接：锁内 coherent snapshot
   （`query_directory`），replacement/transition 竞态下不产生混合
   tuple/revision。
6. 实现 §5.6 health counters（`queries`、`candidatesReturned`、
   `excludedCancelled`、`excludedStaleRevision`、`excludedCapability`、
   `excludedTupleMismatch`），以 per-call `RoutingQueryCounters` 返回。

## 设计决策（预检确认）

### 类型（`router/src/routing/query.rs`）

- `DispatchMode { Unary, ServerStream }`（serde camelCase，wire
  `"unary"`/`"serverStream"`）。
- `DispatchCapabilities { unary, server_stream }` +
  `from_dispatch_modes(&[RuntimeDispatchModeCapability])` +
  `supports(mode)`。
- `CandidateSession { session_epoch, registered, registered_tuple,
  registration_revision, cancelled, capabilities }`：冻结视图单条事实；
  `registered` 对应 corpus `registered` / production `routable`。
- `CandidateDirectoryView { revision: Option<u64>, sessions }`：
  - `Some(n)`：冻结视图标记语义（corpus `directoryRevision`）；session
    revision != n → stale，不作为候选（场景 04）。
  - `None`：生产锁内 coherent snapshot；directory 不暴露全局 revision
    计数器（W-session §3.1 只保留 per-record revision），锁内快照下每个
    session 的 tuple+revision 本来就是其当前完整 revision，无全局标记
    可比对。corpus 测试用 `Some`；`query_directory` 用 `None`。
- `CandidateQuery { mode, deployment: ServiceDeploymentRef }`：
  deployment 来自 captured epoch 的 exact deployment 投影；查询校验
  deployment 在 `epoch.deployment_projection()` 中，否则
  `CandidateQueryError::DeploymentNotInEpoch`（fail closed）。
- `RegisteredSessionLease { session_epoch, registration_revision,
  exact_registered_tuple, cancellation: SessionCancellation,
  capabilities }`。
- `SessionCancellation { cancelled: bool }`：typed cancellation 投影；
  按契约 §2.2 注释“candidate 不持有真 token”，查询不持有/不创建
  tokio CancellationToken；W-dispatch 接线真实 per-session token。
- `RoutingQueryCounters`：per-call 计数；W-dispatch 后续聚合成
  `routingQuery.*` health 字段。
- `RuntimeCandidateQuery`（unit struct，无状态）：
  - `query(epoch, view, query) -> Result<Vec<RegisteredSessionLease>,
    CandidateQueryError>`；
  - `query_with_counters(...) -> Result<(Vec<...>, RoutingQueryCounters),
    ...>`；
  - `query_directory(epoch, directory, capabilities, query)`：锁内
    snapshot（调用方持 `SessionLayer::directory_lock()`），按
    (replica_id, connection_generation) 排序保证确定性；capabilities
    由调用方 map 注入（W-session 尚未保留 dispatch_modes 的 seam 注
    记，见下方 seam note）。

### 投影规则（冻结顺序）

1. exact deployment 校验（query 级；不在 epoch 投影 → error）。
2. epoch 级 exact tuple：`session.registered_tuple ==
   Some(epoch.registered_tuple())`（逐字段相等）。
3. 一个完整 revision：`revision == Some(view.revision)` 时
   `registration_revision` 必须相等，否则 stale 排除（视图标记模式）。
4. `!cancelled`（cancelled 永不进入候选）。
5. capability：`mode == unary` 要求 `unary`，`mode == serverStream`
   要求 `server_stream`。
6. heartbeat 不参与：类型中没有 heartbeat 字段；场景 08 由 corpus
   测试固定该规则。
7. 无候选 → `Ok(vec![])`（admission 层映射 fail closed）。
8. 多 replica 全返回；同一 `session_epoch` 去重（directory invariant
   保证每 replica 一个 current，视图防御性去重）。

counter 归属：按上述顺序第一个命中失败的规则计数，一个被排除 session
恰好计一个排除项。

### seam note（交付给 W-dispatch / W-activation）

- `RuntimeRegistrationDirectory` 不保留 dispatch capabilities，也不暴露
  全局 revision 计数器；`query_directory` 的 capabilities 参数由调用方
  从 `runtime.capabilities` binding（`dispatch_modes`）注入。后续若
  E-session/W-dispatch 把 dispatch_modes 保留进 directory 或补
  revision 访问器，可切换为 `Some` 视图标记模式，投影语义不变。
- 本节点不修改 `router/src/session/`、不写 dispatch/http/bootstrap
  装配，也不接 `run_router`。

## 写入边界

可写：

- `router/src/routing/mod.rs`、`router/src/routing/query.rs`
  （W-routing-query 独占）。
- `router/src/lib.rs`（additive：`pub mod routing;` + re-export）。
- `router/tests/routing_query_corpus.rs`、
  `router/tests/routing_query_negative.rs`、
  `router/tests/routing_query_directory_seam.rs`。
- `doc/implementation/router-rust-migration-w-routing-query-leaf.md`
  （本文件）。

禁止：

- `router/src/bootstrap/` 生产装配、`router/src/dispatch/`、
  `router/src/http/`、router TS、`runtime/` crate、
  `runtime/transport/src`、deployment、AGENTS.md、scripts README、
  verify 文件、`scripts/skiff-instance.mjs`；
- `runtime/transport/testdata/routing-query/`（corpus fixtures owner
  是 contracts-request，本节点只消费）；
- `Cargo.toml`/`Cargo.lock`（本节点不需要新依赖）；
- 操作 stable instance、Mongo、PM2、4004-4007 端口进程；不跑全量
  `pnpm verify`；不跑 chat smoke。

## 自验收矩阵

| 验收项 | 命令 / 证据 |
| --- | --- |
| 生产投影 + corpus 全场景 | `CARGO_TARGET_DIR=<worktree>/target cargo test -p skiff-router routing_query`（corpus + negative + directory seam 全部通过） |
| corpus 与 reference 同 fixtures | `router/tests/routing_query_corpus.rs` 消费 `runtime/transport/testdata/routing-query/scenarios/*.json`（include_str），9 场景逐场景断言 leases/counters |
| 负例 fail closed | `router/tests/routing_query_negative.rs`：deployment 不在 epoch → error；无候选 → 空结果；未注册/重复/撕裂视图排除 |
| directory 对接 | `router/tests/routing_query_directory_seam.rs`：真实 directory + 真实 epoch，多 replica、cancel、transition、replacement、capability 排除 |
| verify selector | `CARGO_TARGET_DIR=<worktree>/target node scripts/verify.mjs --only router-rust`（`cargo test --no-fail-fast --package skiff-router`） |
| 写集干净 | `git status` 仅上述新增文件；`git diff main...HEAD` 聚焦 |
| 禁止区零命中 | `git diff --stat` 不含 session/bootstrap/dispatch/http/runtime/TS/verify 文件 |

## 交接

完成后向 `/root/router_rust_integration_b6` 报告 branch、worktree、提交
hash、自验收命令与结果、seam note；同步通知 root。
