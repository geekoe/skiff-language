# P5-F445H-E23 Concurrency branches combined acceptance

状态：Ready。F445H-E4 之前的高风险独立合流验收；只审查已经集成的 E1/E2/E3，不修改
production。

## 直接父节点

- `P5-F445H-E1-eval-scope-terminal-checkpoint-core-result.md`
- `P5-F445H-E2-lane-local-DAG-scheduler-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

父节点已沿引用链连接唯一权威设计。本任务不得重新解释 timeout、concurrent、错误 owner 或
Actor 语义。

## 当前检查点

审查候选固定为 Skiff integration `95bbf884`：

- E1：current owned execution control、精确 scope terminal、统一 checkpoint 与 E4
  fail-closed bridge；
- E2：lane-local env/heap/scope、linked DAG scheduler、确定性 winner、错误与 tail handoff；
- E3：每 lane 独立 Actor continuation frame、actual-Pending suspend 与 outer gate；
- evaluator 四个 bridge arm 尚未接线，因此现阶段只能验收组合接口和既有行为，不能声称
  F445H 已完成。

## 审查目标

独立检查以下组合合同：

1. E2 的每个 lane current scope 必须是独立 child scope；instruction/request budget仍委托
   parent，outer cancel/deadline可见，lane cancel不得反向取消 parent；
2. baseline、dependency import、winner error materialization、tail result handoff在任何资源失败
   下都不污染 parent heap/env；
3. scheduler 在 winner 后先取消 running child scope，再 drop future；不会启动新 lane或接纳
   late value/error；
4. E3 child frame与 E2 lane一一对应时不共享 lease slot；同步 Actor segment继续串行，真实
   Pending外部 future可以重叠；
5. Actor child complete/abandon/drop与 scheduler normal/error/cancel/drop能够一一收束；outer
   只能在全部 child关闭后恢复，不会泄漏 scheduler guard、scope lease、waiter或 timer；
6. E1 current scope/clock在 E2/E3 seam中没有被 request-root control替换；E4 可以只做 evaluator
   接线，不需要新增公共 API或复制 scheduler/Actor状态机；
7. 四个 E4 bridge arm仍明确 fail closed；不得出现顺序 fallback、wildcard、占位 normal value
   或旧 `maySuspend` 分支重新进入新增模块。

若发现 production defect，记录最小复现、owner、影响和建议修正写集，结果写
`REJECTED`；不得直接修 production。若 E4 必须新增公共 capability/error/IR 设计，记录
`TASK_SCOPE_EXPANDED`，不得自行扩展。

## 验证

使用本 worktree 独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e23-combined/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e23-combined/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e23-combined/build/cargo-target \
  cargo fmt --check
git diff --check
```

完整 eval suite须记录实际 unit/integration/doc test 数和所有失败分类；零测试 filter不算证据。
同时做反向搜索，确认 E2/E3 新增 production没有 `maySuspend`/preemptive suspend、四个 E4
fail-closed arm仍在。

不运行 stable、live、network 或其它仓库测试。

## 写集与交付

只允许新增：

- `P5-F445H-E23-concurrency-branches-combined-result.md`

不得修改 production、既有测试、父结果或其它文档。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e23-combined
branch   codex/p5-f445h-e23-combined
```

结果必须给出 `ACCEPTED`、`REJECTED` 或 `TASK_SCOPE_EXPANDED`，逐条映射七项组合合同，并记录
命令、实际测试数、候选 commit与审查过的关键文件。最终 clean；不得 merge/rebase/push。

这是一个有界独立验收节点，不应派子 Agent。
