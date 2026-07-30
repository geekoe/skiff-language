# P5-F425A Skiff WebSocket authoring/artifact/compiler checkpoint result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 已完成 connect-only WebSocket 的 shared authoring、artifact、identity、compiler 与
deployment checkpoint。没有实现 Router wire/gateway、runtime connect execution、activation
outbound owner、fixture convergence，也没有访问 stable/live。

## 1. 精确输入与提交

| 项 | commit | tree |
| --- | --- | --- |
| 父节点指定 Skiff integration input | `a0cb0e18cf7df2bdbb7f90e0072cac62fe6164fa` | `6553e04a11ac3eedb056ca360575fcf47a9b1f34` |
| mechanical scope clarification | `98e680ce170b77a1a451f8883c6a661ba7db8868` | `8b826e45bd227fda973764d6c5d63b5b3fc629d4` |
| runtime consumer boundary clarification | `7ccb7716aaf659a41767ef4bf35afca2044df52c` | `ba99a7587ed2c652d709652d7fb2dcd6b0746003` |
| HTTP-only match clarification | `54b304d6ccfd42d2ad9ea9d9ae81429cb78aa920` | `afb988d9e92c128867584ee12d7508301a25acc0` |
| implementation | `986e6c4e69d93d2bd9f501b1fdf58661ad163bdd` | `06a53959bda638771c8bec0343659117164a4d63` |

三个 clarification commit 只闭合本 leaf 已授权的 exhaustive match、optional handler propagation
和 HTTP owner fail-closed 范围，没有改变冻结的 WebSocket 设计。

## 2. 实现结果

### 2.1 Strict singleton authoring

- `ServiceManifestAuthoring.websocket` 现在是严格的
  `Option<WebSocketGatewayEntryAuthoring>`，不再是任意 JSON。
- `host` 缺省为 `"*"`，`path` 必填，`connect` 可省略。
- connect 只接受 `handler` 和 `adapterArgs`；显式 `null`、list、named multi-entry map、缺少
  `path`/`handler`、duplicate/unknown field、`id`、`routes`、operation、receive、message 和 context
  shape 均 fail closed。
- compiler input 在进入 driver 前验证 host/path、callable selector、参数名唯一性和 source 闭集。

### 2.2 Fixed connect surface and std model

- artifact model 新增显式 wire kind `websocketConnect`、source
  `websocket.connectRequest` / `websocket.connectionId`、固定 v1 request/result/policy shape 与
  text/binary downlink frame 闭集。
- request/result/policy 不只验证 public type name；compiler 展开实际 schema，并与 canonical
  structural schema 精确比较，伪造同名或近似 shape 不能通过。
- `std/websocket.skiff` 只保留 connect request、connection policy、non-generic 且无 Context 的
  connect result、四个 send native 和两个 JSON helper。
- 已删除 public message/receive/connection/ingress union/close event 及其七个 `std/api.yml` export。
- connect result 只有 `accept`/`reject`；accept 只有可选 `businessIdentity` 和
  `connectionPolicy`。

四个 native 保持：

```text
sendTextToConnection(connectionId: string, payload: string) -> void
sendBinaryToConnection(connectionId: string, payload: bytes) -> void
sendTextToBusinessIdentity(businessIdentity: string, payload: string) -> void
sendBinaryToBusinessIdentity(businessIdentity: string, payload: bytes) -> void
```

其 canonical native summaries 仍为 `may_suspend = false`。

### 2.3 Exact callable and deployment projection

- HTTP projector 的 exact callable resolver 被最小抽取并由新 WebSocket projector复用。
- connect target 必须是 current package 的 exact private implementation symbol、
  `InternalFunction`、non-generic callable，并同时通过 exact callable link、file 和 semantic facts
  join。
- adapter args 必须按 callable signature 顺序一一覆盖所有参数；参数名不得重复或遗漏，source 只允许
  connect request/connection id。HTTP source、额外 source、wrong/nullable/generic return 全部拒绝。
- compiler-owned gateway entry key 固定为 `"websocket"`。path-only authoring生成 exactly one
  ingress binding 和一个 handler-absent gateway entry；connect authoring生成 exact callable target。
- `DeploymentGatewayEntry.handler` 改为 optional，但 invariant 是 HTTP 必须有 handler；
  WebSocket connect 才可无 handler，且无 handler时 adapter args 必须为空。
- HTTP 与 WebSocket selector/key collision fail closed；deployment projection 和
  `RuntimeAssembly.gateway_ingress` 都保留 exact key、entry identity 和 deployment join。
