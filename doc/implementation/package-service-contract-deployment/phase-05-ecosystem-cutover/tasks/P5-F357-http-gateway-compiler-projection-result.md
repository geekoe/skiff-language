# P5-F357 HTTP gateway compiler projection result

状态：Completed（C2 compiler convergence；C3 runtime codec、assembly linking与dispatch仍保持
既有fail closed）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `bd10c33915567288c23ced520612c568a568a560` | `1db5c1f3df1dd2634f67332413f92071e0fff4f6` |
| task checkpoint | `625132b75713d9c528fb62d21b0181d29caba4b6` | `6421603783fe575110b30e254946e7818270b047` |
| production/tests | `a948e64d1833f702e350995e47b3f545511cbde7` | `49bc89fca8b5784558232479a4a70233b9e99ae5` |

工作分支为`codex/p5-f357-http-gateway-projection`，worktree为
`/Users/geek/workspace/skiff-p5-f357-http-gateway-projection`。本leaf没有merge/rebase
integration，没有运行workspace/root、stable/live，没有修改lockfile或三仓库service源码，也没有push。

## 2. Exact callable与signature projection

- 新增独立的`compiler/driver/http_gateway_projection` owner。HTTP handler、pre、guard只从当前
  `PackageArtifact.packageLocalAbi.implementationSymbols`按canonical
  `<modulePath>.<symbol>`读取，不回退到`publicSymbols`、`root.*`、display name、
  `ServiceContract` operation或依赖包selector。
- Resolver逐项校验implementation symbol kind、map/nested callable ID、`callableLinks`、
  `callableSemanticFacts`、target ABI ID、`InternalFunction` kind及exact file/module/source hash。
  handler、pre、guard声明自己的generic参数时均拒绝。
- handler formal必须被ordered `adapterArgs`恰好覆盖一次。unknown、missing、duplicate formal以及
  同一source绑定不同exact type/schema均结构化拒绝。
- `http.request`、guard、pre和raw return只接受compiler-owned exact
  `std.http`类型；`http.context`要求entry-local pre且formal与pre return exact相等。
  typed必须有body，raw禁止body。
- unary/server-stream只从handler exact return推导。raw只接受
  `HttpResponse`或`Stream<HttpResponseStreamEvent>`；typed response/stream item走同一个external
  schema projector。

## 3. Entry-local external schema

- 唯一projector覆盖null/string/number/integer/boolean/bytes、Array、record、nullable、string
  literal、closed structural/named union、transparent alias和representation。
- current-package private named type从exact implementation symbol、local type ID与type link展开，
  不扩张`api.yml`、Package public surface、PackageSchema或ServiceContract。
- fully instantiated private generic先替换外层实参再递归展开；真实
  `Envelope<string> -> Box<T> -> string`用例证明嵌套generic substitution闭合。free type param、
  错误arity、recursive expansion、interface/function/db-object/Map和其它未冻结container均拒绝。
- dependency public type先由当前package的exact requirement绑定到PackageArtifact，再由
  owner/stable key/type ID绑定到PackageSchema record。record内的跨包传递依赖继续在已经验证的完整
  artifact/schema closure中按exact owner record展开，不错误限制为根包直接依赖。
- record `required`直接由canonical schema决定：non-nullable field required，nullable field
  optional；没有第二套optional规则。

## 4. Deployment wiring与identity

- `generate_service_deployment`现在接受missing、null或empty HTTP mapping；每个named HTTP entry生成
  同key的一个`DeploymentGatewayEntry`及一个`IngressSelector -> GatewayEntryKey` binding。
- adapter kind、推导的dispatch mode、canonical external sources、request/response/stream schema和
  fixed-v1 external error projection先经F351 normalizer，再由唯一gateway identity owner计算
  `GatewayEntryIdentity`。compiler没有复制canonicalization或hash。
