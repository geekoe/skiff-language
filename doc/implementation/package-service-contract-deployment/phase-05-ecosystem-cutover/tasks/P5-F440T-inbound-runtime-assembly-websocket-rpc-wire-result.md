# P5-F440T Inbound runtimeAssembly WebSocket RPC wire result

状态：`PASS / W0_SCOPED_WIRE_VALID`。

Rust 与 TypeScript 已成对增加 strict transport sibling：

```text
request.start.runtimeAssembly.websocketJsonRpc
response.end.runtimeAssembly.websocketJsonRpc
```

本节点只建立 DTO、method-null/string 分流、encoder/decoder、payload
presence/size gate 与 parity tests。没有解析 params/result 业务 shape，没有查 handler、执行 runtime，
也没有接入 Host、RuntimeDispatcher、broker、gateway/server 或 outbound `connection.*`。
TypeScript 将新 request 暴露为 transport-only union；现有 executable dispatch candidate union保持不变，
因此本节点不会把 W0 wire 误宣称为 E0/R0b 可执行候选。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `e52edb12ea7951d8e9a09595b89a8fa5d26133a3` | `514c74fa27ce8d86785d98184bfc7df91467da6b` |
| worktree 实际起点 | `f13da626be26e283a4892514f3c76b2aeb24fdbf` | `6e200f6a77892fb5b6d0f6e4275b900b01a4fce4` |
| implementation | `d70892804b391faf4731032e7933fac3c0328184` | `0c15d7ba8b2f17fcd0ae2423d7879fd383119d7c` |

`f13da626` 直接基于任务声明的 `e52edb12`，中间只增加本任务文档，没有 production/test
变化。Implementation 与本文 result 分离提交；result commit/tree 由最终交付消息记录。

## 2. Rust wire

### 2.1 Request

`runtime/transport/src/runtime_assembly_request.rs` 的 closed wire enum新增
`WebSocketJsonRpc`。decoder 先读取：

```text
routing.ingress.protocol
routing.ingress.method
```

并精确分流：

- `http` -> 既有 HTTP；
- `webSocket + method=null` -> 既有 `websocketConnect`；
- `webSocket + method=string` -> 新 `websocketJsonRpc`；
- 其它 method shape拒绝。

新 request DTO要求：

- `mode=unary`、`caller.kind=gateway`、`routing.kind=runtimeAssembly`；
- canonical runtime assembly / gateway entry / WebSocket entry identity；
- method entry `routing.gatewayEntryIdentity` 与
  `websocketJsonRpc.gatewayEntryIdentity` 精确一致；
- exact `jsonrpc-2.0-text` profile；
- canonical ASCII connection id，长度 `1..=255`；
- method UTF-8 bytes `1..=256`；
- canonical request id与可选 business identity分别不超过 `1024` bytes；
- top-level、routing、ingress、caller与 `websocketJsonRpc` nested object均 deny unknown。

payload 必须非空且不超过既有 `CONNECTION_REQUEST_MAX_PAYLOAD_BYTES`（1 MiB）。decoder只搬运原始
bytes，不解析 JSON object/array；测试用 JSON scalar `42` 证明 transport 不夺取 profile/E0 的业务
shape owner。

### 2.2 Response

Rust 新增 closed outcome：

```text
success | invalidParams | internalError | deadlineExceeded
```

header wire固定为：

```text
schemaVersion
type=response.end
requestId
payloadPresent
websocketJsonRpc.outcome
```

`success` 派生并要求 `payloadPresent=true` 且实际 payload非空；`null` 的四个 bytes是合法 payload。
其它三个 outcome派生并要求 `payloadPresent=false` 且实际 payload为空。payload同样受 1 MiB gate；
transport不解析 result。unknown outcome、`cancelled`、header/payload presence不一致、nested/top-level
unknown field均拒绝。

`response_mapper.rs` 提供
`runtime_assembly_websocket_jsonrpc_response_into_frame`，构造 exact schema/type/request correlation，
并在编码前重新走 typed header validation。固定 failure message、stack、error type没有进入 DTO。

## 3. TypeScript mirror

Router protocol层新增：

- `RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader`；
- transport-only `RuntimeAssemblyRequestStartFrameTransportWireHeader`；
- `RuntimeAssemblyWebSocketJsonRpcResponseEndFrameHeader`；
- request/response explicit encoder与strict decoder；
- `runtimeProtocol.ts` request/response declarative schema branches与imperative exact validators。

TypeScript 与 Rust 使用同一限制和分流：

- method `null|string`决定 connect/JSON-RPC sibling；
- v2 method gateway entry identity与nested identity exact match；
- canonical ids/profile/method bounds；
- request required payload、response outcome/payload presence、1 MiB limit；
- Unicode control、unknown field、explicit-null optional field、unknown/`cancelled` outcome拒绝。

