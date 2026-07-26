# P5-F351 HTTP gateway artifact model / identity result

状态：Completed（HTTP/shared foundation only；未接线consumer，未运行stable/live，未push）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| 本leaf base | `7acbd3051e2d57f7d7e028f1bd48c1567ea4dcdd` | `9e50f968a51a173c216bdc2af734a8ec9ae9685c` |
| 最新只读integration authority | `6faa3abf7298fc7468ba189b1c89a808d72db817` | `42f026f13c03470455825ffc64cf200114a89b85` |
| production/tests | `d4d114a5878e56715601d36c53949f4c27f7df41` | `25ac9476117774a4c6517a91803fad0e0c5c48cc` |

工作分支为`codex/p5-f351-gateway-model`，worktree为
`/Users/geek/workspace/skiff-p5-f351-gateway-model`。只读integration authority中的
`P5-H36`与本task已完整复核；没有把该integration commit merge/rebase进leaf。

## 2. 最终shared DTO

`artifact-model`新增以下唯一shared vocabulary：

- `GatewayEntryKey`：service-owner-local opaque key；只能经validated parser/strict
  deserialization构造，拒绝空值、空白和control character。
- `GatewayEntryIdentity`：只能接受
  `skiff-gateway-entry-v1:sha256:<64 lowercase hex>`；与key为不可互换的独立类型。
- `GatewayAdapterKind`：closed `typedJson | rawHttp`。
- `GatewayAdapterSource`：closed `http.request | http.body | http.context`。
- `GatewayAdapterArg { param, source }`：deny unknown fields；validator拒绝空白/重复param、
  adapter/source错配及没有pre的`http.context`。
- `GatewayExternalSchema`：closed
  `null/string/number/integer/boolean/bytes/array/record/closedUnion/nullable/stringLiteral`；
  record使用`BTreeMap`，没有nominal、package、path、codec或untyped JSON escape field。
- `GatewayHttpProtocolSurface`：
  `adapterKind + dispatchMode + externalSources + requestBodySchema + responseSchema +
  streamItemSchema`。
- `GatewayProtocolSurface`：当前只有显式`http`variant；没有unknown、future或WebSocket
  placeholder。
- `GatewayEntryProtocolSurface`：上述HTTP surface加closed
  `externalErrorProjection = { kind: fixed, version: v1 }`。

`http.context`保留在execution-plan source vocabulary中，但不是external protocol source，不能进入
identity surface。Selector、entry key、target参数名、handler/pre/guard target、package/build、
deployment及context codec在projection类型中均无字段。

## 3. Canonical identity

唯一identity owner为`artifact-identity`：

- schema marker：`skiff-gateway-entry-identity-v1`
- prefix：`skiff-gateway-entry-v1:sha256`
- preimage：private `GatewayEntryIdentityProjection { schema, surface }`
- algorithm：canonical JSON bytes -> SHA-256 -> framed validated identity

HTTP typed unary golden preimage为：

```json
{"schema":"skiff-gateway-entry-identity-v1","surface":{"externalErrorProjection":{"kind":"fixed","version":"v1"},"protocol":{"kind":"http","surface":{"adapterKind":"typedJson","dispatchMode":"unary","externalSources":[{"kind":"http.body"},{"kind":"http.request"}],"requestBodySchema":{"fields":{"query":{"kind":"string"},"requestId":{"kind":"string"}},"kind":"record","required":["query","requestId"]},"responseSchema":{"branches":[{"kind":"stringLiteral","value":"accepted"},{"kind":"stringLiteral","value":"ok"}],"kind":"closedUnion"},"streamItemSchema":null}}}}
```

对应identity：

```text
skiff-gateway-entry-v1:sha256:a24d48c28b531ef534b0ffcbff94554c505caab62f0a9de1cd47c4ab0ec4f685
```

Producer normalizer按wire name排序并去重external source、递归规范schema、排序required与union
branch、展开nested union并统一nullability。重复union branch、重复required与缺失required field直接
拒绝，不作修复。Loaded artifact validation重新规范后要求与输入逐值相同，因此非canonical list顺序、
重复source及冗余nullable/union不会静默产生另一identity。

## 4. Validation与identity matrix

测试覆盖并通过：

- key/identity空白、错误prefix、非64位或非小写hex；
- unknown field/kind及任何非HTTP adapter/source，包括WebSocket connect/receive/message raw
  string；
- typed HTTP缺body/response schema，raw HTTP伪造typed body/response；
- unary携带stream item，server stream缺item；
- internal `http.context`进入protocol surface；
- duplicate union branch、duplicate/illegal/missing required field、duplicate raw JSON record
  key、open/unknown schema field及nominal/package/path伪造；
- raw/typed、unary/server-stream、external source、request/response schema与fixed error
  projection/version均进入identity；
- selector、key、callable/build/deployment、adapter param、private nominal/context codec均不进入
  private projection或serialization golden；
- canonical map insertion order不改变结果；非canonical sequence在loaded boundary拒绝，
  producer normalization得到同一identity。

新增production definition反搜确认：

- 没有`ContractOperationId`、`ServiceProtocolIdentity`、`IngressSelector`、
  `PackageCallableId`、`PackageBuildId`、`DeploymentRevision`、`TypeRefIr`、
  `PackageSchemaTypeId`或`serde_json::Value`字段进入preimage；
- 没有WebSocket/receive/`ConnectionMessage`/context-expectation type或variant进入shared
  surface；`websocketConnect`、`websocketReceive`、`websocket.message`只存在于必须fail
  closed的negative test raw strings。

## 5. 验收

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model gateway -- --list` | PASS；5 tests，非零 |
| `cargo test -p skiff-artifact-identity gateway -- --list` | PASS；7 tests，非零 |
| `cargo test -p skiff-artifact-model gateway` | PASS；5 passed |
| `cargo test -p skiff-artifact-identity gateway` | PASS；7 passed |
| `cargo test -p skiff-artifact-model` | PASS；160 passed |
| `cargo test -p skiff-artifact-identity` | PASS；101 unit + 8 CLI passed；1 fixture regeneration test按既有标记ignored |
| `cargo fmt -p skiff-artifact-model -p skiff-artifact-identity -- --check` | PASS |
| `git diff --check` | PASS |
| `cargo fmt --all -- --check` | FAIL；仅命中下述既有、禁止修改的compiler格式漂移 |

Workspace-wide rustfmt合法残余：

1. `compiler/driver/authoring/package_publication/tests.rs:49`
2. `compiler/tests/service_conformance.rs:11`
3. `compiler/tests/websocket_ingress.rs:9`

三处均不在本task允许写入范围，本leaf没有修改。所有本task Rust文件通过两个目标crate的rustfmt
check。

## 6. 残余与限制

- 当前generation只冻结HTTP；没有WebSocket connect/receive/message identity或surface。
- 没有另造未冻结的external documentation metadata扩展槽；后续只有在closed wire字段冻结后才能显式
  扩展并更新generation。
- 本leaf只提供shared model、normalization、validation与identity owner；compiler、deployment、
  runtime、Router及test-runner consumer均未接线。
- 未修改lockfile、service/api authoring、stable instance或live fixture；未运行workspace/root、
  stable/live；未push。
