# P5-F396 Test-runner HTTP gateway final revalidation

状态：Ready（从F387 clean checkpoint恢复）。

## 直接父节点

- `P5-F387-test-runner-http-gateway-convergence-blocker.md`
- `P5-F392-router-current-artifact-generations-result.md`
- `P5-F394-router-runtime-protocol-current-service-identity-result.md`

F392/F394已经关闭F387 isolated activation遇到的全部v3门禁。本节点不重做T1/T2，只引入精确前置、
关闭两个已知test-runner gate并完成真实验证。

## Worktree与恢复

- `/Users/geek/workspace/skiff-p5-f386-package-test-http-gateway`
- branch `codex/p5-f386-package-test-http-gateway`
- clean HEAD `71687e3765fc302611aad5de22a095d1621e4b8f`

1. 核对clean后依序cherry-pick：
   - F392 `e4cf24313717ec8842bf0e4771cc130746e2af34`
   - F394 `540f93c4fa52885bd8498a9144dd1b6dea49ec29`
2. 冲突则停止并精确报告；不得手工复制Router shared seam。

## 已知局部gate

本节点明确授权：

- `test-runner/fixtures/package-service-host/provider/api.yml`
  - 为真实dependency service function `echo`增加`serviceCall: true`；
  - direct fixture contract保持精确1 operation；
  - 不改变函数实现/签名或external gateway。
- `test-runner/src/canonical_package.rs`
  - 通过小型参数对象/职责抽取清理`too_many_arguments`，不加allow。
- `test-runner/src/canonical_std_seed.rs`
  - 通过合理错误边界/boxing或职责抽取清理`result_large_err`，保持诊断与fail-closed行为，不加allow。
- 上述direct tests。

若运行暴露其它production owner，按工作流停止，不扩大。

## 必须重跑

F384 T1/T2全部结构、integration、bins、Node v2 receipt与clippy gate，并运行真实isolated suite：

```bash
cargo test --locked -p skiff-test-runner --lib runtime_execution -- --test-threads=1
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1
cargo check --locked -p skiff-test-runner --bins
cargo clippy --locked -p skiff-test-runner --all-targets --no-deps -- -D warnings
node --test scripts/tests/package-service-ecosystem-http-fixture.test.mjs
node scripts/run-skiff-tests.mjs
git diff --check
```

真实isolated运行必须证明：

- package-test zero-op/one gateway fixture activation成功；
- F385 strict control→Router→Host/eval→`response.end`完整贯通；
- inline test setup effects生效；
- package-service-host provider/consumer service dependency正常；
- 非零case实际执行，不接受只编译/组装；
- 所有临时进程、Mongo、端口和目录清理。

## 交付

写`P5-F396-test-runner-http-gateway-final-revalidation-result.md`，记录cherry-picks、追加修复commit、
非零case/test计数与fresh identities。worktree clean，不merge/rebase/push，不操作stable/live。

新Agent执行，不派子Agent。
