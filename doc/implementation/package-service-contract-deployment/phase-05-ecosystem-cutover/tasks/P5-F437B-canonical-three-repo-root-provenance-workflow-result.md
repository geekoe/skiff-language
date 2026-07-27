# P5-F437B Canonical three-repo root/provenance workflow result

状态：`PASS / CHECKPOINT_COMPLETE / TASK_SCOPE_NOT_EXPANDED`。

Internals canonical workflow 现已在任何 authoring 前 fail closed 地验证 Internals、Skiff 与
skiff-packages 三个 exact checkout，并把三仓 root/commit/tree 及完整 coordinate→source root
映射写入 list output 和 workflow receipt。六个 dependency package 与四个 service 的顺序已冻结为
任务指定顺序；本 leaf 未运行 canonical authoring、assembly、stable 或 live。

## 1. Exact inputs 与提交

| repo | 起始 input | 起始 tree | 本 leaf 结果 |
| --- | --- | --- | --- |
| Internals | `066b5135a8e06f87acfd614e408e05b35453f4eb` | `23be114f0d4b838eff1c7b214a40fc9c57cdd354` | implementation commit `faa11b188c570ca763f107ddd829d52b8fe8861f`，tree `140d3a03851b64d513fd97c5860e713b8fc314de` |
| Skiff toolchain | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` | production/test 只读；仅新增本文 |
| skiff-packages | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` | 全程只读 |

Skiff result worktree 实际起始 HEAD 是
`f74404fbd466e96005a750fbb5b4ccae165cc401`（tree
`4902066652382289d1282536ff6be0885b2cd7a0`）。`64a0ab4e..f74404fb` 的差异只包含
Phase 05 task 文档；没有 Skiff production、test 或 toolchain 差异，因此仍精确消费任务冻结的
toolchain 代码状态。

`TASK_SCOPE_EXPANDED = NO`。没有修改 package/service source、manifest、package scripts、公共 CLI、
skiff-packages、Skiff production/test 或任何 stable/local config。

## 2. 实现证据

- 新增 Internals `scripts/canonical-source-provenance.mjs`，作为唯一 root set、canonical
  coordinate/order 与 provenance owner。
- `prepare-canonical-assembly.mjs` 的 exported workflow 函数显式接收
  `internalsRoot`、`skiffRoot`、`skiffPackagesRoot`；三个 executable 入口与 Agine receipt
  consumer 都要求绝对 `SKIFF_ROOT`、`SKIFF_PACKAGES_ROOT`，不再猜 sibling 或 stable/main。
- validation 在创建 temporary artifact root 和执行任何 authoring 命令前完成：三个 root 必须存在、
  是 canonical absolute path、等于各自 git top-level、可读 HEAD commit/tree 且彼此不同。
  每个 source directory 必须位于声明的 official root 内，并由 `package.yml`（service 另含
  `service.yml`）声明 expected coordinate。
- package 顺序精确为 Skiff std → Internals llm-api → llm-providers → agent →
  skiff-packages http-session → track；service 顺序保持 Codex Relay → AIHub → Agine → Account，
  最后仍只构建一个四-root assembly。
- workflow receipt 顶层 `provenance` 同时记录三仓 root/commit/tree 与 package/service mappings；
  `--list --fixture-only` 输出同一 metadata、通过环境变量显式传入的Skiff与skiff-packages两个仓库根目录、
  完整 6+4 顺序和确定性 command plan。
- 原有单一 temporary ecosystem store、std bootstrap owner、typed record/pointer authoring、
  partial receipt rejection、signal cleanup 与 linked-worktree mutation guard 保持不变。

## 3. 自验收矩阵

| 合同条款 | 代码/测试证据 | 结果 |
| --- | --- | --- |
| 三 root 显式输入；env omission/non-absolute fail closed | `canonicalRootsFromEnvironment`；direct tests 覆盖两个 env omission 与 relative `SKIFF_ROOT` | PASS |
| 三 git top-level、commit/tree、distinct root | `resolveCanonicalSourceProvenance`；direct tests 覆盖 missing、duplicate、nonexistent、wrong top-level 与 provenance fields | PASS |
| exact coordinate/root mapping | canonical definitions + 双 manifest 校验；direct test 篡改 `http-session` coordinate | PASS |
| exact 6 package / 4 service order | `canonicalSourceDefinitions`、`assertCanonicalSourceOrder`；plan 与 direct order assertions | PASS |
| receipt/list provenance | workflow receipt `provenance`；真实 list 输出三仓 provenance 和全部 mappings | PASS |
| partial package/service receipts fail closed | `assertCompleteBuildReceipts` 的 package 与 service direct assertions | PASS |
| 无 legacy/stable source path | command invariant tests + executable reverse search | PASS |
| cleanup / mutation guard 保持 | 原有 success/failure cleanup tests 均通过；未改 mutation guard | PASS |

## 4. 聚焦验证

以下任务指定命令均在 Internals implementation tree 上通过：

```text
node --test scripts/prepare-canonical-assembly.test.mjs \
  scripts/isolated-service-graph.test.mjs \
  scripts/test-isolated-service.test.mjs
=> 16 passed, 0 failed

SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f437b-canonical-roots \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-p5-f437b-canonical-roots \
  node scripts/prepare-canonical-assembly.mjs --list --fixture-only
=> exit 0；三仓 top-level/commit/tree、6 package、4 service、11-step plan 全部输出

node --check scripts/prepare-canonical-assembly.mjs
node --check scripts/check-isolated-service-graph.mjs
node --check scripts/test-isolated-service.mjs
node --check agine/service/test-isolated-service-receipt.mjs
git diff --check
=> 全部 exit 0
```

真实 list 读取到：

| repo | commit | tree |
| --- | --- | --- |
| Internals implementation | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` |
| Skiff result checkout | `f74404fbd466e96005a750fbb5b4ccae165cc401` | `4902066652382289d1282536ff6be0885b2cd7a0` |
| skiff-packages | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

list 只枚举并验证 source/provenance；没有执行 publish 或 assembly。

## 5. 反向搜索与边界

对以下四个 executable/consumer 文件执行反向搜索：

```text
scripts/prepare-canonical-assembly.mjs
scripts/check-isolated-service-graph.mjs
scripts/test-isolated-service.mjs
agine/service/test-isolated-service-receipt.mjs
```

`join(dirname(internalsRoot), 'skiff...')`、`SKIFF_ROOT ??`、
`SKIFF_PACKAGES_ROOT ??`、`.skiff-instance`、`--packages-dir` 和
`--service-artifact-root` 均为零匹配。正向搜索确认每个
`runCanonicalFixtureWorkflow` / `canonicalFixtureInputs` production caller 都传递
`skiffPackagesRoot`。

交付复核时三个 worktree 均 clean：Internals implementation 已提交，Skiff 只有本文 result
提交，skiff-packages 未修改。没有 merge、rebase、push，也没有承接后继 combined。
