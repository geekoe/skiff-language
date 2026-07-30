# P5-F445H Eval and concurrency owner preflight

状态：Ready。F445B-I5 只读 implementation preflight。

## 直接父节点

- `P5-F445B-timeout-expression-implementation-preflight-result.md`
- `P5-F445F-scoped-execution-control-checkpoint-result.md`
- `P5-F445G-timeout-artifact-lowering-link-checkpoint-result.md`

固定 production 输入：

`/Users/geek/workspace/skiff-phase-05-integration` @ `d5812c27`

## 目标

不写代码，审计 I5 是否可作为一个 bounded implementation leaf，或必须拆成多个互斥写集节点。
必须从当前 linked IR、eval、heap/env、stream/catch/checkpoint 和 I4 scope API 反向证明实现路径，
不能只复述 F445B。

至少回答：

1. linked `StmtIr::Timeout`、`ExprIr::{Timeout,ValueBlock,ConcurrentValue}` 和
   `ConcurrentPlanIr` 进入 evaluator 的所有 exhaustive dispatch 点；
2. current `ProgramExecution` / eval context 在哪里保存、克隆和恢复
   `OwnedExecutionControl`，进入 timeout 时如何安装 derived scope guard；
3. normal、throw、local timeout、inherited deadline、ancestor cancel 的现有 carrier 与 catch
   projection，哪些路径必须改；
4. function entry、loop condition、backedge、lane start/end、tail 前、长生成片段的当前
   checkpoints 与缺口；
5. `Env`、slot layout、`RequestHeap` 的 clone/import 能力，lane-local execution 如何只在 normal
   completion 导入 sibling-visible const，而不泄漏 late value/error/mutation；
6. plan dependencies 是否已经包含 runtime 调度所需的全部信息；若没有，精确指出缺失字段及应
   回退 I3 还是可由既有 slot/body metadata无歧义消费；
7. 如何在不共享 `&mut Env` / `&mut RequestHeap` 的前提下实现真实 async overlap、DAG ready
   queue、source-order deterministic error winner、outer deadline优先、tail lane；
8. stream `break`/`return`/timeout/cancel 的 source cancellation owner与 bounded cleanup；
9. invocation-time scope contract 应由 eval 暴露在哪个现有 context，供 I6 host/native读取；
10. T05–T12 每项的最小 hermetic fixture、fake clock/barrier seam与精确 owner。

## 判定

输出必须是以下之一：

- `PREFLIGHT_COMPLETE / TASK_EXECUTABLE`：给出一个 leaf 的精确写集、实现顺序、测试矩阵和退出
  条件；
- `TASK_SCOPE_EXPANDED`：给出最小 DAG、互斥写集、依赖和每个节点退出条件；
- `DESIGN_DECISION_REQUIRED`：只在当前权威合同确实无法唯一决定行为时使用，并把需要用户回答
  的问题压缩为最少。

若 IR 缺失导致 runtime 必须重新推导 source semantics，必须停止并明确回退 owner，不得建议在
eval 猜测。

## 输出与边界

只新增并提交：

`P5-F445H-eval-concurrency-owner-preflight-result.md`

不得修改 production、test、golden 或其它文档；不得派子 Agent、merge/rebase/push、
stable/live/network。最终 clean。

## worktree

`/Users/geek/workspace/skiff-p5-f445h-eval-preflight`

branch：

`codex/p5-f445h-eval-preflight`

base：`d5812c27`，再 cherry-pick 本任务文档。
