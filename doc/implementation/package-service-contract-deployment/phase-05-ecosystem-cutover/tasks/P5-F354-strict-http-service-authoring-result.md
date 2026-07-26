# P5-F354 Strict named HTTP service authoring result

状态：Completed（HTTP authoring leaf；generated deployment在新gateway deployment接线前显式fail
closed）。

## 1. Exact checkpoint

| 项目 | commit | tree |
| --- | --- | --- |
| 本leaf base / integration authority | `acbb4d7ea1174289c9c89c93b866dd1511815e17` | `e21f0cca314e408890631e1f8c09f6b34a4ed5b9` |
| production/tests | `d4922c50c900ba81e5612e0d535ec282337d7007` | `5c59bd4d4d695bfc9bb42f899eb0bf3d754d5f37` |

工作分支为`codex/p5-f354-http-authoring`，worktree为
`/Users/geek/workspace/skiff-p5-f354-http-authoring`。本leaf没有merge/rebase integration，没有运行
stable/live，没有push。

## 2. 最终authoring边界

- `ServiceManifestAuthoring.http`已从`Option<serde_json::Value>`替换为
  `Option<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>>`。
- `HttpGatewayEntryAuthoring`严格拥有default `host: "*"`、required
  `method/path/kind/handler`、entry-local optional `guard/pre`及default-empty `adapterArgs`。
- HTTP mapping使用专用serde visitor，既保留`Option`的missing/null语义，又拒绝duplicate key；
  `BTreeMap`保证key canonical order。Key由F351 `GatewayEntryKey`严格反序列化。
- entry、`GatewayAdapterArg`及`GatewayAdapterSource`递归deny unknown；closed
  `GatewayAdapterKind`只接受`typedJson | rawHttp`。
- 真正compile input reader复用`SourceSymbolSelector::parse`校验handler/guard/pre，并明确拒绝
  `root.` public-path写法；不做public path fallback或callable解析。
- input reader规范化HTTP method为uppercase、host为ASCII lowercase，并校验非空host、HTTP token、
  absolute path、query/fragment/whitespace/control；同时拒绝重复canonical selector。
- input reader直接复用F351 `validate_gateway_adapter_args`，因此raw HTTP拒绝`http.body`、
  `http.context`要求entry-local `pre`、param空白/重复与unknown source全部fail closed。
- `generate_service_deployment`在任何旧API/operation resolution前拒绝存在HTTP authoring的manifest；
  HTTP不再进入旧`operation -> ContractOperationId` reader。原WebSocket opaque authoring与旧consumer
  wire未扩展或重命名。

## 3. 自验收矩阵

| 任务条款 | production证据 | test / 反向证据 |
| --- | --- | --- |
| named mapping、无`routes/entries/id`层 | `artifact-model/src/ecosystem_authoring.rs`的HTTP map与`HttpGatewayEntryAuthoring` | canonical order、duplicate key及旧`routes/entries/id` negative |
| strict fields与递归unknown拒绝 | entry `deny_unknown_fields`，复用F351 closed arg/source DTO | required field、entry/arg/source unknown、`operation`、`handlerArgs`逐项negative |
| key/source selector词法边界 | `GatewayEntryKey` map key；`SourceSymbolSelector::parse` | invalid/whitespace key，invalid handler/guard/pre及`root.` fallback negative |
| host/method/path与adapter args | `compiler/input/src/service_config.rs::validate_http_authoring` | typed/raw、default/explicit host、method/host normalization、path/method/host、raw body、missing pre、duplicate param/selector |
| entry-local guard/pre/context | entry DTO无global层；adapter validator读取同entry `pre` | positive raw entry含guard/pre/context；global guard/pre与context-without-pre negative |
| generated deployment fail closed | `reject_unwired_http_authoring`是generator首个gate；旧parser只剩显式`WebSocket*`类型 | lib probe断言named HTTP不能重解释为service operation；production反搜无HTTP `operation` parser |
| WebSocket与下游保持边界 | `websocket`仍为原`Option<serde_json::Value>`；旧WS operation consumer仅被显式命名 | 未新增WebSocket shared type/source/kind；未修改deployment/runtime/router/test-runner |

