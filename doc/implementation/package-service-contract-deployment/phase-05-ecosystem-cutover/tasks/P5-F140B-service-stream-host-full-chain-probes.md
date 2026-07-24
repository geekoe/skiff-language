# P5-F140B：Service Stream Host Full-chain Probes

状态：Ready（替代 owner 错配的 P5-F140）

## 权威设计与 DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md`。
- 稳定语言语义参考：`doc/reference/runtime.md` 的普通 service `Stream<T>` 生命周期。
- 节点：C1 Runtime 证据 D；前置 P5-D82、P5-F138。
- 完成后解除：AIHub → Agine consumer 重验的 Runtime 证据。

## 写入范围

- `runtime/host/src/loader/assembly_admission/tests/execution/async_stream_cancel.rs`
- 必要时 `artifacts.rs`、`runtime.rs` 测试 fixture。
- 只有精确证明 provider task cleanup 无现有观测点时，允许在
  `runtime/eval/src/assembly_execution/{async_stream_cancel.rs,mod.rs}` 增加 `test-support` counter accessor/re-export。
- 不改 wire、公共生命周期、compiler 或 production dispatch。

## 完成标准

- 复用 `TypedExecutionFixture`，经过 admitted resolved binding 与 `execute_runtime_assembly_addr` 的真实 Host dispatcher；
  不构造 eval-only runtime。
- 在现有 normal end、early break、callback lifetime、registry cleanup 基础上，补齐可执行的双 item 顺序、
  generic substitution、provider error、request cancel 与目标 stream 隔离。
- 只对现有 fixture 无法观测且不影响阶段风险的项给出明确证据，不为覆盖率改 production API。

## 验证

- Runtime host execution/stream 聚焦测试、目标文件格式与 `git diff --check`；不运行完整 gate。
- 若公共语义或跨 owner production 修复必需，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f140b`
- branch `codex/p5-f140b-service-stream-host-probes`
- 一次性新开发会话；提交、不 push、不操作 stable。

