# LEAF-TAKEOVER: fix/spawned-actor-session-reconcile（2026-08-02）

父节点：`.task-contracts/TAKEOVER-20260802.md` §5.3（skiff 会话循环语义协调，进行中）。
仓库：`/Users/geek/workspace/skiff`；worktree：`/Users/geek/workspace/skiff-session-reconcile`；
分支：`fix/spawned-actor-session-reconcile`；基线：`6dee837b8ae916dbc0602d412f9250b3c9421d10`
（integration checkpoint）。

## 任务与结论

把 main 侧已验收的 activation+admission-rank 状态机语义（ranked liveness、exact abort、
prepare task 不变量）移植进 spawned-actor 分支的新会话循环架构（child tasks RAII、
FIFO/generation/断连清理），两侧测试集同时全绿，不二选一。**已完成**。

## 协调决策（为什么这样合）

冲突面是 `runtime/host/src/host/router_session.rs` 的会话消息循环：

- 分支侧（spawned-actor，`29c01e0e`）把会话重构为
  `run_connected_session_with_bootstrap` + `ConnectedRouterSessionGuard` RAII +
  `RouterSessionChildTasks`（actor owner 工作作为 session 子任务，退出时同步 drop），
  `ASSEMBLY_ACTIVATION_FRAME_TYPE` 在 `dispatch_router_binary_frame_inner` 里同步
  `apply_bootstrapped_assembly_activation_control`（无状态机、无取消、无 terminal 队列）。
- main 侧（上一会话验收的 activation+admission-rank）在旧会话循环里把 activation 帧在
  `handle_router_session_message` 处拦截进 `SessionActivationState` 状态机：
  Prepare 以可取消子任务执行（`JoinSet` + `CancellationSource`）、重复同 tuple Prepare 幂等、
  exact Abort 在 Preparing/TerminalReady/Idle 三态按 tuple 处理、terminal 只从
  TerminalReady 出站（带 2ms 竞态 Abort 探测，防止 terminal 后发而出现 late terminal）、
  退出时 `cleanup_session_activation` 对 Preparing/TerminalReady 补精确合成 Abort；
  `dispatch_router_binary_frame_inner` 对 activation 帧报
  “must be handled by the live Router session”。

合成结果（本分支 `router_session.rs`，唯一写文件）：

1. 保留分支的新循环骨架（guard RAII、child tasks、FIFO 顺序、generation 精确寻址），
   并把状态机接进 `run_connected_session_with_bootstrap`：
   - 循环顶做 `assert_task_invariant`（Preparing ⇔ 恰好 1 个 prepare task）；
   - TerminalReady 后先做一次有界（2ms）入站探测（给与 prepare 完成竞态的 exact Abort
     一次读取机会），再强制发送 terminal，避免入站洪泛饿死 terminal（与旧 main 一致）；
   - select 新增 `activation_prepare_tasks.join_next()` 分支：任务完成→`complete_prepare`
     →TerminalReady；任务 Err/JoinError/消失/多任务均按旧 main 语义 fail the session；
   - `handle_router_session_message` 恢复：activation 帧走 `dispatch_session_activation_frame`，
     其余帧走 `dispatch_router_binary_frame_with_health`（含 child tasks 透传）；
   - 循环退出后 `drop(child_tasks)` → `cleanup_session_activation` → `session_guard.close()`
     （activation 清理先于连接 teardown，与旧 main 顺序一致）。
2. `dispatch_router_binary_frame_inner` 的 `ASSEMBLY_ACTIVATION_FRAME_TYPE` 分支保留
   分支侧的直接 apply 实现（而非旧 main 的拒绝分支）。原因：三个 main 侧单元测试
   （`assembly_activation_fails_closed_before_connection_bootstrap`、
   `activation_rejects_superseded_transient_service_db_wire`、
   `activation_rejects_environment_other_than_runtime_trust_domain_before_resolution`）
   通过 `dispatch_router_binary_frame_inner` 直接分发 activation 帧并断言域错误；这些测试
   在 spawned-actor 分支（383/383 host 全绿）即以此形态通过。生产路径中 activation 帧在
   `handle_router_session_message` 被拦截，永不进入该内层分支；该分支实际只在
   `#[cfg(test)]` 测试助手路径可达，保留它等价于保留分支侧测试的既有语义，不构成
   双实现竞态。10 个 `activation_prepare` 测试走真实 WebSocket 会话路径，由状态机满足。
3. `ConnectionBootstrap` 保持 `#[derive(Clone)]`（状态机 Prepare 需要 clone bootstrap）。
4. 测试文件零改动：`tests/activation_prepare.rs` 与 tests.rs 均为集成 checkpoint 原样，
   无删减、无弱化、无机械适配。

## 测试矩阵（全部在 fresh CARGO_TARGET_DIR 下复跑，避免共享缓存交叉污染）

