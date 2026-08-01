# Leaf task: implement `spawn` with actor-method targets (self-message / async advancement)

## Parent / authority

- 主 Agent：`/root`（任务信封 `/root/wave_spawn_to_actor`）。
- 权威设计：`doc/architecture/actor-model.md`“spawn 到 actor 方法（自消息 / 异步推进）”小节（候选版本，基线 HEAD `415fe06c` 已含）；`doc/reference/spawn.md`。
- 流程：`/Users/geek/workspace/multi-agent-development.md`“开发 Agent”章节。

## Baseline / worktree

- Repo：`/Users/geek/workspace/skiff`，基线 `integration/actor-wave-a` @ `415fe06c7e072e93e1b336204ff7a16754022cd7`（`git rev-parse` 已验证；主 worktree 已检出）。
- 分支：`dev/spawn-to-actor`；worktree：`/Users/geek/workspace/wt-skiff-spawn-actor`。
- 独立 `CARGO_TARGET_DIR=/Users/geek/workspace/wt-skiff-spawn-actor/target`；不写共享 `skiff/target`。
- 集成 Agent：`skiff_integration`。不 merge main、不 push、不碰集成分支。
- 兄弟 worktree `wt-skiff-advance`（`dev/actor-reserved-advance` @ 98a1e78b）已被 415fe06c 设计取代，本次不触碰。

## 语义（设计为准，摘要）

1. `spawn` 可 target actor 方法（含同一 actor 实例内 `spawn self.method(...)`）：提交的调用是该 actor 的一次独立方法调用，按 identity 路由、在单线程 executor 上排队执行。
2. 调用方不等结果：只等“已接收”，目标方法返回值不可用（fire-and-forget）。
3. actor 不在 live 时按 entry 保存的创建输入激活实例后排队目标方法。
4. 同一实例多个 spawned 调用串行执行；spawned 调用不嵌套在发起方调用栈内。
5. 与现有 spawn 语义一致：caller 生命周期分离、caller cancel/timeout 不影响、提交即 admission；不新增保留方法/wake 原语。

## 预检事实与实现决策（只读预检证据）

- 语法无需改动：`Stmt::Spawn { call }` 已接受任意 call 表达式（`compiler/source/src/expression_model.rs`、syntax AST）。
- 返回类型约束已存在：`expression_type_model.rs::check_spawn_stmt` 检查 void/null。
- 编译期禁止 create 内 `spawn self.method`：`actor_method_validation.rs::check_self_calls` 已覆盖 `Stmt::Spawn`。
- 现有函数 spawn 链路：lowering 在 call 上写 `spawnSubmit` metadata（targetKind/target）→ 运行时 `spawn_ops::submit_spawn_statement` 编码 recoverable args → `spawn.submit.request` control frame → router `RuntimeDispatcher.handleSpawnSubmit` 派生 request.start 到同一 runtime。`spawn_submit_target` 与 linker `build_spawn_routes` 目前只接受 `targetKind == "function"`。
- actor 同步调用链路（复用对象）：runtime `prepare_actor_method`（receiver 单独走 header，args 编码为 `skiff-actor-arguments-v1` canonical JSON 数组）→ `actor.method.invoke` → router `ProductionActorMethodRouter`/`ActorMethodDispatcher`（admission + 不 live 时 activateInitial + 按 identity 选 owner）→ `actor.owner.invoke` → owner runtime `ActorMethodExecutor`（激活/串行执行）。owner 侧不需要改动。
- **关键缺口 1（actor 方法内 spawn 的父请求认证）**：`requireSpawnParent` 只认 `RuntimeDispatcher.pending` 中的 runtime assembly request；actor 方法执行中的 `callerRequestId` 是 actor invocation id，位于 `ProductionActorMethodRouter.pending`。必须新增 actor-method parent 分支。
- **关键缺口 2（wire）**：`spawn.submit.request` 协议在 TS/Rust 两侧都只有 function 字段，且显式 `forbiddenField('actorRef')`；需要新增 actorMethod 目标元数据（actorRef/declarationOwner/三个 identity）。
- 可恢复边界：actor spawn args 在 caller 侧先跑 recoverable encode gate（`encode_spawn_args_payload`，method executable 的 expected plan），wire payload 复用 `skiff-actor-arguments-v1` canonical JSON 数组（owner executor 零改动解码）。receiver 不进入 args payload，作为 header 元数据。
- spawned actor 调用 deadline：固定 `DEFAULT_DERIVED_SPAWN_TIMEOUT_MS`（120s），不继承 caller deadline（与“caller timeout 不影响”一致）。
- 激活/串行/逐出安全：复用 `ActorMethodDispatcher` ledger（admit/transition），spawn 结果帧只结算 ledger 不回传。

## 写集（实际）

### Compiler
- `compiler/lowering/src/function_lowering.rs`：`spawn_function_target_metadata` 增加 `CallTargetIr::ActorMethod` 分支，metadata `{targetKind:"actorMethod", target:"actorMethod:<actorSymbol>:<methodIdentity>"}`。
- `compiler/core/src/spawn_targets.rs`：`spawn_submit_has_projected_target` 接受 function/actorMethod（actorMethod 跳过 artifact spawn target 投影）；`spawn_targets/tests.rs` 加 actorMethod 投影跳过测试。
- `runtime/linker/src/assembly_execution.rs`：`build_spawn_routes` 校验 actorMethod metadata 与 `LinkedCallTarget::ActorDispatch` plan 一致，不注册 function route。

