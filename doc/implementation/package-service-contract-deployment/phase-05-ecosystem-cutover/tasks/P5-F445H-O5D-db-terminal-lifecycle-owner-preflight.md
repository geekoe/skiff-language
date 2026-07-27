# P5-F445H-O5D DB terminal lifecycle owner preflight

状态：Ready。O6 命中 transaction/lease waiter-drop强制停止条件后的只读前置审计。本节点必须冻结
最小 lifecycle ownership seam、取消/terminal真值表、请求收束owner与后继实现DAG；不修改
production或tests。

## 直接父节点

- `P5-F445H-O6-evaluator-db-state-machines-result.md`

审计起点为 Skiff integration `a5920a32`，并包含 O5C hermetic baseline
`98add404`。父结果已经证明：

- 普通DB、`DbQuery`和lease read可在O6写集内实现；
- transaction commit/abort会在provider terminal前从request state移走session；
- lease release在provider wait后才更新request state；
- capability接口只有可被调用方drop的borrowed async future；
- evaluator局部bool/phase不能让已经drop的terminal future继续完成；
- 裸 `tokio::spawn`、阻塞Drop、future重建或自然TTL都不能证明 exactly-once收束。

本任务文件完整描述本节点需求。只读取直接相关代码与既有owner模式，不从更高层设计增加新语言、
DB、Actor、timeout或wire需求。

## 审计问题

### 1. 冻结真实资源所有权图

逐层列出并精确引用：

1. `DbCapabilityContext`、`DbCapabilityStore`、`DbCapabilityStoreApi` 的request/store所有权；
2. 所有production与test `DbCapabilityStoreApi` implementor；
3. `ServiceDbCapabilityHandle`、`ServiceDbStore`、`DbRequestState`、`DbTransactionState`；
4. transaction begin/body/commit/abort时session在哪个owner中；
5. lease claim/renew/release/lost时hold、renew task、provider side effect和request-state entry在哪个
   owner中；
6. request/eval/runtime最终释放各capability context的位置，以及是否已有可await的request
   cleanup/join checkpoint；
7. repository中已有的structured cleanup模式：request lifecycle、stream cleanup、
   outbound lease、provider unary、Actor lease等；明确哪些可复用、哪些因async terminal或
   first-poll语义不能复用。

不得只搜索类型名后推断。必须沿至少一条真实request构造→eval→request completion路径确认
`DbCapabilityContext`的生命周期和最终收束点。

### 2. 冻结 cancellation 与 terminal 真值表

至少覆盖以下阶段：

- begin尚未启动、begin first-Ready、begin Pending、begin成功；
- body运行、body error、body cancel/drop；
- commit尚未选择、commit first-Ready、commit Pending、commit provider已执行但ack未返回、
  commit error；
- abort first-Ready、abort Pending、abort error/drop；
- claim尚未成功、claim成功；
- renew idle/awaiting/result/failed；
- body normal/error/cancel/drop；
- release尚未启动、first-Ready、Pending、provider已执行但request-state尚未更新、error。

对每格给出：

- transaction/lease的唯一terminal owner；
- evaluator可见结果；
- waiter drop后原future继续、转abort/release或无需动作；
- operation是否允许被重建；
- request completion是否必须等待ack；
- Actor `await_if_pending`看到的第一次poll必须是什么。

尤其必须回答一个潜在语义冲突：

- evaluator在commit intent已被选择、原commit future已经真实Pending后被取消，是否仍要求
  “切换为abort”，还是必须让同一个commit继续terminal并把请求取消只作用于等待者；
- provider已经执行release但ack/request-state更新尚未完成时，如何避免重放又保证本地状态收束。

如果现有权威设计/代码不能唯一决定该真值表，并且两种选择有不同用户可观察结果，状态必须是
`DECISION_REQUIRED`，给出具体选项、后果和推荐；不得把产品语义伪装成内部实现选择。

### 3. 证明 preserving-first-poll 的可行协议

审查并选择一种具体协议，至少与下列性质等价：

1. evaluator首次poll时直接poll原provider future，因此provider同步Ready不经过channel/task调度
   伪装成Pending；
