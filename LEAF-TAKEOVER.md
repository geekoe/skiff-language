# LEAF-TAKEOVER: Slice 1 — concurrent/serial v1 暂缓（前置片）

## Reference chain

- 权威设计：`doc/architecture/actor-shared-heap-design.md` v4（baseline `dc61c020d784050dfdad0392b22f0b9eb5801e87`），§6 Concurrent：v1 暂缓，§12 Slice 1。
- 直接父节点：设计 §12 Slice 1（无中间 DAG 任务文档）。
- 集成 Agent：`/root/integration_actor_shared_heap`（不自行 merge / push）。

任务文件不得覆盖设计；本文件只补充执行合同与证据 owner。

## Scope（写范围）

- `compiler/`：`compiler/source`（execution_semantics 拒绝 concurrent/serial/concurrent value；删除 concurrent 语义模块与仅 lane 使用的 helper）、相关编译测试（`compiler/source/src/tests/timeout_source_semantics.rs`、`compiler/tests/timeout_artifact_lowering.rs`）、新增编译拒绝回归测试。
- `runtime/eval/`：删除 `env/concurrent_plan.rs`、`env/concurrent_scheduler.rs(+batch.rs)`、`env/lane_control.rs`、`env/lane_state.rs`、`env/concurrent_scheduler_test_support.rs`、`eval_context/concurrent.rs(+tests.rs)`、`actor_executor/actor_concurrent_continuation/bridge.rs`、`actor_executor/tests/actor_concurrent_continuation/*`、`env/tests/concurrent_scheduler*.rs`、`tests/f445h_e4r_combined/r3_concurrent_case.rs`；移除对应 module/reexport/分发接线；保留 `ActorExecutionFrame` 核心与 `with_actor_execution_frame`。
- `doc/reference/runtime.md` §6：加一行 v1 不支持 concurrent/serial（编译期拒绝）。

## Out of scope（禁止）

- `router/`、`.github/workflows/verify.yml`、artifact-model schema、`runtime/linked-program` 的 concurrent plan schema（保持可达但不可达使用）、router-rust batch worktrees/branches、主 worktree dirty 文件。
- 不改变普通 request 语义、actor 事务路径（Slice 5 再禁）、artifact ABI/wire。

## Execution contract

- Baseline：`dc61c020`；分支 `feat/actor-concurrent-disable`；worktree `/Users/geek/workspace/skiff-actor-slice1`；独立 `CARGO_TARGET_DIR=build/cargo-target`。
- Compiler 语义层：`Stmt::Concurrent` / `Stmt::Serial` / `Expr::ConcurrentValue` 在 owner analysis 报 `concurrent/serial ... is not supported in v1`；parser 与 syntax tests 保留。
- Lowering 保持 schema 中性：ConcurrentPlanIr 相关 lowering 代码可保留为不可达（artifact-model / linked IR 不变）。
- 聚焦验证：
  - `cargo test -p skiff-compiler-source`（含新拒绝回归测试）
  - `cargo test -p skiff-compiler --test timeout_artifact_lowering`
  - `cargo test -p skiff-runtime-eval`（lib + 现存 integration tests）
  - `cargo fmt --check`（touched crates）
- 提交：`feat(compiler): reject concurrent/serial in v1; remove runtime concurrent machinery`。

## 自验收矩阵 owner

开发 Agent（本会话）负责在提交时附矩阵；集成 Agent 负责身份/边界核对与合并。
