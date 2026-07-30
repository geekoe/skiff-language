# P5-F357 HTTP gateway compiler projection

状态：Ready（C2 compiler convergence；依赖F351–F356已合流checkpoint）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F351-gateway-artifact-model-identity-result.md`
- `P5-F352-service-call-root-selection-result.md`
- `P5-F353-public-generic-schema-availability-result.md`
- `P5-F354-strict-http-service-authoring-result.md`
- `P5-F355-deployment-http-gateway-model-result.md`
- `P5-F356-compiler-owned-std-type-resolution-result.md`

以上父节点沿引用链连接唯一权威设计。本任务只完成H36的C2 HTTP compiler projection；不得重新设计
HTTP authoring、gateway identity、deployment DTO、service-call API、错误投影或WebSocket业务消息入口。

## Exact base

- integration commit：`bd10c33915567288c23ced520612c568a568a560`
- integration tree：`1db5c1f3df1dd2634f67332413f92071e0fff4f6`
- branch：`codex/package-service-phase-05`

本checkpoint已经包含：

- strict named `service.yml.http`；
- explicit `api.yml serviceCall` roots和generic schema规则；
- compiler-owned exact `std` owner；
- deployment v2 `gatewayEntries`与`selector -> GatewayEntryKey`；
- HTTP/WebSocket authoring在未接线状态下的明确fail-closed。

## 目标

把每个已经规范化的named HTTP authoring entry一次性投影为：

```text
service.yml HTTP entry
  -> exact current-package implementation callable(s)
  -> validated adapter execution plan
  -> canonical entry-local external HTTP schema
  -> GatewayEntryProtocolSurface
  -> GatewayEntryIdentity
  -> DeploymentGatewayEntry
  -> IngressSelector -> GatewayEntryKey
