# Verify Task Runner（验证任务调度器）

状态：authoritative design。本文是 verify plan 中 `phase` 概念改造为 `task`、并引入并行调度、
隔离与失败独立性改造的唯一权威设计文档。与本文冲突的既有文档段落（`scripts/README.md`、
`AGENTS.md`、`doc/architecture/test-runner-runtime-isolation.md` 中关于 verify phase 的描述）
以本文为准，并由文档同步任务更新。

本文不涉及以下既有概念，它们保持原名和语义不变：

- `test-runner` 的 `HttpPhase`（HTTP 请求子阶段，用于 deadline 归因）；
- `.test.skiff` case 执行生命周期中的 setup/body/finalization 阶段；
- `check-compiler-boundaries.mjs` 规则元数据里的 compiler pipeline `phase` 字段；
- Skiff 语言 runtime 内的其他阶段概念。

## 1. 背景与目标

当前 `scripts/verify.mjs` 把 selector 展开为扁平 `phases` 列表并串行 fail-fast 执行
（`scripts/lib/verify-runner.mjs`）。目标：

1. 把 verify plan 的执行单元 `phase` 改名为 `task`，消除与流水线阶段语义的混淆。
2. 引入并行调度：单个并发参数 `--jobs <n>`，默认 1（串行，最小并发）；每个 task 声明自己
   占用的并发槽位数 `slots`（默认 1）。
3. task 之间相互独立：任一 task 的失败（blocked、preflight 失败、命令失败、spawn 失败）
   只计入该 task 的结果，不阻止其他 task 启动或继续。
4. task 之间相互隔离：进程隔离、输出隔离、以及“mutating task 读公共仓库、写私有目录”。

## 2. 非目标

- 不引入 `--fail-fast` 或全局中止语义（除用户中断外）。
- 不引入 `dependsOn` / DAG 依赖；plan 顺序只作为调度顺序，当前 36 个 task 无真实依赖。
- 不引入每 task 独立 `CARGO_TARGET_DIR`；共享 target 目录继续由 cargo 自身锁仲裁。
- 不引入文件系统监控来强制 readonly/hermetic 声明（v1 以声明 + 测试为契约）。
- 不引入容器、overlayfs、sandbox-exec 等 OS 级沙箱。
- 不引入第二个并发参数（如内存预算）；`--jobs` 是唯一并发参数。
- 不 push；合并与清理按仓库规则本地完成。

## 3. Task 契约

`plan.tasks` 中每个 task 是以下形状（未列字段继承现状）：

```js
{
  id: 'implementation:compiler:rust',   // 全局唯一，现状保留
  kind: 'implementation:compiler',      // 现状保留
  cwd: '/abs/repo/root',                // 现状保留
  command: 'cargo',                     // executable task 保留
  args: ['test', '--no-fail-fast', ...],
  displayArgs: undefined,               // 现状保留
  executionPreflight: undefined,        // 现状保留（函数）
  preconditionError: undefined,         // blocked task 保留
  tier: undefined,                      // registry 元数据保留
  ownership: undefined,                 // registry 元数据保留

  slots: 1,                             // 新增，正整数，默认 1
  exclusive: false,                     // 新增，布尔，默认 false
  mutation: undefined,                  // 新增，见 §6
}
```

命名替换范围：`phase` → `task`、`phases` → `tasks`、`phaseId` → `taskId`、
`phaseBuilders` → `taskBuilders`、`assertRegistryPhaseMetadata(phase)` →
`assertRegistryTaskMetadata(task)`，以及所有测试、CLI 输出（`phases: N` → `tasks: N`）、
help 文本和文档中的对应引用。

## 4. 并发调度

### 4.1 参数

唯一并发参数为 CLI `--jobs <n>`，默认 1，最小 1。不新增环境变量。

### 4.2 规则

- 总预算 `budget = jobs`。
- 运行中 task 的 `slots` 之和必须 `<= budget`。
- `exclusive: true` 的 task 只有在运行集为空时才能启动，启动后独占整个 budget；
  `tier === 'live/manual'` 的 registry task 强制视为 exclusive。
- blocked task（有 `preconditionError`）不消耗槽位、不执行，直接记录 blocked 结果。
- 调度按 plan 顺序派发；budget 不足时 task 等待，不抢占、不调整顺序。
- `executionPreflight` 在 task 获得调度资格后、spawn 前执行；失败则记录 failed 且不 spawn。
- 计划构建（selector 展开、catalog 校验、integrity 校验）仍在任何 task 启动前完成。

## 5. 失败独立性与结果汇总

- 每个 task 独立结算：`passed | failed | blocked | interrupted`。
- 所有非 blocked task 最终都会被尝试启动（在并发限制和计划顺序内），不因其他 task 失败而取消。
- 用户中断（SIGINT/SIGTERM）是唯一全局中止：停止派发、终止所有在飞 task 的进程组、
  在飞 task 记为 interrupted，已完成的 task 保留结果。
- 全部结束后输出汇总：按 plan 顺序列出每个 task 的结果；任一 failed/blocked/interrupted
  则进程退出码为 1，否则为 0。
- 每个 task 的输出（stdout/stderr）完整捕获，作为连续块输出，不允许与并行 task 交错；
  输出排序由实现选择（完成时打印或结束后按 plan 顺序打印），汇总必须按 plan 顺序。

## 6. 隔离

### 6.1 分类

