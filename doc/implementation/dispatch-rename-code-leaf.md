# dispatch-rename-code leaf

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；用户可见关键字固定为 `dispatch`，旧 `spawn` surface 被完整取代，不保留兼容语法/第二条易失执行路径）。
- 批次父节点：`doc/implementation/dispatch-rename-batch.md`（集成 agent `/root/dispatch_integration` 创建；截至本叶子落盘时尚未存在，批次名按任务信封标注为 `dispatch-rename`）。
- 本叶子：`dispatch-rename-code`（owner：开发 agent `/root/rename_code`）。
- 基线：repo `skiff`，main HEAD = `13068249715281076d0e0b9134d4a97ec72a36be`（工作区干净，预检锚定该 commit）。

## 目标与边界

纯机械改名（语义不变）：把 detached-call 的旧 `spawn` surface 全链路改名为 `task`（内部标识符/wire family）/ `dispatch`（语言关键字与用户可见错误消息），覆盖 syntax、artifact-model、compiler、runtime、router 与受影响的 wire 语料/测试。

### 写入范围（预检确认的实际写集）

1. 语言关键字与 AST/IR：
   - `syntax/src/parser/stmt.rs`：`match_ident("spawn")` → `match_ident("dispatch")`；错误消息 `spawn statement expects a call expression` → `dispatch statement expects a call expression`；`syntax/src/parser/expr.rs` 与 `mod.rs` 的 `spawn is a statement...`、`use actors and spawn instead` 同步。
   - `syntax/src/ast.rs` `Stmt::Spawn` → `Stmt::Dispatch`；`syntax/src/ast_utils.rs` 全部 match 分支。
   - `syntax/src/parser/tests/spawn.rs` → `dispatch.rs`，测试源与断言同步（关键字、错误消息、`Stmt::Dispatch`）。
   - `artifact-model`：`StmtIr::Spawn` → `StmtIr::Dispatch`、`SpawnTargetIr`/`SpawnTargetKindIr` → `TaskTargetIr`/`TaskTargetKindIr`、`SpawnPayload`（recoverable/artifact-model）→ `TaskDispatchPayload`、`BoundaryProjection::Spawn` 与 `ValueEscapeLane::Spawn` 同步。
   - `compiler/core/src/spawn_targets.rs` → `dispatch_targets.rs`（含 `tests.rs` 子模块），`service_spawn_targets_with_packages` → `service_task_targets_with_packages` 等全部内部标识符；metadata key `"spawnSubmit"` → `"dispatchSubmit"`。
   - `compiler/lowering`/`source`/`projection`：`Stmt::Spawn`/`StmtIr::Spawn` match、`EscapeLane::Spawn`/`ValueEscapeLane::Spawn` → `Dispatch`、`lower_spawn_stmt`、`collect_spawn_executable_seeds`、`service_spawn_targets` 等。
