# P5-F440Z3C Current GatewayEntry wire identity preflight result

状态：`PASS / ONE_ALL_BRANCH_V2_HARD_CUT_REQUIRED`。

本 leaf 已沿真实 Router producer、TypeScript lexical/metadata/schema、Rust transport、
request/eval/Host 与 cross-system corpus 完成只读审计。结论唯一：不能只 cut
`websocketConnect`。真实 current snapshot 对 HTTP、test-control、WebSocket connect 与
WebSocket JSON-RPC 全部提供 GatewayEntry v2；TypeScript 只有 JSON-RPC 已接受 v2，HTTP、
test-control 与 connect 仍卡在 v1。Rust transport 的共同 typed reader 已经对所有分支只接受
v2。

因此应以一个有界 checkpoint 一次 hard cut 全部 RuntimeAssembly gateway request branch，
同步更新 current-positive fixture/corpus；不得 dual read、不得在 Gateway 改写 prefix，也不得
把 HTTP/test 留在 v1。

## 1. 基线与审计边界

| 项目 | 值 |
| --- | --- |
| worktree | `/Users/geek/workspace/skiff-p5-f440z3c-gateway-wire-preflight` |
| branch | `codex/p5-f440z3c-gateway-wire-preflight` |
| 审计起点 | `3bfadf7e82c0a7a8b777202d3bd838db6514789a` |
| production/test 修改 | 无 |
| live/stable/network/完整 suite | 均未运行 |

只读取了 leaf、两个直接父 result 与 leaf 允许范围内的直接 owner。没有读取或修改其它
task/result。

## 2. 必须矩阵

| Request branch | TS producer identity | TS validator/schema | Rust consumer | Current 一致性 | Exact owner |
| --- | --- | --- | --- | --- | --- |
| HTTP unary/stream | **v2 candidate**。current deployment reader只接收并重算 v2，`assemblyHttpRequestHeader` 原样复制 `binding.gatewayEntryIdentity`；两种 mode 共用同一 producer。当前 validator 会在发送前抛错，所以真实 current path 不会产出 frame。 | lexical routing **只接收 v1**；HTTP metadata 不重复携带 identity；exported schema 对 routing identity **仅要求 string**，既不限定 v1 也不限定 v2。 | transport routing typed field经 `GatewayEntryIdentity::parse`，**只接收 v2**；Host、request target/execution再要求等于 current linked entry；eval只消费已 pin 的 current target。 | **断**：v2 producer 与 v1 TS lexical 无交集；Rust反而要求 v2。 | producer：`runtimeAssemblyDeploymentSnapshot.ts`、`assemblyHttpGateway.ts`；TS wire：`runtimeAssemblyRequest.ts`、`runtimeProtocol.ts`；Rust：`artifact-model/src/compile_identity.rs`、`runtime/transport/src/runtime_assembly_request*`、`runtime/host/src/host/request_entry/assembly_wire.rs`、`runtime/request/src/http_gateway_{target,execution}.rs` |
| WebSocket connect | **v2 candidate**。current physical binding只接受 v2；`assemblyWebSocketConnectRequestHeader` 把同一值写入 routing 与 `websocketConnect` metadata。当前 validator 在 upgrade 内抛错，Z3B 已真实观测 HTTP 500。 | routing lexical **v1**；connect metadata **v1 + 两字段相等**；schema 的 routing/metadata 两处均 **v1**。 | transport routing 与 metadata均为 typed `GatewayEntryIdentity`，**只接收 v2 + 相等**；Host/request/eval均与 current physical entry精确相等。 | **断**：TS 在 transport 前拒绝 current v2。 | producer：`runtimeAssemblyWebSocketSnapshot.ts`、`webSocketGateway.ts`；TS wire：三个 protocol 文件；Rust：transport、`assembly_wire.rs`、`websocket_connect_{target,execution}.rs`、`runtime/eval/src/runtime_websocket_connect.rs` |
| WebSocket JSON-RPC | **v2 emitted**。captured method table来自 current v2 method entry；bridge把同一 v2写入 routing 与 `websocketJsonRpc` metadata。 | lexical **v2**；metadata **v2 + 相等**；schema routing/metadata均 **v2**。 | transport **v2 + 相等**；Host按 pinned connection generation解析 current method route并校验 v2；request target重算 current method identity；eval不再重新解释 wire identity，只消费已 pin target。 | **一致**；但 production reachability 仍被前置 connect 500遮挡。 | producer：`runtimeAssemblyWebSocketSnapshot.ts`、`webSocketRpcBridge.ts`；TS wire：三个 protocol 文件；Rust：transport、`assembly_wire.rs`、`websocket_jsonrpc_{target,execution}.rs`、`runtime/eval/src/runtime_websocket_jsonrpc.rs` |
| test/runtime assembly | **v2 candidate**。`/__skiff/test-dispatch` 的 routing必须精确等于 active current binding；`assemblyTestHttpRequestHeader` 先构造同一 v2 production HTTP header，再只打开 test effects。当前 decode/header validation使它无法产出 frame。 | 与 HTTP 完全共用：routing lexical **v1**、无 identity metadata、schema仅 **string**。传 v2先被 lexical拒绝；传 v1即使过 lexical，随后又会因不等于 active v2 binding而拒绝。 | 与 HTTP 共用同一 v2 transport/Host/request/eval path；`testEffectsEnabled` 不改变 identity类型。 | **断且无可用值**：v1过 TS但不匹配 active binding，v2匹配 binding但过不了 TS。 | producer/入口：`assemblyControlPlane.ts`、`assemblyHttpGateway.ts`；TS wire：`runtimeAssemblyRequest.ts`、`runtimeProtocol.ts`；Rust owner与 HTTP相同 |