- WebSocket entry 不进入 `ServiceContract`、service operation 或 `ContractOperationId`；
  authoring变化不改变 `ServiceProtocolIdentity`，但会按既有规则改变 deployment revision、
  deployment identity 和 assembly identity。

### 2.4 Canonical identity

`GatewayEntryIdentity` 由固定 protocol surface 产生；内部 `WebSocketEntryId` 只由
`serviceId + compiler-owned GatewayEntryKey` 产生，没有复用旧 operation preimage。language-neutral
golden 为：

```text
gateway preimage:
{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"websocketConnect","surface":{"connectRequestShape":"v1","connectResultShape":"v1","connectionPolicyShape":"v1","downlinkFrames":["binary","text"],"externalSources":[{"kind":"websocket.connectRequest"},{"kind":"websocket.connectionId"}]}}}}

gateway identity:
skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936

websocket entry preimage:
{"gatewayEntryKey":"websocket","schema":"skiff-websocket-entry-identity-v1","serviceId":"example.com/chat"}

websocket entry id:
skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616
```

### 2.5 Mechanical downstream closure

- loader/linker/host 的 closed enum 和 optional handler consumers 已机械更新。
- Host assembly admission 仍显式拒绝 WebSocket surface；本 leaf 没有偷跑 connect execution。
- runtime eval/request 的 HTTP-only owners 对 `websocketConnect` 显式 fail closed；没有构造 connect
  request或执行 connect target。
- request 的 direct focused test已执行；eval 的 direct focused test已落地，但其 test binary 被
  下述禁止范围 D4 test-runner compile seam阻塞。production check通过。

## 3. 自验收矩阵

| 完成标准 | 状态 | 证据 |
| --- | --- | --- |
| path-only无 handler生成一个 exact binding | PASS | compiler WebSocket integration + deployment assembly projection test |
| private connect callable与两种 source成功 | PASS | exact resolver/link/facts及 signature-order正例 |
| malformed/legacy/multiple authoring拒绝 | PASS | artifact model与compiler input strict DTO负例 |
| generic/wrong/nullable signature与 HTTP source拒绝 | PASS | WebSocket projector mutation matrix |
| no-handler只对 WebSocket 合法 | PASS | artifact identity deployment validation正负例 |
| gateway/entry/deployment/assembly identity稳定 | PASS | canonical preimage golden与deployment/assembly tests |
| WebSocket不进入 ServiceContract/operation | PASS | contract normalization删除旧 builtin admission，compiler isolation test |
| ServiceProtocolIdentity不因 entry变化 | PASS | path-only/connect identity boundary test |
| 四个 send native signature与non-suspending summary不变 | PASS | compiler-published std exact signature test |
| authoring/compiler不再产生 receive/message surface | PASS | reverse search只剩 rejection assertions；production producer residue为零 |
| runtime legacy只保留D2 owner且producer不可达 | PASS | 第5节精确 allowlist；contract/compiler producer edge已移除 |
| Router/wire/runtime connect execution未越界 | PASS | 无 Router、wire、native execution或activation业务改动 |

## 4. 验证证据

### 4.1 Green focused verification

