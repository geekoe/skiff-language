# P5-F440P WebSocket RPC std / runtime transport checkpoint result

状态：`TASK_SCOPE_EXPANDED / SCOPED_CHECKPOINT_VALID`。

T0 写集内的 public std surface、native signature/context route、五分支 exact error carrier、
Rust/TypeScript strict `connection.*` wire、runtime-session fenced registry、Host queue/demux source与
Router `RuntimeEndpoint` captured-source boundary已经建立。已执行的 shared-layer selectors全部通过。

但 production native invocation目前不能取得 caller-linked
`std.websocket.WebSocketRequestError` named-union owner；`NativeCallPlan`只有参数/返回值 plan，
`RuntimeNativeInvocation`也不携带该 owner。伪造 platform identity或猜测 `TypeAddr`都会破坏 exact catch，
所以当前实现按要求 fail closed，没有越界修改 `runtime/eval` 或 `runtime/linked-type-plan`。因此本文不把
完整 production call path声明为可执行。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 production baseline | `2176bdb0e476c54ca7406ec05cb18a102afaa94f` | `205cc8f818652076f31bbb837b9dfa026eeb66ee` |
| worktree 实际起点 | `bb15c95953838465fc32a284f3098d70122641a1` | `689bcab7b3fb28cf438878af6f8f8a0ae66aa392` |
| implementation | `194a8f026e84fad2ed063deacfa7e25c0441f684` | `9662083ea1baf9d901b6e44234e28c584cdc5531` |

`bb15c959` 的直接父提交是任务声明的 `2176bdb0`；二者只差本任务文档，没有 production/test
变化。Implementation 与本文 result保持分离提交；result commit/tree由最终交付消息记录。

## 2. 写集内已完成的 canonical owners

### 2.1 Public std 与 native contract

- `std/websocket.skiff` / `std/api.yml` 是 public owner：
  `WebSocketRequestError discriminator "kind"` 精确包含
  `connectionUnavailable | transportUnavailable | protocolError | resourceLimit | remote`，并导出
  `requestJsonToConnection<TRequest,TResponse>(string,string,TRequest)->TResponse`。
- `artifact-model/src/native_signature.rs` 是 signature/effect owner：2 个 type param，
  `[string,string,T0] -> T1`，新 request 的 `may_suspend=true`；四个 raw send仍为 false，其它
  caller alias/write effect保持 false。
- `runtime/native-contract` 将新 binding精确路由到 required context `Websocket`；
  `runtime/native` 的 route matrix不把它落入 generic registry。
- native request只在等待 capability future时挂起；raw sends仍在同一次 poll内完成。`TRequest`
  先走调用点 boundary plan，且编码后顶层必须为 object/array；success payload先完成 registry
  terminal，再按 `TResponse` plan decode，不会重开 pending。

### 2.2 Exact error identity

`runtime/model/src/service_error.rs` 是 ordinary error carrier owner。每个分支投影为：

```text
CatchIdentity::NamedUnionBranch {
  union: NamedUnionOwnerIdentity::LocalExecution(<caller-linked exact TypeAddr>),
  branch: SyntheticDiscriminator {
    discriminator_field: "kind",
    discriminator_value: <exact branch spelling>,
  },
}
```

五个 branch均由同一 finite enum构造；`std.websocket.WebSocketRequestError`没有注册为
`PlatformBuiltinErrorIdentity`。本地 JSON codec失败仍走 `std.json.DecodeError` identity；
deadline走既有 `TimeoutError` projection；ancestor cancel走不可捕获 `RuntimeError::Cancelled`。

### 2.3 Rust/TypeScript strict wire

Rust owner位于 `runtime/transport/src/connection_protocol.rs`，Router owner位于
`router/src/protocol/{envelope,runtimeProtocol}.ts`：

- `connection.request`：strict schema/type、bounded canonical ids、canonical
  `WebSocketEntryId`、固定 `jsonrpc-2.0-text` profile、bounded method、可选 positive safe-integer
  deadline、必有 UTF-8 JSON object/array payload；
- `connection.request.cancel`：复用 closed `RequestCancelReason` spelling，且是独立、无 payload
  的 control frame；
- `connection.response`：七个 exact outcome；success payload必有，`null`合法；remote
  code为 JS safe integer，message有界，`dataPresent`与 payload presence严格一致；其它 outcome
  禁止 remote metadata和 payload。

三类 header及 deadline/remote nested object均拒绝 unknown fields。补充 parity red后，Rust/TS
同时拒绝无效日历、非法时区、额外 deadline 后缀及 unsafe `timeoutMs`，且同时允许有界、非空但保留
peer空白的 remote message。

