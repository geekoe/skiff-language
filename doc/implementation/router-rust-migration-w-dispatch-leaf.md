# Router Rust Migration Batch 6 — W-dispatch Leaf Task

日期：2026-08-02
状态：execution leaf（一次性有界开发会话）
Agent：`/root/dev_w_dispatch`
集成目标：`/root/router_rust_integration_b6`

## 引用链

- 直接父批次：`doc/implementation/router-rust-migration-batch-6.md`
  （W-dispatch 节点；baseline `main@8cabf352`）。
- 权威设计：`doc/implementation/router-rust-migration-plan.md`（draft v5），
  重点 §3.2（`RuntimeAdmissionPool` / `RequestDispatcher` owner 合同）、
  §3.3（capture → query → reserve → revalidate → enqueue → terminal 恰好
  释放一次）、§3.4（identity/fence）、§3.6（session cancellation 是所有
  session-keyed pending 的共享 terminal 观察）、§3.8（boundedness、
  deadline 在 admission/dispatch 前重检）、§5.4（C-dispatch + M-request →
  W-dispatch）、§7（E-dispatch）。
- 冻结契约：
  - `doc/implementation/router-rust-migration-c-dispatch-contract.md`
  - `doc/implementation/router-rust-migration-c-routing-query-contract.md`
    （消费其 typed 输出 `RegisteredSessionLease`）
  - `doc/implementation/router-rust-migration-c-model-request-contract.md`
    （request wire、stream 状态机、cancel reason 词表）
  - `doc/implementation/router-rust-migration-c-session-contract.md`
    （session cancellation/consumer manifest 对接）
- 同链 corpus（test-only reference model，W-dispatch 消费同一 fixtures）：
  - `runtime/transport/testdata/dispatch-admission/scenarios/*.json`（19 场景）
  - `runtime/transport/tests/dispatch_admission_corpus.rs`

冲突时以权威设计为准；本叶子只记录 W-dispatch 实现决策，不改变冻结契约语义。

## 零 worktree 只读预检结论（锚定 main@8cabf352）

1. 基线：`git rev-parse main` = `origin/main` =
   `8cabf35289e87a610c0940b6aa10af3a0e67d64e`；批次 6 执行父文档位于
   integration 分支 `23ddab00`（main 之上仅 docs commit，不进入本节点基线）。
2. W-routing-query 交付接口：**尚未落地**（`router/src/routing` 不存在、
   无 production `RegisteredSessionLease`；`RuntimeRegistrationDirectory`
   只有 `candidates(&tuple) -> Vec<RuntimeSessionEpoch>`，没有 revision/
   capability/cancellation 的 typed lease 输出）。按批次文档“以
   contracts-request 契约为准”在 `router/src/dispatch/candidate.rs` 定义
   dispatch 消费侧 typed seam（`RegisteredSessionLease`、`CandidateQuery`
   port、`LeaseRevalidate` port），corpus 测试使用 `FakeCandidateQuery`；
   集成时由 W-routing-query 的真实实现对齐本 seam（若 W-routing-query 落
   地了不同接口，集成 Agent 只需替换 trait 实现/类型映射，不改本节点语义）。
3. W-session directory 类型（已合入 main）：`RuntimeSessionEpoch`、
   `RegisteredAssemblyTuple`、`RuntimeRegistrationDirectory`、
   `ConsumerKind::RequestDispatcher`、`SessionConsumer`、
   `RuntimeSessionClosed` 均在 `router/src/session/`；`bootstrap::RoutingEpoch`
   已是完整 immutable epoch（`ActiveRoutingEpochStore` 原子发布），dispatch
   直接消费 `Arc<RoutingEpoch>`，不另建 epoch 副本。
4. transport request codec（已合入 main）：`RuntimeAssemblyRequestStartFrameHeader`
   （HTTP unary/serverStream）、`RequestCancelFrameHeader`、
   `ResponseStartFrameHeader`/`ResponseChunkFrameHeader`/
   `ResponseEndFrameHeader`/`ResponseErrorFrameHeader` +
   `validate_response_error_frame`、`encode_binary_frame`/
   `decode_typed_binary_frame`、`RequestCancelReason::CONTRACT_H`（9 项
   wire reason）全部存在；`skiff-router` Cargo.toml 已依赖
   `skiff-runtime-transport`，不需要新依赖。dispatch 不在 router 私建
   codec 副本；outbound wire 编码由 `RuntimePeer` seam 的集成实现负责
   （W-model-request/W-session），本节点只传递 typed header/bytes。
