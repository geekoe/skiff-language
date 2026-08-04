# Leaf Task: agent-tests db require could not find model.AgentThread（共享 blocker 定位/修复）

## 引用链

- 任务来源：主 Agent `/root`（internals 批次共享 blocker 排查）。
- 集成 Agent：`/root/skiff_integration_testinfra`（canonical:
  `skiff_integration_testinfra`），worktree
  `/Users/geek/workspace/skiff-integration-testinfra`，branch
  `integration/agent-tests-db-schema`。
- internals 集成分支：`integration/agent-timeout-dispatch` @
  `b3aa8f575c94c6008fabb41aa264e46aad1ff6b7`，worktree
  `/Users/geek/workspace/internals-integration-timeout`。
- internals baseline：`0746336bb9fe2b2e6918521f4b2faf3dbfd0b445`，共享
  worktree `/Users/geek/workspace/internals`（只读）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`。
- 现象：`node skiff/scripts/skiff.mjs test <agent-tests> --artifact-root <dir>`
  在 isolated runtime 中，actor-method 路径全部报 `UnhandledServiceError →
  runtime JsonDecode → Actor method execution failed: db require could not find
  model.AgentThread`。

## 环境与基线

- skiff baseline：`6d2b13e789851f1bb675b367834ee96d8d40b2d5`（main HEAD，
  已 `git rev-parse` 验证）。
- 本节点 worktree：`/Users/geek/workspace/skiff-dev-testinfra`，branch
  `dev/agent-tests-db-schema`。
- 隔离构建目录：`CARGO_TARGET_DIR=/Users/geek/workspace/.cargo-target-skiff-testinfra`。
- 约束：不改 stable instance 配置；不激活/重启 stable instance；不碰其他
  批次 worktree；共享 dev-home store 的恢复按任务第 6 步单独核对。

## 预检结论（只读，基线 6d2b13e）

- 错误文本来自 `runtime/eval/src/program_db.rs`：`db require could not find
  {display_type_name}` 只出现在 `command.required && found == None` 分支
  （`FindOne`，两处：recoverable 与普通路径）。它不是“schema 未注册”错误；
  schema registry 未命中会先由 `ServiceDbMetadata::collection_for_target_key`
  报另一类错误。因此 blocker 的直接含义是：**运行时 schema registry 中存在
  该 DB target，但按目标 identity 查询返回了空行**（数据不在此部署对应的
  database/collection，或查询 key 与写入 key 不一致）。
- DB target 解析：`DbCapabilityTarget::lookup_key()` 是
  `("skiff-db-object-target-v1", package_artifact_ref, file_ir_ref,
  type_index)` 的 JSON 精确 identity；`type_name` 只用于显示。
- isolated test runtime 启动链：
  `scripts/skiff.mjs test` → `scripts/lib/isolated-test-runtime.mjs`
  （spawn supervisor + Mongo + activation seed）→
  `test-runner`（`cargo run --bin skiff-test-runner`）→
  `canonical_fixture::run_package_cases` 分批 publish 到 runtime artifact root，
  每批激活一个 assembly。
- DB provider 输入由 `runtime/host/src/loader/active_assembly_context.rs`
  `candidate_db_metadata` / `activation_db_metadata` 生成：按根 deployment
  BFS 收集 execution image 中所有包的 `file.declarations.db`，每 deployment
  一个 `DbProviderBuildInput`（`service_id` = deployment.service_id）。
- actor method 执行（`runtime/host/src/host/actor_owner_execution.rs`）使用
  `route.db_source()`，route 由 `actor_execution_route` 按
  `actor_ref.service_id` 在 active assembly 的 deployments 中唯一匹配。
  因此 actor 方法查询的 database 由 **actor 所属 deployment 的 service_id**
  决定；测试侧写入由 **test service deployment 的 service_id** 决定。
- 假设（待复现验证）：测试根 deployment 的 service_id 与 actor 路由 deployment
  的 service_id 不同（或 assembly 组装/DB metadata 归属不一致），导致同一
  model 类型被映射到不同 database/collection；或 actor 路由部署的 DB
  metadata 不含 `subjectImpl/model.*`（此时错误应不同，需用日志核对）。
- 待核对：`runtime/log` 中的 activation/DB provision 事实、以及
  `activation_db_metadata` 对 agent-tests 批次的输出（通过测试日志/证据）。

## 确定性复现计划

1. internals 集成分支：
   `node skiff/scripts/skiff.mjs test /Users/geek/workspace/internals-integration-timeout/packages/agent-tests --artifact-root <fresh-temp> --deny-skips --require-tests`
2. internals baseline：
   `node skiff/scripts/skiff.mjs test /Users/geek/workspace/internals/packages/agent-tests --artifact-root <fresh-temp> --deny-skips --require-tests`
3. 记录失败集合与 isolated runtime 日志证据
   （`scripts/lib/isolated-test-runtime-log-evidence.mjs` 保留机制）。

## 根因与修复决策

### 确定性复现结果

- internals baseline（0746336，`/Users/geek/workspace/internals/packages/agent-tests`）：
  5 PASS / 12 FAIL；失败集合 = `thread_actor_drain` ×7 + `tool_attempt_timeout` ×5。
- internals 集成分支（b3aa8f5，`internals-integration-timeout`）：5 PASS / 13 FAIL；
  失败集合 = 同样 12 项 + 新增探针 `scheduled foreground timeout fires after a short
  real delay`（branch 比 baseline 多 1 个新增探针失败，与预期一致）。
- 复现命令（隔离 temp artifact root，先 seed bootstrap 并 publish llm-api/agent）：
  `node scripts/skiff.mjs test <agent-tests> --artifact-root <temp> --deny-skips --require-tests`。
- 原始错误：`Actor method execution failed: db require could not find model.AgentThread`
  （baseline 与集成分支相同，确定性复现）。

### 根因（skiff 仓库，已修复）

1. **DB 路由错误（共享 blocker 的直接根因）**：
   `router/src/supervisor/actor_sink.rs::actor_owner_service_id` 对
   `ActorOwnerUnitFrameHeader::Package(slot)` 使用
   `actor_catalog().entries().iter().find(|entry| package_build_id == build_id)`，
   取**第一条**匹配 entry。canonical test-runner 每个 test case 生成一个独立
   deployment（独立 Mongo database，service id 形如
   `test.skiff/p-.../e-.../case-N`），且每个 case deployment 都绑定同一个
   `agine.ai/agent` 包，因此投影里每个 case 各有一条 entry，`.find()` 恒返回
   case-0。结果：所有 actor 方法都被重写到 case-0 deployment，读取 case-0 的
   database，而测试写入的是 case-N 的 database → `db require` 必然 not found。
   - 证据：保留的 isolated runtime Mongo 数据中，case-N 的
     `AgentThread_U8-AMMJhcWYj` 行存在（`tad_thread_*`），但 actor 方法的
     `find {_id: <case-N threadId>}` 发生在 case-0 database 且 nret=0；
     actor-routing `current.json` 每个 case 各有一条 entry 且
     `actor.serviceId == deployment.serviceId == case-N`。
   - 修复：优先匹配 `entry.deployment.service_id == header.actor_key.service_id`
     （caller 自己的 deployment 恰好绑定该包时），否则回退到首条匹配。不改变
     公共契约：同 service actor 路径不变；cross-package caller 不绑定包时行为
     与原来一致。
2. **actor 方法 outbound lease 二次 receive 竞态（同一 actor 路径上的真实 panic）**：
   `runtime/host/src/eval_capability_adapter/actor.rs::invoke_actor_method` 的
   `tokio::select!` 同时等待 `lease.receive()` 与 `response_committed.wait()`；
   多线程下 commit 发生在 `receive()` 取走 oneshot receiver 之后、其 future 变
   Ready 之前，第二个分支再次 `lease.receive()` 触发
   `Actor method outbound lease can only be received once` panic。所有完成路径
   （`complete`/`complete_failure`/`fail_all`）都会 send oneshot，Notify 分支冗余。
   - 修复：删除冗余的 `response_committed` 分支及配套
     `ActorMethodResponseCommitted` 机制（`actor_method_outbound.rs`），只保留
     `lease.receive()` 单一等待路径。

### 剩余问题（非共享 blocker，设计级，未实现修复）

修复 DB 路由后，`thread_actor_drain` / `tool_attempt_timeout` 仍以
**非确定性**方式失败（不同运行 PASS/FAIL 集合不同），错误为
`std.json.DecodeError`（测试支撑 `firstActorToolCall` 抛 “missing tool call”）
或 `assertion failed`。profile 证据显示：spawn 的 `self.tick()` 与测试显式
`actor.tick()` 的 DB 操作交错执行，两个 actor 方法可同时运行 LLM 轮次，测试读取
工具调用行时其插入尚未完成。

这是 skiff actor v1 的**既有设计语义**，不是本次 blocker：
- `doc/architecture/actor-shared-heap-design.md` §3.2：真实挂起时释放 arena
  借用，恢复时 reacquire（可能等待其他段）；§2：v1 编译期拒绝 `serial`/`concurrent`。
- 既有验收记录（P5-F445H-E3 / E4R5AS）明确
  “同步 segment 串行、外部 future 重叠”与“pre-suspend 让 queued competitor
  先运行”是预期行为。
- TAD 测试注释假设“spawned tick 与 explicit tick 在 actor 上串行”，与当前 v1
  语义冲突。改为严格按 actor 串行属于设计变更，超出本任务“小边界修复”决策门，
  故不实现，仅报告。

### 自验收矩阵

| 条款 | 代码证据 | 反向搜索 | 命令 |
| --- | --- | --- | --- |
| DB 路由修复 | `router/src/supervisor/actor_sink.rs::actor_owner_service_id` 优先 caller deployment | `rg "actor_owner_service_id"` 仅该函数 | 两目标 agent-tests：`db require could not find` 出现 0 次 |
| outbound lease 修复 | `eval_capability_adapter/actor.rs::invoke_actor_method` 仅 `lease.receive()` 单路径；`actor_method_outbound.rs` 移除 Notify 机制 | `rg "ActorMethodResponseCommitted"` 无残留 | `cargo test -p skiff-runtime-host --lib actor_method_outbound` 4/4；`... eval_capability_adapter::actor` 13/13 |
| 无回归 | router lib 测试 | - | `cargo test -p skiff-router --lib` 69/69 |

### 两目标最终结果（提交前复跑）

- baseline：5 PASS / 12 FAIL；`db require could not find` = 0。
- 集成分支：5 PASS / 13 FAIL（12 项相同 + 1 新增探针）；`db require could not find` = 0。
- 剩余失败全部为上述 actor 并发语义导致的非确定性 JsonDecode/assertion。

### 共享 dev-home store（第 6 步）

`skiff/.skiff-instance/dev-home/artifacts/pointers/package-artifacts/agine~dai~sagent/0.1.0.json`
指向 `5fa39fa2…`（internals 批次手动发布的未合并构建）。本任务复现全程使用隔离
temp artifact root，未依赖该 pointer；集成批次是否正在使用该状态不明，按任务要求
“状态不明，报告不擅自改”，未改动，由集成 Agent 决定是否恢复。