现有 `RuntimeAssemblyRequestStartFrameWireHeader` 仍只表示当前可执行 HTTP/connect候选；新
`TransportWireHeader` 只由 protocol validator与frame encoder/decoder返回。这样
`pnpm --dir router type-check` 可以在不修改 `RuntimeDispatcher`/gateway execution owner的前提下通过，
且 R0b 仍必须显式接入新 sibling。

## 4. Test-first RED

production实现前先增加真实 Rust decoder positive：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-transport \
  runtime_assembly_websocket_jsonrpc_decoder_accepts_method_bearing_request \
  -- --nocapture
```

结果：exit `101`，`1 failed / 86 filtered`。旧 decoder 把
`protocol=webSocket + method="status.get"` 送入 `websocketConnect`，精确报：

```text
invalid type: string "status.get", expected unit
```

这是 decoder runtime assertion RED，不是零测试、测试自身 compile error或依赖遮罩。

## 5. 规定 GREEN

所有 Cargo命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-transport runtime_assembly_websocket_jsonrpc` | 8 | PASS：8 passed / 86 filtered；integration binary 0 executed / 2 filtered |
| task原样 `pnpm --dir router exec vitest list --root router ...` | 0 | wrapper exit 0但无 listing，未计为证据 |
| `router/node_modules/.bin/vitest list --root router tests/protocol.test.ts tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts` | 57 | PASS：listing精确 57 |
| `router/node_modules/.bin/vitest run --root router tests/protocol.test.ts tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts` | 57 | PASS：2 files，57/57 |
| `pnpm --dir router type-check` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

Router worktree无依赖安装。验证时临时链接
`router/node_modules -> /Users/geek/workspace/skiff-phase-05-integration/router/node_modules`，完成后已删除；
未把 dependency tree或 symlink提交。

## 6. 补充 non-live 回归证据

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-transport connection_ --no-fail-fast` | 6 | PASS：F440P `connection.*`/send corpus 6/6 |
| Router `runtime-assembly-request-wire.test.ts` + `runtime-protocol-websocket-response.test.ts` | 110 | PASS：HTTP/connect request与connect response 2 files，110/110 |
| `cargo check -p skiff-runtime-transport` | — | PASS |

另运行 Rust旧 shared current-wire selector时得到 `2 passed / 1 failed`。失败发生在本节点新增
JSON-RPC branch之前的 existing HTTP case：fixture仍写
`skiff-gateway-entry-v1`，而 baseline current `GatewayEntryIdentity::parse` 已只接受 v2。任务禁止修改
fixture/artifact identity，也要求 HTTP/connect identity不变，因此本节点没有用 compatibility
parser、fixture rewrite或 identity dual-read掩盖该既有不一致。新 v2 connect method-null探针与所有规定
selector均通过；该诊断不属于本任务 mandatory selector。

没有运行 complete verify、live、stable instance、watch、server或 chat smoke。

## 7. 自验收矩阵

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| method null/string精确分流 | Rust enum custom deserialize；TS `runtimeAssemblyRequestWireKind` | Rust disjoint sibling test；TS direct disjoint test |
| unary/profile/identity exact | Rust lexical/typed DTO + decoder cross-field check；TS routing/metadata validators | shared request mutation matrix |
| payload opaque且有界/present | Rust decoder 1 MiB gate；TS frame encoder/decoder gate | object与scalar byte round-trip、missing/oversize拒绝 |
| success含 `null` 合法 | response outcome/presence validator | Rust mapper/decoder与TS encoder/decoder |
| 三 failure无 payload | closed four-outcome enum/union | 三 outcome positive + payload mismatch negatives |
| unknown/`cancelled` 拒绝 | serde deny unknown/closed enum；TS exact field/outcome validator | Rust/TS response mutation matrix |
| HTTP/connect与F440P不回归 | existing code未改语义；TS executable union保持窄 | TS 110/110；Rust `connection_` 6/6；new v2 connect probe |

## 8. 范围与反向审计

Implementation提交共修改/新增 `13` 个文件，全部位于任务唯一写集：

- Rust transport request/lexical/tests、response mapper/tests；
- Router protocol DTO/parser/encoder/schema与 protocol tests；
- 一个只测本 wire 的 direct TS test。

`git diff -U0` 对新增行反向搜索确认没有新增：

- outbound `connection.request` / `connection.response` shape；
- RuntimeDispatcher、broker、gateway/server action；
- runtime request/eval/Host/loader；
- artifact schema/identity、fixture、README或其它 task/result。

没有业务 JSON parser、handler lookup、runtime execution、第二套 correlation id、fixed failure detail或
peer raw socket id进入新 DTO。未派子 Agent；未 merge、rebase或 push。