5. 19 个 dispatch-admission 场景文件存在且与 reference machine 完全一致
   （场景 06 duplicate 以同一 requestId 记录 rejectReason、场景 08/09
   revalidate 失败释放后重选、场景 10 cursor 轮转、场景 15 replacement
   终结 old pending、场景 19 ambiguous 双拒绝）。
6. `verify --only router-rust` 展开为 `cargo test --no-fail-fast --package
   skiff-router`；workspace lint `tests_outside_test_module = deny`、
   `too_many_lines` threshold 534（`clippy.toml`）。

## 任务目标

在 `router/src/dispatch/` 实现 W-dispatch：

- `RuntimeAdmissionPool`：per-session capacity permits（`maxConcurrency`
  per session/connection）、selection cursor 轮转策略、`ReservationToken`/
  `PermitReleased`、revalidate 失败释放 + 重选、admission 计数；
- `RequestDispatcher`：ordinary unary/stream 与 derived function-spawn
  correlation、pending/terminal 状态机、reservation token、health 快照；
  actor-method spawn 只做 parent 解析与转发（不进入 dispatcher pending、
  不占 permit）；
- 流水线：epoch capture（`RoutingEpochSource`）→ `CandidateQuery`
  （W-routing-query seam）→ select/reserve → revalidate（`LeaseRevalidate`
  seam）→ enqueue（失败释放并重选/fail closed）→ pending 持
  epoch+lease+permit → terminal 恰好释放一次；
- 全部 terminal source 与 cancel 帧规则（§4.2/§4.3）：runtime 终态不发
  cancel、timeout/caller_abort/client_disconnect/backpressure/protocol_error/
  callback_error/router_shutdown 发 cancel、runtime_disconnect 不发；
  unknown runtime cancel reason → protocol_error terminal；
- session disconnect/replacement/shutdown：终结该 session 全部 pending，
  pending/permit 归零；shutdown 后拒绝新 admission；
- 消费 C-dispatch 19 场景 corpus（router 测试直接驱动 production
  `RequestDispatcher` + `FakeCandidateQuery`/`FakeRevalidate`/
  `FakeRuntimePeer` 等 fake seam，断言与 reference machine 可观测结果一致），
  并补 pending/permit 归零断言与竞态单元测试。

## 实现决策（在冻结契约语义内）

1. `RequestDispatcher` 是纯同步 reducer（`Arc<Mutex<DispatcherInner>>`，
   方法与 W-session 一致不跨 `.await` 持锁）；`RuntimeAdmissionPool` 内部
   `Arc<AdmissionInner>`（`Mutex<HashMap<session, in_flight>>` + cursor +
   counters），`Reservation`/`Permit` token 持 pool 引用，Drop 兜底释放
   （正常路径显式 release/commit，保证异常路径 permit 不泄漏）。
2. 选择策略与 reference machine 一致：优先 `preferSession`（若为候选且有
   容量），否则从 cursor 位置轮转跳过无容量 session；成功后
   `cursor = (position+1) % candidates.len()`；revalidate 失败重选从候选
   列表头部扫描、跳过失败 session 与无容量 session、**不推进 cursor**；
   无剩余候选 → `revalidate_fail_closed`。
3. 候选与 truth 边界：dispatcher 不拥有 session truth；corpus harness 的
   `FakeCandidateQuery` 在 disconnect/replacement 事件时同步更新目录状态
   （生产上由 W-routing-query 读 `RuntimeRegistrationDirectory`）。
   dispatcher 另维护 `closed_sessions` 集合（`on_session_closed` 幂等写入）
   作为 session terminal 观察，选择时过滤已关闭 session，覆盖“查询返回
   stale lease 的竞态”负例。
4. spawn correlation：request parent = dispatcher pending；actor parent =
   `ActorMethodSpawnControl::is_active_invocation_parent`（fake 注入）。
   双命中 → `ambiguous`；均未命中 → `no_parent`；function spawn 且仅
   actor parent → `wrong_parent_kind`；actor-method spawn 非双命中且非
   双缺失 → 转发 actor lane（与 reference machine 一致）。function spawn
   额外执行 exact authority 校验（pending 捕获的
   assemblyIdentity/assemblyGeneration/deployment/session 与 spawn 提交的
   `RequestAuthority` 全字段相等，§5.1），不一致 → `parent_authority_mismatch`
   （corpus 无此场景，额外单元测试覆盖）。derived spawn 的 deadline 由
   `SpawnSubmit.deadline` 携带（caller 计算 parent 剩余与默认派生 timeout
   较小者），提供 `derived_deadline` 帮助函数（取 timeoutMs 较小者）并文档
   化“完整 expiresAt 剩余计算归 W-http”。
