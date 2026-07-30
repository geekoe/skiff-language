# P5-F420H Tooling closure combined probe result

状态：`COMBINED_PASS`。F420G 五个 owner 在同一 executable candidate 上共同接线正确；
唯一 combined Node invocation 精确发现并执行 `97/97` tests，三个 canonical plan 均通过
integrity，tooling 精确为 `52` phases。允许主 Agent 冻结 N4 候选。

## 1. Exact candidate、task checkout 与 ancestry

- executable candidate / tree：
  `95e5c5e2f4549aedddfad28c7f6cdc9e4609bca2` /
  `8f9a261640709c973517578b02c1372ca36aab73`；
- task checkout / tree：
  `ad6a4f2a125fef2215492b8e440b4c0ab1d250b0` /
  `afc6a122caef0d857c6aefd2d771ea2685f1e1b1`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

`git rev-parse HEAD^` 精确返回 executable candidate，task checkout 相对 candidate 只新增
`P5-F420H-tooling-closure-combined-probe.md`。`git rev-parse <candidate>^{tree}` 精确返回冻结
tree；`git merge-base --is-ancestor <accepted-F415> <candidate>` 返回 0。启动时
`git status --porcelain` 为空。

## 2. Combined Node probe

按任务原样运行一个真实 invocation：

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

实际 TAP 汇总：

```text
tests 97
suites 0
pass 97
fail 0
cancelled 0
skipped 0
todo 0
```

即 discovery `97`、execution `97`、通过 `97/97`，没有 skip 或未执行项。

## 3. Canonical plan 证据

三个任务指定命令均以 exit 0 完成，plan integrity 全部通过：

| selector | phase 数 | exact package-store discovery execution 数 |
| --- | ---: | ---: |
| default `verify` | 270 | 1 |
| `tooling` | 52 | 1 |
| `checks` | 16 | 0 |

这里的 exact execution 是命令与参数精确等于
`node scripts/check-package-store-discovery.mjs`。default plan 另有不同参数形态的
`node --check scripts/check-package-store-discovery.mjs` syntax phase，不属于 discovery
workload。tooling 的 `52` phases 与任务预期精确一致。

## 4. 反搜、环境准备与 clean 状态

任务指定反搜：

```bash
rg -n \
  "runPackageServiceGenerationLifecycleSmoke|r05-generation-lifecycle|entrypoints\\[2\\]" \
  scripts --glob '*.mjs'
```

结果为 0 matches（`rg` exit 1），旧 generation / third-entrypoint owner 均不存在。

Router 环境预检发现 `router/node_modules/ws` 缺失，且从 `router/package.json` 上下文
`require.resolve('ws')` 返回 `MODULE_NOT_FOUND`。随后只执行任务允许的准备命令：

```bash
pnpm --dir router install --frozen-lockfile --ignore-scripts
```

安装 exit 0，只生成 ignored `router/node_modules`；manifest、lockfile 与 tracked tree 均未
改变。

写入本 result 前，`git diff --check` PASS，`git status --porcelain` 为空。没有修改实现；
本节点唯一 tracked 写入是本 result。result-only commit 后的 commit/tree 与最终 clean 状态由
交付消息记录。

没有运行完整 tooling verdict、stable、live 或 instance；没有派子 Agent，也没有 merge、
rebase 或 push。
