# P5-F445H-I6R Host/native current-scope refresh preflight

状态：Ready。I6 的旧预检已经被 E4R 实现和后续 internal-stop 决策部分覆盖、部分推翻。
本节点只读刷新当前 production 事实，产出新的最小 I6 实现 DAG；只新增 result，不修改
production、tests 或权威设计。

## 直接父节点

- `P5-F445H-I6-host-native-current-scope-preflight-result.md`
- `P5-F445H-E4R5C-combined-reacceptance-result.md`
- `P5-F445H-D1-internal-execution-stop-semantics-result.md`
- `P5-F445H-D2-websocket-peer-cancel-hard-cut-result.md`

引用链继续追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md`。旧 I6 result 只提供历史 owner 与缺口事实；
凡与 E4R、D1、D2 或当前权威文档冲突的结论均已失效。

## 固定输入

```text
integration commit       1c042d207c278f0f69d27e12ffee671898dc8985
integration tree         348a85bf8a2a9d5323efe68313209f14fa81504e
production/tests commit  bf55ede018526751a2db101a42900c4e07fe08a8
production/tests tree    61323e4772061c3b50abc189712767bde716ea24
```

E4R 已通过 `411/411` 的唯一完整 eval gate并明确 `I6_UNBLOCKED = YES`。本预检不得重跑
该 gate，也不得把 E4 已拥有的 evaluator actual-Pending、catch、concurrent、Actor continuation、
activation、program stream或 response stream实现重新分配给 I6。

## 已冻结的语义修正

旧 I6 task/result 中以下要求已失效，不得进入新 DAG：

1. 不存在公开 request cancellation、`CancelError`、按 request id 取消或 stop inspection API。
2. WebSocket 第一版不发送或接收 peer `$/cancelRequest`，也没有 `-32800`。
3. Internal stop只要求本地 pending先收束、晚到结果隔离与资源 owner 的异常
   best-effort清理；不等待 cleanup acknowledgement，不承诺 cleanup 恰好一次。
4. 第一版不增加通用 Host lifecycle metadata、cancel-safety、commit point、cleanup action或
   cleanup grace配置。旧 result 的 M0 前置默认删除；只有当前权威设计另有明确、具体 owner条款时，
   才能报告冲突，不能恢复旧提案。
5. 普通 WebSocket send是 non-suspending；语言没有显式 `yield`。不得给同步 operation制造虚假
   suspension point。
6. WebSocket `requestJsonToConnection` 的 Skiff surface仍是三参数；timeout/internal stop是
   execution scope，不是第四个业务参数。
7. Legacy service relay是禁止写集，不得为了旧测试或旧 DTO恢复。

## 必答问题

### 1. E4R 后的最小 delta

按旧 result §11逐项核对当前代码，并把每一项分类为：

```text
SATISFIED_BY_E4R | STILL_MISSING_IN_I6 | OBSOLETE_BY_D1_D2 | I7_ONLY
```

至少覆盖：

- capability façade borrowed/owned execution control；
- native invocation projection；
- HTTP unary、stream、SSE；
- WebSocket四个普通 send与 `requestJsonToConnection`；
- canonical in-process service call；
- Actor create/call、Actor method Host路径；
- time/sleep；
- file source与其它 request-local external source；
- response/source stream wait和cleanup；
- request/root最终 timeout owner。

不能只按符号名判断。每项必须给出当前真实调用链、读取 current execution scope 的时点、lower
收到的数据，以及仍缺失时的最小 production owner。

### 2. Façade 与 invocation-time carrier

确认旧 result 中下列事实是否仍成立：

- `runtime/host/src/eval_capability_adapter/execution.rs` 的
  `RuntimeExecutionControl` / `RuntimeOwnedExecutionControl` 是否仍未转发
  `execution_scope()` / `derive_scope(...)`；
- `ProgramExecutionContext` 虽可切换 current control，但 capability contexts是否仍在 request
  构造时冻结 root control；
- native invocation是否已有统一、调用时读取的 current-scope carrier，还是各 capability仍各自
  保存 snapshot。

若需要共享 checkpoint，必须给出最小 API、唯一 owner、constructor/fixture机械跟随范围和聚焦
测试；不得把多个 consumer各自实现同一投影。

### 3. Operation-by-operation deadline / stop

对每个仍属 I6 的 operation记录：

- caller request、外层 `timeout(...)`、primitive timeout与 current scope如何取最早 deadline；
- current scope的全部 internal-stop signals怎样到达 lower或 pending waiter；
- deadline产生 `TimeoutError`、ancestor internal stop不产生用户 error的现有 owner；
- 本地 pending如何先原子settle并拒绝 late value/error；
- 已越过外部副作用点时如何保持 unknown outcome，不伪装成撤销；
- 正常 terminal与异常 best-effort cleanup的具体资源 owner。

不要求 lower物理取消，也不要求通用 cleanup receipt。普通同步 operation只需在调用前/后经过已有
execution checkpoint，不新增 yield。

### 4. Service timeout条款的设计完整性

当前 `doc/reference/runtime.md` §8仍写明一次远程调用或 Host operation的可见 deadline包含：

```text
caller request
outer timeout(...)
consumer dependency timeout
callee operation timeout
primitive operation timeout
```

必须在当前 repository中追踪 consumer dependency timeout与 callee operation timeout的真实声明、
配置、artifact/IR、compiler、loader、Host和runtime owner，并回答：

1. 两者是否已经有完整数据模型，只是尚未接到 current scope；
2. 是否由现有 deployment `policy.timeoutMs`、service依赖声明或 operation metadata明确表达；
3. 若没有，是否只有这一句规范而不存在可唯一推导的 public/config语义。

不得根据字段名自行把 deployment timeout解释成 dependency timeout或 callee operation timeout。
若现有权威设计不能唯一回答配置位置、粒度、持久化和优先级，result必须返回
`DECISION_REQUIRED`，列出精确缺失事实与最小选项；同时把不依赖该决策的 I6共享 checkpoint和
consumer节点标记为可继续。若当前代码/文档已经唯一回答，则给出逐层证据，不向用户重复提问。

### 5. 新 I6 DAG与 I7 handoff

产出不超过必要数量的节点，优先采用：

```text
shared invocation-scope checkpoint
  ├─ HTTP
  ├─ WebSocket request
  └─ time / file / Actor / service（仅按真实写入边界拆分）
       ↓
