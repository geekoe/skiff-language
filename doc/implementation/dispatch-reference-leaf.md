# Leaf Task: dispatch 正式参考文档落地与配套文档收敛

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`。它是唯一语义事实源；
  本任务只把用户已确认的正式参考文档与配套文档收敛到该契约，不改代码语义。
- 直接父节点：批次 `dispatch-rename`（集成 Agent `/root/dispatch_integration`）。
  父文档 `doc/implementation/dispatch-rename-batch.md` 已在 baseline 存在并引用权威设计。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`。
- baseline：`438d8056a021080a71a36c802e8ee5526c319de9`（rename-docs 批次合入后；
  `doc/reference/spawn.md` 已变为 `doc/reference/dispatch.md`，live 文档用户可见 spawn
  已机械改为 dispatch）。
- worktree：`/Users/geek/workspace/skiff-reference-dispatch`，branch `dispatch-reference`。
- 集成 Agent：`/root/dispatch_integration`。本任务不 merge、不 push、不写共享集成分支；
  共享主 worktree 只读。

## 任务合同（任务信封摘要）

最终参考文档内容为 `/Users/geek/workspace/dispatch-reference-draft.md`（主 Agent 已定稿：
求值顺序按草案、concurrent v1 不支持故无单独规则、kind 拼写按草案），状态为“正式参考文档”。

1. 用草案全文替换 `doc/reference/dispatch.md`（当前是 rename-docs 机械改名版），链接改为
   仓库相对路径：权威文档 `../architecture/durable-task-dispatch.md`、
   recoverable `../architecture/recoverable-value.md`；保留“状态：正式参考文档”与全文内容。
2. `doc/reference/syntax.md` §6：expression statement 最外层表达式枚举增加
   dispatch 表达式（“……或 dispatch 表达式”）。
3. `doc/reference/static-semantics.md`：
   - §9 保留名：加一句 `dispatch` 是保留关键字（该节已声明关键字不能作为用户标识符）。
   - §18 recoverable boundary：核对 `dispatch` target 参数已机械改名；新增
     `std.task.TaskRef` 属于可恢复类型，可进入 DB stored field / persistent payload。
4. `doc/reference/runtime.md`：
   - 尾调用段：`dispatch` 表述收敛为“提交持久 detached task”；`emit` 表述保留。
   - pending 容量段：改为 leased task attempt 对应的 active request 计入；
     scheduled / ready backlog 不计入任何 Runtime connection。
   - provider 能力列表核对机械改名即可；concurrent 相关列表（v1 不支持）保持不动。
5. `doc/reference/queue.md`：按权威文档收敛语义矛盾：lease expiry 后 recovery 产生新
   attempt（at-least-once），不是 terminal failure；execution image 冻结提交时的 build，
   不是到期按 service version 解析 current build；不得提供业务 dedupe key；第一版不向
   业务源码暴露 queue（Queue Exposure Boundary）；未来 work queue 与 event stream 必须
   分开设计。保留 queue 作为平台底层调度机制的描述。
6. `doc/architecture/actor-model.md`：核对 router 控制面 `get` / `dispatch` / owner 路由；
   第 129 行改为 durable activation state 丢失（operator 删除 / 数据丢失）时 entry 丢失并
   用 `get` 重建；普通 router 重启不触发该路径。
7. `doc/architecture/runtime-deployment-topology.md`：核对 actor/dispatch 及其它跨 request
   control frame 等表述已机械收敛，无旧 spawn / direct-spawn 残留。
8. 禁止：改任何代码；改 `doc/implementation/**` 既有文件；改
   `doc/architecture/durable-task-dispatch.md` 的历史“旧 spawn”表述；push。

## 禁止

- 不改任何代码（本任务全部是文档）。
- 不改 `doc/implementation/**` 既有文件（本叶子文件为新增任务合同，不修改既有记录）。
- 不改 `doc/architecture/durable-task-dispatch.md`。
- 不 push、不写共享集成分支、不动共享主 worktree。

## 自验收矩阵

`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`

- 条款 1：dispatch.md 全文为定稿草案（仅链接改仓库相对路径）| `git diff --stat`、
  `git show HEAD:doc/reference/dispatch.md` 与草案 diff 仅链接两处 | `rg -n "skiff/doc/architecture" doc/reference/dispatch.md` 为空 | `git diff --check`
- 条款 2：syntax.md §6 枚举含 dispatch 表达式 | `rg -n "dispatch 表达式" doc/reference/syntax.md` | 无旧枚举残留 | 同上
- 条款 3：static-semantics §9 含 “`dispatch` 是保留关键字”；§18 含 TaskRef 可恢复 | `rg -n "dispatch.*保留|TaskRef" doc/reference/static-semantics.md` | `rg -n "spawn" doc/reference/static-semantics.md` 为空 | 同上
- 条款 4：runtime.md 尾调用与 pending 容量收敛；concurrent 列表未动 | `rg -n "持久 detached task|leased task attempt|scheduled / ready backlog" doc/reference/runtime.md` | `git diff` 确认 264/493 行未改 | 同上
- 条款 5：queue.md 无 lease expiry→terminal failure / current build / dedupe key 等矛盾条款，含 Queue Exposure Boundary 与 work queue/event stream 分离 | `rg -n "Queue Exposure Boundary|work queue|dedupe|current build|recovery" doc/reference/queue.md` | `rg -n "收敛为 terminal failure|解析当前 build" doc/reference/queue.md` 为空 | 同上
- 条款 6：actor-model.md 129 行新表述；控制面 `get` / `dispatch` / owner 路由 | `rg -n "durable activation state|dispatch.*owner 路由" doc/architecture/actor-model.md` | 无旧“router 重启后 entry 丢失”表述 | 同上
- 条款 7：topology 无 spawn / direct-spawn 用户面残留 | `rg -n "actor/dispatch|direct-spawn|actor/spawn" doc/architecture/runtime-deployment-topology.md` | 同上 | 同上
- 全局：live 文档（doc/reference、doc/architecture，排除 doc/implementation）旧易失
  spawn/direct-spawn 用户面表述与断链清理 | `rg -n "spawn|direct-spawn" doc/reference doc/architecture` 仅剩历史/内部白名单 | `rg -n "spawn.md" doc` 无 live 断链 | `git diff --check`
- 零代码改动 | `git diff --stat` 仅文档 | `git status --short` 无代码文件 | 同上

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_integration`，并通知主 Agent `/root`。
