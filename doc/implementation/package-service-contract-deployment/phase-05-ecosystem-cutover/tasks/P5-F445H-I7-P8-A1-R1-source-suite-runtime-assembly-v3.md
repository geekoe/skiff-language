# P5-F445H I7 P8 A1-R1 source-suite RuntimeAssembly v3 closure

状态：

```text
IN_PROGRESS
DECISION_REQUIRED = NO
```

## 1. Parent, baseline and scope

- 直接父节点：
  `P5-F445H-I7-P8-A1-top-level-alias-instance-method-closure.md`
- 父节点结果：
  `P5-F445H-I7-P8-A1-top-level-alias-instance-method-closure-result.md`
- 架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- baseline：
  commit `f28ecd9a2099c575bfbe6e3aad40296d7157e559`，
  tree `1cd60da6c2c4379914d1abf15e3dd34b45e3bcbb`。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。

本节点只修复 canonical source-test suite 对当前 RuntimeAssembly identity 的陈旧校验。禁止修改
artifact schema、identity 生成、compiler、runtime、Router 或 A1 fixture。

## 2. Preflight facts

- `artifact-model/src/schema.rs::RUNTIME_ASSEMBLY_SCHEMA_VERSION` 唯一当前值为
  `skiff-runtime-assembly-v3`。
- `artifact-model/src/activation_lexical.rs::RUNTIME_ASSEMBLY_IDENTITY_PREFIX` 唯一当前前缀为
  `skiff-runtime-assembly-v3:sha256`；v1、v2 都是历史负例。
- `test-runner/src/package_service_host_fixture.rs` 从 compiler authoring receipt 反序列化出
  `RuntimeAssemblyRef`，再写入 `baseAssembly`，因此真实 producer 正确输出 v3。
- `scripts/lib/skiff-source-test-suite.mjs::readPackageServiceHostFixtureReceipt` 仍只接受 v2，导致真实
  v3 receipt 在 linked source suite 进入 runner 前被拒绝。
- 该读取函数的运行消费者是 canonical source suite 与 Host negative probe；两者都应共享同一严格 v3
  校验，不需要各自新增路径。

## 3. Implementation and evidence

最小写集：

```text
scripts/lib/skiff-source-test-suite.mjs
scripts/tests/skiff-source-test-suite.test.mjs
scripts/tests/platform-source-transport-combined.test.mjs
本task及result
```

要求：

- v3 lowercase 64-hex identity通过；
- 历史v2、错误版本、uppercase和错误长度均拒绝；
- 不宽松接受任意`skiff-runtime-assembly-vN`，不保留v2 fallback；
- canonical source runner继续使用receipt中的精确identity。

验证：

```text
node --test scripts/tests/skiff-source-test-suite.test.mjs
node --test scripts/tests/platform-source-transport-combined.test.mjs
node --check scripts/lib/skiff-source-test-suite.mjs
git diff --check
```

必要时运行最小 canonical linked source receipt 探针；不运行完整阶段 gate。

## 4. Handoff

提交implementation与result，报告branch、worktree、commit/tree、实际写集、RED/GREEN和反向搜索证据，
交给`/root/phase05_integration_steward`串行集成与清理。不merge、不push。