## 3. TypeScript 三层的精确现状

### 3.1 Lexical

`router/src/protocol/runtimeAssemblyRequest.ts:140-143,294-303` 定义 v1/v2两个 pattern，
但以 `wireKind === "websocketJsonRpc" ? v2 : v1` 分流。因此：

- HTTP unary/stream：v1；
- WebSocket connect：v1；
- WebSocket JSON-RPC：v2；
- test-control：它最终构造 HTTP branch，所以是 v1。

这也是只 cut connect 不成立的直接代码依据：HTTP 与 connect 当前处于同一个
“非 JSON-RPC 即 legacy v1”分支。

### 3.2 Metadata

`router/src/protocol/runtimeAssemblyRequestMetadata.ts:117-165` 对 connect metadata要求 v1，
并要求等于 routing identity；`:167-217` 对 JSON-RPC metadata要求 v2并要求相等。HTTP/test
没有第二个 identity metadata field。

### 3.3 Exported schema

`router/src/protocol/runtimeProtocol.ts` 的 `request.start` schema 当前为：

- HTTP routing复用 `requestStartFrameProperties.routing`
  (`:612-638,1525`)，`gatewayEntryIdentity` 只是 `type: "string"`；
- connect routing `:1612-1615` 与 connect metadata `:1691-1694` 均为 v1；
- JSON-RPC routing `:1747-1750` 与 metadata `:1786-1789` 均为 v2。

schema 与手写 lexical 并不等价：HTTP schema过宽，而 HTTP runtime validator仍只收 v1。
hard cut必须同时修 lexical、metadata与schema，不能只改报错前的一层。

## 4. Rust 已经是单一 current v2 owner

`artifact-model/src/compile_identity.rs:66,212-227` 把 canonical prefix固定为
`skiff-gateway-entry-v2:sha256`，typed parser不接受 v1。
`runtime/transport/src/runtime_assembly_request/lexical.rs:180-187` 直接复用该 parser；
HTTP routing、connect routing、JSON-RPC routing以及两个 WebSocket metadata字段都落到该
typed type。connect/JSON-RPC frame decoder随后还要求 metadata identity等于 routing
identity。

transport之后没有 compatibility seam：

- Host `runtime/host/src/host/request_entry/assembly_wire.rs` 对 HTTP/connect/JSON-RPC分别
  lookup current route，并将 wire identity与 admitted route精确比较；
- HTTP target与 WebSocket target用 `skiff-artifact-identity::gateway_entry_identity`
  重算 current surface identity；
- HTTP/connect request execution再次校验 wire/target equality；
- connect eval再次校验 request/target identity；
- JSON-RPC eval接收的是 Host已按 v2 identity与 pinned connection generation解析出的 target，
  不再读取原始字符串。

所以 Rust不需要 production 修改，也不存在可供 Router继续发送 v1的 consumer。

## 5. v1 inventory：positive、incidental 与 deliberate stale-negative

允许范围中另有 `45` 个 GatewayEntry v1 literal hit，分布于 `21` 个 test/corpus文件。按测试
意图分类后，现有明确 deliberate stale-negative 只有下面四个路径，其 v1应保留：

- `artifact-identity/src/tests/gateway.rs`：证明 v1 prefix与错误 diagnostic均被 current
  identity reader拒绝；
- `artifact-identity/src/tests/deployment.rs`：明确断言 stale gateway generation在 typed reader
  失败；
- `router/tests/filesystem-runtime-assembly-snapshot-loader.test.ts`：current filesystem reader的
  “GatewayEntry v1 identity” negative；
- `router/tests/runtime-assembly-websocket-rpc-snapshot.test.ts`：current WebSocket snapshot join的
  “GatewayEntry v1” negative。

