# Spawn → Dispatch 全链路改名批次（dispatch-rename-integration）

日期：2026-08-03
状态：integration batch（集成 Agent 调度文档）

## 引用链

权威设计：`doc/architecture/durable-task-dispatch.md`。
用户已确认：关键字改为 `dispatch`、全链路改名、按 `multi-agent-development.md` 执行。
本批次文档是 spawn→dispatch 改名批次的父节点；叶子任务引用本文件，本文件引用权威设计。
权威设计是最终语义事实源；本批次只做全链路机械改名，不修改设计语义。

## 批次目标

把用户可见 `spawn` surface 完整替换为 `dispatch`（语法关键字、公开类型名、运行时/编译器/
router 代码标识与文档），不保留兼容语法、旧 artifact 分支或第二条易失执行路径。

- `rename-code`：代码侧全链路改名（语法/AST、lowering、runtime 控制帧、router 协议与
  dispatcher、测试与 fixture 中的机械重命名），不含文档。
- `rename-docs`：文档侧全链路改名（`doc/reference/spawn.md` 与 doc 下用户可见 `spawn`
  残留替换为 `dispatch`，含引用与链接更新），不含代码。

集成 Agent 只处理 import / constructor / 生成索引等不改变行为的机械合并冲突；遇到语义冲突、
共享 owner 竞争、基线失效或任务结论不一致时停止并上报主 Agent `/root`。

## DAG 节点

| 节点 | 职责 | 基线 | 分支 / worktree | commit/tree | 自验收矩阵 | 合并状态 |
| --- | --- | --- | --- | --- | --- | --- |
| rename-code | 代码侧全链路机械改名 | main@13068249 | 交接后填写 | 交接后填写 | 交接后填写 | pending |
| rename-docs | 文档侧全链路机械改名 | main@13068249 | 交接后填写 | 交接后填写 | 交接后填写 | pending |
| dispatch_reference | reference/dispatch.md 与配套文档语义收敛（纯文档） | integration@438d8056 | 交接后填写 | 交接后填写 | 交接后填写 | pending |

节点可并行；集成 Agent 串行合入 `dispatch-rename-integration`。

## 基线 / 集成分支

- Repo：`/Users/geek/workspace/skiff`，基线 main HEAD
  `13068249715281076d0e0b9134d4a97ec72a36be`（已 `git rev-parse` 验证；工作区干净）。
- 集成分支：`dispatch-rename-integration`；集成 worktree：
  `/Users/geek/workspace/skiff-dispatch-integration`。
- 共享主 worktree `/Users/geek/workspace/skiff` 只读：不切换分支、不 reset、不 checkout、
  不覆盖；发现 main 推进时先并入该提交再继续。
- 不 push、不跑完整 gate、不实现功能、不修改设计。

## 集成探针（本批次唯一 owner）

- code 合并后：受影响 crates 的 `cargo check`；一次相关聚焦测试
  （skiff-syntax / skiff-runtime-transport / skiff-router 的测试 target）。
- docs 合并后：`rg` 检查 `doc/reference` 与 `doc/architecture` 里用户可见 `spawn` 残留；
  检查引用 `spawn.md` 的断链。
- 不重复开发 Agent 的完整自验收。

## 合并记录表

| 顺序 | 任务 | 分支 | 合并 commit/tree | 集成探针 | 清理 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | rename-docs | rename-dispatch-docs | 438d8056 | PASS（见上） | 已清理 | merged |
| 2 | rename-code | 交接后填写 | 交接后填写 | 交接后填写 | 交接后填写 | pending |
| 3 | dispatch_reference | 交接后填写 | 交接后填写 | 交接后填写 | 交接后填写 | pending |

每次合并成功后立即删除已合并的一级 worktree 与临时分支，并向主 Agent 报告新 commit/tree、
合并任务、探针结果与 worktree 审计清单。

## 停止条件

- 语义冲突、共享 owner 竞争、基线失效或任务结论不一致：停止并报告主 Agent `/root`。
- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测设计。
- 批次结束向主 Agent 报告最终 commit/tree 与证据汇总。
