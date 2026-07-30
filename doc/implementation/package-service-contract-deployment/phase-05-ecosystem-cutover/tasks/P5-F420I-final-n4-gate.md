# P5-F420I Final N4 gate

状态：Ready（冻结候选上的独立只读 gate）。

## 直接父节点

- `P5-F420H-tooling-closure-combined-probe-result.md`

F420H 已在五组 repair合流后的 executable candidate 上得到 `97/97 COMBINED_PASS`，tooling plan
精确52 phase，default/tooling/checks plan完整，旧 generation/third-entrypoint owner为0。当前没有
在途 implementation写入或待决设计问题；本节点只对冻结候选执行一次完整 N4 verdict。

## 冻结候选

- candidate：
  `0d33d26acf631184603d8bdc2c78a7ac67971392`；
- tree：
  `e5961076f15a719bd755c8ac4e0445adf6eeae98`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时证明 candidate/tree与F415 ancestry。task文档与最终result只增加文档，不使 executable证据
失效。

## 唯一写入与环境

唯一 tracked写入：

```text
本任务 result
```

其余文件全部只读。Gate前置依赖由主 Agent 在本 worktree中以 frozen install准备；忽略的
`router/node_modules`、`vscode/node_modules`和共享 Cargo target不是 tracked写入。不得修改或
修复实现、test、fixture、manifest、lockfile、验证计划或生态仓库；不得派子 Agent、
merge/rebase/push、stable/live/instance/watch registry。

所有 Cargo命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 唯一 N4 gate

按顺序执行；某项失败仍继续执行彼此独立的后续项以收集同一候选的完整 verdict，但不得修复：

```bash
node scripts/verify.mjs --only tooling

node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs

node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs

node scripts/verify.mjs --only router

cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --list
cargo test --locked -p skiff-test-runner \
  --test package_service_contract_deployment -- --test-threads=1

node scripts/run-skiff-tests.mjs

cargo fmt --all -- --check
git diff --check
git status --porcelain
```

同时只读确认：

```bash
node scripts/verify.mjs --only tooling --list

rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]" \
  scripts --glob '*.mjs'

rg -n \
  "AssemblyWebSocketGateway|canonicalAssemblyWebSocketIngressIdentity|assemblyWebSocketGateway" \
  router --glob '*.{ts,tsx}'
```

预期 tooling精确52 phase；两组 retired owner反搜为0。记录每条命令的实际 discovery、
pass/fail/skip，不能用旧证据代替。

## Verdict 与交付

仅当全部动态命令、identity checker、format/diff、反搜和tracked clean均通过，result才写：

```text
N4_PASS
F421_RELEASED
```

否则写 `N4_FAIL`，逐项记录失败、owner与证据失效范围；不得自行修复。提交唯一 result并保持
worktree clean。result至少记录：

- candidate/task checkout/final commit与tree；
- tooling phase及测试总数；
- Node五文件组、identity、Router、test-runner listing/execution、source suite的实际计数；
- format/diff/status与两组反搜；
- 环境准备未改变tracked tree；
- 是否解除F421。