2. provider future第一次真实Pending后仍是同一个future，不drop/rebuild；
3. waiter存在时由waiter驱动；waiter drop时可把同一个 pinned owned future转交给结构化owner；
4. structure owner有明确注册、join/ack与request teardown位置，不是裸spawn；
5. body阶段drop尚未创建commit时，lifecycle owner能创建并驱动唯一abort/release；
6. terminal被选择后使用CAS/phase所有权，commit/abort或release不能双启动；
7. late terminal不能重新写已结束的evaluator heap/env；
8. provider永不返回时的请求收束策略来自既有request deadline/cancellation合同；不得在本节点
   私自发明timeout。

可以审查 `Pin<Box<dyn Future + Send + 'static>>` 的 poll-once后移交、request-owned supervisor、
DB-local worker、guard/receipt等方向，但必须给出Rust所有权可行性与精确owner。排除方向也要说明
原因。

如果没有既有request收束点、而新增generic supervisor会跨越未分配owner，必须明确拆出独立节点；
不得把所有工作压给O6。

### 4. API 与实现DAG

输出足够让后继Agent无需重新做架构选择的接口草图。至少冻结：

- capability-context新增/替换的owned transaction/lease lifecycle类型；
- prepared terminal wait、receipt、guard或supervisor注册接口；
- trait object的`Send + 'static`、one-shot与drop合同；
- concrete service-db如何在terminal完成前保持session/hold可达；
- request completion如何join/ack；
- fake store如何观察start/poll/terminal/drop；
- O6R最终只需消费哪些接口。

列出所有必须迁移的production implementor/call site和test fixture；语言未发布，不保留旧borrowed
API兼容层，除非同一提交内仍有明确的非evaluator生产调用者需要薄组合。

把后继拆成最小互斥DAG。每个节点必须写明：

- 直接前置；
- 独占production/test写集；
- test-first RED；
- focused GREEN命令与实际应非零的selector；
- exactly-once/Ready/Pending/drop验收；
- 停止条件；
- 哪个节点负责删除旧接口；
- 哪个节点负责combined acceptance；
- O6R何时可重发。

如果 capability seam、service-db provider和request lifecycle可以由一个单owner安全实现，可以给
一个节点；若写集/责任明显不同，应拆分，不能为少写任务文档而制造巨型节点。

### 5. 影响面与非目标

必须明确检查：

- raw DB operations在transaction中持有`&mut session`跨await的现状是否也需要新owner；
- O5R2 prepared runtime waits如何继续使用同一个transaction/session；
- file cascade、retention roots、lease-lost与Mongo error顺序；
- request cancellation、timeout、runtime disconnect和future drop是否共享同一teardown；
- transaction nested/illegal flow与heap checkpoint现有语义；
- tests中fake store是否需要模拟never-ready和waiter-drop；
-真实Mongo live测试仍保持ignored，后继不得依赖网络。

非目标：

- 不实现普通O6 evaluator状态机；
- 不改变DB语言、transaction原子性、lease TTL、错误类型或catch语义；
- 不修改Actor E3、timeout E1/I6、service/package/http/websocket设计；
- 不引入语言级yield、unsafe heap alias或historical compatibility。

## 交付与判定

只新增：

- `P5-F445H-O5D-db-terminal-lifecycle-owner-preflight-result.md`

result必须是以下之一：

- `READY_FOR_IMPLEMENTATION_DAG`：owner、真值表、接口、write set、DAG和测试合同全部唯一；
- `DECISION_REQUIRED`：只剩明确的用户可观察语义选择，列出推荐；
- `TASK_SCOPE_EXPANDED`：仍有多个未知owner或需要超出预期的大范围架构重做。

不得以“需要更多研究”结束；要给精确证据和下一步。

只读审计默认不运行Cargo。允许运行`rg`、读取source/tests、`git`只读命令和
`git diff --check`；不得修改production/tests、运行stable/live/network/Mongo、merge/rebase/push。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5d-lifecycle-preflight
branch   codex/p5-f445h-o5d-lifecycle-preflight
```

只提交result，最终worktree clean。本节点不派子Agent。若审计发现范围超出或依然有多个不明确
问题，按上述状态如实结束，不自行实现。
