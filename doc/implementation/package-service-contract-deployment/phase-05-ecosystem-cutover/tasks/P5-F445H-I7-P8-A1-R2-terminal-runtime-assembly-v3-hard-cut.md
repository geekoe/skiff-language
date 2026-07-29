# P5-F445H I7 P8 A1-R2 terminal RuntimeAssembly v3 hard cut

状态：

```text
COMPLETE
DECISION_REQUIRED = NO
```

## 1. Parent, baseline and boundary

- 直接父节点：
  `P5-F445H-I7-P8-A1-R1-source-suite-runtime-assembly-v3-result.md`。
- 架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`。
- baseline：
  commit `cc14290d2313e0db56eaf6d47eaf22bb545ae6dd`，
  tree `17eeff992aad7b0626ac0f38e324913147262d50`。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。

父节点已经把 canonical source suite 切到当前
`skiff-runtime-assembly-v3:sha256:<64 lowercase hex>`；本节点只机械闭合其反向搜索列出的 terminal
consumer、fixture、store corpus、native actor fixture和Runtime README。不得修改producer、schema、
identity generation、其它generation、公共契约或抽取新的共享常量。

## 2. Write set

```text
scripts/lib/package-service-authoring.mjs
scripts/lib/encrypted-storage-live-contract.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-i02-combined-oracle.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-authoring.test.mjs
scripts/tests/package-service-dev-sync.test.mjs
scripts/tests/encrypted-storage-live-harness.test.mjs
scripts/tests/platform-source-transport-combined.test.mjs
scripts/tests/package-service-i02-combined.test.mjs
cross-system-fixtures/package-service-ecosystem/ecosystem-store-cases.json
runtime/native/src/dispatch/prepared_tests/actor.rs
runtime/README.md
本task及result
```

测试正例全部使用current v3。`package-service-authoring`和encrypted-storage receipt测试必须新增精确v2
拒绝断言，防止机械替换丢失hard-cut负例。

## 3. Explicit exclusions

以下v2只作为历史拒绝/变异语料保留：

```text
artifact-identity/src/runtime_assembly.rs
artifact-model/src/schema.rs
scripts/tests/skiff-source-test-suite.test.mjs
cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json
doc/implementation/**
```

不得把它们改成v3正例，也不得加入v2兼容路径。

## 4. Validation

RED先证明active write set仍含v2。GREEN运行：

```text
node --test \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-dev-sync.test.mjs \
  scripts/tests/encrypted-storage-live-harness.test.mjs \
  scripts/tests/platform-source-transport-combined.test.mjs \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
cargo test -p skiff-runtime-native all_four_actor_registry_routes_are_owned_external_waits
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
node --check <每个触碰的 .mjs>
git diff --check
```

最后反向搜索证明active consumer中v2归零，仓库剩余v2精确落在上述allowlist。若暴露非机械语义问题，
停止并上报，不扩大本节点。

## 5. Handoff

提交implementation与result，报告branch、worktree、commit/tree、实际写集、测试和allowlisted v2，
直接交给`/root/phase05_integration_steward`串行集成与清理。本节点不merge、不push。
