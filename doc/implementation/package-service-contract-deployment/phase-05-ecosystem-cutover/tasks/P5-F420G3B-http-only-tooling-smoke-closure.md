# P5-F420G3B HTTP-only tooling smoke closure

状态：Ready（G3A 验收范围修正版）。

## 直接父节点

- `P5-F420G3A-http-only-tooling-smoke-closure-result.md`

G3A 证明原全 `scripts` 的 v1 反搜错误纳入三个无关 synthetic fixture；它们分别验证 isolated
readiness、Host negative probe和 fake CLI参数，不属于 generation/ecosystem owner。本后继不修改
这些 fixture，只把终态反搜限定到本批 owner。当前预检已证明其它旧 symbol命中全部位于允许删除/
迁移文件内。

## 精确起点

- integrated start：
  `bc0925396261812cfc9f5bee07246eda14cdff6c`；
- tree：
  `3b48111e724d722a6ee040682515fcc1b1225cbb`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 start/tree 与 F415 ancestry。

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
scripts/tests/package-service-i02-combined.test.mjs
scripts/lib/package-service-http-unary.mjs                 # 可新建
scripts/tests/package-service-http-unary.test.mjs          # 可新建
本任务 result
```

不得修改 Router、Rust/test-runner、fixture source、manifest、lockfile、verify plan、三个范围外
v1 synthetic fixture或其它 test；不得派子 Agent、merge/rebase/push/stable/live。

## 实现终态

1. Shared receipt、shared fixture和 I02 fixture统一使用 Rust current producer的
   `test.skiff/package/<safe-coordinate>/case-0` service id；v2 仍只接受精确两个 HTTP
   entrypoint并 fail closed。
2. `package-service-ecosystem-smoke-real.mjs` 只保留 I02/actor acceptance当前仍消费的 fixture
   root/Cargo args等中性 helper；删除无 CLI owner的 Assembly-WebSocket runner、`ws` loader、
   marker/lifecycle exports及其两个旧 tests。
3. 删除旧 generation lifecycle CLI、real/oracle modules和四个旧 tests。不得恢复 v1、第三
   entrypoint、旧 `name/contract/operation`、Assembly WS connection pin或兼容读取。
4. 把 I02 仍需要的 bounded HTTP unary request、raw body、oversize、diagnostic redaction与
   RuntimePayload string validation迁入 `package-service-http-unary.mjs`；I02 改从新 owner
   import，并新增 focused test覆盖成功、非200、截断、脱敏与 payload decode。
5. 不触碰 Router通用 WebSocket / connection lifecycle / generation registry。

## 聚焦验证

Cargo fixture（如触发）使用：

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
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]" \
  scripts --glob '*.mjs'

rg -n "skiff-runtime-assembly-v1" scripts \
  --glob 'package-service-ecosystem-smoke-*.mjs' \
  --glob 'package-service-generation-lifecycle-*.mjs' \
  --glob 'run-package-service-generation-lifecycle-smoke.mjs'

git diff --check
```

两组反搜均为0。删除6个旧 test、增加1个 HTTP utility test后，tooling plan应从57精确变为52
phase；若实际 discovery 不同，先说明具体文件，不得静默缩 selector。不得运行完整 tooling或
其它 N4 gate。

## 交付

实现/result分开提交。result记录 commit/tree、`/case-0`一致性、删除/保留清单、tooling
`57 -> 52`、focused真实计数、两组反搜、diff与clean状态。范围扩张立即停止。