> 重要：本会话发现 `skiff-spawned-actor-test-capability/build/cargo-target` 共享缓存不能
> 跨 worktree 复用。两个 worktree 的 `skiff-runtime-transport` 源码不同但包名/版本/特性
> 相同，rlib 文件名碰撞；先用共享缓存跑本分支得到 83/86 + 20 个
> `loader::assembly_admission::tests::execution::*` 链接失败，随后在
> `/tmp/skiff-branch-fresh-target` 里用同一分支源码复跑该失败用例为 **ok**，证明是
> 缓存碰撞而非源码问题。本分支最终验证全部改用自身 `build/cargo-target`（fresh）。

| 项 | 命令 | 结果 |
| --- | --- | --- |
| main 侧 10 个 activation_prepare + 分支侧会话全套 | `cargo test -p skiff-runtime-host --lib -- router_session` | **86 passed / 0 failed**（含 10 个 activation_prepare、9 个 connection_lifecycle、11 个 websocket_generation_lifecycle、actor owner、runtime_assembly_request、foreign_db） |
| connection_lifecycle（精确 selector） | `cargo test -p skiff-runtime-host --lib -- connection_lifecycle` | 9 passed |
| websocket_generation_lifecycle | `cargo test -p skiff-runtime-host --lib -- websocket_generation_lifecycle` | 11 passed |
| actor_owner_invocations | `cargo test -p skiff-runtime-host --lib -- actor_owner_invocations` | 5 passed |
| transport actor_owner（generation wire） | `cargo test -p skiff-runtime-transport --lib -- actor_owner` | 9 passed |
| eval actor_executor（无 create cancel/deadline） | `cargo test -p skiff-runtime-eval --lib -- actor_executor` | 65 passed |
| 三包 tests 编译门 | `cargo check -p skiff-runtime-host -p skiff-runtime-eval -p skiff-runtime-transport --tests` | 通过 |
| Router 类型门 | `npx tsc --noEmit`（router） | 通过 |
| FIFO + generation authority | `npx vitest run tests/runtime-endpoint-actor-message-fifo.test.ts tests/actor-test-capability-authority.test.ts` | 2 files / 9 tests passed |
| pending-create 断连 + actor runtime disconnect + assembly endpoint | `npx vitest run tests/actor-runtime-disconnect.test.ts tests/actor-get-create-activation.test.ts tests/assembly-runtime-endpoint.test.ts` | 3 files / 43 tests passed |
| generation lifecycle（router/wire）+ 同 ID 碰撞 | `npx vitest run tests/websocket-generation-lifecycle-router.test.ts tests/websocket-generation-lifecycle-wire.test.ts tests/runtime-dispatcher-self-ingress-actor-parent.test.ts` | 3 files / 22 tests passed |
| rustfmt | `cargo fmt --check -p skiff-runtime-host` | 通过 |

## 已知独立问题（非本任务引入，交集成 Agent 决策）

- `loader::assembly_admission::tests::execution::*`（20 个）在 integration checkpoint
  `6dee837b` 本身失败：`typed_execution_async_stream_cancel_reaches_owned_provider_future_full_chain`
  等在 main worktree（无本分支改动）复现同样失败，报
  “whole-assembly candidate link failed … unresolved unique implementation package
  interface export … example.phase-four-consumer”。spawned-actor 分支 `29c01e0e`
  （fresh target dir）同一用例 **ok**。根因疑似 dirty main 的 linker/fixture 组合
  （本分支只改了 `router_session.rs`，与 loader/linker 无交集）。建议由 skiff 集成 Agent
  或 linker owner 排查 6dee837b 基线，不在本任务扩张。
- 共享构建缓存污染：本会话在 `skiff-spawned-actor-test-capability/build/cargo-target`
  中留下了本分支（session-reconcile 源码）编译产物，导致该缓存对 spawned-actor 分支
  不再可信；分支如需重新构建，请用 fresh `CARGO_TARGET_DIR`（例如
  `/tmp/skiff-branch-fresh-target`，本会话已用它验证分支 1 个用例 + `cargo check` 通过）。
- 磁盘：会话结束时 `df` 约 11 GiB 可用（高于 10 GiB 阈值）。`/tmp/skiff-branch-fresh-target`
  （约 4.7 GiB）为诊断用 fresh 构建目录，可删除（本会话因沙箱禁止 rm 未清理）。
- 本 worktree 按分支惯例创建了 `node_modules`、`router/node_modules` → main
  `node_modules` 的符号链接（git 不追踪）。

## 写集

- `runtime/host/src/host/router_session.rs`（+155/-40，唯一改动文件）。
- `LEAF-TAKEOVER.md`（本文件）。
- worktree 内 `build/cargo-target`（fresh 构建产物，不提交）。

## 交接

- 分支 `fix/spawned-actor-session-reconcile`；worktree `/Users/geek/workspace/skiff-session-reconcile`。
- 未 merge、未 push、未动 stable、未动 main。
