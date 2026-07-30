# P5-F420G3A HTTP-only tooling smoke closure

状态：Ready（G3 合同范围修正版）。

## 直接父节点

- `P5-F420G3-http-only-tooling-smoke-closure-result.md`

G3 在首次写入前证明：Rust current producer、shared receipt oracle 与 I02 focused fixture必须同时
采用 `/case-0` service id，但原合同遗漏
`scripts/tests/package-service-i02-combined.test.mjs`。本后继补齐这个唯一 test owner，并完成原
G3 的 HTTP-only 收敛；没有新设计问题。

## 精确起点

- integrated start：
  `1010929ed2508d3b5d4bfcd1537d4eef3c599aa3`；
- tree：
  `7be5b76d7a234c731fef9044a772b118951da3b9`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 start/tree 与 F415 ancestry。当前状态已含 G1、G2、G4，写入与本节点不相交；G5
并行修改 verify plan，同样不得触碰。

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

不得修改 Router、Rust/test-runner、fixture source、manifest、lockfile、verify plan 或其它 test；
不得派子 Agent、merge/rebase/push/stable/live。

## 必须实现

1. Shared package-test receipt 的 service id 精确为
   `test.skiff/package/<safe-coordinate>/case-{index}`；本批 fixtures 都是 `/case-0`。同步修正
   shared fixture 与 I02 test 中两行旧 override。v2 仍精确只有两个 HTTP entrypoint，缺失/额外
   项 fail closed。
2. 将 `package-service-ecosystem-smoke-real.mjs` 收窄为仍由 I02/actor acceptance 使用的
   fixture root/Cargo args等中性 helper。删除没有 CLI owner的 Assembly-WebSocket runner、
   `ws` loader、marker和 WebSocket lifecycle exports；删除只证明该旧路径的两个 test files。
3. 删除旧 generation lifecycle CLI、real/oracle modules及四个对应 tests。它们基于
   RuntimeAssembly v1、第三 entrypoint、旧 `name/contract/operation` 和 Assembly WebSocket
   connection pin；不得迁回兼容层或伪装成 HTTP 测试。
4. 把仍被 I02 使用的 bounded HTTP unary request、raw body、oversize、diagnostic redaction与
   RuntimePayload string validation迁到中性的 `package-service-http-unary.mjs`，I02 改从新
   owner import；新增 focused tests，迁移旧 suite 中直接覆盖这些 current HTTP utility 的成功、
   非 200、截断、脱敏与 payload decode 断言。
5. 不删除或修改 Router 的通用 WebSocket、connection lifecycle或generation registry测试。
   删除的只是已经没有 current RuntimeAssembly owner 的 tooling lane。

## 聚焦验证

为可能触发的 Cargo fixture使用：

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

反搜必须为0；tooling list的 phase下降必须与删除的六个旧 test files精确对应，并保留新 HTTP
utility test。不得运行完整 tooling或其它 N4 gate。

## 交付

实现/result 分开提交。result 记录：

- start、implementation/final commit/tree；
- `/case-0` producer/consumer一致性；
- 删除的 CLI/module/test清单、tooling phase数量变化；
- 新 HTTP utility保留的覆盖及 focused实际计数；
- 四类旧残留反搜为0、diff通过、worktree clean；
- 无 merge/rebase/push/stable/live。

任何未授权 production/test owner需求都返回 `TASK_SCOPE_EXPANDED`。

