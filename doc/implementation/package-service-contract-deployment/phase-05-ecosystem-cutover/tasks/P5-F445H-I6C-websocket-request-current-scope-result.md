# P5-F445H-I6C WebSocket request current-scope consumer result

状态：

```text
TASK_SCOPE_EXPANDED = YES
TASK_NOT_EXECUTABLE = YES
IMPLEMENTATION_COMMIT = NONE
I6_C_WEBSOCKET_COMPLETE = NO
I6_J_WEBSOCKET_CASE_UNBLOCKED = NO
```

本节点在有界 production 调用链核对后停止。没有修改 production、tests、Cargo/lockfile、Router、
wire 或公开 std/native surface，也没有实现 root-snapshot、task-local 或其它影子替代路径。

## 1. 输入与 worktree 状态

| 项 | 值 |
| --- | --- |
| 合同固定 production base commit | `8db08c539acaf0b3fc41733365f06e9883bdbdd8` |
| 合同固定 production base tree | `71123064dd0948d5946ad8c6312df909670794e0` |
| 探查开始 HEAD | `baf2547d37e2f9103a360c9615fb29a9bb6584c9` |
| 探查开始 tree | `f936f711eba2bd2ca73ce7b59e8d404004b6923f` |
| branch | `codex/p5-f445h-i6c-websocket` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i6c-websocket` |

`8db08c53..baf2547d` 只新增 I6-B/C/D 任务合同；production 与 tests 仍是合同固定输入。
探查前后 production/tests 均无 diff。

## 2. 被证伪的合同前提

任务预期允许写集中的 Host consumer能够直接消费 I6-A carrier，并把调用点 current child scope
传给 `RuntimeConnectionRequestParts` 与 `ConnectionRequestRegistry::install`。实际调用链中，
carrier owner 与 Host request adapter 之间缺少内部下传 seam：

1. `runtime/eval/src/native_capability.rs:44-45` 在每次 native projection 时正确读取一次
   `context.execution().owned()`；`148-152` 把同一
   `RuntimeNativeInvocationExecutionControl` 放入
   `RuntimeNativeWebsocketCapabilityContext`。
2. `runtime/eval/src/capabilities.rs:580-592` 证明该 wrapper 的第二字段持有 I6-A
   `OwnedExecutionControl`。
3. 但是同文件 `1180-1193` 的
   `NativeWebsocketCapability::request_json_to_connection` 只调用
   `self.0.request_json_to_connection(connection_id, method, payload)`，没有读取或转发第二字段。
4. 同文件 `40-46` 的内部 `WebsocketRequestCapabilityApi` 也只接收三个业务参数，没有
   invocation execution/scope 参数。
5. 因而 Host 实现
   `runtime/host/src/eval_capability_adapter/websocket.rs:96-132` 只能读取
   `RuntimeOwnedWebsocketParts`；其 `RuntimeConnectionRequestParts` 在 `12-17` 仅保存
   request-construction 的单个 `CancellationToken` 与 root `deadline`。
6. `runtime/host/src/eval_capability_adapter/factory.rs:92-118` 继续用上述 root token/deadline
   构造 request transport；真实 assembly caller
   `runtime/host/src/eval_capability_adapter/assembly_execution_context.rs:186-194` 和
   `241-247` 明确传入 `execution.cancellation_token()` / `execution.deadline()`。

因此，仅修改原合同允许的：

```text
runtime/capability-context/src/connection_request.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
```

无法取得 native 调用点 derived scope 的全部 ancestor/local signals、absolute deadline 与
deadline owner。继续实现只能错误复用 root snapshot，违反 I6-C 条款 2、RED/GREEN 要求与 I6-A
carrier消费约束。

## 3. 新暴露的 production owner

缺失 owner 是：

```text
runtime/eval/src/capabilities.rs
```

该文件同时拥有：

- `RuntimeNativeWebsocketCapabilityContext` 中 I6-A carrier；
- native WebSocket request 的真实 delegation；
- Eval 内部 `WebsocketRequestCapabilityApi` seam。

这不是 Router/wire/public std/native API 变更，也不需要新的架构或用户语义决策；它是 I6-A carrier
到既有 Host consumer之间遗漏的内部 production接线 owner。但它不在当前任务允许写集中，依据
`multi-agent-development.md` 的“有界探查后的强制停止”规则必须停止。

## 4. 最小后继合同

主 Agent应修订或新增一个最小 I6-C 接线合同，依赖顺序如下：

1. 授权 `runtime/eval/src/capabilities.rs`，让
   `RuntimeNativeWebsocketCapabilityContext::request_json_to_connection` 从其既有 I6-A carrier
   取得同一 `OwnedExecutionControl` / `ExecutionScope`。
2. 通过 Eval 内部 `WebsocketRequestCapabilityApi` 将该 execution/scope传给 Host request adapter；
   `requestJsonToConnection(connectionId, method, value)` 的三参数 Skiff/native公开 surface保持不变。
3. 原 I6-C 允许写集随后让 `RuntimeConnectionRequestParts` 与 registry install消费 current scope，
   建立 ancestor/internal stop、有效 deadline、response 的 winner顺序，并保持 session/generation
   fence、CAS先本地settle/清timer与lease、可丢失internal hint及 late `complete=false`。
4. 测试授权至少应覆盖 Eval wrapper到 Host adapter 的 carrier receipt；可在
   `runtime/eval/src/capabilities.rs` 内部测试或明确授权现有 Eval execution-scope fixture。
   原合同两个聚焦 selector与反向搜索仍由恢复后的 I6-C owner执行。

不需要授权 Router/profile/request broker、transport wire schema、std/native公开签名、peer
cancellation、business identity fan-out、E4 actual-Pending、I6-A/B/D 或 Cargo/lockfile。

## 5. 未运行项与证据状态

以下合同命令全部未运行：

```text
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture
cargo check -p skiff-runtime-capability-context -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

原因是任务在第一次 production修改前已证明需要未授权 owner；没有可安全提交的 scoped
implementation，也不存在可供 GREEN 验证的候选。没有运行完整 gate、server/network、stable/live
或 MongoDB。

合同反向搜索未作为完成证据运行。只进行了只读 `rg`、`git` 与精确源码阅读，用于确认 carrier owner和
缺失 seam；这些证据只支持 `TASK_SCOPE_EXPANDED`，不支持实现 PASS。

## 6. 结论

当前提交只能固化 result，不能作为 implementation checkpoint。I6-C 与 I6-J WebSocket case均未解除；
最小后继是先把 `runtime/eval/src/capabilities.rs` 纳入 I6-C内部 carrier接线 owner，再由新的有界开发
会话完成原 registry/Host生命周期与聚焦验证。
