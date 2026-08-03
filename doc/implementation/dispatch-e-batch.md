# Dispatch 阶段 E（用户面收尾）批次（dispatch-e-integration）

日期：2026-08-04
状态：integration batch（集成 Agent 调度文档）

## 引用链

权威设计：`doc/architecture/durable-task-dispatch.md`。
正式用户面契约：`doc/reference/dispatch.md`。
本批次文档是 dispatch 接入阶段 E 的父节点；开发叶子任务直接交接给集成 Agent
（`/root/std_task_surface` → `/root/actor_task_target` → `/root/e2e_observability`；
以实际派发为准），集成 Agent 串行合入 `dispatch-e-integration`。权威设计是最终语义
事实源；本批次只做各节点实现接入，不修改设计语义。

## 批次目标

dispatch 用户面收尾：

- `E1 std_task_surface`：标准库 task surface。
- `E2 actor_task_target`：actor task target 接入。
- `E3 e2e_observability`：端到端可观测性。

节点按序串行合并；集成 Agent 只处理 import / constructor / 生成索引等不改变行为的
机械合并冲突；遇到语义冲突、共享 owner 竞争、基线失效或任务结论不一致时停止并上报
主 Agent `/root`。

## DAG 节点

| 节点 | 职责 | 基线 | 分支 / worktree | commit/tree | 自验收矩阵 | 合并状态 |
| --- | --- | --- | --- | --- | --- | --- |
| E1 std_task_surface | 标准库 task surface | main@033391ba（待核对） | 待交接 | 待交接 | 见交接 + 集成探针 | pending |
| E2 actor_task_target | actor task target 接入 | integration@E1 合并点（待核对） | 待交接 | 待交接 | 见交接 + 集成探针 | pending |
| E3 e2e_observability | 端到端可观测性 | integration@E2 合并点（待核对） | 待交接 | 待交接 | 见交接 + 集成探针 | pending |

节点串行；集成 Agent 每轮核对 branch/worktree/commit/tree/写集后合并。

## 基线 / 集成分支

- Repo：`/Users/geek/workspace/skiff`，基线 main HEAD
  `033391ba`（已 `git rev-parse` 验证；工作区干净）。
- 集成分支：`dispatch-e-integration`；集成 worktree：
  `/Users/geek/workspace/skiff-e-integration`（创建时 `git worktree add -b`，HEAD
  与基线一致）。
- 共享主 worktree `/Users/geek/workspace/skiff` 只读：不切换分支、不 reset、不
  checkout、不覆盖；发现 main 推进时先并入该提交再继续。
- 不 push、不跑完整 gate、不实现功能、不修改设计。

## 集成探针（本批次唯一 owner）

- 每轮合并后：受影响 crates 的 `cargo check` + 对应聚焦测试
  （transport/task-control/router/compiler/eval/host 等按节点写集）。
- 不重复开发 Agent 的完整自验收。

## 合并记录表

| 顺序 | 任务 | 分支 | 合并 commit/tree | 集成探针 | 清理 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | E1 std_task_surface | 待交接 | 待交接 | 待交接 | 待交接 | pending |
| 2 | E2 actor_task_target | 待交接 | 待交接 | 待交接 | 待交接 | pending |
| 3 | E3 e2e_observability | 待交接 | 待交接 | 待交接 | 待交接 | pending |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新
commit/tree、合并任务、探针结果与 worktree 审计清单。

## 停止条件

- 语义冲突、共享 owner 竞争、基线失效或任务结论不一致：停止并报告主 Agent `/root`。
- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测设计。
- 批次结束向主 Agent 报告最终 commit/tree 与证据汇总。