Production反搜结果：

- `artifact-model/src`与`compiler/{input,driver}`中不存在
  `http: Option<serde_json::Value>`、HTTP `RouteAuthoring`或HTTP `resolve_route`。
- `compiler/driver/generated_deployment.rs`中的`operation`字段只存在于
  `WebSocketRouteAuthoring`及其WebSocket resolver；HTTP在generator入口直接失败。
- 改动文件精确为artifact authoring/export、service input、generated deployment及其直接tests；
  没有lockfile、F351 gateway model/identity、deployment DTO、runtime/router或WebSocket authoring改动。

## 4. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model service_manifest -- --list` | PASS；2 tests，非零 |
| `cargo test -p skiff-compiler-input service_config -- --list` | PASS；11 tests，非零 |
| `cargo test -p skiff-compiler generated_service_deployment -- --list` | BLOCKED；在枚举前由base中`compiler/tests/std_package_imports.rs`的既有`TypeRefIr`/`ConcreteNominal`不收敛编译错误阻断 |
| `cargo test -p skiff-artifact-model service_manifest` | PASS；2 passed |
| `cargo test -p skiff-compiler-input service_config` | PASS；11 passed |
| `cargo test -p skiff-compiler generated_service_deployment` | BLOCKED；同一既有`std_package_imports.rs`编译错误 |
| `cargo test -p skiff-compiler --lib generated_service_deployment -- --list` | PASS；1 test，非零 |
| `cargo test -p skiff-compiler --lib generated_service_deployment` | PASS；1 passed |
| `cargo test -p skiff-compiler --test generated_service_deployment` | BLOCKED；base的std package generic boundary validation在fixture compile阶段拒绝四个既有WebSocket generic declaration；本leaf专用lib probe独立通过 |
| `cargo check -p skiff-artifact-model -p skiff-compiler-input -p skiff-compiler` | PASS；仅既有warning |
| `cargo fmt -p skiff-artifact-model -p skiff-compiler-input -p skiff-compiler -- --check` | BLOCKED；只命中下述3个既有、禁止修改文件 |
| `rustfmt --edition 2021 --check <本leaf五个Rust文件>` | PASS |
| `git diff --check` | PASS |

Exact compiler selector的base blocker：

1. `compiler/tests/std_package_imports.rs:635`仍按不存在的
   `TypeRefIr::ConcreteNominal.type_arguments`匹配；
2. `compiler/tests/std_package_imports.rs:656`未覆盖base已有的`TypeRefIr::AppliedNominal`。

Exact rustfmt的base blocker：

1. `compiler/driver/authoring/package_publication/tests.rs:49`
2. `compiler/tests/service_conformance.rs:11`
3. `compiler/tests/websocket_ingress.rs:9`

以上5个阻断位置都不在本leaf diff或允许写入范围；没有为制造绿色证据越权修改。

## 5. 残余与限制

- 本leaf只冻结并验证HTTP authoring；handler/pre/guard到exact callable、signature/external schema、
  execution plan、gateway entry/selector artifact的解析属于后续C2/C1 deployment节点。
- generated deployment尚无新gateway deployment DTO，因此任何出现HTTP字段的manifest（包括empty
  mapping）都会明确失败，不会静默忽略或回退到service operation。
- WebSocket仍保持原opaque authoring与旧generated-deployment consumer；业务消息模型继续待设计。
- integration合流后应在generic schema leaf修复上述base编译阻断的checkpoint重跑两条exact compiler
  selector命令；同样应由对应owner收敛3个既有rustfmt漂移后重跑exact fmt gate。
- 未运行workspace/root、stable/live，未修改lockfile或本地instance，未push。