### 2.4 Registry、Host 与 Router source boundary

- `ConnectionRequestRegistry` 使用 registry-lifetime 单调、不复用 correlation id；pending以
  exact runtime session、request id和 entry token完成单次 CAS。
- lease/timer/cancel-on-drop在 Host queue前安装；cancel/deadline先移除 pending并释放 lease/timer，
  再 best-effort发专用 `connection.request.cancel`。late/duplicate response返回 false。
- writer/session断开和 registry drop把已接纳请求 settle为 `transportUnavailable`；重连后的同名
  runtime新 session不能完成旧 pending。
- Host `router_session`只把 strict `connection.response`交给当前 captured router session的 registry；
  session close清理该 session pending。
- Router `RuntimeEndpoint`只做 strict decode、registered runtime断言、冻结
  `{sender,sessionToken}` source和回原 socket/session的 response API。它没有解析 peer JSON、生成 peer
  id、建立 broker table或接入 gateway。

## 3. Test-first red 证据

初始 production变更前的 red如下：

| Selector | Red |
| --- | --- |
| `cargo test -p skiff-artifact-model native_signature` | 1 failed / 175 filtered：缺少新 signature |
| `cargo test -p skiff-runtime-native-contract` | 1 failed / 4 filtered：缺少 Websocket required context |
| capability-context `connection_request` selector | compile red：registry symbols/modules不存在，0 tests |
| transport `connection_` selector | compile red：`connection_protocol`不存在，0 tests |
| Router protocol focused selector | 1 failed / 1 passed / 50 skipped：不认识新 frame type |
| RuntimeEndpoint focused selector | 1 failed / 5 skipped：`onConnectionRequest`不存在 |

提交前追加 Rust/TS deadline/message parity测试时，Rust transport再次真实 red：
`4 passed / 2 failed / 80 filtered`；失败分别证明旧实现接受无效 RFC3339 calendar值，并拒绝 TS
允许的 bounded remote message shape。

随后把 cancel sender回调内的 registry计数写入断言，capability-context selector真实得到
`3 passed / 2 failed / 38 filtered`：pending/timer已经为 0，但 cancel与deadline两条路径的 lease仍为
1。修复后两条路径都在发专用 cancel前观察到 pending/lease/timer全部为 0。

合计记录到 `8` 个 assertion/runtime test failure以及 `2` 个 compile-red Rust selector；没有用
零测试或只编译结果代替 red。

## 4. Green 验证

所有 Cargo命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

### 4.1 Rust selectors

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-artifact-model native_signature` | 17 | PASS：17 passed / 159 filtered |
| `cargo test -p skiff-runtime-native-contract` | 5 | PASS：5 passed；0 doctests |
| `cargo test -p skiff-runtime-native websocket` | 5 | PASS：5 passed / 93 filtered |
| `cargo test -p skiff-runtime-capability-context` | 45 | PASS：43 unit + 2 compile-fail doctests |
| `cargo test -p skiff-runtime-request-contract` | 1 | PASS：1 passed；0 doctests |
| `cargo test -p skiff-runtime-transport connection_` | 6 | PASS：6 passed / 80 filtered；integration binary 0 executed / 2 filtered |
| supplemental `cargo test -p skiff-runtime-model websocket_request_errors_keep_all_five_exact_named_union_branch_identities` | 1 | PASS：1 passed / 88 filtered |

必跑且能进入测试的 Rust selectors共 `79 passed`；supplemental exact-identity probe再 `1 passed`。

Registry的 43 个 unit中，新增 5 个测试分别覆盖：

- ancestor cancel赢、late response无效、pending/lease/timer归零；
- deadline赢、专用 cancel frame、三类计数归零；
- reconnect session fence与 disconnect terminal；
- registry lifetime correlation不复用；
- registry drop使已接纳 waiter得到 `transportUnavailable`。

### 4.2 Host 被 E0 blocker遮挡

以下两条必跑命令均在编译本任务 Host tests之前，被现有 out-of-scope
`skiff-runtime-eval`错误挡住；没有 Host test实际执行：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-host connection_request --no-fail-fast` | BLOCKED，exit 101，0 Host tests |
| `cargo check -p skiff-runtime-host` | BLOCKED，exit 101 |

两次都是相同的 3 个 `E0004`：

1. `runtime/eval/src/runtime_http_gateway.rs:85` 未覆盖
   `GatewayAdapterKind::WebSocketJsonRpc`；
2. `runtime/eval/src/runtime_http_gateway.rs:439` 未覆盖
   `GatewayAdapterSource::{WebSocketJsonRpcParams,WebSocketBusinessIdentity}`；