其余 `17` 个 literal路径把 v1当作可构造/可发送的 current positive，或把 v1放在本应只测试
“identity mismatch / uppercase / short / 其它字段错误”的 fixture里。它们都必须改成 v2；
否则要么继续掩盖真实 current producer，要么在 Rust中先死于 typed parse，根本没有命中测试
声称的下游 invariant。

Router current-positive/incidental路径：

- `router/tests/assembly-http-gateway-stream.test.ts`
- `router/tests/assembly-replica-dispatch.test.ts`
- `router/tests/assembly-runtime-endpoint.test.ts`
- `router/tests/router-websocket-trust-dispatch.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`
- `router/tests/service-error-cross-layer-convergence.test.ts`
- `router/tests/websocket-gateway.test.ts`

Rust current-positive/incidental路径：

- `runtime/activation/src/tests.rs`
- `runtime/host/src/loader/active_assembly_context.rs`
- `runtime/package-test/tests/package_artifact.rs`
- `runtime/request/src/http_gateway_execution/tests.rs`
- `runtime/request/src/http_gateway_target.rs`
- `runtime/request/src/websocket_connect_execution.rs`

current-positive cross-system corpus：

- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`
- `cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json`

其中现有 corpus 的 v1 “uppercase/short/mismatched”与四个 raw negative都不是 gateway-generation
negative；v1只是旧 base或附带值。更新时必须先改为 v2以保留原测试维度，再各新增命名明确的
`stale GatewayEntry v1 generation` mutation。connect corpus应分别覆盖 routing v1与 metadata
v1，避免其中一层遮挡另一层。

另外以下文件没有 v1 literal，但把上述 corpus当作 current positive读取，属于必须验证的间接
consumer：

- `router/tests/protocol.test.ts`
- `router/tests/runtime-assembly-request-wire.test.ts`
- `runtime/transport/src/runtime_assembly_request/tests.rs`
- `cross-system-fixtures/package-service-ecosystem/verify.mjs`

`router/tests/runtime-protocol-websocket-response.test.ts` 虽读取同一个 connect JSON文件，但只消费
response cases；response不携带 GatewayEntryIdentity，不是本 hard cut写 owner。

## 6. 为什么必须一次 hard cut 全部 branch

1. current HTTP、test与connect producer都不生成 v1；它们只转发 current snapshot的 v2。
2. HTTP/test与connect的 lexical选择由同一个 `websocketJsonRpc ? v2 : v1`条件拥有。只修
   connect会人为制造第三套分流，却仍让真实 HTTP/test保持必断。
3. test-control要求 caller routing与 active binding精确相等，因此不存在“暂时传 v1”的合法
   workaround。
4. Rust共同 typed parser只接收 v2；即使绕过 TypeScript，把 v1发到 runtime也会在transport
   失败。
5. JSON-RPC已经是v2；只修connect会留下同一 `RuntimeAssemblyRequestStartFrameWireHeader`
   union内 HTTP/test与其余分支 generation不一致。

结论：hard cut所有 gateway request branch；不修改 producer，不做 prefix转换，不做dual read。

## 7. 精确 implementation 写集

### 7.1 Production（三文件，且仅三文件）

1. `router/src/protocol/runtimeAssemblyRequest.ts`
   - 删除 legacy pattern/条件分支；
   - HTTP、connect、JSON-RPC routing统一要求 current v2；
   - diagnostic统一声明 v2。
2. `router/src/protocol/runtimeAssemblyRequestMetadata.ts`
   - connect metadata改用 current v2 pattern；
   - 保留 routing/metadata exact equality。
3. `router/src/protocol/runtimeProtocol.ts`
   - HTTP routing schema从任意 string收紧为 v2 pattern；
   - connect routing与metadata两处 v1 pattern改为v2；
   - JSON-RPC v2 schema不变。

`assemblyHttpGateway.ts`、`assemblyControlPlane.ts`、`webSocketGateway.ts`、
`webSocketRpcBridge.ts` 与所有 Rust production已在正确地传递/消费 current identity，不应
修改。

### 7.2 Test 写集

把第 5 节九个 Router current-positive路径与六个 Rust current-positive路径的 v1改为 v2；
所有“wrong identity”仍使用**合法 v2、不同 digest**，避免把 mismatch test降级成 lexical
generation test。

另外修改：

- `router/tests/protocol.test.ts`：直接证明 HTTP/connect/JSON-RPC schema均接受v2、拒绝v1；
- `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`：connect current positive改
  v2，并显式保留 JSON-RPC stale-v1 negative。

`runtime-assembly-request-wire.test.ts` 与 Rust transport corpus harness可不改测试代码；更新
corpus mutation后它们会自动覆盖 HTTP/connect current v2与stale v1。四个现有 deliberate
stale-negative文件不改。

### 7.3 Cross-system fixture 写集

只有：

- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`
  - 三个 current HTTP header、所有非-generation raw/mutation附带 identity改v2；
  - uppercase/short mutation使用 v2；
  - 新增单独 stale-v1 generation mutation。