- `mutation` 为 `undefined`：只读或 hermetic task。允许写 OS 临时目录、租约端口、以及
  工具自管理的共享构建目录（`build/`、`target/`、`node_modules` 缓存，cargo/pnpm 自带锁）。
  调度器不做额外约束。当前默认计划中的全部 task 都属于这一类。
- `mutation.paths` 非空：mutating task，写仓库状态（源码树、`.skiff-instance`、`var/` 等）。
  - 必须同时 `exclusive: true`（integrity 校验强制）；
  - runner 在 `<repo>/var/verify/tasks/<sanitized-task-id>/` 创建私有根；
  - `mutation.redirect` 是 `{ envVar: repoRelativePath }` 映射，路径必须属于 `mutation.paths`；
    runner 为每个映射创建私有根下对应相对路径的目录，并把该 env var 设为私有绝对路径；
  - runner 同时注入 `SKIFF_VERIFY_TASK_PRIVATE_ROOT=<privateRoot>`；
  - task 的约定：读公共仓库（通过 `cwd`/自身解析的 root），写只经过私有根；
  - task 结束后 runner 删除私有根。

### 6.2 命令执行 API

`scripts/lib/owned-command.mjs`（或 `command-execution.mjs`）新增一个“captured owned command”
变体：子进程 stdio 走管道、支持 AbortSignal、按进程组终止，返回
`{ code, signal, error, stdout, stderr }` 而不是抛错。这是本次改造唯一新增的 production
执行 API；现有 `runAttachedCommand` / `runOwnedCommand` / `captureAttachedCommand` 保持可用。

### 6.3 校验

`assertPlanIntegrity` 新增：

- `slots` 为正整数；
- `exclusive` 为布尔；
- `mutation` 形状合法：`paths` 为非空仓库相对路径数组（不允许绝对路径、`..`），
  `redirect` 的 key 为非空 env var 名，value 属于 `paths`；
- 有 `mutation` 的 task 必须 `exclusive: true`。

## 7. CLI

```text
node scripts/verify.mjs [--only <selectors>] [--jobs <n>] [--list]
```

- `--jobs <n>`：并发槽位预算，默认 1，最小 1。
- `--list`：展开计划但不执行；输出 `tasks: N` 及每个 task 的 id/kind/cwd/command。
- help 文本删除“Execution is fail-fast”描述，改为“运行全部选中 task 并汇总所有失败”。

## 8. 测试要求

### 8.1 重写

`scripts/tests/verify.test.mjs` 中锁定旧语义的测试必须改写为任务级语义：

- 全局 blocker 聚合后阻止所有命令启动；
- fail-fast：第一个失败后停止后续 task；
- 缺 executable 阻止后续 task。

### 8.2 新增

- `--jobs 1` 串行、`--jobs N` 并发上限为 N（fixture task 用文件记录 start/end）；
- `slots` 生效：jobs=2 时一个 slots=2 的 task 单独运行；
- `exclusive` task 独占调度；
- 失败 task、blocked task、preflight 失败 task 都不阻止其他 task 执行；
- 中断（AbortController 路径）终止在飞 task 并汇总 interrupted；
- mutating task 的 redirect：fixture task 通过 redirect env var 写入私有根，
  公共仓库不被写入；
- integrity 校验负例：非法 slots/exclusive/mutation。

### 8.3 重命名同步

`verify-taxonomy.test.mjs`、`verify-live-registry.test.mjs`、`verify-rust-quality.test.mjs`、
`verify-live-plan-platform-source.test.mjs`、`verify-live-plan-runtime-generation.test.mjs`
等测试中的 `phase` 引用同步改为 `task`。

## 9. 文档同步

以下文件由文档任务更新（不改变本节外语义）：

- `scripts/README.md`：verify 段落改为 task 语义、`--jobs`、失败独立、隔离契约；
- `AGENTS.md`：verify 段落同步；
- `doc/architecture/test-runner-runtime-isolation.md`：verify plan phase 引用改为 task；
- `scripts/lib/verify-cli.mjs` 的 help 文本（属核心代码任务）。

## 10. 完成标准

- 全仓库 `rg` 中，verify plan 语境不再有 `phase` 残留（仅 §0 列出的既有概念保留）。
- `node scripts/verify.mjs --list` 输出 `tasks: N` 且正常展开。
- 新增/改写测试全部通过；受影响的 `scripts/tests/verify*.test.mjs` 通过
  `node --test` 聚焦运行。
- 默认 `--jobs 1` 行为串行；`--jobs N` 在 fixture 测试中满足槽位与独占约束。
- 默认 verify 不包含 mutating task；mutating 机制有 fixture 测试证明读公共/写私有。
- 合入 `main` 前经过独立验收；不 push。

## 11. 变更范围与 owner

- 核心代码：`scripts/lib/verify-plan.mjs`、`verify-runner.mjs`、`verify-cli.mjs`、
  `verify-live-plan.mjs`、`verify-live-registry.mjs`、`verify-live-catalog.mjs`、
  `verify-selector-graph.mjs`、`verify-rust-subjects.mjs`、`verify-checkers.mjs`、
  `owned-command.mjs`/`command-execution.mjs`、`scripts/verify.mjs` 及
  `scripts/tests/verify*.test.mjs`。
- 文档：`scripts/README.md`、`AGENTS.md`、`doc/architecture/test-runner-runtime-isolation.md`。
- 禁止修改：`scripts/check-rust-file-lines.mjs`、其他 checker 语义、`test-runner/`、
  `doc/reference/testing.md`（case 生命周期概念）。
