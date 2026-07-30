# P5-F420G5 Verify plan single command owner result

状态：Completed。Package-store discovery command 已收敛为 checker registry 中的单一
canonical 声明，并由 `scripts-dev-sync` / `implementation:tooling` 唯一执行；default verify、
tooling 和 checks-only plan 均通过 integrity。

## 1. Exact candidate 与 implementation checkpoint

- integrated repair start / tree：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59` /
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- task checkout / tree：
  `65efc72a08896549c6d5f1c6abb5b6fedb5b2a22` /
  `197f6fc0165d77a968b56578846168942a026bd8`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation commit / tree：
  `36fa8490f51c78c7ab007940deebaff3f0bb36c3` /
  `feb4276ab4258a83f6a3769fa56383cfee7c5cd2`。

启动时已确认 integrated repair start 是 task checkout 的 ancestor，且其 tree 精确匹配 batch
冻结值。implementation 只修改两个授权 production 文件；本结果由后续独立 result-only commit
承载。

## 2. 单一 owner 收敛

`scripts/lib/verify-checkers.mjs` 现在是
`node scripts/check-package-store-discovery.mjs` 的唯一 literal command 事实源：

- checker classification 继续是 `default verify`；
- invocation id 为 `implementation:tooling:dev-sync-fixture`；
- invocation selector 为 `scripts-dev-sync`。

`scripts/lib/verify-plan.mjs` 的 `scripts-dev-sync` builder 不再直接复制 command，而是消费
`checkerPhases(root, 'scripts-dev-sync', { kind: 'implementation:tooling' })`。因此该 phase 保持
既有 id、kind、cwd 和命令形态，同时经
`tests -> implementation-tests -> tooling -> scripts -> scripts-dev-sync` 进入 default verify。

旧的 `checks:package-store-discovery` invocation 和 phase builder 中的 direct command 均已删除。
`checks-default` 不再展开该 checker；`assertPlanIntegrity` 的 duplicate id/execution gate 保持零
diff，selector graph、live registry 和 tests 也保持零 diff。

## 3. Plan 证据

| selector | plan integrity | phase 数 | exact execution 数 |
| --- | --- | ---: | ---: |
| default `verify` | PASS | 282 | 1 |
| `tooling` | PASS | 57 | 1 |
| `checks` | PASS | 16 | 0 |

这里的 exact execution 指 command/args 精确为
`node scripts/check-package-store-discovery.mjs`。default plan 还会按通用 syntax owner 列出
`node --check scripts/check-package-store-discovery.mjs`；它的参数不同，不是 package-store
discovery workload，也不会触发 duplicate execution gate。

反向搜索：

```bash
rg -n \
  "check-package-store-discovery\\.mjs|implementation:tooling:dev-sync-fixture|checks:package-store-discovery" \
  scripts/lib/verify-checkers.mjs scripts/lib/verify-plan.mjs
```

只命中 checker registry 中的 path 与新 invocation；旧 checks id 和 plan builder literal 均为
零。

## 4. 聚焦验证

| gate | 结果 |
| --- | --- |
| 四文件 `node --test` | 66/66 PASS |
| `node scripts/verify.mjs --list` | PASS；exact execution 1 |
| `node scripts/verify.mjs --only tooling --list` | PASS；exact execution 1 |
| `node scripts/verify.mjs --only checks --list` | PASS；exact execution 0 |
| `git diff --check` | PASS |

首次四文件运行因 worktree 尚未安装 `router/package.json` 声明的 `ws` 而在两个 loop-risk fixture
precondition 处停止；执行 `pnpm --dir router install --frozen-lockfile` 后，同一命令 66/66
通过。安装只生成 ignored `router/node_modules/`，lockfile 和 tracked tree 均无变化。

## 5. 自验收矩阵与边界

| 任务条款 | 代码 / 证据 | 结果 |
| --- | --- | --- |
| checker registry 为事实源 | registry 持有唯一 literal command，builder 只消费 `checkerPhases` | PASS |
| 归属 `scripts-dev-sync` / `implementation:tooling` | invocation selector 与生成 phase kind 精确匹配 | PASS |
| default/tooling 一次，checks-only 零次 | 三个 CLI list 均通过 integrity，exact count 为 1 / 1 / 0 | PASS |
| 不放宽 duplicate gate | `assertPlanIntegrity` 零 diff；duplicate execution test 通过 | PASS |
| 不改 tests 或 live/default 边界 | tests、selector graph、live registry 全部零 diff | PASS |
| 严格唯一写入 | implementation 仅两个授权文件；另加本 result | PASS |

没有运行完整 tooling、Router、test-runner Rust suite、`run-skiff-tests`、stable 或 live；没有
merge、rebase 或 push，也没有修改 scanner、tests、lockfile、其它 production 域或生态仓库。
