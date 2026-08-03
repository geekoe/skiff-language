# Task control（durable task dispatch 核心）批次（task-control-integration）

日期：2026-08-03
状态：integration batch（集成 Agent 调度文档）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`。权威设计是最终语义事实源。
- 已确认的正式用户面契约：`doc/reference/dispatch.md`（用户可见关键字 `dispatch`、公开类型
  `std.task.TaskRef`、`std.task.status` / `std.task.cancel` 的 grammar 与拼写）。
- 本批次文档是 task control（durable task dispatch 核心）批次的父节点；叶子任务引用本文件，
  本文件引用权威设计与正式用户面契约。

本批次实现 task control 核心：持久 task 提交、定时可见、claim / lease、at-least-once 恢复、
取消与 terminal settlement 所对应的控制面节点。集成 Agent 只处理 import / constructor / 生成
索引等不改变行为的机械合并冲突；遇到语义冲突、共享 owner 竞争、基线失效或任务结论不一致时
停止并上报主 Agent `/root`。

## 批次目标

按 `doc/architecture/durable-task-dispatch.md` 的目标态实现 durable task dispatch 核心链路的
持久化与控制面节点：

- `task_store`：TaskStore 权威 owner——task identity、状态、`due_at`、attempt、lease、
  取消结果与 terminal outcome 的 durable create / conditional transition / due-time
  visibility / lease fencing / terminal retention。第一个节点，作为共享契约检查点交接。
- `task_scheduler`：scheduler 只从 TaskStore 的可见事实选择工作；负责 timing、fairness、
  capacity、claim 与 Runtime candidate selection，不解释业务 payload。

节点串行合入；集成 Agent 按交接顺序合并并跑便宜集成探针，不重复开发 Agent 的完整自验收。

## DAG 节点

| 节点 | 职责 | 基线 | 分支 / worktree | commit/tree | 自验收矩阵 | 合并状态 |
| --- | --- | --- | --- | --- | --- | --- |
| task_store | TaskStore 权威 owner（共享契约检查点） | main@25e430f5 | task-store | bf9c14a1（tree b300d8d9） | 见交接 + 集成探针 | merged |
| task_scheduler | 调度 / claim / lease / Runtime candidate selection | integration（task_store 合并后） | TBD（待交接） | TBD | 见交接 + 集成探针 | pending |

## 基线 / 集成分支

- Repo：`/Users/geek/workspace/skiff`，基线 main HEAD
  `25e430f5967c704106994e609f281797dbe6c42b`（已 `git rev-parse` 验证；工作区干净）。
- 集成分支：`task-control-integration`；集成 worktree：
  `/Users/geek/workspace/skiff-task-control-integration`。
- 共享主 worktree `/Users/geek/workspace/skiff` 只读：不切换分支、不 reset、不 checkout、
  不覆盖；发现 main 推进时先并入该提交再继续。
- 不 push、不跑完整 gate、不实现功能、不修改设计。

## 集成探针（本批次唯一 owner）

- 新 task-control crate 合并后：该 crate 的 `cargo check` + 单元测试
  （`cargo test -p <新crate名>`，crate 名以交接为准）。
- skiff-router 有依赖改动时：`skiff-router` 的 `cargo check`。
- 不重复开发 Agent 的完整自验收。

## 合并记录表

| 顺序 | 任务 | 分支 | 合并 commit/tree | 集成探针 | 清理 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | task_store | task-store | 71633eab（tree d384310b） | PASS：cargo check -p skiff-task-control；cargo test -p skiff-task-control 19 unit + 3 memory contract 全过（1 ignored Mongo live probe）；cargo check -p skiff-router（仅 1 个预存 unused_variables warning，文件未被本合并改动） | 已清理 | merged |
| 2 | task_scheduler | TBD | TBD | TBD | TBD | pending |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新 commit/tree、
合并任务、探针结果与 worktree 审计清单。

## 停止条件

- 语义冲突、共享 owner 竞争、基线失效或任务结论不一致：停止并报告主 Agent `/root`。
- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测设计。
- 批次结束向主 Agent 报告最终 commit/tree 与证据汇总。
