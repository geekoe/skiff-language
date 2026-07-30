# P5-F15：Test Runtime Readiness Barrier

## 输入、owner与限制

- 输入：D16完成；exact integration `bcbdc2c55c08ab310b68bf1e6ac7101faad0d404` / tree
  `5401add98ed9513fd495dd3eba4ac92e7ef3bce2`，已包含F14/R14 PASS repair。
- 独立worktree/branch，一个clean commit，不merge/push。
- owner只限`test-runner/src/runtime_execution.rs`与新增聚焦direct tests。
- 不改Router、Runtime、linked-program、artifact、source suite、fixture、activation wire/receipt、manifest/Cargo.lock或stable。

## 完成态

activation HTTP 2xx后、发送唯一业务request前，runner轮询同control origin的`/__router/health`，用typed fail-closed
decode要求：

- pending activation为null；
- active environment、generation=`expected_generation + 1`与activation返回的exact assembly identity一致；
- 至少一个replica对同environment/generation/assembly为`healthy && connected`；
- 同runtime/replica identity的capability connection仍connected。

低于expected generation可在有界deadline内继续等待；generation前进、同代identity冲突、malformed/non-2xx health立即
失败。deadline必须有界并使用短poll/backoff，不固定sleep。barrier成功后业务request只发送一次；503/timeout/error不
重试，不回退或切换generation。

## 验证

```bash
cargo test --locked -p skiff-test-runner runtime_execution
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment
cargo clippy --locked -p skiff-test-runner --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

direct tests覆盖commit receipt→draining→healthy序列、pending、capability缺失、forward generation、identity冲突、
malformed/non-2xx、deadline，以及成功/失败均不重复业务request。每个filter非零。Clippy/fmt若base失败必须精确
base/candidate归因且changed files通过。回报health状态矩阵、commit/tree/lock、single clean、reverse与extra-review；
不在本任务宣称F04 verdict。

## R15 first receive

candidate `b7e0f4fc0a8d36550936b4b546ade809c6ce8786`被独立R15判FAIL：同步DNS可越过deadline，pending未调用
canonical activation validator，HTTP使用lossy UTF-8，且单文件膨胀到1273行并混合多职责。D17/F15A从原base重建；
不得把首次candidate合流或局部修补后继续验收。
