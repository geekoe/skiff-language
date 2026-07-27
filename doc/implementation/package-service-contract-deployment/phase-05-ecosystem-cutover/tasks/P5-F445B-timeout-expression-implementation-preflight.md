# P5-F445B Timeout expression implementation preflight

状态：Ready。只读、有界预检；不实现。

## 直接父节点

- `P5-F444C-agine-service-terminal-connect-only-cutover-result.md`

从该 result 沿引用读取当前 `doc/reference/{syntax,runtime,static-semantics,std-surface}.md` 及必要架构。
文档已经拥有 `timeout` 语义；本预检不能用 WebSocket RPC私有参数、service轮询或120秒deployment
timeout偷换15秒局部deadline。

## 输入

| Repo | Root / commit |
| --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` / `c81266f3` |
| Internals F444C draft | stash commit `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420`（只读） |

输入 worktree必须 clean；stash不得 apply/pop/drop。

## 要回答的问题

1. 盘点 reference已经承诺的完整语法与语义：
   duration token、statement、`timeout(...) value`、canonical modifier组合、作用域、类型、effect、
   effective deadline、`TimeoutError`、ancestor cancel、nested timeout和cleanup。
2. 逐层定位缺口：
   lexer/parser/AST/visitor、source IR/lowering/type/effect、compiled IR/emission/artifact schema、
   runtime eval/frame/deadline/cancellation、host/service/WebSocket operation deadline传播。
3. 审计现有 request deadline、`concurrent value`、value block、cancel checkpoint和
   `runtime_websocket_jsonrpc`机制，明确哪些可复用，哪些生产 owner缺失。
4. 判定是否可以在一个有界实现leaf完整兑现当前reference。若需DAG，按互斥写集与显式依赖拆分；
   不能只实现F444C能parse的happy path，也不能让纯CPU、nested timeout或value typing静默错误。
5. 给出独立于Agine的最小RED/GREEN fixture矩阵：
   normal completion、value、timeout throw/catch、nested earliest deadline、request deadline更早、
   ancestor cancel不可捕获、host/remote call传播、pure loop取消、cleanup/late result、非法duration和
   concurrent surface拒绝。
6. 列出artifact/schema/identity/fixture/receipt是否变化，以及F444C恢复时应使用的精确source spelling。

## 允许读取/运行

- Skiff syntax、compiler、artifact-model、runtime相关production/test；
- 当前reference/architecture和F444C stash中三项调用的只读片段；
- 只运行最小test listing或无写入的现有聚焦测试。

不得修改文件，不得运行完整gate、stable/live/network，不得让预检顺手实现。

## 输出

只新增并提交：

`P5-F445B-timeout-expression-implementation-preflight-result.md`

结论：

- `PREFLIGHT_COMPLETE / TASK_EXECUTABLE`
- `TASK_SCOPE_EXPANDED`
- `DESIGN_DECISION_REQUIRED`

必须给出精确DAG、owner/写集、先后依赖、测试矩阵和F444C解除条件。20分钟有界；无法完整兑现reference时
停止上报，不提交半语义建议。不得派子 Agent、merge/rebase/push。

worktree：

`/Users/geek/workspace/skiff-p5-f445b-timeout-preflight`

branch：

`codex/p5-f445b-timeout-preflight`