combined integration probe
       ↓
independent I6 acceptance
```

只有写集互斥且接口已经确定时才并行。每个建议叶子节点必须给出：

- 直接父节点；
- production/test owner与精确允许写集；
- 禁止写集；
- 第一处预期修改；
- 聚焦 RED、通过命令和反向搜索；
- 哪个后续节点被解除；
- 证据失效条件。

I7只拥有跨层真实 receipt：真实 `.skiff` source编译、artifact/golden、Router wire、
Agine/codex-relay consumer、stable/live/chat smoke。I6只做 hermetic Host/runtime验证，不运行
network、stable instance、MongoDB或其它 live target。

## 输出

只新增：

`P5-F445H-I6R-current-scope-refresh-preflight-result.md`

result必须包含：

- `READY_FOR_I6_DAG`、`DECISION_REQUIRED`、`TASK_SCOPE_EXPANDED`或
  `TASK_NOT_EXECUTABLE`；
- E4R delta分类表；
- current operation owner表；
- service timeout条款审计；
- 删除旧 M0与 peer cancellation要求的明确结论；
- 最小 DAG、写集、test-first矩阵、combined probe与 I7 handoff；
- 当前是否需要用户决策，以及不受决策影响的 ready queue；
- 所有结论锚定的精确 commit/tree。

允许 `rg`、`git`、源码阅读及不执行测试的命令展开；不得运行 Cargo、完整 gate、真实网络、
stable instance、live service或 MongoDB。

## Worktree 与提交

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i6r-preflight
branch   codex/p5-f445h-i6r-preflight
```

只提交 result，最终工作树 clean；不得 merge、rebase或push。不得派子 Agent。本节点是只读预检，
启动五分钟内应确认直接 owner并建立 result骨架；若仍无法形成有界检查路径，立即返回
`TASK_NOT_EXECUTABLE`。若范围扩张或设计仍不完整，如实结束，不得自行补设计或继续实现。