2. Runtime wire：
   - `runtime/transport/src/protocol/spawn.rs` → `task.rs`；frame family `spawn.*` → `task.*`（`spawn.submit.request`/`response`/`error` 等）；`SpawnSubmitRequestFrame*`/`SpawnSubmitResponseFrameHeader`/`SpawnSubmitAcceptance`/`SpawnCallerKind`/`SpawnTargetKind`/`ActorSpawnRuntimeErrorFrameHeader`/`SPAWN_SUBMIT_*` 常量 → `Task*`/`TASK_*`；wire 字段 `spawn_id` → `task_id`（JSON `spawnId` → `taskId`）。
   - `runtime/transport/src/protocol.rs`：`RuntimeFrameFamily::Spawn` → `Task`、`mod spawn`/re-exports。
   - `runtime/transport/src/runtime_assembly_request.rs` + `lexical.rs`：`RuntimeAssemblySpawnRequest*` → `RuntimeAssemblyTaskRequest*`、`deserialize_spawn_*` → `deserialize_task_*`、invocation.kind wire 值 `"spawn"` → `"task"`。
   - `runtime/transport/src/control_mapper.rs`、`protocol/session.rs`（`spawned_tasks_active` → `task_requests_active`，JSON `spawnedTasksActive` → `taskRequestsActive`）。
   - `runtime/transport/testdata/spawn-wire/` → `task-wire/`（frames.json + scenarios/*.json 内容同步；frameHex 由 codec 重新生成）。
   - `runtime/request-contract`/`runtime/request`/`runtime/capability-context`/`runtime/host`/`runtime/eval`/`runtime/linker`/`runtime/linked-program`/`runtime/boundary`/`runtime/model`/`runtime/driver`：`SpawnSubmitControlRequest`/`SpawnCallerKind`/`ActorMethodSpawnTargetControl`/`RecoverableSpawnPayload`/`RuntimeSpawn*`/`spawn_ops` → `task_ops`/`spawn_routes` → `task_routes`/`recoverable_spawn_payload.rs` → `recoverable_task_dispatch_payload.rs`/`spawn_execution.rs` → `task_execution.rs`/`PayloadBoundaryKind::SpawnPayload` → `TaskDispatchPayload` 等（含 `RuntimeRecoverableBoundaryKind::SpawnPayload` as_str `"spawnPayload"` → `"taskDispatchPayload"`）。
3. Router：
   - `router/src/actor/spawn.rs` → `task.rs`、`spawn_sink.rs` → `task_sink.rs`；`SpawnWireStore`/`PendingSpawnWire`/`SpawnSubmitRouter`/`SpawnErrorCode`/`spawn_error_code`/`SpawnParent*`/`FunctionSpawnParentResolver`/`ActorSpawnParentResolver`/`SpawnSubmitError`/`SpawnCounters` 等 → `Task*`。
   - `router/src/supervisor/session_ports.rs`：`SpawnSubmit` → `TaskSubmit`、`send_spawn_submit` → `send_task_submit`、`spawn_wire_store` → `task_wire_store`、`SPAWN_SPAN_SEQUENCE` → `TASK_SPAN_SEQUENCE`。
   - `router/src/session/demux.rs`：`classify_spawn` → `classify_task`、`InboundSinkSet.spawn` → `task`。
   - `router/src/dispatch/`（仅 spawn 相关部分）：`SpawnRejectReason` → `TaskRejectReason`、`SpawnHealth` → `TaskHealth`、`SpawnSubmitResult` → `TaskSubmitResult`、`DerivedSpawnResult` → `DerivedTaskResult`、`ActorMethodSpawnDispatch` → `ActorMethodTaskDispatch`、`ActorMethodSpawnControl` → `ActorMethodTaskControl`、`SpawnSubmit` → `TaskSubmit`、`SpawnTargetKind` → `TaskTargetKind`、`spawn_request_id` → `task_request_id`、`PendingKind::DerivedSpawn` → `DerivedTask`、`spawn_parents`/`register_spawn_parent`/`unregister_spawn_parent`/`spawn_parent_facts` → `task_parents`/`register_task_parent`/...，`dispatch/frame.rs` `submit_spawn`/`send_spawn_submit` 同理。
   - `router/src/health/`、`router/src/supervisor/`、`router/src/lib.rs`：`spawned_tasks`/`SpawnedTaskCounters` → `tasks`/`TaskCounters`、`ActorSpawnHealthDto`/`actor.spawn` → `ActorTaskHealthDto`/`actor.task`、`derived_spawn`（JSON `derivedSpawn`）→ `derived_task`（JSON `derivedTask`）、`SPAWNED_ACTOR_METHOD_*` → `TASK_ACTOR_METHOD_*`。
4. 测试语料/文件名（git mv）：
   - `syntax/src/parser/tests/spawn.rs` → `dispatch.rs`
   - `compiler/core/src/spawn_targets/` → `dispatch_targets/`
   - `runtime/eval/src/spawn_ops/` → `task_ops/`
   - `runtime/eval/src/recoverable_spawn_payload.rs` → `recoverable_task_dispatch_payload.rs`
   - `runtime/request/src/spawn_execution.rs` → `task_execution.rs`
   - `runtime/host/src/host/router_session/spawn_submit.rs` → `task_submit.rs`
   - `runtime/host/src/host/router_session/tests/h_spawn_parent_cut.rs` → `h_task_parent_cut.rs`
   - `runtime/transport/src/protocol/spawn.rs` → `task.rs`
   - `runtime/transport/testdata/spawn-wire/` → `task-wire/`
   - `runtime/transport/tests/spawn_wire_corpus.rs` → `task_wire_corpus.rs`
   - `runtime/transport/tests/w_model_spawn_corpus.rs` → `w_model_task_corpus.rs`
   - `runtime/tests/w_model_spawn_consumer.rs` → `w_model_task_consumer.rs`
   - `router/tests/w_model_spawn_consumer.rs` → `w_model_task_consumer.rs`
   - `runtime/tests/h_spawn_parent_cut_corpus.rs` → `h_task_parent_cut_corpus.rs`
   - `router/tests/spawn_repair_direction.rs` → `task_repair_direction.rs`
   - `router/tests/spawn_repair_acceptance.rs` → `task_repair_acceptance.rs`
   - `router/tests/actor_spawn_router.rs` → `actor_task_router.rs`
   - `router/tests/spawn_parent_ws_connect.rs` → `task_parent_ws_connect.rs`（预检闭合）
   - `.skiff` fixture 中语言关键字 `spawn <call>` → `dispatch <call>`：`test-runner/fixtures/actor-full-chain-acceptance/main.skiff`、`main.test.skiff`、`test-runner/fixtures/package-service-i02-spawn-submit/main.skiff`；同步内嵌 `.skiff` 测试源（compiler/source tests、runtime/eval/host tests 等）。
   - `syntax/src/parser/tests/data/fixture-parse-output-baseline.txt` 用 `UPDATE_PARSER_PHASE0_BASELINE=1` 重生成。
   - `runtime/transport/testdata/dispatch-admission/scenarios/17/18/19` 内容：这些 scenario 的事件键（`spawnFunction` 等）是 dispatch-admission harness 自有 JSON 词汇，不属于 wire frame family；`dispatch_admission_corpus.rs`（router 与 runtime/transport 两侧）按“禁止名单”保留文件名、scenario 名与 JSON 键。router 侧 corpus 因引用了被改名的 router dispatch 类型，只做编译必需的机械引用更新（import/构造/字段访问），不改语义。

### 非目标 / 禁止修改

- 不改 `RequestDispatcher`、`DispatchSubmit`（ordinary unary/stream submit）、`dispatch_admission_corpus.rs` 的测试语义/文件/JSON 键、HTTP/WS 既有 dispatch 命名、`router/src/dispatch/` 与 spawn 无关部分。
- 不改 OS/进程 spawn：`tokio::spawn`、`thread::spawn`、`tasks.spawn`、`child_process.spawn`、`Command::spawn`、`spawn_finalizer`、`ActivationCoordinator::spawn`（进程/协程）等一律保留。
- 不动 `doc/` 目录（兄弟 agent `/root/rename_docs` 负责）；本叶子只阅读权威设计。
- 不改变运行时语义、行为、兼容逻辑；wire 字段值只改名不换语义；不 push。
- 不重命名 `.skiff` fixture 中的用户标识符（如 `spawnExternal`、`Spawned`、`SpawnStreamFailure`）与数据字符串（如 URL `spawned-first`），它们不是 language/wire surface。

### 预检结论与自主闭合

- 预检锚定 baseline `1306824971`（main worktree clean）；集成 worktree `/Users/geek/workspace/skiff-dispatch-integration`（branch `dispatch-rename-integration`）已存在且干净，批次父文档未落盘 → 本叶子引用批次名 `dispatch-rename`。
- 受影响 crate：`skiff-syntax`、`skiff-artifact-model`、`skiff-compiler-core/source/lowering/projection`、`skiff-runtime-transport/request-contract/request/capability-context/boundary/model/linked-program/linker/eval/host/driver`、`skiff-router`，以及 `test-runner`（health wire 校验路径，仅 wire 改名同步）。
- 已知命名冲突处理：`router/tests/dispatch_admission_corpus.rs` 引用被改名的 dispatch 类型 → 最小机械编译修正（文件本身与语义不动）。`runtime/transport/tests/dispatch_admission_corpus.rs` 完全自包含 → 不修改。
- 停止条件：若出现无法闭合的命名冲突或需要改变设计/公共契约，停止并报告主 agent `/root`。

## 验证

- 聚焦：`cargo check -p skiff-syntax -p skiff-artifact-model -p skiff-compiler-core -p skiff-compiler-source -p skiff-compiler-lowering -p skiff-compiler-projection -p skiff-runtime-transport -p skiff-runtime-request-contract -p skiff-runtime-request -p skiff-runtime-capability-context -p skiff-runtime-boundary -p skiff-runtime-model -p skiff-runtime-linked-program -p skiff-runtime-linker -p skiff-runtime-eval -p skiff-runtime-host -p skiff-runtime-driver -p skiff-router -p skiff-test-runner`（driver 仅在成员中存在时执行）。
- 聚焦测试：syntax parser（spawn→dispatch）、compiler core spawn_targets→dispatch_targets、runtime-transport protocol/task 与 corpus、runtime-eval task_ops、runtime-host router_session task submit、router dispatch/actor 相关测试。
- 反向搜索：production（syntax/compiler/runtime/router/artifact-model，排除 scripts/ 进程 spawn 与 doc/）无遗留 spawn 关键字/wire 名；测试残留仅限被禁止保留的 harness 词汇与 fixture 数据。
- 自验收矩阵见最终 result；聚焦验证，完整套件待集成探针。

## Git 交接

- worktree：`/Users/geek/workspace/skiff-rename-code`；branch：`rename-dispatch-code`；baseline：`13068249715281076d0e0b9134d4a97ec72a36be`。
- 集成 agent：`/root/dispatch_integration`；主 agent：`/root`。
