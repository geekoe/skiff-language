# P5-F420H Tooling closure combined probe

状态：Ready（F420G repair batch 合流探针，只读）。

## 直接父节点

- `P5-F420G-tooling-closure-batch.md`
- `P5-F420G1-crate-public-api-current-oracles-result.md`
- `P5-F420G2-dev-sync-runtime-assembly-v2-fixture-result.md`
- `P5-F420G3B-http-only-tooling-smoke-closure-result.md`
- `P5-F420G4-test-runner-target-current-inventory-result.md`
- `P5-F420G5-verify-plan-single-command-owner-result.md`

五个 owner 已合流到同一候选。本节点只证明这些 owner在合流状态上共同接线正确，不运行完整
tooling verdict，也不修改实现。

## 精确候选

- candidate：
  `95e5c5e2f4549aedddfad28c7f6cdc9e4609bca2`；
- tree：
  `8f9a261640709c973517578b02c1372ca36aab73`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 candidate/tree 与 F415 ancestry。

## 唯一写入

```text
本任务 result
```

其余 tracked 文件全部只读。缺少 Router test依赖时允许执行
`pnpm --dir router install --frozen-lockfile --ignore-scripts`，只生成 ignored
`router/node_modules`；不得修改 manifest/lockfile。不得派子 Agent、merge/rebase/push、
stable/live/instance。

## Combined probe

运行一个真实 Node test invocation：

```bash
node --test \
  scripts/tests/crate-public-api-gate.test.mjs \
  scripts/tests/crate-public-api-policy.test.mjs \
  scripts/tests/package-service-dev-sync.test.mjs \
  scripts/tests/package-service-ecosystem-http-fixture.test.mjs \
  scripts/tests/package-service-http-unary.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/test-runner-runtime-isolation.test.mjs \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-rust-quality.test.mjs \
  scripts/tests/verify-taxonomy.test.mjs \
  scripts/tests/verify.test.mjs
```

预期精确 `97/97`。随后运行：

```bash
node scripts/verify.mjs --list
node scripts/verify.mjs --only tooling --list
node scripts/verify.mjs --only checks --list

rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]" \
  scripts --glob '*.mjs'

git diff --check
git status --porcelain
```

预期 tooling精确52 phase；default/tooling/checks plan integrity通过，package-store discovery
执行数分别为1/1/0；旧 generation/third-entrypoint反搜为0；除 result外零 tracked diff。

## 交付

提交 result，记录 exact candidate、97项实际 discovery/execution、三个 plan计数、反搜、环境准备、
diff/clean状态。全部通过才报告 `COMBINED_PASS`，允许主 Agent冻结 N4候选。任何失败只分类和记录，
不得修复。