5. writer 失败：`RuntimePeer::send_request_start` / `send_request_cancel`
   返回 Err → 该 pending `callback_error` terminal + best-effort cancel 帧
   （`protocol_error`）+ 经 `SessionAbortControl` 请求 abort exact session
   （§7.4；corpus 无此场景，单元测试覆盖）。
6. deadline 重检：`TimeoutCheck` seam（默认实现：`timeoutMs == 0` 视为
   过期；expiresAt 解析归 caller/W-http），submit 前与 enqueue 前各重检
   一次，过期 → `deadline_expired` reject（enqueue 前过期已 reserve 的
   permit 计入 releases）。
7. health 与 trace 分离：`DispatcherHealthSnapshot` 只含
   `dispatcher.pending.{unary,stream,derivedSpawn}`、
   `dispatcher.terminal.bySource`（11 source 计数）、
   `admission.{permitsHeld,releases,queueFullRejects,revalidateFailures,
   reselects,noCandidateRejects,duplicateRequestIdRejects}`、
   `spawn.{derivedSpawns,actorLaneSpawns,ambiguousRejects}`；requestId 不进
   health。corpus 断言所需的 outcomes/rejectReasons/terminalSources/
   sessionBindings 由测试 harness 从各方法返回值聚合，生产 dispatcher 不
   保存 request-id 日志。
8. 模块拆分：`dispatch/candidate.rs`（seam）、`dispatch/admission.rs`、
   `dispatch/frame.rs`（peer/abort/actor-control/response frame/deadline）、
   `dispatch/dispatcher.rs`、`dispatch/health.rs`、`dispatch/mod.rs`。
   `router/src/lib.rs` 仅 additive `pub mod dispatch;` + re-export。

## 写入边界

可写：

- `router/src/dispatch/`（仅本节点）；
- `router/src/lib.rs`（仅 additive dispatch 模块声明与 re-export）；
- `router/tests/`（新增 `dispatch_admission_corpus.rs`、
  `dispatch_invariants.rs`，`dispatch_*` 前缀）；
- `doc/implementation/router-rust-migration-w-dispatch-leaf.md`。

禁止：

- `router/src/routing/`、`router/src/http/`、`router/src/bootstrap/`、
  `router/src/session/`、`router/src/listener.rs`、`router/src/main.rs`、
  router TS、runtime crate、`runtime/transport/src`、deployment、AGENTS.md、
  scripts README、verify 注册表/selector graph/verify.yml、
  `scripts/skiff-instance.mjs`；
- 修改公共契约/corpus fixtures（契约与 corpus 已冻结，由 contracts-request
  链拥有）；
- 在 skiff-router 私建 transport codec 副本；
- 操作 stable instance / Mongo / PM2 / 4004-4007；不跑全量
  `pnpm verify`。

## 自验收矩阵

| 项 | 命令 / 证据 |
| --- | --- |
| dispatch 测试（19 场景 corpus + invariants） | `cargo test -p skiff-router dispatch`（含 `dispatch_*` 文件） |
| 全部 19 场景与 reference 可观测一致 | corpus 测试逐场景断言 outcomes/rejectReasons/terminalSources/sessionBindings/cancelFrames/permitsHeld/releases/derivedSpawns/actorLaneSpawns |
| pending/permit 归零 | 每场景末断言 `permitsHeld == expect` 且 `releases == expect`；disconnect/replacement/shutdown 后 pending 计数归零、per-session in-flight 归零 |
| 聚焦 verify | `node scripts/verify.mjs --only router-rust` |
| 格式/clippy | `cargo fmt --check`、`cargo clippy --package skiff-router --all-targets`（exit 0） |
| 写集干净 | `git status` 仅本叶子声明文件；`git diff main...HEAD` 聚焦 |

## 交接