- deployment entry保存exact private handler/pre/guard callable ID与authoring ordered adapter plan。
  HTTP路径不读取或生成`ContractOperationId`。
- 同一source executable同时成为显式service-call root和HTTP handler时，public service-call callable
  ID、private implementation callable ID、contract operation ID与gateway identity保持独立；两种
  callable ID仍交叉证明落到同一file/executable。
- authoring driver把完整resolved PackageSchema closure交给HTTP projector；进入既有
  `project_service_deployment`前再严格裁剪回ServiceContract声明的record closure，保持F355 deployment
  projection输入闭合。
- WebSocket authoring继续在旧operation ingress解释前明确拒绝，没有新增connect/receive/message
  语义。

## 5. 自验收矩阵

| 任务条款 | 代码/反向证据 | 测试 |
| --- | --- | --- |
| exact private callable resolver | `http_gateway_projection/resolver.rs::ExactCallableResolver`；无public/contract fallback | missing、wrong-kind、public path、target mismatch负例 |
| signature与adapter规则 | `http_gateway_projection/mod.rs::validate_handler_args`、`validate_guard`、`validate_pre` | request/body/context、guard/pre正例；missing/unknown/duplicate、same-source mismatch、context/pre/guard/raw负例 |
| typed/raw unary与stream | `project_handler_return`只读exact return | private typed/raw unary及typed/raw server stream |
| private schema与generic | `schema.rs::project_private_symbol`及外层实参替换 | record/nullable/union/alias/representation/Array、嵌套generic正例；recursive/interface/function/Map/generic负例 |
| dependency schema closure | `project_package_symbol`、`schema_owner_artifact` | direct与transitive dependency正例；owner/key/id mutation负例 |
| zero operation与surface隔离 | HTTP private symbols不进入public/schema；gateway不读contract operation | zero-operation/nonzero-gateway、zero-gateway及serviceCall+HTTP双surface |
| identity不变量 | gateway identity只来自normalized protocol surface；deployment仍包含selector/implementation/plan | selector、body、shape、handler/param mapping mutation matrix |
| WebSocket fail closed | `reject_unwired_websocket_authoring`保留 | generated deployment WebSocket负例 |

## 6. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test http_gateway_projection -- --list` | PASS；7 tests，非零 |
| `cargo test -p skiff-compiler --test generated_service_deployment -- --list` | PASS；10 tests，非零 |
| `cargo test -p skiff-compiler --test http_gateway_projection` | PASS；7/7 |
| `cargo test -p skiff-compiler --test generated_service_deployment` | PASS；10/10 |
| `cargo test -p skiff-compiler-input service_config` | PASS；11 selected |
| `cargo test -p skiff-artifact-identity gateway` | PASS；9 selected |
| `cargo test -p skiff-artifact-identity deployment` | PASS；3 selected |
| `cargo check -p skiff-compiler -p skiff-deployment -p skiff-artifact-identity` | PASS；仅既有warning |
| changed Rust `rustfmt --edition 2021 --check` | PASS |
| `git diff --check` | PASS |

Production反搜：

```text
rg 'http.*ContractOperationId|ContractOperationId.*http|http.*contract_operation_id|contract_operation_id.*http' \
  compiler artifact-model deployment -g '*.rs'
```

结果为零匹配。`generated_deployment.rs`中剩余operation projection只处理显式service-call
`ServiceContract.operations`，不被HTTP projector调用。

## 7. 明确残余

1. C3仍负责从exact linked callable signature形成Runtime codec、把gateway entry链接进
   RuntimeAssembly，并完成Host/Router dispatch；本leaf没有生成codec DTO或修改runtime/router。
2. WebSocket business gateway模型仍未冻结；本leaf只保持现有明确fail closed。
3. 没有修改F351 gateway DTO/normalization/identity、F354 authoring shape、F355 deployment DTO/schema、
   PackageArtifact、ServiceContract或ServiceProtocol generation。
