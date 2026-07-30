# P5-F420G3 HTTP-only tooling smoke closure

状态：Ready。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`

F420F 的 B3 已完整确认三个有序残留：package-test service id 少 `/case-0`；ecosystem helper
仍消费已删除的第三个 Assembly WebSocket entrypoint；R05 generation CLI/helper/tests 仍建立在
RuntimeAssembly v1、旧 receipt 字段和 Assembly WebSocket connection pin 上。F420B 已冻结
RuntimeAssembly v2 HTTP-only，因此不需要新设计决策。

## 允许写入

```text
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-ecosystem-smoke-real.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/package-service-ecosystem-smoke-lifecycle.test.mjs
scripts/tests/package-service-ecosystem-smoke-real.test.mjs

scripts/run-package-service-generation-lifecycle-smoke.mjs
scripts/lib/package-service-generation-lifecycle-smoke-oracle.mjs
scripts/lib/package-service-generation-lifecycle-smoke-real.mjs
scripts/tests/package-service-generation-lifecycle-fixture-combined.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-lifecycle.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-oracle.test.mjs
scripts/tests/package-service-generation-lifecycle-smoke-real.test.mjs

scripts/lib/package-service-i02-combined-real.mjs
scripts/lib/package-service-http-unary.mjs                 # 可新建
scripts/tests/package-service-http-unary.test.mjs          # 可新建
本任务 result
```

从 batch exact start/tree 启动。不得修改 Router、Rust/test-runner、fixture source、manifest、
lockfile、verify plan 或其它 tests；不得派子 Agent、merge/rebase/push/stable/live。

## 必须收敛

1. Current package-test receipt 的 service id 使用 Rust producer 的 exact
   `test.skiff/package/<safe-coordinate>/case-{index}`；本 fixture 为 `/case-0`。维持 v2 精确两个
   HTTP entrypoint、缺失/额外项 fail closed。
2. `package-service-ecosystem-smoke-real.mjs` 只保留仍被 I02/actor acceptance 使用的 current
   fixture root/Cargo args 等中性 helper；删除无生产 CLI owner的 Assembly-WebSocket run path、
   `ws` loader、marker与 WebSocket lifecycle exports。相应纯旧 WS tests 删除，不伪装为 current
   HTTP coverage。
3. 删除旧 `run-package-service-generation-lifecycle-smoke.mjs`、Assembly-WebSocket generation
   helper/oracle及其四个 test files。它们不能恢复第三 entrypoint、connection pin、v1 receipt、
   `packageTest.name` 或 `contract/operation` dual-read。
4. 旧 generation helper 中仍被 I02 使用的 bounded HTTP unary request、raw body、oversize、
   diagnostic redaction与 RuntimePayload string validation，迁入语义中性的
   `package-service-http-unary.mjs`；I02 改从新 owner import。新增 focused test，把旧 tests 中
   与这些 current HTTP utility直接相关的成功、失败、截断和脱敏覆盖迁来。
5. 删除后反向搜索必须证明旧 generation CLI/helper、第三 entrypoint与 Assembly WS probe marker
   没有 production/test owner。不要删除 Router 的通用 WebSocket、connection lifecycle 或
   generation registry tests；它们不在本任务范围。

## 聚焦验证

所有 Cargo 调用使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

运行：

```bash
node --test \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-http-unary.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs

node scripts/verify.mjs --only tooling --list

rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]|skiff-runtime-assembly-v1" \
  scripts --glob '*.mjs'

git diff --check
```

若保留了任何旧命名 test，必须解释它证明的 current HTTP 行为；否则应删除而不是维持虚假
Assembly-WebSocket 正例。记录删除前后 tooling phase 精确变化、保留 utility coverage和所有反搜。
实现/result 分开提交并保持 clean。发现 current I02 需要修改未授权 production owner时返回
`TASK_SCOPE_EXPANDED`。

