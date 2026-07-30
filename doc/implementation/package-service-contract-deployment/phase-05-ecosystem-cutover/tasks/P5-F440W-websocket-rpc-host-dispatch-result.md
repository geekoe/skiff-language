# P5-F440W WebSocket RPC Host dispatch / outcome result

状态：`TASK_SCOPE_EXPANDED / UPSTREAM_PIN_CONTEXT_REQUIRED`。

本 leaf 没有保留 production 或 test 修改。真实 Host 入口的 test-first RED 已证明
`RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc` 仍进入 F440U 留下的显式
`Unsupported`；继续完成 dispatch 则需要 generation pin resolver 同时返回与 target 同源的 old-generation
Host route context。该 API 不在本任务唯一写集内，因此没有用 current assembly lookup、不可用 capability
或 target 扩面绕过。

## 1. 基线与范围

| 状态 | Commit |
| --- | --- |
| 任务声明的 implementation baseline | `b3ca0f0e` |
| worktree 实际起点 | `582bc04752ae42a9c5db464dcf44265616524285` |

`582bc047` 相对 `b3ca0f0e` 只增加 F440V1/F440W 调度文档，没有 production/test 变化。

Worktree：
`/Users/geek/workspace/skiff-p5-f440w-rpc-host-dispatch`

Branch：
`codex/p5-f440w-rpc-host-dispatch`

没有 implementation commit；本文是唯一交付提交。

## 2. 真实 Unsupported RED

先临时增加 direct Host test：

```text
host::router_session::tests::websocket_generation_lifecycle::
websocket_jsonrpc_target::
websocket_jsonrpc_host_dispatches_pinned_method_instead_of_unsupported
```

测试建立 current captured Router session、physical connection 的 exact acquired generation pin，再通过
真实 `request.start.runtimeAssembly.websocketJsonRpc` binary frame 发送 opaque record params，并要求
typed `response.end.websocketJsonRpc.success` 与精确 payload。

执行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host \
  websocket_jsonrpc_host_dispatches_pinned_method_instead_of_unsupported \
  -- --nocapture
```

结果为 exit `101`，实际执行 `1`，`1 failed / 297 filtered`。旧入口返回 ordinary
`response.error`，typed response decoder 精确失败：

```text
unknown field `errorKind`, expected one of
`schemaVersion`, `type`, `requestId`, `payloadPresent`, `websocketJsonRpc`
```

这是真实 acquired-pin Host dispatch RED，不是零测试、compile failure、依赖下载失败或 synthetic
resolver probe。按停止工作流，临时失败测试随后已完整撤销，未提交常态 RED。

## 3. 缺失的 pinned Host context

F440U 当前
`WebSocketGenerationRegistry::websocket_jsonrpc_target(...)` 在 generation registry 内先取得
`physical_route`，再构造 exact `method_route` 与
`RuntimeAssemblyWebSocketJsonRpcTarget`，但只把 target 返回给调用者。

target 已安全持有或暴露：

- pinned execution image、request activation 与 eval resolver；
- assembly identity/generation、deployment owner、implementation build；
- method selector/key/identity、physical selector/key/identity/`WebSocketEntryId`；
- linked handler、adapter plan、profile。

完整复用 current Host assembly capability construction还缺少仅由同一 old
`ActiveAssemblyRoute`/`ActiveAssemblyContextSet`持有的事实：

- exact `DbCapabilitySource`，由 admission 时的 service DB binding与 pinned activation共同构造；
- exact owner `ServiceProtocolIdentity`，来自 pinned linked deployment contract；
- route-owned execution context join，供 Host证明上述 capability facts与 target activation/image是同一
  immutable generation。

deployment policy可从 pinned activation owned bindings恢复，但这不能补出 DB source或 service protocol
identity。`RuntimeAssemblyEvalTarget` 的公开 resolver seam只提供 activation/contract/schema/operation
target，不提供 Host concrete DB capability source；target本身也刻意不暴露 loader/Host route。

## 4. 为什么不能查 current assembly

`RuntimeHost::lookup_active_assembly_request_route` 与
`AssemblyAdmissionController::route`只读取 current committed assembly。若 connection 先 pin A、随后
active replacement为 B，再按 method selector回查 current route，会得到 B：

```text
captured session + connection pin -> target A
current selector lookup          -> context B
```

这样会把 A 的 handler/image与 B 的 DB source、service protocol identity或policy拼成混合执行上下文，
违反 F440U 的 old-generation owner链，也会让 active replacement改变已建立 socket的执行语义。若 B
不再有该 selector，合法的 A request还会被错误拒绝。故 current lookup不是 fail-closed替代方案。

同样不能使用 `DbCapabilitySource::unavailable()`、空 service protocol identity或从 request metadata猜值：
这些都会把已 admission 的 capability降级或制造第二个 trust owner。

## 5. 最小上游 API

建议由 F440U / generation pin owner增加一个不改变 target语义的 Host-private resolver：

```text
WebSocketGenerationRegistry::websocket_jsonrpc_execution_route(...)
  -> ResolvedWebSocketJsonRpcExecution {
       target: RuntimeAssemblyWebSocketJsonRpcTarget,
       method_route: ActiveAssemblyRoute,
     }
