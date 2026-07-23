# P5-F46A：Test-runner Replica Health Parity

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第8、10、11条，§12及§14。

DAG节点F46A，依赖I35B exact FAIL。目标是让test-runner canonical activation receipt/router health strict decoder精确
接受当前Router replica schema中的`connectionPinCount`与`connectionReleaseAckCount`，不放宽unknown fields。

独占写入：

- `test-runner/src/runtime_execution/wire.rs`
- 对应test-runner wire/fixture聚焦tests。

要求：

- 两字段均为required non-negative safe integer，与Router字段名/语义逐字一致；
- receipt、health或共享ReplicaSnapshot的所有decode路径一致；
- missing、negative、fractional、unsafe、string及unknown额外字段fail closed；
- 不增加serde catch-all、optional兼容或legacy fallback；
- 不修改Router/Runtime、scripts/fixture、shared control wire或Cargo/lock。

开发owner运行：

```bash
cargo test --locked -p skiff-test-runner runtime_execution::wire
cargo check --locked -p skiff-test-runner
git diff --check
```

禁止运行真实fixture/I02/R05、instance/stable/full gate。独立worktree/branch，5分钟内修改，否则
`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
