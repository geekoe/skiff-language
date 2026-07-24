# P5-F137：Boundary Projection 测试 Arity 修复

状态：Ready

## 权威设计与 DAG

- 权威设计：`doc/architecture/package-service-contract-deployment.md`。
- 节点：C1 前置机械 blocker；恢复 compiler projection 聚焦测试可执行性。
- 前置：当前 Phase 5 integration checkpoint。
- 完成后解除：service-call stream compiler projection checkpoint。

## 写入范围

- `compiler/projection/src/package_artifact/tests/boundary.rs`
- 只修复 `project_boundary_callable` 新增 `public_type_ids` 参数后遗留的测试调用，不改 production API/语义。

## 完成标准与验证

- 所有调用使用与 fixture 匹配的 public type id 集合；搜索同类旧 arity 残留。
- 运行 boundary projection 聚焦测试与 `git diff --check`；不运行完整 gate。
- 若需要改变 public type closure 语义，返回 `TASK_NOT_EXECUTABLE`。

## Worktree

- `/Users/geek/workspace/skiff-p5-f137`
- branch `codex/p5-f137-boundary-test-arity`
- 一次性开发会话；提交、不 push、不操作 stable。