- `cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json`
  - canonical HTTP与两个 connect case的 routing/metadata/canonicalJson改v2；
  - mismatch mutation改为合法不同 digest的v2；
  - 新增 routing stale-v1与metadata stale-v1 mutations。

`verify.mjs` 当前按 production validator执行 corpus，内容无需改；若 F0希望加入显式的
“所有 positive均为v2且stale mutation存在”静态断言，那是可选 tooling增强。

## 8. 最早不受上游遮挡的直接探针

HTTP与test真实路径也已断，不只是connect：

- HTTP：在 `assemblyHttpRequestHeader` 直接传入一个 v2
  `RuntimeAssemblyIngressBinding`。当前最早失败点是该函数末尾的
  `validateRuntimeAssemblyRequestStartFrameHeader`，尚未进入 registry/dispatcher/runtime。
  现有 HTTP tests使用v1 binding，所以掩盖了真实 current reader。
- test header：对同一个 v2 snapshot/binding调用 `assemblyTestHttpRequestHeader`。它首先调用
  production HTTP header builder，并在同一 lexical gate失败。
- test endpoint：向 loopback `/__skiff/test-dispatch` POST与 active v2 binding完全相等的
  routing。当前更早在 `decodeRuntimeAssemblyTestDispatch` 的 header validation失败，尚未到
  `exactTestDispatchBinding`；改传v1则会在 exact binding comparison失败。
- connect：Z3B已经用真实 loopback证明 v2 candidate在
  `assemblyWebSocketConnectRequestHeader` validation失败并表现为 upgrade HTTP 500。

这四个探针共同证明错误不在 selector、replica选择或Rust runtime。

## 9. 有界 checkpoint、Z3B与F0

实现可以并且应当作为**一个有界、单一 invariant checkpoint**完成：3个production owner、
current-positive test fixture、2个共享 corpus与direct non-live验证。没有Rust production
改动，也没有wire adapter，范围明确。

Z3B恢复前的最小**可验收** checkpoint就是该完整 hard cut。仅提交三个production文件虽然足以
让真实 connect candidate通过TypeScript，但会立即让现有v1 positive Router/corpus tests变红，
不能作为绿色基线交给Z3B。

因此两个 corpus不是可延后的“纯装饰 fixture”。若组织归属强制由F0写cross-system文件，最小
顺序DAG只能是：

```text
all-branch TS v2 hard cut + local positive fixture refresh
  -> F0: two cross-system corpus refresh + parity verification
  -> Z3B restore
```

该拆法的中间节点会有已知 corpus consumer RED，只有F0完成后才形成 checkpoint；没有并行
安全性收益。推荐同一 checkpoint完成。真正可留给F0的只有可选 `verify.mjs`静态generation
断言、README/checker等纯 tooling/documentation跟随项；它们不应再改变wire fixture语义。

## 10. 必跑 non-live 命令

Router direct：

```bash
router/node_modules/.bin/vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts \
  tests/router-websocket-trust-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts \
  tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
  tests/websocket-gateway.test.ts \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts
pnpm --dir router type-check
```

Rust typed reader与被刷新 fixture：

```bash
cargo test -p skiff-artifact-identity gateway_identity_marker_parser_and_preimage_match_exact_golden
cargo test -p skiff-artifact-identity deployment_identity_is_stable_under_reorder_and_rejects_stale_generation
cargo test -p skiff-runtime-transport runtime_assembly_request
cargo test -p skiff-runtime-request runtime_http_gateway
cargo test -p skiff-runtime-request websocket_connect_request
cargo test -p skiff-runtime-activation activation_context_websocket_entry_is_typed_optional_and_matches_all_exact_facts
cargo test -p skiff-runtime-package-test --test package_artifact entrypoint_validation_rejects_non_exact_gateway_facts
cargo test -p skiff-runtime-host websocket_admission_rejects_gateway_identity_and_surface_mismatch
```

Cross-system与静态检查：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs
git diff --check
```

均为本地、non-live、非完整suite。完成该 checkpoint后再恢复Z3B规定的真实 loopback Gateway
tests；无需启动stable instance、watch、MongoDB或外部network。

## 11. Scope确认

- 没有修改 production、test、fixture、schema、README/checker或其它task/result；
- 没有运行stable、network、live、server或完整suite；
- 没有派子Agent；
- 没有 merge、rebase或push；
- 本 result之外无写入。
