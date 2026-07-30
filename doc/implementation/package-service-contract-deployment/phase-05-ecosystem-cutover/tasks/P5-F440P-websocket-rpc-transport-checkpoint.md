# P5-F440P WebSocket RPC std / runtime transport checkpoint

状态：Ready。对应 F440B DAG 的 **T0**；P0 已通过。

## 直接父节点

- `P5-F440O-bidirectional-rpc-prerequisite-gate-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `doc/reference/std-surface.md` §13
- `doc/reference/runtime.md` §11–12
- `doc/architecture/actor-model.md` 的 suspension 规则

实现基线为 `2176bdb0e476c54ca7406ec05cb18a102afaa94f`
（tree `205cc8f818652076f31bbb837b9dfa026eeb66ee`）。

权威 reference 的当前 spelling优先于较早审计示例：`WebSocketRequestError` discriminator是
`kind`，不是 `tag`。语言没有 `yield`；普通 send保持 non-suspending，request只有在真实等待response时
才释放actor executor。

## 目标

建立 E0 与 R0a 共同依赖的唯一 shared checkpoint：

1. `std.websocket.requestJsonToConnection<TRequest,TResponse>` 公开 ABI、effects与五分支
   `WebSocketRequestError`；
2. Rust/TypeScript成对的 `connection.request`、`connection.request.cancel`、
   `connection.response` strict typed-binary wire；
3. runtime `ConnectionRequestRegistry` 的 correlation、lease、deadline、cancel、terminal CAS和
   runtime-session fencing；
4. native/Host共享边界可以编码 request params、安装等待并接收 opaque terminal；E0 后续只负责调用点
   typed result decode与执行恢复；
5. Router `RuntimeEndpoint` 只完成 strict frame decode、registered sender/session source传递和回原
   runtime session的 response入口；本 leaf不实现 peer broker、JSON classifier或gateway hookup。

## 唯一写集

- `std/websocket.skiff`
- `std/api.yml`
- `artifact-model/src/native_signature.rs`
- `runtime/model/src/service_error.rs`
- `runtime/native-contract/**`
- `runtime/native/**`
- `runtime/capability-context/**`
- `runtime/request-contract/**`
- `runtime/transport/**`
- `runtime/host/src/capability_context/**`
- `runtime/host/src/eval_capability_adapter/{websocket.rs,factory.rs,mod.rs}`
- `runtime/host/src/host/router_session/**`
- `runtime/host/src/host/{runtime_host.rs,mod.rs}`
- 上述 Host 范围中新建的 connection request registry模块及 colocated tests
- `router/src/protocol/{envelope.ts,runtimeProtocol.ts}`
- `router/src/router/runtimeEndpoint.ts`
- `router/tests/protocol.test.ts`
- `router/tests/runtime-endpoint-connection-send-trust.test.ts`
- 本 leaf result

禁止修改 runtime/eval、runtime/request、Host request-entry/loader、Router broker/gateway/server、
cross-system fixtures、test-runner、scripts、README、真实service root、其它 task/result或权威设计。
不得派子 agent。

## Public surface 与 suspension

增加并导出：

```skiff
type WebSocketRequestError discriminator "kind" =
  { kind: "connectionUnavailable", message: string }
  | { kind: "transportUnavailable", message: string }
  | { kind: "protocolError", message: string }
  | { kind: "resourceLimit", message: string }
  | { kind: "remote", code: integer, message: string, data: Json? }

native function requestJsonToConnection<TRequest, TResponse>(
  connectionId: string,
  method: string,
  value: TRequest
) -> TResponse
```

Native signature精确为：

- type params 2；
- params `[string, string, T0]`，return `T1`；
- required context `Websocket`；
- external write，conflict key由exact connection id形成，cancel safety为response-discardable；
- `may_suspend=true`，其它alias/write summary按现有native模型保持最小；
- 四个 raw send native仍 `may_suspend=false`，helper不能被误升成挂起点。

五个 error branch必须保持 named-union branch exact catch identity。不得注册成一个扁平
`RuntimeError::ProviderUnavailable`或重新引入新的 platform builtin error identity。Local encode/decode
错误仍是 `std.json.DecodeError`；deadline仍是 `TimeoutError`；ancestor cancellation是不可捕获 terminal。

## Strict wire

按 F440B §7.2 实现 exact header和payload presence：

- `connection.request`
  - `requestId`、`serviceId`、canonical `websocketEntryId`、`connectionId`、
    `profile: "jsonrpc-2.0-text"`、non-empty `method`、可选 absolute/effective deadline；
  - payload必有且是UTF-8 opaque params JSON，顶层必须 object/array。
- `connection.request.cancel`
  - exact original `requestId`、现有 closed `RequestCancelReason`；
  - payload必须为空。
- `connection.response`
  - exact `requestId`；
  - outcome仅
    `success | deadlineExceeded | connectionUnavailable | transportUnavailable |
     protocolError | resourceLimit | remote`；
  - success payload必有（JSON `null`也算）；
  - remote header仅在 remote outcome存在，code为safe integer、message有界、
    `dataPresent`与payload presence精确一致；
  - 其它 outcome无remote且payload为空。

三类 decoder均 deny unknown fields。不得复用 `request.cancel` 或
`RuntimeDispatcher.pending`；`connection.request.cancel`是独立控制帧。

Rust `runtime/transport` 与 Router TypeScript protocol必须成对接受/拒绝同一 shape。T0不得修改
F0-owned shared corpus；P0记录的4个 gateway-v1 corpus failure可以保留，但新 `connection_*` focused
selectors必须全绿。

## Registry、deadline 与 source trust

`ConnectionRequestRegistry` 至少满足：

- runtime-session correlation id在该session生命周期内不复用；
- 先安装 lease/timer/cancel-on-drop，再把 request写入Host queue；
- completion以exact entry/token做单次 CAS，late/duplicate terminal无效；
- success/remote settlement后才把opaque payload交给后续typed decode，decode失败不重开pending；
- ancestor cancel先移除pending、释放lease并best-effort发专用cancel frame，不生成普通error；
- deadline同样先settle，向调用方保持 `TimeoutError`，并best-effort发cancel；
- writer/session断开使已接纳请求得到 `transportUnavailable`；
- admission前connection不可用得到 `connectionUnavailable`；
- pending/method/payload上限得到 `resourceLimit`；
- malformed/forged response得到 `protocolError`；
- registry drop、runtime reconnect和同名runtime新session均不能接收旧response；
- 每个测试结束 pending/timer/lease为0。

Router `RuntimeEndpoint` 必须把registered WebSocket sender及session token与合法 request/cancel一起交给未来
broker callback；malformed header/payload或伪造source不写peer。response API只允许写回captured original
runtime socket/session。它不得在本 leaf解析 peer JSON、生成peer id或拥有outbound broker table。

## 测试先行与验证

先新增真实 red，至少覆盖：

1. std/native registry当前缺少新 callable、2个type param及 `may_suspend=true`；
2. Rust/TS decoder当前不认识三类 frame；
3. registry cancel/deadline/late response/reconnect的 pending cleanup；
4. 五个 named-union branch identity与 `std.json.DecodeError` / `TimeoutError` 分流；
5. raw send仍 false，新 request true。

必跑：

```bash
cargo test -p skiff-artifact-model native_signature
cargo test -p skiff-runtime-native-contract
cargo test -p skiff-runtime-native websocket
cargo test -p skiff-runtime-capability-context
cargo test -p skiff-runtime-request-contract
cargo test -p skiff-runtime-transport connection_
cargo test -p skiff-runtime-host connection_request
cargo check -p skiff-runtime-host
pnpm --dir router exec vitest list --root router \
  tests/protocol.test.ts tests/runtime-endpoint-connection-send-trust.test.ts \
  -t 'connection request|connection response'
pnpm --dir router exec vitest run --root router \
  tests/protocol.test.ts tests/runtime-endpoint-connection-send-trust.test.ts \
  -t 'connection request|connection response'
cargo fmt --all -- --check
pnpm --dir router type-check
git diff --check
```

根据当前 package script调整 Vitest invocation时，必须先列出非零selector，并在result记录实际命令与
计数；不得让参数意外展开完整Router suite。Cargo统一使用：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

补充反向搜索：

```bash
rg -n 'requestJsonToConnection|WebSocketRequestError|connection\.request|connection\.response' \
  std artifact-model runtime router/src
rg -n 'yield|may_suspend|sendTextToConnection|sendBinaryToConnection' \
  std artifact-model/src/native_signature.rs runtime/native-contract runtime/native
```

result逐层说明 canonical owner；不能以匹配总数代替。`yield`不得成为源码关键字或runtime frame。

## 停止与交付

若实现共享 checkpoint必须修改 runtime/eval、Host request-entry/loader或 Router broker/gateway，
返回 `TASK_SCOPE_EXPANDED`并列出应归 E0/R0a/R0b 的精确API缺口；不得越界。若 public signature、
error discriminator或wire outcome与权威 reference冲突，返回 `TASK_NOT_EXECUTABLE`。

不运行完整 verify、live、instance、stable或chat smoke。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440p-websocket-rpc-transport`
- branch：`codex/p5-f440p-websocket-rpc-transport`
- result：`P5-F440P-websocket-rpc-transport-checkpoint-result.md`

Implementation 与 result 分开提交；不 merge/rebase/push。
