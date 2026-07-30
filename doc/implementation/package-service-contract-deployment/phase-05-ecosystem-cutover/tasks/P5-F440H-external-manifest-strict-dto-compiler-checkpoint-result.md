# P5-F440H external manifest strict DTO/compiler checkpoint result

状态：`M0_IMPLEMENTATION_PASS / EXPECTED_M1_BLOCKED`。M0 已完成 strict DTO、三文件 reader、
root inventory、compiler projection 与 generated deployment producer；没有触发
`TASK_SCOPE_EXPANDED`。新增 artifact vocabulary 按任务要求使尚未迁移的 M1
artifact-identity/deployment follower 在 exhaustive match 处停止。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 固定 implementation 输入 | `a829bde6d250cd348a28f25c6246de6cbed2df9e` | `7d875d29a0b00fb93c618f4ff08ec2e381c11d60` |
| leaf dispatch 基线 | `e01fa01d624929bd73c163fe0ec6168f91438b91` | `a75ee80da0cb9993671f512f2bb41a720527abbb` |
| M0 implementation | `8933e08f722c0a201ade6c444834ad360f97ac87` | `9b418a4a59bf12783f26c0e4949c12fe687df827` |

implementation 只修改任务声明的 M0 写集：

- `artifact-model/src/{ecosystem_authoring,gateway,schema,lib}.rs`
- `compiler/input/src/{service_config,lib}.rs`
- `compiler/driver/authoring.rs`
- `compiler/driver/generated_deployment.rs`
- `compiler/driver/http_gateway_projection/{mod,schema}.rs`
- `compiler/driver/websocket_gateway_projection.rs`
- `compiler/tests/{http_gateway_projection,websocket_ingress,generated_service_deployment}.rs`

本文是独立 result-only 提交；其 commit/tree 由最终交付消息记录。

## 2. Test-first red

首次加入 `service_manifest_rejects_inline_external_fields` 后运行：

```bash
cargo test -p skiff-artifact-model service_manifest_rejects_inline_external_fields
```

得到真实 `1 run / 1 failed`：旧 `ServiceManifestAuthoring` 仍接受
`service.yml` 内联 `http: {}`。实现后该 case 随 authoring focused suite 转绿。

## 3. Strict DTO 与 reader

- `service.yml` 现在只拥有 `id`、`kind`、`serviceCalls`；`http`、`websocket`、`timeout`
  均 fail closed。
- `http.yml` 是 entry-key 直接映射；`{}` 合法，empty/null/scalar/list/wrapper、重复 key/field
  和旧 `routes`/`entries` 形状失败。
- `websocket.yml` 顶层只允许 `path`、可选非 null `connect` 和 `jsonRpc`；文件存在时即使只有
  `path` 也合法。
- JSON-RPC key 与 external method 分别唯一；空 method、`$/` 前缀、重复 method、未知/重复
  handler field、guard/pre、transport request id、错误 phase source 均失败。
- adapter args 必须唯一并完整覆盖 linked handler formal；`websocket.jsonRpcParams` 恰好一次，
  connection id/business identity 各至多一次。
- `service_config.rs` 是 `service.yml`、`http.yml`、`websocket.yml` 的唯一 reader owner。
  `ServicePackageRoot` 分开保存三份 typed authoring。
- external file 与 `package.yml`、`api.yml`、`service.yml` 都必须是同 root regular file；
  authoring driver 在 package-only 分支前盘点，external-only、ordinary package + external、
  缺 API/service 和目录/符号链接形状都 terminal 失败。
- `PackageCompileInput`、`PackageSourceInput` 和 package source/resource graph 没有新增 external
  manifest 字段。

## 4. Compiler projection 与 deployment producer

- HTTP projection只接收独立 `HttpGatewayDocumentAuthoring`。
- 每个合法 `websocket.yml` 恒生成 compiler-owned 物理 entry
  `GatewayEntryKey("websocket")`；kind 为 `websocketConnect`，connect handler 可空，selector
  为 WebSocket + host `*` + path + `method=None`。
- 物理 connect surface 增加
  `rpcProfiles: ["jsonrpc-2.0-text"]`，保留 connect request/result/policy v1、固定 source 和
  text/binary downlink classes。
- 每个 `jsonRpc` record 独立生成 `websocketJsonRpc` entry；selector 复用物理 host/path，
  `method=Some(external method)`。
- method protocol surface 包含 closed profile、unary dispatch、按 wire name canonical
  sort/dedup 的实际 source kind 集合、params schema 与 result schema。
- params 使用 linked formal 的 executable external-schema projection；所有合法顶层值必须是
  object/array（record、array或只由两者组成的 closed union），nullable/scalar/untyped/generic
  失败。return 必须可投影，`Stream<T>` 失败，`void` 精确投影为 `Null`。
- `websocket.connectionId` 要求精确 builtin `string`；
  `websocket.businessIdentity` 要求精确 nullable builtin `string?`。
- HTTP、WebSocket method 和 compiler-reserved `websocket` key 共用一个冲突域。HTTP 即使没有
  `websocket.yml` 也不能占用保留 key；跨 `http.yml`/`websocket.yml` 重复 key 在 merge 时失败。
- external method 不进入 method gateway identity surface，但进入 deployment ingress selector。
  method rename test固定了 gateway identity不变、selector/revision改变。