| 命令 / suite | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model -p skiff-artifact-identity -p skiff-compiler-input -p skiff-deployment` | PASS：model 172；identity 128 + CLI 8；input 90；deployment 61；另有1个明确 ignored regeneration test |
| `cargo test -p skiff-compiler --lib` | PASS：27 |
| changed compiler integrations：`builtin_canonical_spelling`、`compiler_owned_std_type_resolution`、`generated_service_deployment`、`http_gateway_projection`、`public_generic_schema_availability`、`websocket_ingress` | PASS：2 + 3 + 12 + 8 + 2 + 5 |
| compiler bin unit tests | PASS：5 |
| `cargo test -p skiff-compiler --test std_package_imports std_` | PASS：3 task-related filtered tests |
| `cargo test -p skiff-compiler-lowering -p skiff-compiler-projection -p skiff-compiler-source` | PASS：53 + 63 + 321 |
| `cargo test -p skiff-runtime-request` | PASS：36 |
| `cargo check -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-host` | PASS |
| `cargo check -p skiff-compiler -p skiff-runtime-eval -p skiff-runtime-request` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

### 4.2 Authoritative discovery not falsely reported green

任务给出的 combined command确实运行到了整个 `skiff-compiler` integration inventory。当前仓库存在
与本 leaf 无关的 latent/stale suites，因此没有把 combined command伪报为通过：

- `actor_dispatch_linking` 在机械更新已删除的 `global_ingress` field后进入执行，但现有 fixture返回
  `MissingHydratedSchemaIndex`；
- `prelude_std_schema` 仍预期 `Stream` 不可用，实际 current server-stream schema已可用；
- `root_path` / `runtime_slots` 存在既有 DB state、generic receiver 和 `std.http.json` fixture drift；
- `std_package_imports::user_packages_reject_native_declarations` 的 fixture缺少当前必需的 `api.yml`。

诊断这些 discovery failure时没有保留任何越界修复；只保留了授权范围内为 current field name所需的
`global_ingress -> gateway_ingress` mechanical test update。所有本 leaf 直接覆盖面均由4.1中的 green
suites独立执行。

`cargo test -p skiff-runtime-eval
runtime_http_gateway_refuses_websocket_connect_surface_before_execution` 在编译 test binary前被
`skiff-test-runner` 的 downstream optional-handler drift阻塞：

- `test-runner/src/canonical_test_gateway.rs:97` 仍把 handler赋成非 optional值；
- `test-runner/src/package_test_assembly.rs:238` 仍直接比较/格式化 optional handler。

`test-runner` 明确属于禁止写入范围和后继 D4 fixture/tooling convergence，因此本 leaf没有修改它。
对应 eval production package已通过 `cargo check`，HTTP fail-closed direct test源码已落地；这不是新增
设计 seam，也不解除 D1/D2/D3/D4 的依赖关系。

## 5. Reverse search与后继D2 legacy allowlist

对 `WebSocketIngressEvent`、`WebSocketReceiveEvent`、`ConnectionMessage`、
`WebSocketConnection<`、`WebSocketConnectResult<`、`receiveEvent`、`websocketReceive`、
`websocket.receive`、`contextCodec`、`contextPayloadPresent`、旧 operation/routes symbol和
Assembly WebSocket spelling执行了反向搜索。

结论：

- `std`、authoring、compiler lowering/projection和deployment producer中没有旧 receive/message/
  Context shape；
- compiler/artifact新文件中的旧 spelling只出现在 strict rejection assertions；
- test/fixture中的删除名单断言、HTTP owner的 `websocketConnect` fail-closed test和浏览器观测标签不构成
  producer；
- 下列 legacy consumer是父节点明确冻结给后继D2的精确 allowlist。它们已无法由 current
  authoring/compiler/contract/deployment producer生成，本 leaf没有越界删除。

| 层 | 精确D2-owned allowlist |
| --- | --- |
| compatibility artifact API | `artifact-model/src/websocket_ingress.rs`、其直接tests与`artifact-model/src/lib.rs` re-export |
| runtime boundary | `runtime/boundary/src/type_descriptor.rs`、`websocket_shape_descriptor.rs`、`service_value_plan/compile.rs`及其`json_convert`/service value plan直接tests |
| linked type plan | `runtime/linked-type-plan/src/type_plan.rs`、`websocket_shape.rs`、`websocket_shape_parity_tests.rs` |
| eval legacy consumer | `runtime/eval/src/assembly_execution/{websocket_contract_plan,websocket_identity,websocket_ingress,websocket_response}.rs`、`websocket_adapter.rs`，以及`invocation.rs`、`invocation_builder.rs`、`http_adapter.rs`中的legacy branches |
| request contract | `runtime/request-contract/src/{envelope,response_event}.rs` |
| request consumer | `runtime/request/src/assembly_ingress.rs`、`assembly_ingress/websocket_request.rs`、`eval_invocation_builder.rs`、`websocket_ingress.rs`及直接tests |
| old transport | `runtime/transport/src/{ingress_selector,protocol,request_mapper,response_mapper}.rs`及直接tests |
| host compatibility test owner | `runtime/host/src/eval_capability_adapter/request_contexts.rs`中的旧 receive target assertion |

runtime D2 allowlist之外没有同类 production projection owner；当前 `GatewayWebSocket*` 名称只描述新的
connect protocol surface，不是已删除的 `service_unit::GatewayWebSocket/GatewayWebSocketRoute`
authoring模型。

## 6. Scope与生命周期

- 没有修改 Router、current connect wire、runtime connect business execution、native outbound
  capability、activation owner、test-runner、Internals或skiff-packages。
- 没有 merge、rebase、push、stable/live/instance操作。
- 没有吞并后继D1 wire、D2 Runtime/Host consumer、D3 Router consumer或D4 fixture convergence。
- implementation提交后 worktree clean；本result文档单独提交。
