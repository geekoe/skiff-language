# P5-F445H I7 P8 K2 test-runner pipe output flush

状态：

```text
IMPLEMENTED
READY_FOR_INTEGRATION
```

## Parent, baseline and scope

- 直接父节点：
  `P5-F445H-I7-T0-internals-isolated-gate-tooling-result.md`
- 相关runner结果：
  `P5-F445H-I7-P8-K-test-runner-http-entry-closure-result.md`
- Skiff baseline：
  `6bfddf0bcb45398239f26ed0fdff74c047774d34`
  （tree `3f47d5be3009649441e8eb8433a4747b01a3e608`）
- integration owner：
  `/root/phase05_integration_steward`

目标是修复`skiff-test-runner`在失败后立即调用`process::exit(1)`导致pipe环境可能丢失已生成
逐case输出的问题。

## Zero-worktree preflight

- production入口、逐case输出和失败退出都在`test-runner/src/main.rs`。
- `execute`先向stdout输出逐case结果，再把失败数作为`Err`返回；`main`向stderr输出错误和usage，
  随后调用`process::exit(1)`。
- 当前没有覆盖“多case失败、stdout/stderr均为pipe、exit 1”的回归测试。
- 修复可以只在test-runner内完成，无需修改协议、输出格式、compiler、runtime、router或脚本。

## Write set and completion

预期写集：

```text
test-runner/src/main.rs
test-runner/src/main/tests.rs
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-P8-K2-test-runner-pipe-output-flush.md
  P5-F445H-I7-P8-K2-test-runner-pipe-output-flush-result.md
```

实现要求：

1. `main`正常返回退出码，不在production失败路径调用`process::exit`。
2. 返回前显式flush stdout和stderr；不得使用sleep。
3. 保持现有逐case、message、failure summary、usage和成功summary文本不变。
4. pipe子进程回归证明多case失败时逐case结果和failure summary完整、退出码为1。
5. 回归证明成功路径输出和退出码不变。

验证：

```text
cargo test --locked -p skiff-test-runner --bin skiff-test-runner
cargo test --locked -p skiff-test-runner --no-fail-fast
cargo fmt --all -- --check
git diff --check
```

若实现需要跨crate、改变CLI格式或引入测试专用production协议/参数，则停止并返回
`TASK_SCOPE_EXPANDED`。

实现与验证结果见
`P5-F445H-I7-P8-K2-test-runner-pipe-output-flush-result.md`。