### Runtime
- `runtime/capability-context/src/outbound_control.rs` + `lib.rs`：`SpawnSubmitControlRequest` 增加 `actor_method: Option<ActorMethodSpawnTargetControl>`；`runtime/request-contract` 同步 re-export。
- `runtime/transport/src/protocol.rs` + `control_mapper.rs`：spawn.submit frame 增加可选 `actorMethod` 元数据并映射（含 test initializer 补字段）。
- `runtime/eval/src/spawn_ops.rs`：`spawn_submit_target` 接受 `actorMethod`；新增 actor 方法 payload 编码（receiver 校验 + recoverable gate + `skiff-actor-arguments-v1` JSON 数组 wire payload + metadata/plan 一致性校验）。
- `runtime/eval/src/actor_executor/actor_concurrent_continuation.rs`：`ActorExecutionFrame::current_actor_ref()`——actor 方法内 `self` 不是普通 ActorRef 值，spawn self 的 receiver 由当前执行帧派生。
- `runtime/host` 三个 test initializer 补 `actor_method: None`。

### Router
- `router/src/protocol/envelope.ts` + `runtimeProtocol.ts`：`SpawnSubmitTargetKind` 增加 `'actorMethod'`；frame 类型与 schema/validator 增加 `actorMethod` 元数据（function 时 forbidden；保留 top-level actorRef/methodName forbidden）。spawn.submit 的 `serviceProtocolIdentity` 允许 `skiff-actor-abi-v1`（actor 方法执行上下文的协议身份是 actor ABI；function parent 仍由 authority 严格比对）。
- `router/src/router/runtimeDispatcher.ts`：`handleSpawnSubmit` 按 targetKind 分支；`requireSpawnParent` 支持 actor-method parent（经 `ActorMethodSpawnControl` seam + `RuntimeDispatchRegistry.spawnSubmitParentAuthority` 校验；actor parent 的 serviceProtocolIdentity 与 active actor invocation 的 actor ABI identity 关联）。
- `router/src/router/assemblyRuntimeRegistry.ts`：新增 `spawnSubmitParentAuthority(ws, header)`（复用 replica/binding 事实，返回完整 authority）。
- `router/src/router/productionActorMethodRouter.ts`：实现 `ActorMethodSpawnControl`（`submitSpawn` 构造 actor.method.invoke、经 ActorMethodDispatcher admission/激活、发送 owner.invoke、pending 无 caller、结果仅结算 ledger；`hasActiveActorInvocation`）；`invoke` 与 spawn 共享 `registerPendingInvocation`。
- `router/src/router/server.ts`：注入 seam。
- `router/tests/protocol.test.ts`：spawn.submit schema 契约更新；`router/tests/actor-spawn-submit.test.ts` 新增 3 项契约测试（owner dispatch + fire-and-forget、不 live 激活、actor-method parent）。

### 文档
- `doc/reference/spawn.md`：target 规则、执行语义（actor 路由/激活/排队、固定 120s deadline）、移除不支持项。

### 测试
- `test-runner/fixtures/actor-full-chain-acceptance/main.skiff` + `http.yml` + `scripts/lib/actor-full-chain-acceptance-real.mjs`：扩展正例 `spawn actor.method`（外部函数提交、500ms record、提交不等结果）、`spawn self.method`（kickSelf 自消息）、`spawn self` 三次 fanout（同一实例串行、history=="abc"）。

## 自验收矩阵（聚焦）

| 项 | 命令/证据 |
| --- | --- |
| compiler 相关 cargo 测试 | `cargo test -p skiff-compiler-core --lib spawn_targets`：5 passed（含新增 actorMethod 投影跳过）；`cargo test -p skiff-compiler-lowering --lib`：72 passed |
| runtime 相关 cargo 测试 | `cargo test -p skiff-runtime-eval -p skiff-runtime-host -p skiff-runtime-transport -p skiff-runtime-capability-context`：全部 passed（460/344/98/71+...） |
| router TS 测试 | `pnpm test`（router）：57 files / 774 tests passed（含新增 actor-spawn-submit 3 项） |
| 既有 actor full-chain acceptance 仍 PASS | `node scripts/run-actor-full-chain-acceptance.mjs`：PASS（2 replicas；含新增 spawn 正例） |
| 新增 spawn-to-actor 契约测试 | router `actor-spawn-submit.test.ts`（owner dispatch/fire-and-forget、不 live 激活、actor-method parent）+ acceptance `spawnExternal`/`spawnSelfKick`/`spawnFanout` |
| 文档 | spawn.md 与实现一致；rustfmt 通过 |

## 停止条件

- 需要改变公共契约/架构语义、新增语言概念、触碰兄弟任务 owner、或发现与既有 actor 机制冲突时，先上报主 Agent，不自行扩张。