- `GeneratedServiceDeploymentInput` 分开接收 service/http/websocket；canonical generated revision
  包含三份 typed authoring。真实 external-file mutation 证明 PackageArtifact 与 ServiceContract
  exact bytes不变，而 deployment revision改变。
- `DeploymentPolicy.timeoutMs` 继续只读取 `config.<profile>.yml` 的 scalar `timeout`。

F440B §4.1–4.4 补充冻结项已逐条核对：物理 entry、RPC profile、method surface、三类 key 冲突、
structured params 与 method identity/selector 边界均已落在本 M0 写集。

## 5. Artifact vocabulary 与 schema generation

新增/改变的 strict vocabulary：

- `GatewayAdapterKind::WebSocketJsonRpc`，wire `websocketJsonRpc`
- `GatewayAdapterSource::WebSocketJsonRpcParams`，wire `websocket.jsonRpcParams`
- `GatewayAdapterSource::WebSocketBusinessIdentity`，wire `websocket.businessIdentity`
- `GatewayWebSocketRpcProfile::JsonRpc2_0Text`，wire `jsonrpc-2.0-text`
- `GatewayWebSocketConnectProtocolSurface.rpc_profiles`
- `GatewayWebSocketJsonRpcProtocolSurface`
  （`profile/dispatchMode/externalSources/paramsSchema/resultSchema`）
- `GatewayProtocolSurface::WebSocketJsonRpc`

wire shape 已改变，因此 M0 将：

- `SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION` 从 v3 提升到 v4；
- `SERVICE_DEPLOYMENT_SCHEMA_VERSION` 从 v2 提升到 v3。

PackageArtifact、ServiceContract 及其 schema generation保持不变。

## 6. M0-owned 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model ecosystem_authoring` | PASS，`8 passed / 0 failed` |
| `cargo test -p skiff-artifact-model gateway::tests::` | PASS，`7 passed / 0 failed` |
| `cargo test -p skiff-artifact-model schema::tests::suspension_schema_generations_are_atomic_and_unrelated_domains_remain_stable` | PASS，`1 passed / 0 failed` |
| `cargo check -p skiff-artifact-model` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

已实际运行的 M0-owned focused tests 合计 `16 passed / 0 failed`。

## 7. 精确 M1 blocker

以下四条命令都在同一 M1 owner 编译边界停止，尚未进入 compiler-input/compiler test body：

```bash
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-compiler --test http_gateway_projection --test websocket_ingress --test generated_service_deployment
cargo check -p skiff-compiler-input
cargo check -p skiff-compiler
```

四个命令的 blocker 均为同一组四个 `E0004`，没有用 wildcard 或 compatibility alias 掩盖：

| 文件 | 行 | 未消费 vocabulary |
| --- | ---: | --- |
| `artifact-identity/src/deployment/validation.rs` | 277 | `&GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs` | 54 | owned `GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs` | 347 | `&GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs` | 397 | `GatewayAdapterKind::WebSocketJsonRpc` |

这是任务预先声明的 expected M1 blocker，不是 M0 scope expansion。

M1 必须显式消费：

1. 上述新 adapter kind、两个新 source，以及现有 `WebSocketConnectionId` 在 JSON-RPC phase 的合法性；
2. connect `rpcProfiles` 的 closed singleton、canonicalization 与 validation；
3. JSON-RPC profile、unary dispatch、canonical source set、structured params schema、result schema
   和 fixed external error projection；
4. WebSocket ingress `method=None` 只能绑定物理 connect entry，
   `method=Some` 只能绑定 JSON-RPC method entry，且同 service method共享物理 WebSocket entry id；
5. method string/key/handler/formal 名与顺序不进入 gateway identity，method selector与完整 adapter
   plan仍进入 deployment identity；
6. deployment input v4、deployment v3 strict loader/validation。

因为既有 connect identity preimage新增必填 `rpcProfiles`，且 deployment接受了新的 entry/selector
语义，M1 还必须显式刷新 GatewayEntry 与 DeploymentArtifact identity marker/prefix generation及其
fixtures；不能让旧 marker解释新 preimage。RuntimeAssembly/loader如因新 ingress语义改变，也应由
对应 follower显式决定并刷新其 generation。Package/Contract marker不应改变。

## 8. 反向搜索与隔离

- 权威命令
  `rg -n 'service\.(http|websocket)|ServiceManifestAuthoring.*(http|websocket|timeout)' artifact-model compiler`
  为 0 命中。
- `WebSocketGatewayEntryAuthoring|deserialize_http_gateway_entries` 为 0 命中；没有兼容 alias。
- removed vocabulary `handlerArgs`、`websocketReceive`、`websocket.message`、
  `websocket.requestId` 的剩余命中全部是 strict negative test/source-text fixture，没有 production
  ownership。
- package input/source owner反搜 `HttpGatewayDocumentAuthoring|WebSocketGatewayDocumentAuthoring`
  为 0 命中。
- 未修改 artifact-identity、deployment、Router、Runtime、scripts/tooling、权威设计或 stable/live
  状态；未运行 instance/watch/reload/固定端口 workload。
- 未 merge、rebase、push；未派子 agent。implementation 提交后 clean；result 提交后的最终 clean
  状态由交付消息记录。
