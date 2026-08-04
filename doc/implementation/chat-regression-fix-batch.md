# Chat 回归修复批次（chat-fix-integration）

日期：2026-08-04
状态：integration batch（集成 Agent 调度文档）

## 引用链

- 节点：`chat_regression_fix`；开发叶子任务由 `/root/chat_regression_fix` 交接给
  集成 Agent `/root/chat_fix_integration`，串行合入 `chat-fix-integration`。
- baseline：`ee12eb53`（main HEAD，`git rev-parse` 验证；工作区仅一个无关
  untracked 文件 `doc/architecture/profile-stack-deployment.md`，不在本批次写集）。
- 集成 worktree：`/Users/geek/workspace/skiff-chat-fix-integration`，分支
  `chat-fix-integration`（`git worktree add -b` 自 `ee12eb53` 创建）。

## 批次目标

合并 chat 回归修复批次节点 `chat_regression_fix`，跑受影响 crates 的集成探针，
并完成 worktree/分支清理与批次结束报告。集成 Agent 只处理 import / constructor /
生成索引等不改变行为的机械合并冲突；遇到语义冲突、共享 owner 竞争、基线失效或
任务结论不一致时停止并上报主 Agent `/root`。

## DAG 节点

| 节点 | 职责 | 基线 | 分支 / worktree | commit/tree | 自验收矩阵 | 合并状态 |
| --- | --- | --- | --- | --- | --- | --- |
| chat_regression_fix | chat 回归修复（交接后补全职责与写集） | main@ee12eb53 | TBD / TBD | TBD | 见交接 + 集成探针 | pending |

## 基线 / 集成分支

- Repo：`/Users/geek/workspace/skiff`，基线 main HEAD `ee12eb53`
  （已 `git rev-parse` 验证）。
- 集成分支：`chat-fix-integration`；集成 worktree：
  `/Users/geek/workspace/skiff-chat-fix-integration`（创建时 HEAD 与基线一致）。
- 共享主 worktree `/Users/geek/workspace/skiff` 只读：不切换分支、不 reset、不
  checkout、不覆盖；发现 main 推进时先并入该提交再继续。
- 不 push、不跑完整 gate、不实现功能、不修改设计；最终合并到 main 由主 Agent
  复验后指示。

## 集成探针（本批次唯一 owner）

- 每轮合并后：受影响 crates `cargo check`（runtime-eval / runtime-host /
  runtime-native）+ 对应聚焦测试（actor_submit / task corpus）。
- 不重复开发 Agent 的完整自验收。

## 合并记录表

| 顺序 | 任务 | 分支 | 合并 commit/tree | 集成探针 | 清理 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | chat_regression_fix | TBD | TBD | TBD | TBD | pending |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新
commit/tree、合并任务、探针结果与 worktree 审计清单。
