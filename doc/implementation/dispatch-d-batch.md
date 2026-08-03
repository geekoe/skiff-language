# Dispatch 接入阶段 D 批次（dispatch-d-integration）

日期：2026-08-03
状态：integration batch（集成 Agent 调度文档）

## 引用链

权威设计：`doc/architecture/durable-task-dispatch.md`。
正式用户面契约：`doc/reference/dispatch.md`。
本批次文档是 dispatch 接入阶段 D 的父节点；开发叶子任务直接交接给集成 Agent
（`/root/dispatch_wire` → `/root/router_control` → `/root/dispatch_compiler_runtime`；
实际派发含独立 D3 子节点 `/root/dispatch_grammar`），
集成 Agent 串行合入 `dispatch-d-integration`。权威设计是最终语义事实源；本批次只做
各节点实现接入，不修改设计语义。

## 批次目标

把 durable task dispatch 接入链路落地：

- `D1 dispatch_wire`：dispatch wire / control 契约检查点（首个节点，共享
  wire/control 契约）。
- `D2 router control`：router control 接入。
- `D3 dispatch_grammar`：dispatch 表达式语法 / compiler 接入（D3/D4 的语法与
  compiler 侧）。
- `D4 dispatch_compiler_runtime`：runtime 接入（待派发）。

节点按序串行合并；集成 Agent 只处理 import / constructor / 生成索引等不改变行为的
机械合并冲突；遇到语义冲突、共享 owner 竞争、基线失效或任务结论不一致时停止并上报
主 Agent `/root`。

## DAG 节点

| 节点 | 职责 | 基线 | 分支 / worktree | commit/tree | 自验收矩阵 | 合并状态 |
| --- | --- | --- | --- | --- | --- | --- |
| D1 dispatch_wire | dispatch wire / control 契约检查点 | main@16c177a0 | dispatch-wire | 18b0da77（tree 1b34cd17） | 见交接 + 集成探针 | merged |
| D2 router_control | router control 接入 | integration@e5df67f8（D1 合并点） | router-control | 3a0c138c（tree ad85274b） | 见交接 + 集成探针 | merged |
| D3 dispatch_grammar | dispatch 表达式语法 / compiler 接入 | main@16c177a0 | dispatch-grammar | 91a1322e（tree 60481ec5） | 见交接 + 集成探针 | merged |
| D4 dispatch_compiler_runtime | runtime 接入 | 待交接确认 | 待开发 Agent 交接 | 待交接 | 见交接 + 集成探针 | pending |

节点串行；集成 Agent 每轮核对 branch/worktree/commit/tree/写集后合并。

## 基线 / 集成分支

- Repo：`/Users/geek/workspace/skiff`，基线 main HEAD
  `16c177a099a3927eb9b89ce0afd61e419ad91ff7`（已 `git rev-parse` 验证；工作区干净）。
- 集成分支：`dispatch-d-integration`；集成 worktree：
  `/Users/geek/workspace/skiff-d-integration`（创建时 `git worktree add -b`，HEAD
  与基线一致）。
- 共享主 worktree `/Users/geek/workspace/skiff` 只读：不切换分支、不 reset、不
  checkout、不覆盖；发现 main 推进时先并入该提交再继续。
- 不 push、不跑完整 gate、不实现功能、不修改设计。

## 集成探针（本批次唯一 owner）

- 每轮合并后：受影响 crates 的 `cargo check`（至少 `skiff-runtime-transport` /
  `skiff-task-control` / `skiff-router`）+ 对应聚焦测试（task wire corpus 等）。
- 不重复开发 Agent 的完整自验收。

## 合并记录表

| 顺序 | 任务 | 分支 | 合并 commit/tree | 集成探针 | 清理 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | D1 dispatch_wire | dispatch-wire | e5df67f8（tree 68280832） | PASS：cargo check skiff-runtime-transport/skiff-task-control/skiff-router；task_wire_corpus 10/10、w_model_task_corpus 7/7、actor_task_router 7/7、task_repair_acceptance 3/3、task_repair_direction 6/6；零冲突 | 已清理 | merged |
| 2 | D2 router_control | router-control | 400078ac（tree d28f9b9a） | PASS：cargo check transport/task-control/router/syntax/compiler-*；task_wire_corpus 10/10、w_model_task_corpus 7/7、task_control_unit 14/14、skiff-task-control 25+3+9；零冲突 | 已清理 | merged |
| 3 | D3 dispatch_grammar | dispatch-grammar | 90fc4a3b（tree 2c7c70b6） | PASS（与 D2 合并后同一轮探针）；dispatch_grammar 3/3；零冲突 | 已清理 | merged |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新
commit/tree、合并任务、探针结果与 worktree 审计清单。

## 停止条件

- 语义冲突、共享 owner 竞争、基线失效或任务结论不一致：停止并报告主 Agent `/root`。
- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测设计。
- 批次结束向主 Agent 报告最终 commit/tree 与证据汇总。
