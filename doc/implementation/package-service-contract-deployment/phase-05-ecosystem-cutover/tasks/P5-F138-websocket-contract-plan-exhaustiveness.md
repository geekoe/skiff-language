# P5-F138：WebSocket Contract Plan Exhaustiveness 修复

状态：Ready

## 权威设计与 DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md`。
- 节点：C1 前置机械 blocker；恢复 Runtime eval 聚焦测试可执行性。
- 前置：当前 Phase 5 integration checkpoint。
- 完成后解除：service-call stream Runtime full-chain probes。

## 写入范围

- `runtime/eval/src/websocket_contract_plan.rs`
- 必要的同模块聚焦测试。
- 只补齐 `ContractTypeRef::PackagePublic` / `TypeParam` 的既定 fail-closed 或 projection 行为，不改变 WebSocket
  公共契约、wire 或生命周期。

## 完成标准与验证

- match 穷尽且行为由现有 owner/相邻分支证明；搜索同类非穷尽映射。
- 运行该模块/Runtime eval 聚焦测试与 `git diff --check`；不运行完整 gate。
- 若现有设计无法决定行为，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f138`
- branch `codex/p5-f138-websocket-plan-exhaustiveness`
- 一次性开发会话；提交、不 push、不操作 stable。

