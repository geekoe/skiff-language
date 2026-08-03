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
| E1 std_task_surface | std.task status/cancel 用户面（error 帧 / router / compiler / runtime 映射） | main@033391ba | std-task-surface / skiff-e1-std-task | 2fab6d66（tree b95016ad） | 见交接 + 集成探针 | merged |
| E2a actor_task_submit | actor-method dispatch 提交侧（ActorActivationSnapshot 冻结 + create recoverable gate） | integration@6e67b216（E1 合并点） | actor-task-submit / skiff-e2a-actor-submit | 947a9075（tree 2f0695c6） | 见交接 + 集成探针 | merged |
| E2b actor_task_execute | actor-method durable target 执行侧（get-or-activate + settlement 映射 + store 扩展） | integration@6f05572c（E2a 合并点） | actor-task-execute / skiff-e2b-actor-exec | f571d0de（tree a9e5c8f2） | 见交接 + 集成探针 | merged |
| E3a e2e_vertical | durable task dispatch E2E 纵向链路 + TaskRef DB stored field 往返 + live 探针注册 | integration@b9614c25（E2 合并点） | e2e-vertical / skiff-e3a-e2e | 578d23c4（tree ac85ab4b） | 见交接 + 集成探针 | merged |
| E3b obs_docs | 观测事件面（router telemetry / SchedulerObservation / host submit events）+ 文档收敛 | integration@b9614c25（E2 合并点） | obs-docs / skiff-e3b-obs-docs | 4bd88f01（tree 7b76fa73） | 见交接 + 集成探针 | merged |
| F0a pattern_match_fix | compiler record-pattern match 降级修复（E3a 记录的已知项 1） | integration@62d8ee99 | 待交接 | 待交接 | 见交接 + 集成探针 | pending |
| F0b actor_submit_context | actor-method dispatch 提交上下文（E2a 记录的 HTTP 内 dispatch 限制） | integration@62d8ee99 | 待交接 | 待交接 | 见交接 + 集成探针 | pending |

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
| 1 | E1 std_task_surface | std-task-surface | 6e67b216（tree 1b37e6b6） | PASS：cargo check transport/router/compiler-core/source/lowering/eval/host/boundary/model/native/native-contract/request-contract/request/capability-context/linked-type-plan/artifact-model/test-runner；task_wire_corpus 11/11、w_model_task_corpus 7/7、router task_control_unit 18/18 + task_repair_direction 6/6 + w_model_task_consumer 4/4、dispatch_grammar 5/5、compiler-source lib 371/371、eval lib 458/458、boundary lib 172/172、host lib 427/427、test_service_flow 16/16；零冲突 | 已清理 | merged |
| 2 | E2a actor_task_submit | actor-task-submit | 6f05572c（tree 2f0695c6） | PASS：cargo check request-contract/capability-context/transport/eval/host/router；transport lib 141/141 + task_wire_corpus 11/11 + w_model_task_corpus 7/7、eval lib 463/463、host lib 428/428、runtime w_model 4/4 + h_task_parent_cut 4/4、router task_control_unit 18/18 + w_model_task_consumer 4/4 + task_repair_direction 6/6 + dispatch_admission_corpus 2/2；零冲突（探针在代码一致的 E2a worktree 温缓存执行，磁盘 100% 满） | 已清理 | merged |
| 3 | E2b actor_task_execute | actor-task-execute | b9614c25（tree a9e5c8f2） | PASS：cargo check task-control/router/transport/eval/host/request-contract/capability-context；task-control 25+3+9、router task_actor_method_execution 10/10 + task_control_unit 18/18、transport lib 141/141 + task_wire_corpus 11/11 + w_model_task_corpus 7/7；零冲突（探针在代码一致的 E2b worktree 温缓存执行） | 已清理 | merged |
| 4 | E3b obs_docs | obs-docs | d5adc522（tree 7b76fa73） | PASS：cargo check task-control/router/transport/eval/host；task-control 25+3+10、router lib 69/69 + task_telemetry 5/5 + task_control_unit 18/18 + task_actor_method_execution 10/10 + w_model_task_consumer 4/4 + task_repair_direction 6/6、host lib 429/429；零冲突（探针在代码一致的 E3b worktree 温缓存执行） | 已清理 | merged |
| 5 | E3a e2e_vertical | e2e-vertical | 3564a217（tree f31568d5） | PASS：cargo check boundary/service-db/router；boundary lib 174/174、service-db lib 145/145（+6 ignored）、router lib 69/69 + task_control_unit 18/18 + task_actor_method_execution 10/10 + dispatch_admission_corpus 2/2；supervisor/mod.rs 自动合并零冲突；verify-live-registry 18/20（两个 loop-risk 失败在 pre-E3a 基线 bd8aa252 复现，既有问题与本节点无关） | 已清理 | merged |
| 6 | F0a pattern_match_fix | 待交接 | 待交接 | 待交接 | 待交接 | pending |
| 7 | F0b actor_submit_context | 待交接 | 待交接 | 待交接 | 待交接 | pending |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新
commit/tree、合并任务、探针结果与 worktree 审计清单。

## 停止条件

- 语义冲突、共享 owner 竞争、基线失效或任务结论不一致：停止并报告主 Agent `/root`。
- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测设计。
- 批次结束向主 Agent 报告最终 commit/tree 与证据汇总。