完成后向 `/root/router_rust_integration_b6` 报告 branch、worktree、提交
hash、测试命令与结果、seam 清单（`RuntimeCandidateQuery`（canonical）/
`CandidateViewSource`/`LeaseRevalidate`/`RoutingEpochSource`/`RuntimePeer`/
`SessionAbortControl`/
`ActorMethodSpawnControl`/`TimeoutCheck`）与集成对齐点；同步通知 root。

## Rework（Batch 6 主 Agent 裁决：与 W-routing-query 对齐）

日期：2026-08-02（原任务窄范围返工；原分支已合入，不再修改）

基线：集成分支 `integration/router-rust-migration-batch-6` @ `a3704e22`
（含 W-model-request `bf7512d0` + W-routing-query `ccd39c78` + W-dispatch
原合并）。

### 裁决内容

按 C-routing-query 冻结契约为 canonical，dispatch 侧消除 4 处公开类型名
冲突（E0252）：`CandidateQuery`（dispatch trait vs routing struct）、
`RegisteredSessionLease`（cancellation 形状）、`DispatchCapabilities` /
`DispatchMode`（构造面）。

### 对齐方式（最小充分，语义与全部测试不变）

1. **端口统一**：dispatch 不再定义 candidate-query trait；admission 直接
   消费 W-routing-query 的 canonical `RuntimeCandidateQuery`（stateless
   投影，输入 `CandidateQuery` struct + `CandidateDirectoryView`）。
   dispatch 侧只保留没有 routing 对口的端口：`CandidateViewSource`
   （C-dispatch §3 step 2 的 directory view 来源）、`LeaseRevalidate` /
   `RevalidateOutcome`（step 5）、`RoutingEpochSource`（step 1）。
2. **Lease 形状**：`RegisteredSessionLease` 统一为 routing typed 形状
   （`cancellation: SessionCancellation`）；dispatch 不再自建
   `cancelled: bool` 副本，防御性过滤读 `lease.cancellation.cancelled`。
3. **类型单一化**：`DispatchMode` / `DispatchCapabilities` 只保留 routing
   canonical 类型；wire 构造收敛为 dispatch 适配函数
   （`dispatch_mode_from_wire` / `dispatch_mode_as_str` /
   `capabilities_from_wire_names` / `candidate_query_from_request`），
   全部集中在 `router/src/dispatch/candidate.rs`。
4. **lib.rs re-export 去重**：canonical 名只从 `routing` 导出一次；
   dispatch 导出名单去掉重复项，新增适配函数与 `CandidateViewSource`。
5. **corpus 增强**：19 场景 corpus 现在经由 `FakeCandidateViewSource`
   提供 typed view，由**真实** `RuntimeCandidateQuery` 投影生成 leases
   （与原 fake 投影语义等价：view revision Some(1)、capability 由 wire
   名映射、epoch 含 exact deployment projection），验证 dispatch 与
   W-routing-query 的真实接缝。

### Rework 写集（全部在原边界内）

- `router/src/dispatch/candidate.rs`（重写为适配层）
- `router/src/dispatch/types.rs`（`RequestAuthority.deployment` 改为
  canonical `ServiceDeploymentRef`；删除本地 mode/deployment query）
- `router/src/dispatch/dispatcher.rs`（options/端口换接 + 投影调用）
- `router/src/dispatch/admission.rs`（仅 import 指向 routing lease）
- `router/src/dispatch/mod.rs`、`router/src/lib.rs`（re-export 去重）
- `router/tests/dispatch_harness/mod.rs`（`FakeCandidateViewSource` +
  epoch 携带 deployment）
- `router/tests/dispatch_admission_corpus.rs`、
  `router/tests/dispatch_invariants.rs`（接缝换接）
- 本叶子文件（rework 记录）

未改：C-routing-query/C-dispatch 契约语义、W-routing-query 交付
（`router/src/routing/`）、W-model-request、其他节点文件。

### Rework 自验收

| 项 | 结果 |
| --- | --- |
| `cargo test -p skiff-router dispatch` | 19 场景 corpus + 22 invariants 全绿 |
| `cargo test -p skiff-router` | 全量无回归 |
| `node scripts/verify.mjs --only router-rust` | passed |
| `cargo fmt -p skiff-router -- --check` + clippy | 通过 / router 零警告 |
| rg 负例 | dispatch 内无 `CandidateQueryInput` / `ServiceDeploymentQuery` / 重复 canonical 类型定义 |
