# P5-F440T Inbound runtimeAssembly WebSocket RPC wire

状态：Ready。E0前置的窄W0 shared checkpoint；只实现Rust/TypeScript成对wire。

## 直接父节点

- `P5-F440S-runtime-websocket-rpc-execution-preflight-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md` §8
- `P5-F440P-websocket-rpc-transport-checkpoint-result.md`
- `P5-F440Q-websocket-rpc-invocation-linkage-result.md`

F440S证明inbound runtimeAssembly WebSocket JSON-RPC request/response DTO缺失。F440B冻结exact shape与
outcome；F440P/Q冻结的是反方向`connection.*`，本leaf不得修改。

实现基线为`e52edb12ea7951d8e9a09595b89a8fa5d26133a3`。

## 目标

Rust与TypeScript成对增加strict sibling：

```text
request.start.runtimeAssembly.websocketJsonRpc
response.end.runtimeAssembly.websocketJsonRpc
```

使transport能够无损携带opaque params/result与exact routing/pin metadata。只建立DTO、
decoder/encoder与parity tests；不解析业务JSON、不查handler、不执行runtime、不接broker/gateway。

完成后解除E0；它不是可执行RPC候选。

## 唯一写集

Rust：

- `runtime/transport/src/runtime_assembly_request.rs`
- `runtime/transport/src/runtime_assembly_request/{lexical.rs,tests.rs}`
- `runtime/transport/src/response_mapper.rs`
- `runtime/transport/src/response_mapper/tests.rs`
- 上述enum新增variant所需的同crate机械exhaustive match

TypeScript：

- Router现有runtimeAssembly request/response DTO、parser/encoder module
- `router/src/protocol/{envelope.ts,runtimeProtocol.ts}`中本wire的exact type/mirror
- `router/tests/protocol.test.ts`及可新增一个只测runtimeAssembly JSON-RPC wire的direct test
- TypeScript enum新增variant所需的同protocol层机械exhaustive match

本leaf result。

禁止修改artifact schema/identity、outbound `connection.*`、runtime request/eval/Host、Router broker/
RuntimeDispatcher/gateway/server、fixture、README、其它task/result。不得派子Agent，不得启动server/live。

## Request shape

新增request必须是unary runtimeAssembly sibling：

```text
routing.kind = runtimeAssembly
routing.assemblyIdentity / assemblyGeneration = captured pin
routing.gatewayEntryIdentity = exact method entry
routing.ingress.host / path = pinned physical selector
routing.ingress.protocol = webSocket
routing.ingress.method = exact external method (non-null)
mode = unary

websocketJsonRpc.profile = jsonrpc-2.0-text
websocketJsonRpc.connectionId = bounded canonical connection id
websocketJsonRpc.websocketEntryId = exact physical entry id
websocketJsonRpc.gatewayEntryIdentity = exact method entry identity
websocketJsonRpc.businessIdentity = optional bounded string

payload = required opaque params JSON bytes
```

DTO/decoder必须：

- 以`protocol=webSocket`且`method=Some`选择JSON-RPC sibling；`method=None`仍是connect；
- 要求top-level与nested object deny unknown fields；
- 要求mode unary；
- 要求routing gateway identity与nested identity精确一致；
- 验证canonical identity/id/profile/method及已有payload size/presence限制；
- payload必有但保持opaque bytes；transport不解析object/array，profile与E0负责防御性shape校验；
- HTTP/connect既有shape与identity不变。

transport id、peer raw socket id、Router request id不得进入业务payload。

## Response shape

新增response end metadata：

```text
websocketJsonRpc.outcome =
  success | invalidParams | internalError | deadlineExceeded
```

规则：

- `success`必须payload present，JSON `null`也算；
-其它三种outcome必须payload absent；
- 不存在`cancelled` outcome；
- nested/top-level deny unknown；
- response/request correlation复用既有request id，不新造第二套id；
- fixed failure message/stack/error type不得进入wire。

Rust encoder与TS decoder、TS encoder与Rust decoder（若现有双向seam支持）必须对同一shape接受/拒绝。至少用
共同golden/mutation矩阵证明payload presence、unknown field、method null/string分流与outcome闭集一致。

## Test-first与验证

先增加strict positive/mutation tests，使旧decoder因未知variant或把method-bearing WebSocket误判为connect
而失败。至少覆盖：

- canonical request round-trip；
- method `None`仍connect、method `Some`只JSON-RPC；
- wrong mode/profile/identity mismatch/missing payload/unknown field拒绝；
- success含`null`payload合法；
- success缺payload及error outcome带payload拒绝；
- unknown/`cancelled` outcome拒绝；
- HTTP/connect和F440P `connection.*` corpus不回归。

必跑：

```bash
cargo test -p skiff-runtime-transport runtime_assembly_websocket_jsonrpc
pnpm --dir router exec vitest list --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts
pnpm --dir router exec vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts
pnpm --dir router type-check
cargo fmt --all -- --check
git diff --check
```

若未新增专用TS文件，从命令删除不存在路径并记录实际non-zero test count。pnpm wrapper若误解析root，按
F440P/F440R使用现有Vitest binary。

Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若request/response shape需要改变artifact surface、outbound wire或Router broker action，返回
`TASK_SCOPE_EXPANDED`；不得吞入E0/R0b。若F440B shape无法唯一映射current envelope，返回
`TASK_NOT_EXECUTABLE`并给出唯一冲突。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440t-inbound-rpc-wire`
- branch：`codex/p5-f440t-inbound-rpc-wire`
- result：`P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`

Implementation与result分开提交；不merge/rebase/push。