```

实现应复用当前 `acquired_physical_route -> websocket_jsonrpc_method_route ->
websocket_jsonrpc_target` 单次 exact join，把已经存在的 old `method_route`连同 target一起返回；不得再查
assembly controller。现有 `websocket_jsonrpc_target(...)` 可委托新 resolver并只投影 `target`，保持
F440U public/测试语义不变。

最小上游测试：

1. pin A后激活 B，resolver返回的 target与method route均为 A，且 activation/execution image owner指针
   精确一致；
2. A/B使用可区分的 DB source、service protocol identity与policy，返回值只携带 A facts；
3. wrong session、connection、assembly generation、physical id、host/path/method、method identity与profile
   在暴露 route context前拒绝；
4. tentative/no-receipt pin不能取得 target或route；
5. release/disconnect后 resolver拒绝且 old route可回收；
6. source audit证明新 resolver不调用 current assembly controller或artifact I/O。

该 checkpoint 合入后，F440W只需在原唯一写集内完成：

- wire header到 resolver参数的 exact projection；
- target + old route驱动的共享 HTTP/JSON-RPC execution context builder；
- supervisor cancellation/deadline与 F440V terminal到 F440T response end映射。

## 6. Business identity trust结论

business identity不是本次 blocker。current generation registry不保存 connect-time business
identity；canonical owner是 captured Router session上 strict decoded
`websocketJsonRpc.businessIdentity` metadata：

- `router_session_id`由已建立 Router connection传入
  `dispatch_router_binary_frame_inner`，不来自 request frame；
- resolver以 `(router_session_id, connection_id)` 查 exact acquired pin，因此伪造 frame不能选择另一
  runtime session；
- transport decoder deny unknown fields并验证 optional business identity边界；
- peer params保持独立 opaque payload，Host不解析其中字段；
- F440V input把 `params` bytes与 trusted `business_identity`作为不同参数，peer JSON中的同名字段不能覆盖
  trusted source。

因此上游只需保留“同一 captured session + acquired pin”约束，无需把 peer params或第二个 identity source
加入 registry。

## 7. 验证与范围审计

- 完整读取任务、F440V/F440U/F440T/F440Q直接父节点与 F440B §§8–9。
- RED使用规定 shared Cargo target，实际执行数为 `1`。
- 没有运行 live/server/stable instance/watch/chat smoke。
- 没有修改或提交 runtime request/eval、target、wire、Router、loader/generation、artifact/compiler/native/std。
- 没有派子 Agent；没有 merge、rebase或 push。
- mandatory GREEN未运行：本 leaf按 scope expansion停止，没有可验收 implementation tree。
