# core-task: verify plan phase → task runner（并行调度 / 失败独立 / mutating 隔离）

状态：Ready。

## 直接父节点

- `/Users/geek/workspace/skiff/doc/architecture/verify-task-runner.md`
  （authoritative design，baseline `4e8a15e0`，唯一语义事实源）

## DAG 位置

- 集成分支：`codex/verify-task-runner`（集成 Agent `/root/verify_task_runner_integration`
  维护，本任务不写）。
- 本任务分支：`codex/verify-task-runner-core`，worktree
  `/Users/geek/workspace/skiff-verify-task-runner-core`。
- 兄弟任务：文档 Agent（owner：`scripts/README.md`、`AGENTS.md`、
  `doc/architecture/test-runner-runtime-isolation.md`）。
- 本任务是一次性核心开发节点，完成后不承接新任务。

## 写入范围

核心代码：

```text
scripts/verify.mjs
scripts/lib/verify-plan.mjs
scripts/lib/verify-runner.mjs
scripts/lib/verify-cli.mjs
scripts/lib/verify-live-plan.mjs
scripts/lib/verify-live-registry.mjs
scripts/lib/verify-live-catalog.mjs
scripts/lib/verify-selector-graph.mjs
scripts/lib/verify-rust-subjects.mjs
scripts/lib/verify-checkers.mjs
scripts/lib/owned-command.mjs
```

机械闭合（同一调用链上因果直接相关，必须记录）：

```text
scripts/lib/command-execution-ledger.mjs   # 为新增 captureOwnedCommand 登记 spawn owner
scripts/tests/command-execution-policy.test.mjs  # ledger 计数 11→12、spawn 9→10
scripts/tests/command-execution.test.mjs   # captureOwnedCommand 行为测试
```

测试：

```text
scripts/tests/verify.test.mjs
scripts/tests/verify-taxonomy.test.mjs
scripts/tests/verify-live-registry.test.mjs
scripts/tests/verify-rust-quality.test.mjs
scripts/tests/verify-live-plan-platform-source.test.mjs
scripts/tests/verify-live-plan-runtime-generation.test.mjs
scripts/tests/verify-task-runner.test.mjs   # 新增：调度/中断/隔离/integrity 负例
```

本文件。

## 非目标

- 不引入 `--fail-fast` / 全局中止（用户中断除外）、`dependsOn`、每 task 独立
  `CARGO_TARGET_DIR`、文件系统监控、OS 级沙箱、第二个并发参数；不 push。
- 不改变 live registry 语义：tier/ownership/kind 校验、`kind === tier` 不变。

## 禁止修改的并行表面

- `scripts/README.md`、`AGENTS.md`、`doc/architecture/test-runner-runtime-isolation.md`
  （文档 Agent owner）。
- `scripts/check-rust-file-lines.mjs`、`test-runner/` 下任何文件、
  `doc/reference/testing.md`。
- 其他 checker 语义：`scripts/check-command-execution-policy.mjs`、scanner、
  policy 校验规则本身不修改；ledger 仅追加新 spawn owner 登记。
- `verify-discovery.mjs`（无 phase 引用，不需要改）、`package.json`、
  `.github/workflows/verify.yml`、`scripts/package.json`。

## 架构级机械决策（设计未逐字覆盖；均为直接推导，不改变设计语义）

- `slots > jobs`：无法满足“运行中 slots 之和 <= budget”，`runVerifyPlan` 在派发前整体
  报错（计划/预算契约无效），不进入任务级结算。
- `tier === 'live/manual'` 强制 exclusive 在调度器层实现（effective exclusive）；
  integrity 只按设计 §6.3 校验 mutation→exclusive，不额外强制。
- 输出：每个 task 完成时打印连续块（stdout 后 stderr，不与并行 task 交错），汇总按
  plan 顺序；`--list` 输出 `tasks: N`。
- 私有根命名：task id 中 `[^A-Za-z0-9_-]+` 替换为 `-`、去除首尾 `-`、空则 `task`；
  `<repo>/var/verify/tasks/<sanitized-id>/`，task 结束后删除（`var/` 已 gitignore）。
- 私有根清理失败：`console.error` 警告，不改写该 task 结果。
- 新增 production 执行 API 为 `owned-command.mjs` 的 `captureOwnedCommand`（stdio 管道 +
  AbortSignal + 进程组 TERM/KILL + 返回 outcome 不抛错），复用 `childCompletion`；
  新增 spawn 调用点按仓库既有 ledger 机制登记。

## 完成标准

- 写入范围内（含机械闭合文件）`rg -i phase` 无 verify plan 语境残留（`HttpPhase`、
  compiler pipeline `phase`、case 生命周期等既有概念不属于本范围）。
- `node scripts/verify.mjs --list` 输出 `tasks: N` 且正常展开。
- 聚焦测试全部通过；`--jobs 1` 串行、`--jobs N` 满足槽位与独占约束。
- 默认 verify 不包含 mutating task；mutating 机制有 fixture 测试证明读公共/写私有。
- 合入 `main` 前经过独立验收；不 push。

## 聚焦验证命令

