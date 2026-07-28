# P5-F445H-I6E invocation carrier delivery seam preflight

状态：Ready。I6-A 已创建 invocation-time owned execution carrier，但 I6-B、I6-C、I6-D 的真实
调用链证明 HTTP、WebSocket request、time、file、Actor control/spawn 在进入下层 consumer 前丢失
carrier。本节点只冻结最小共享接线与恢复 DAG，不修改 production/test。

## 直接父节点

- `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
- `P5-F445H-I6B-http-current-scope-result.md`
- `P5-F445H-I6C-websocket-request-current-scope-result.md`
- `P5-F445H-I6D-host-operation-current-scope-result.md`

## 固定输入

```text
base commit  1000d290ce9ebc3cd5a792cf01f27b5835496a2a
base tree    90c69b694fb38c7ec544149aec3b87a3b632496c
```

## 唯一目标

用只读源码证据冻结一条最短且可编译的 carrier delivery 接线，使后续 HTTP、WebSocket request、
time、file、Actor control/method/spawn consumer 都能取得同一次 native invocation 的 current
`OwnedExecutionControl`或等价 owned `ExecutionScope`，同时保持公开 Skiff/native API 不变。

本节点不是实现节点，不重复讨论 timeout/cancel/yield 语义，不为每条能力另造 carrier，也不允许
task-local/global side channel。

## 必查调用链

1. HTTP unary、body-stream open、SSE open：
   `RuntimeNativeHttpClientCapabilityContext` 到 Host `http_client_runtime`。
2. WebSocket `requestJsonToConnection(connectionId, method, value)`：
   Eval wrapper、`WebsocketRequestCapabilityApi`、Host adapter、connection request registry。
3. `std.time.sleep`：
   Eval time wrapper、`NativeTimeCapability` trait、native time dispatch；同步 time helper不得被改造成
   挂起操作。
4. file direct operation、provider operation、source stream：
   Eval file wrapper、Host file context/runtime与source waiter。
5. Actor get/create/replace/find/remove、method、spawn：
   Eval Actor wrapper、Actor dispatch、Host actor context与`spawn_ops.rs`。

## 必须冻结的输出

### A. 共同接口

- carrier 在每条链上精确经过哪些内部 trait、context、constructor、adapter与method。
- 后续传递 owned control、owned scope还是一个 crate-private invocation view；说明生命周期、clone、
  lease与clock保持的理由。
- 哪个节点读取 current scope，哪个节点拥有 pending waiter；不得在 native projection时再次冻结
  relative deadline。
- public std/native surface、artifact/schema、Router/wire为何无需修改。

### B. 精确写集

逐文件、逐 symbol 列出：

- 唯一共享 owner；
- 为让共享 checkpoint单独编译必须同步修改的接口实现/constructor；
- 下层 consumer恢复节点各自的互斥写集；
- 纯机械 caller跟随与行为改动的区别。

必须明确核对至少：

```text
runtime/eval/src/capabilities.rs
runtime/eval/src/spawn_ops.rs
runtime/native/src/capability.rs
runtime/native/src/dispatch/time.rs
runtime/host/src/host/http_client_runtime.rs
runtime/capability-context/src/connection_request.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
runtime/host/src/host/file_runtime.rs
runtime/host/src/host/file_stream.rs
runtime/host/src/capability_context/actor.rs
runtime/eval/src/actor/dispatch.rs
```

若实际 owner不同，记录精确替代路径，不把整目录加入写集。

### C. 最短实现 DAG

目标是缩短墙钟时间，而不是追求节点数量。必须回答：

1. 能否先完成一个单独可编译的共享 delivery checkpoint；
2. 若能，之后哪些下层 consumer可完全并行；
3. 若不能，最少需要几个串行 owner checkpoint，原因是什么；
4. 哪些文件因共同 owner必须由一个 Agent修改，不能伪装成并行；
5. 每个实现节点的直接父文档、base、唯一写集、最小非零测试与停止条件。

不得使用固定“三个任务/三波”等数字阈值。并行度由写集和接口依赖决定。

### D. 真实接收证据

为共享 checkpoint冻结至少一条真实的 carrier receipt测试：不能只断言 wrapper含字段，必须证明 carrier
穿过新内部 seam到达下层 adapter/context。其余下层节点分别冻结 current deadline、ancestor/internal
stop、normal completion、late completion、lease/timer归零等测试。

说明哪个节点第一次形成一条从 native projection到真实 pending consumer的纵向闭环；在此之前 I6-A
只算内部检查点，不能写成完整能力。

## Agent 使用

任务 Agent可派最多三个只读子 Agent，分别核对：

1. HTTP + WebSocket；
2. time + file；
3. Actor + spawn。

子 Agent不得再委派。父 Agent必须亲自统一接口和 DAG，不能直接拼接互相矛盾的建议。

## 唯一写集

只允许新增：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md
```

禁止修改 production、tests、fixture、Cargo/lockfile、父任务或权威设计。

## 禁止项

- 不运行 Cargo测试、build、full gate。
- 不访问 stable/live/network/MongoDB。
- 不实现 carrier seam或下层 consumer。
- 不新增公开 cancel、yield、operation lifecycle metadata。
- 不把 root token/deadline包装成伪 current scope。
- 不 merge、rebase、push。

## 停止条件

若五条链需要互相冲突的公共语义、必须修改公开 std/native API，或精确 owner仍无法冻结，提交
`DECISION_REQUIRED` / `TASK_SCOPE_EXPANDED` result并停止。若只是文件数多，不构成停止理由。

## 完成条件

result必须包含：

1. 五条 carrier调用链；
2. 共同接口与生命周期选择；
3. 精确逐文件写集；
4. 最短可并行实现 DAG；
5. 每节点测试、停止条件与结果文档名；
6. 第一条纵向真实接收证据；
7. 明确 `READY_FOR_I6_RESUME_DAG = YES/NO`。

完成后只提交 result，保持 worktree clean并报告 commit/tree。
