# Leaf Task: 文档全链路 `spawn` → `dispatch` 改名（用户可见 surface 与交叉链接）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`。它规定用户可见关键字固定为
  `dispatch`，旧 `spawn` surface 被 `dispatch` 完整取代；本任务只改文档，不改代码语义。
- 直接父节点：批次 `dispatch-rename`（集成 Agent `/root/dispatch_integration`）。
  父文档 `doc/implementation/dispatch-rename-batch.md` 尚未创建；按任务信封标注批次名
  `dispatch-rename`。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`13068249715281076d0e0b9134d4a97ec72a36be`（`git rev-parse` 已验证；
  main HEAD，工作区干净）。
- worktree：`/Users/geek/workspace/skiff-rename-docs`，branch `rename-dispatch-docs`。
- 集成 Agent：`/root/dispatch_integration`。本任务不 merge、不 push、不写共享集成分支；
  共享主 worktree 只读。

## 任务合同（任务信封摘要）

1. `doc/reference/spawn.md` → `doc/reference/dispatch.md`：正文用户可见关键词
   spawn→dispatch（保留当前描述语义），标题改为 “Skiff Dispatch Reference”，文件开头
   加定位说明：“本文描述当前实现的 dispatch 语句；目标态 durable task dispatch 契约
   以 ../architecture/durable-task-dispatch.md 为准。”
2. 更新所有引用 `spawn.md` 的交叉链接为 `dispatch.md`：`doc/README.md`、
   `doc/overview.md`、`doc/architecture/recoverable-value.md`、根 `README.md`。
3. 更新 live 文档中把 spawn 当用户可见 surface 的表述：
   - `doc/reference/{runtime,interface,static-semantics,observability,db,testing,any-interface,any-interface-value}.md`；
   - `doc/architecture/actor-model.md`（含 router 控制面 `get` / `dispatch` / owner 路由）、
     `runtime-deployment-topology.md`、`router-rust.md`（derived function-spawn correlation
     → derived task correlation），以及其余 architecture 文档中的用户可见 spawn 表述
     （recoverable-value、any-interface-value、actor-shared-heap-design、
     package-service-contract-deployment、runtime-layered-crate-architecture、
     tail-call-execution、test-runner-runtime-isolation、open-issues、queue）。
4. `doc/architecture/durable-task-dispatch.md` 的旧 `spawn` 历史表述一律不动。
5. `doc/implementation/**` 是历史迁移记录，一律不改（本叶子文件为新增任务合同，不修改
   既有记录）。
6. `doc/reference/queue.md` 本次只做机械关键词替换（用户可见 spawn→dispatch），不重写
   语义；其与权威文档的矛盾（自动重试、lease expiry、build pinning）在交接报告中列出，
   属于后续阶段收敛范围。

## 分类原则（用户可见 vs 内部 / 历史）

改名范围是“把 spawn 当用户可见 surface”的表述，包括 `spawn` 语句/关键字、
`spawned call`、`direct-spawn`、`Actor spawn`、`spawn payload`、`spawn target`、
`spawn submit`、`spawn 载荷`、`spawn ops` 等派生形式，统一改为 dispatch 形式
（dispatch 语句 / dispatched call 等）。

以下表述保持原文并在交接报告中列出：`tokio::spawn`、`SpawnPayload` 内部类型名、
`spawn.submit` wire 标识、verify-task-runner 的进程 spawn、actor-shared-heap 的
spawned task、runtime-layered 的 spawn worker / internal spawn invocation branch /
spawn submit receipt / spawn 专用 frame、`any-interface-value.md` 划线历史条款、
根 `TASK.md` 历史任务记录、`doc/implementation/**`、`durable-task-dispatch.md`
历史表述。

## 禁止

- 不改任何代码。
- 不改 `doc/implementation/**` 既有文件。
- 不改 `doc/architecture/durable-task-dispatch.md`。
- 不改 `doc/reference/queue.md` 语义。
- 不 push、不写共享集成分支、不动共享主 worktree。

## 自验收矩阵

`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`

- 任务条款 1：dispatch.md 标题/定位句/正文全 dispatch | `doc/reference/dispatch.md` |
  `rg -n spawn doc/reference/dispatch.md` 为空 | `git diff --check`
- 任务条款 2：无 spawn.md 断链 | `rg -n "spawn\.md"`（live 文档为空，仅历史记录残留）
  | `rg -n "dispatch\.md" doc README.md` | 同上
- 任务条款 3：live 文档用户可见 spawn 无残留 | `rg -n -i spawn doc/reference doc/architecture`
  仅剩内部/历史白名单 | 同上 | 同上
- 任务条款 4/5/6：durable-task-dispatch.md、doc/implementation、queue.md 语义未改 |
  `git show HEAD:...` 对比 | `git diff --stat` | `git diff --check`

## 交接

完成后把 branch、worktree 路径、commit/tree、实际写集和自验收矩阵直接报告给
`/root/dispatch_integration`，并通知主 Agent `/root`。