3. `runtime/eval/src/runtime_websocket_connect.rs:171` 未覆盖同两个 source。

因此 Host colocated 的 install-before-queue与 router-session demux tests状态是
`COMPILE_MASKED`，本文不把它们计为 passed。按任务禁令没有修补 `runtime/eval`。

### 4.3 Router listing、execution 与 type-check

worktree没有安装依赖。测试只在单个 shell生命周期内临时链接现有 dependency trees，并以 `trap`
删除。`pnpm --dir router exec vitest list ...` wrapper没有产生 listing输出，因此不作为 selector
证据；实际命令为：

```text
router/node_modules/.bin/vitest list --root router \
  tests/protocol.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  -t 'connection request|connection response'
```

listing精确得到 `3` 个测试。随后：

```text
router/node_modules/.bin/vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-endpoint-connection-send-trust.test.ts \
  -t 'connection request|connection response'
```

结果为 `2 files passed`，`3 passed / 55 skipped`，两文件 inventory `58`。再运行同两个文件、不加
name filter，结果 `58/58 passed`。

`pnpm --dir router type-check` 使用临时
`router/node_modules -> /Users/geek/workspace/skiff/router/node_modules`，以及仅为提供已存在
`mongodb` package的
`node_modules -> /Users/geek/workspace/skiff/telemetry/node_modules`，结果 PASS。没有执行安装，两个
链接均已删除。

### 4.4 静态 checks

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

## 5. Scope expansion 与默认 fail-closed

当前可验证事实：

1. `NativeWebsocketCapability::request_json_to_connection` 的默认实现立即返回
   `Unsupported("...execution is not attached")`；未接 E0时不会悄悄发 frame。
2. `NativeWebsocketCapability::websocket_request_error_owner()` 默认返回 `None`。
3. capability即使返回五个 ordinary terminal之一，只要没有 exact owner，native error
   materialization就返回 `InvalidArtifact`，不会伪造成 platform builtin或错误 local `TypeAddr`。
4. wire/registry可独立验证，但 production native call仍不能同时获得 Host request future和
   caller-linked exact error owner。

最小后继写集建议归 E0 / linked-plan owner：

- `runtime/native-contract/src/call_plan.rs`：让 request native plan显式携带 exact named-union owner；
- `runtime/linked-type-plan/src/native_call_plan.rs`：从当前 linked program/executable解析
  `std.websocket.WebSocketRequestError` 的 exact `TypeAddr`，拒绝缺失/歧义；
- `runtime/native/src/dispatch/invocation.rs` 与
  `runtime/eval/src/native_invocation.rs`：把 owner随当前 call plan注入
  `RuntimeNativeInvocation`，由当前 invocation而非全局猜测提供 catch identity；
- `runtime/eval/src/capabilities.rs`、
  `runtime/host/src/capability_context/native_projection.rs` 与
  `runtime/host/src/eval_capability_adapter/{websocket.rs,factory.rs}`：把当前
  registry/session/cancellation/effective deadline接到已经冻结的 Host request future；
- 先由 E0 owner补齐上述 3 个
  `runtime/eval/src/{runtime_http_gateway.rs,runtime_websocket_connect.rs}` match arms，使 Host
  selectors能实际编译执行。

这组缺口不需要 Router broker/gateway/server；R0a/R0b仍应在各自后继写集中消费本文冻结的
`RuntimeEndpoint` API和 strict wire。

## 6. 反向搜索与范围审计

规定的两条 `rg` 已执行。逐层结论如下：

- `requestJsonToConnection` / `WebSocketRequestError`只出现在 std export、artifact signature、
  native contract/dispatch、exact model carrier及相应 tests；
- `connection.request` / `connection.response`的 production owners只在
  capability/request control、Rust transport/Host demux和 Router protocol/RuntimeEndpoint；
- Router没有新增 broker/gateway/server owner；
- `yield` 为 `ZERO_MATCHES`；没有引入语言关键字或 runtime frame；
- `may_suspend`证据保持新 request为 true、四个
  `sendTextToConnection` / `sendBinaryToConnection`及 business-identity raw sends为 false。

Implementation commit共修改/新增 `30` 个文件，全部位于任务唯一写集。没有修改
`runtime/eval`、`runtime/linked-type-plan`、Host request-entry/loader、Router
broker/gateway/server、fixture、test-runner、README、其它 task/result或权威设计。

未运行 complete verify、live、stable、instance、watch或 chat smoke；未派子 agent；未 merge、
rebase或 push。
