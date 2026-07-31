# Docs Task：Verify Task Runner 文档同步

状态：leaf task（文档开发 Agent，一次性有界任务）。

目标能力：按权威设计把 verify plan 的 `phase` 语义改为 `task`（含 `--jobs`、失败独立、
隔离契约），不触碰其他概念。

## 权威

- 唯一语义事实源：`doc/architecture/verify-task-runner.md`（baseline main HEAD
  `4e8a15e0` 已提交，提交信息 `docs: freeze verify task runner design`）。
- 保留概念（不得改动）：test-runner 的 `HttpPhase`、`.test.skiff` case 的
  setup/body/finalization 阶段、check-compiler-boundaries.mjs 的 compiler pipeline
  `phase` 字段、Skiff 语言 runtime 内其他阶段概念。

## 写入范围

仅以下文件（外加本任务文件）：

- `scripts/README.md`：verify 段落改为 task 语义（`--jobs` 默认 1、运行全部并汇总失败、
  隔离契约）；只改 verify 相关段落，不重写其他内容。
- `AGENTS.md`：verify 相关段落同步（phase→task、`--jobs`、失败独立语义）。
- `doc/architecture/test-runner-runtime-isolation.md`：只把 verify plan 语境下的 phase
  引用改为 task（包括 live registry/verify 段落）；保留该文档其他内容。

禁止：

- `scripts/` 代码、`scripts/tests/` 测试、`scripts/lib/`、`scripts/verify.mjs`
  （help 文本属核心 Agent）；
- `doc/reference/testing.md`；
- 上述保留概念；
- `scripts/check-rust-file-lines.mjs` 或其他无关文件。

## 具体修改点

### scripts/README.md（Canonical Live Verification Registry 段）

- `interprets the registry into prerequisites and phases` → `... and tasks`；
- `checked again before the first phase` → `... first task`；
- `Generated phases receive` → `Generated tasks receive`；
- 补充：`--jobs <n>` 是唯一并发参数、默认 1（串行）；运行全部选中 task 并汇总所有失败；
  task 相互独立，失败只计入自身；隔离契约（mutating task 读公共仓库、写私有根），
  当前默认计划无 mutating task。

### AGENTS.md

- 跨语言 verify 入口段补充 `--jobs <n>`（唯一并发参数、默认 1）与失败独立/汇总语义；
- `生成的 phase 只收到绝对 --config 路径` → `生成的 task ...`；
- `canonical runtime live phase 固定启用` → `canonical runtime live task 固定启用`。

### doc/architecture/test-runner-runtime-isolation.md（Live Verification Ownership 段）

- `generated phase` → `generated task`；`phase 才包含` → `task 才包含`；
- `live phase 的 fixed id/idPrefix` → `live task ...`；
- `并生成 phase` → `并生成 task`；`普通 phase builder` → `普通 task builder`；
- `所有 live/manual phase` → `所有 live/manual task`；`phase 只传` → `task 只传`。

## 验证

```bash
rg -n "phase" scripts/README.md AGENTS.md doc/architecture/test-runner-runtime-isolation.md
```

剩余 phase 引用只允许属于保留概念（HttpPhase、case 生命周期、compiler stage 等），
verify plan 语境无残留。不跑 node 测试，不执行 verify.mjs。

## 提交与报告

单次 commit 包含本任务文件与三份文档修改。提交后把 branch、worktree 路径、commit/tree、
实际写集与证据报告给集成 Agent `/root/verify_task_runner_integration`，并通知主 Agent
`/root`。