```

HTTP ingress不得读取或生成`ContractOperationId`。同一source callable即使同时是显式service-call root，
HTTP entry仍绑定implementation callable identity；两个identity域彼此独立。

## 必须完成

### 1. Exact private callable resolver

新增一个职责单一的compiler-owned resolver；不要继续膨胀
`compiler/driver/generated_deployment.rs`。Resolver消费已经验证的
`SourceSymbolSelector`和当前实现`PackageArtifact`，只按
`<modulePath>.<symbol>`精确读取：

- `packageLocalAbi.implementationSymbols`中的`Callable`；
- 同一`PackageCallableId`的`callableLinks`；
- 同一ID的`callableSemanticFacts`；
- link内exact `OperationTargetRef`。

必须交叉验证map key、nested ID、target ABI ID、source module、source executable和
`InternalFunction` callable kind。缺失、重复、错配、指向Type/Constant/PublicInstance、public-path-only
或依赖包selector都fail closed；不得回退到`publicSymbols`、`root.*`、display name、stable key或
`ServiceContract` operation。

Handler、pre、guard都只允许top-level function，且声明自身的`typeParams`必须为空。Fully
instantiated signature type仍可包含concrete generic container或nominal type；不得按
`std.http`或其它名字增加generic declaration特例。

### 2. HTTP callable signature contract

所有handler formal必须被`adapterArgs[].param`恰好覆盖一次；不得有未知、缺失、重复或额外formal。
同一source可以绑定多个不同formal，但这些formal必须解析为同一个exact linked type和同一个canonical
schema/plan，否则单一source无法无歧义解码，必须拒绝。

Standard source规则：

- `http.request`
  - formal type必须是compiler-owned exact `std.http.HttpRequest`；
  - typed/raw HTTP均可使用。
- `http.body`
  - 只允许`typedJson`；
  - 至少有一个body formal；
  - formal type必须可投影为本任务第3节定义的external schema。
- `http.context`
  - 只在同entry声明`pre`时允许；
  - formal type必须与该pre的exact return type相同；
  - context是同一次Runtime请求内的内部值，不进入external schema或
    `GatewayEntryIdentity`。

Guard/pre不复用handler adapter args：

- guard：exact一个`std.http.HttpRequest`参数，return必须是
  `std.http.HttpResponse?`；null表示继续，response表示短路。
- pre：exact一个`std.http.HttpRequest`参数；return是该entry唯一context type。没有
  `http.context` formal时允许pre仅执行校验/副作用；有context formal时逐值校验exact return type。
- guard/pre的generic declaration同样拒绝；它们可挂起，不在本leaf新增suspend限制。

Handler return：

- `typedJson` unary：任意external-schema-eligible exact type；
- `typedJson` server stream：exact `Stream<T>`，其中`T`必须external-schema-eligible；
- `rawHttp` unary：exact `std.http.HttpResponse`；
- `rawHttp` server stream：exact `Stream<std.http.HttpResponseStreamEvent>`。

Dispatch mode只从exact handler return推导，authoring不新增mode字段。不得接受
`Stream<bytes>`、错误std owner、nullable raw response、任意“形状相似”的用户类型或
unknown/generic return。

### 3. Entry-local external schema projection

新增唯一compiler projector，把真正跨external boundary的exact type投影到F351已经冻结的
`GatewayExternalSchema`：

- `null/string/number/integer/boolean/bytes`；
- `Array<T>`；
- record；
- closed structural/named union；
- nullable；
- string literal；
- transparent alias与representation按其underlying external shape投影；
- fully instantiated generic type必须先做exact substitution再投影。

Record的`required`必须与现有strict JSON/runtime type-plan语义一致：non-nullable field required，
nullable field optional；不得另造第二套可选字段规则。

Current-package private named type从exact implementation type/link facts展开；它的source path、symbol、
local type identity、nominal name和package identity不得进入external schema。Exact package dependency
type只可从已经绑定的PackageArtifact/PackageSchema record closure展开，且必须交叉验证package owner、
stable key与schema identity；不得从显示名猜测。

以下情况结构化fail closed：

- unresolved/forged owner、missing type/link/schema record；
- free type param、未完全实例化的generic；
- interface/callback/function/actor/db-object或其它不在F351 closed vocabulary中的type；
- Map及其它未冻结container；
- open/ambiguous union、unsupported literal；
- recursive structural expansion不能形成有限F351 schema；
- 同一external source映射到不一致的exact type/schema。

Projector只生成external docs/identity schema，不生成Runtime codec DTO。Runtime codec仍只能在C3从exact
linked callable signature形成。不得把private ingress type补进`api.yml`、
`PackageLocalAbi.publicSymbols`、PackageSchema records或ServiceContract requirements。

### 4. Generated deployment wiring

移除`generate_service_deployment`对nonempty/empty HTTP mapping的C1拒绝，改为：

- `http: null`或missing：empty gateway map与empty ingress；
- `http: {}`：同样是合法zero/zero；
- 每个named entry生成同key的一个`DeploymentGatewayEntry`和一个HTTP
  `DeploymentIngressBinding`；
- selector精确使用F354已规范化的host/method/path；
- adapter plan逐值保留authoring中ordered args；
- handler/pre/guard保存resolver得到的exact `PackageCallableId`；
- protocol surface由adapter kind、推导的dispatch mode、规范化external sources、request/response/
  stream schema以及fixed-v1 external error projection组成；
- 通过F351唯一normalizer/identity owner生成`GatewayEntryIdentity`，再交给F355 deployment
  validation/projection；不得在compiler复制hash或canonicalization。

WebSocket保持当前明确fail-closed；不得恢复旧operation ingress或新增connect/receive/message
authoring。

### 5. 跨对象不变量

必须用真实source compilation证明：

1. 只有private HTTP handler/type时，ServiceContract允许零operation，Package public/schema surface不因
   ingress扩张。
2. 同一callable同时显式`serviceCall: true`时，ServiceContract operation和HTTP gateway entry各自存在，
   且operation ID、gateway entry identity、implementation callable ID不能互换。
3. 只改host/method/path：
   - `ServiceProtocolIdentity`不变；
   - `GatewayEntryIdentity`不变；
   - `DeploymentArtifactIdentity`改变。
4. 只改handler body：
   - `PackageBuildId`和deployment identity改变；
   - `PackageLocalAbiIdentity`、`ServiceProtocolIdentity`、`GatewayEntryIdentity`不变。
5. 改external request/response/stream shape或adapter kind：
   `GatewayEntryIdentity`与deployment identity改变，ServiceProtocolIdentity不变。
6. 改target参数名/source映射但保持相同external wire shape：
   `GatewayEntryIdentity`不变，deployment identity改变。

## 写入范围

主要owner：

- 新的`compiler/driver/http_gateway_projection/**`或同等独立leaf module；
- `compiler/driver/generated_deployment.rs`的薄接线；
- compiler直接integration tests和必要的`compiler/Cargo.toml` test target；
- 仅为调用F351/F355既有API所需的局部exports。

允许对`compiler/projection`或`compiler/contract`抽取一个纯类型解析helper，但必须复用既有
PackageArtifact/type owner，不能复制schema/identity算法。

禁止修改：

- F351 shared gateway DTO、normalization或identity preimage；
- F354 authoring shape；
- F355 deployment DTO/schema/identity；
- PackageArtifact、ServiceContract或ServiceProtocol schema/generation；
- RuntimeAssembly、runtime/linker/Host/Router/test-runner执行；
- WebSocket DTO/authoring/message模型；
- std source、stable/live配置、lockfile、三仓库service源码。

若实现必须新增或改变任一公共artifact/codec模型、修改上述generation、定义新的错误wire、或发现
guard/pre/typed/raw signature仍有多个方向性未决问题，立即停止并上报，不自行扩大C2。

## 验证

先枚举并确认非零selector，再运行：

```bash
cargo test -p skiff-compiler --test http_gateway_projection -- --list
cargo test -p skiff-compiler --test generated_service_deployment -- --list
cargo test -p skiff-compiler --test http_gateway_projection
cargo test -p skiff-compiler --test generated_service_deployment
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-artifact-identity gateway
cargo test -p skiff-artifact-identity deployment
cargo check -p skiff-compiler -p skiff-deployment -p skiff-artifact-identity
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

正向至少覆盖：

- private raw unary；
- private typed unary，含record/nullable/closed union与private named types；
- raw server stream；
- typed server stream；
- request/body/context组合、guard、pre；
- zero operation/zero gateway与zero operation/nonzero gateway；
- 同callable serviceCall + gateway双surface；
- 本任务第5节identity不变量。

负向至少覆盖：

- selector missing/wrong kind/public fallback/link或target错配；
- generic handler/pre/guard；
- adapter arg缺失/未知/重复formal与同source不同type；
- request/context/pre/guard exact signature错配；
- typed缺body、raw使用body；
- raw response/stream item错配；
- unsupported/private recursive/interface/function/Map/free generic external type；
- dependency schema owner/key/id错配；
- WebSocket继续fail closed；
- production反搜没有HTTP `operation -> ContractOperationId` resolver。

不运行workspace/root、stable/live，不push。

## 有界探查与强制停止

开发Agent在实现前可派至多一个只读子Agent，只回答一个明确问题，例如：
“现有PackageArtifact中哪些facts足以完成private callable/type exact resolution”。该子Agent不得再派
下一层。

若探查后发现实际写入必须跨入C3、需要新的DAG owner、范围明显大于本合同，或仍有多项会改变公开语义的
不明确问题，当前任务立即结束并如实上报：

- `TASK_SCOPE_EXPANDED`：列出证据、实际owner和建议拆分；
- `TASK_NOT_EXECUTABLE`：列出缺失决策及最小可执行前置条件。

不得静默扩修，也不得把只读探查写成Completed。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f357-http-gateway-projection`
- branch：`codex/p5-f357-http-gateway-projection`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result记录exact base/commit/tree、resolver/schema算法证据、验证矩阵、反向搜索与C3残余；
- worktree保持clean，不merge/rebase integration。