```bash
node --test \
  scripts/tests/verify.test.mjs \
  scripts/tests/verify-taxonomy.test.mjs \
  scripts/tests/verify-live-registry.test.mjs \
  scripts/tests/verify-rust-quality.test.mjs \
  scripts/tests/verify-live-plan-platform-source.test.mjs \
  scripts/tests/verify-live-plan-runtime-generation.test.mjs \
  scripts/tests/verify-task-runner.test.mjs
node --test scripts/tests/command-execution.test.mjs \
  scripts/tests/command-execution-policy.test.mjs
node scripts/verify.mjs --only tooling --list
git diff --check
```

## 证据 owner

- 本文件与自验收矩阵：核心开发 Agent `/root/verify_task_runner_core`。
- 实现提交后向 `/root/verify_task_runner_integration` 报告 branch/worktree/commit/tree/
  实际写集/自验收矩阵，并通知 `/root`。

## 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| 全部 rename：phase→task、phases→tasks、phaseId→taskId、phaseBuilders→taskBuilders、assertRegistryPhaseMetadata→assertRegistryTaskMetadata；CLI `phases: N`→`tasks: N` | `verify-plan.mjs`（taskBuilders/assertPlanIntegrity(tasks)）、`verify-runner.mjs`（tasks: N）、`verify-live-plan.mjs`（liveSelectorTasks/assertRegistryTaskMetadata）、`verify-selector-graph.mjs`（assertOrdinaryTaskBuilderCoverage）、`verify-rust-subjects.mjs`（taskId）、`verify-checkers.mjs`（checkerTasks）、`verify-cli.mjs`（help 文案）及全部测试 | `rg 'liveSelectorPhases\|assertRegistryPhaseMetadata\|checkerPhases\|assertOrdinaryPhaseBuilderCoverage\|phaseBuilders\|phaseId' scripts scripts/tests` 零命中；`rg '\bphase\b\|\bphases\b'` 在写集 verify 文件零命中 | `node --test scripts/tests/verify*.test.mjs`；`node scripts/verify.mjs --only rust-quality --list` 输出 `tasks: 2` |
| 新字段 slots（正整数默认 1）/exclusive（布尔默认 false）/mutation（{paths,redirect}，有 mutation 必须 exclusive）；assertPlanIntegrity 校验 | `verify-plan.mjs` `assertTaskSchedulingMetadata`/`assertMutationShape`（slots/exclusive/mutation 形状、绝对路径/`..`/redirect key/value/独占强制） | 无旧字段语义残留 | `verify-task-runner.test.mjs`“assertPlanIntegrity rejects…”；`verify.test.mjs` integrity 用例 |
| 调度：`--jobs` 默认 1 最小 1；运行 slots 之和 <= budget；exclusive 仅运行集空时启动且独占；tier=live/manual 强制 exclusive；blocked 不占槽直接记结果；按 plan 顺序派发 | `verify-cli.mjs`（parseJobsValue）、`verify-runner.mjs`（dispatch 循环/usedSlots/hasExclusiveRunning/blocked inline） | 无 fail-fast/全局聚合残留 | `verify-task-runner.test.mjs` tests 1–6 |
| 失败独立：每 task 结算 passed/failed/blocked/interrupted；不因其他失败取消；汇总按 plan 顺序；任一非 passed 退出码 1 | `verify-runner.mjs`（settleOutcome/printSummary）、`verify.mjs`（exitCode 聚合） | 旧“全局 blocker 聚合”“fail-fast”“缺 executable 阻止后续”测试已改写为任务级断言 | `verify.test.mjs`、`verify-live-registry.test.mjs` 聚焦运行 |
| 中断：SIGINT/SIGTERM 停止派发、终止在飞进程组、在飞记 interrupted、已完成保留 | `verify.mjs`（AbortController 接线）、`verify-runner.mjs`（abort 分支 + 未派发任务记 interrupted）、`owned-command.mjs`（abort→进程组 TERM/KILL） | — | `verify-task-runner.test.mjs`“abort stops dispatch…”（含孙进程 marker 不写入证明） |
| mutating 隔离：`var/verify/tasks/<sanitized-id>/`、redirect env + SKIFF_VERIFY_TASK_PRIVATE_ROOT、结束后删除私有根；默认计划无 mutating | `verify-runner.mjs` `createTaskPrivateState`/`removeTaskPrivateRoot`/`sanitizeTaskId` | 默认 36 task 均无 mutation 字段 | `verify-task-runner.test.mjs`“mutating tasks write…”（公共路径不被写入、私有根删除） |
| captured owned command：stdio 管道 + AbortSignal + 进程组终止 + 返回 outcome 不抛错；既有 API 保持可用 | `owned-command.mjs` `captureOwnedCommand`；ledger 新增 `owned-captured-process-group` 登记（机械闭合） | `command-execution.mjs` 未改；`runAttachedCommand/runOwnedCommand/captureAttachedCommand` 签名不变 | `node --test scripts/tests/command-execution.test.mjs` |
| CLI：`[--jobs <n>]`、help 删除 fail-fast、--list 输出 tasks: N | `verify-cli.mjs` usage/options/help 文案 | 无第二个并发参数、无并发 env var | `verify-task-runner.test.mjs`“verify CLI parses --jobs…” |

备注：`scripts/tests/command-execution-policy.test.mjs` 的“actual production ledger…”在
baseline `4e8a15e0` 即失败（`scripts/check-rust-file-lines.mjs` 未登记的 `execFileSync`
导入，cc57de4d 引入；该文件属禁止修改表面）。本任务未触碰该文件，ledger 自身校验通过。
