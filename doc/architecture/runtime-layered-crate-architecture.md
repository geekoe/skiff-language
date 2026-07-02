# Runtime Layered Crate Architecture

日期：2026-06-21

本文定义 Skiff runtime 目标态的分层 crate / 模块边界。它是长期内部架构契约，面向
compiler、runtime、router、artifact 维护者；不是用户可见语言规范，也不是实施
checklist。具体迁移切片应写入 `../implementation/`。

Skiff 尚未发布。本文不要求兼容旧 runtime internal API、旧 artifact activation shape、
旧 binary payload、旧 `__skiffType` JSON shape 或旧 package-test synthetic service 生成
路径。

## Scope

本文负责：

- runtime 内部长期分层和 crate 拆分目标。
- artifact graph、linked program image、runtime activation、request execution、
  boundary conversion、native capability、package-test runtime 的归属。
- 允许和禁止的依赖方向。
- 每层的输入输出契约和跨层数据流。
- 什么条件下可以把 runtime 内部模块提升为独立 crate。

本文不负责：

- 具体代码重排步骤、commit 顺序和迁移 checklist。
- 用户可见语言语义、std API 文档和 service.yml reference。
- router 的 gateway / runtime adapter 边界。该契约见
  `gateway-runtime-adapter-boundary.md`。
- 普通 runtime value 物理布局、type erasure 和 request-scope memory 细节。该契约见
  `runtime-value-layout-and-type-erasure.md`。
- compiler / runtime 共享 artifact DTO 归属。该契约见
  `runtime-compiler-shared-artifact-types.md`。

## Goals

目标态 runtime 必须满足：

- 新增一个 Skiff 类型节点时，只需要改 canonical type / boundary contract 的归属层，不
  需要在 JSON、binary、HTTP、native、DB 多处手写同类规则。
- 新增一个 native binding 时，signature、arg/return `RuntimeTypePlan` 和 required context
  只声明在 `skiff-runtime-native-contract`；`BoundaryConversionPlan` 由 native adapter 在
  调用边界构造；handler 只声明在 `skiff-runtime-native`，并通过同一个 binding key 引用
  contract spec。
- 新增一个 ingress / response mode 时，不需要扩大 request runner 的中央分发表。
- artifact loading、linking、activation、route registry、request execution 不能通过
  `RuntimeHost` 互相读写内部状态。
- package-test synthetic service 生成不能驻留在 production runtime 主路径。
- crate 边界用于强化依赖方向，而不是把一个大文件机械拆成多个互相 re-export 的 crate。

## Non-Goals

本文不要求：

- 一开始就把所有目标层都拆成 crate。
- 引入通用 pass manager、plugin system 或 runtime framework。
- 创建 `runtime-core`、`runtime-utils` 这类容易腐化成万能依赖的 crate。
- 通过兼容 shim 支持旧 artifact、旧 payload 或旧内部 runtime API。
- 让 boundary codec、linker 或 native registry 访问 host、router socket、service DB、
  telemetry exporter、request cancellation table 或 activation mutable state。

## Layer Model

长期依赖方向是 DAG，不是单线链。下表表示允许的直接依赖；未列出的跨层依赖默认禁止。

| Boundary | May Depend On |
| --- | --- |
| `runtime-host` | `runtime-transport`, `runtime-request`, `runtime-package-test`, `runtime-loader`, `runtime-linker`, `runtime-linked-program`, `runtime-activation`, `runtime-capability-context`, `runtime-model` |
| `runtime-transport` | `runtime-request` response event contract, runtime protocol DTO, `runtime-model` |
| `runtime-package-test` | `runtime-loader` `ArtifactGraph`, `runtime-linker` behavior, `runtime-linked-program` DTOs, `runtime-activation`, `skiff-artifact-model`, `runtime-model` |
| `runtime-request` | `runtime-eval`, `runtime-boundary`, `runtime-capability-context`, `runtime-linked-program` linked image DTOs, `runtime-activation` activation types, `runtime-model` |
| `runtime-eval` | `runtime-native`, `runtime-boundary`, `runtime-linked-program` DTOs, `runtime-activation` read-only facts, `runtime-model` |
| `runtime-native` | `runtime-native-contract`, `runtime-boundary`, `runtime-capability-context`, `runtime-model` |
| `runtime-capability-context` | `runtime-native-contract`, `runtime-boundary`, `runtime-model` |
| `runtime-activation` | `runtime-linked-program` linked image DTOs, `runtime-linker` activation facts / errors while that contract remains linker-owned, `runtime-boundary`, `runtime-model`, `skiff-artifact-model` |
| `runtime-linker` | `runtime-loader` `ArtifactGraph`, `runtime-linked-program`, `runtime-native-contract`, `runtime-boundary`, `runtime-model`, `skiff-artifact-model`, `skiff-artifact-identity` |
| `runtime-linked-program` | `runtime-model`, `skiff-artifact-model` DTO aliases |
| `runtime-native-contract` | artifact-model native signature DTO, `runtime-model` |
| `runtime-loader` | `runtime-model`, `skiff-artifact-model`, `skiff-artifact-identity` |
| `runtime-boundary` | `runtime-model`, artifact DTO type descriptors |
| `runtime-model` | pure shared primitives only |

上述名字表达目标边界，不要求目录或 crate 名必须逐字一致。实际 crate 名可沿用仓库习惯
使用 `skiff-runtime-*` 前缀。

更准确的数据流：

```text
artifact roots + pointer selection
  -> ArtifactGraphLoader
  -> ArtifactGraph
  -> LinkedProgramImage
  -> RuntimeActivation
  -> ServiceRuntimeContext
  -> ServiceOperationContext
  -> RequestOperationContext
  -> RequestExecution
  -> BoundaryResponse
```

类型和值的边界转换走独立 contract：

```text
RuntimeTypePlan
  -> BoundaryConversionPlan
  -> RuntimeValue / wire JSON / binary payload / HTTP value / native args / DB value
```

规则：

- 上层可以依赖下层，下层不能依赖上层。
- 下层不能通过 callback、trait object、global singleton 或 test helper 反向读取上层状态。
- 跨层传递必须使用窄 DTO、plan 或 capability context，不能传全量 host / request /
  activation 对象。
- `serde_json::Value` 只能出现在第三方 JSON schema、opaque JSON value 或 artifact/raw
  file 入口边界。Skiff-defined schema 的 typed pipeline 不得把它当内部通用表示。

## Crate Promotion Rule

runtime 内部模块满足以下条件时，才应提升成独立 crate：

- 它有稳定输入输出类型。
- 它不依赖 `RuntimeHost`、router session、telemetry exporter、service DB client 或 request
  mutable state。
- 它的依赖方向单向，提升后不会形成循环依赖或通过 re-export 伪装循环依赖。
- 它的测试可以在没有本机 router / runtime WebSocket / Mongo 实例的情况下运行。
- 它能防止实际架构漂移，而不仅仅减少单个文件行数。

不满足这些条件时，应先在 `runtime` crate 内形成明确模块边界，再提升成 crate。

## Target Crates And Ownership

### `skiff-runtime-model`

`skiff-runtime-model` 拥有 runtime 各层共享但不携带 host state 的基础类型。

典型内容：

- typed ids 和地址：`ServiceId`、`BuildId`、`RuntimeId`、`FileAddr`、
  `ExecutableAddr`、`TypeAddr`。
- linked program image 的只读 DTO 基础类型。
- `RuntimeTypePlan`、`RuntimeTypeContext`、boundary 可消费的 type plan。
- `RuntimeValue`、request heap value graph 的纯值类型，若这些类型已满足
  `runtime-value-layout-and-type-erasure.md` 的约束。
- 共享 runtime error projection contract：`RuntimeErrorPayload`、`TypeIdentity` 和
  `WirePayload`。该合同覆盖 wire `payload()`、catch projection，以及 typed recovery
  所需的 `as_any()` downcast。
- 纯错误类型和 diagnostics context，不包含 host IO handle；共享 projection contract
  不表示 model 拥有具体 runtime/root error variants，也不拥有任何 host IO 或 lifecycle
  error state。

禁止：

- artifact 读取。
- linking phase 的 mutable resolver state。
- service DB、file backend、router sender、telemetry exporter。
- 运行时配置 reload、route registry、loaded build lifecycle。

该 crate 不是 `runtime-core`。任何需要 IO、host activation、request lifecycle 或
capability dispatch 的类型都不属于这里。

### `skiff-runtime-boundary`

`skiff-runtime-boundary` 是最优先收敛的行为边界之一。crate promotion 顺序仍以
§Suggested Promotion Order 为准。它拥有 canonical boundary conversion contract。

职责：

- `RuntimeBoundaryContract`。
- `BoundaryConversionPlan`。
- JSON wire、runtime binary payload、HTTP typed body / response、native args / return、
  DB business value、stream / bytes boundary 的 shared plan 解释。
- `alias`、representation type、nullable、union、record、map、bytes、stream、resource、
  external object 的统一接受 / 拒绝规则。
- boundary error 分类、path context 和 expected / actual diagnostics。

候选类型：

```rust
struct RuntimeBoundaryContract {
    types: Arc<RuntimeTypeContext>,
    policy: BoundaryPolicy,
}

enum BoundaryUse {
    JsonWire,
    RuntimePayload,
    HttpRequest,
    HttpResponse,
    NativeArgument,
    NativeReturn,
    DbDocument,
    DbQueryValue,
    ConfigValue,
}

struct BoundaryConversionPlan {
    expected: RuntimeTypePlan,
    use_case: BoundaryUse,
    direction: BoundaryDirection,
}
```

规则：

- boundary crate 只能依赖 runtime model、artifact DTO 中的 type descriptors 和纯
  encoding helper。
- boundary crate 不能依赖 eval、host、request、native dispatch、service DB client、
  router protocol writer 或 activation manager。
- HTTP / native / DB 专用模块可以提供 shape helper，但不能各自重新解释
  `RuntimeTypePlan`。
- runtime binary framing 可以留在 transport 层；但 plan-driven encode/decode 必须调用
  boundary contract。
- DB adapter 可以处理 BSON / Mongo 特有映射；但 Skiff value 到 DB business JSON 的类型
  解释必须来自 boundary contract。

完成后，下列模块不再拥有独立类型解释语义，只保留适配入口：

- `value_codec/boundary_json.rs`
- `value_codec/runtime_convert/*`
- `value_codec/type_descriptor.rs` 中的 boundary decode/encode 分支
- `transport/runtime_payload_codec.rs`
- `eval/boundary.rs`
- `config_view.rs`
- `host/service_db.rs`

### `skiff-runtime-loader`

`skiff-runtime-loader` 拥有 artifact root / pointer 到 `ArtifactGraph` 的读取边界。

职责：

- artifact root containment 和 artifact-root-relative path validation。
- release / dev pointer selection 后的 service unit path resolution。
- artifact file IO。
- canonical artifact DTO parse、schema version check 和 unknown-field rejection。
- service unit、package unit、service file unit、package file unit 的 graph assembly。
- service/package/file artifact identity 和 content identity validation。
- file、package、service unit 和 `ArtifactGraph` cache type。

目标输入输出：

```text
ArtifactRoot + ArtifactPointer
  -> ArtifactGraph
```

规则：

- loader 可以依赖 `skiff-artifact-model`、`skiff-artifact-identity` 和 runtime model。
- loader 不能依赖 linker、activation、host route registry、request runner、service DB、
  telemetry exporter 或 router socket。
- loader 输出 canonical artifact graph，不输出 `LinkedProgramImage`、`RuntimeActivation`、
  `ServiceRuntimeContext` 或 request state。
- raw JSON / bytes 只允许存在于 artifact file 入口；loader 出口必须是 typed artifact DTO。
- loader 负责 raw artifact parse/schema validation；linker 只负责 link-time semantic
  validation，例如 symbol resolution、operation target validation 和 native call validation。
- loader 拥有 artifact graph 层 cache。linked image cache 和 activation cache 的类型分别
  属于 linker / activation 层；cache instance 和 eviction policy 由 host-level
  `ActivationManager` 组合管理。

### `skiff-runtime-linker`

`skiff-runtime-linker` 拥有：

```text
ArtifactGraph -> LinkedProgramImage
```

职责：

- service / package / file symbol table 构造。
- type ref linking。
- call target resolution。
- binding / package dependency operation resolution。
- operation target validation。
- receiver ABI 和 native call signature validation。
- linked package export overlay 构造。

目标 pipeline：

```text
ArtifactGraph
  -> LinkInput
  -> SymbolTable
  -> TypeRefLinker
  -> CallTargetResolver
  -> OperationTargetVerifier
  -> NativeCallVerifier
  -> LinkedProgramImage
```

规则：

- linker 依赖 runtime model、runtime boundary 和 `skiff-runtime-native-contract`。它可以
  引用 `ArtifactGraph` 中的 typed artifact DTO，但不读取 artifact files。
- linker 不能依赖 host、request runner、router session、service DB runtime、telemetry、
  artifact cache、runtime config reload 或 package-test runner。
- linker 输出只读 linked image。它不能绑定本机 runtime config、DB connection、telemetry
  producer、spawn worker 或 route registry mutable state。
- native validation 只使用 native contract 中的 signature metadata；不能依赖
  `skiff-runtime-native` handler / dispatch crate，不能调用 native handler。
- linker 不修改 canonical artifact DTO，也不把 linked facts 写回 artifact JSON。

### `skiff-runtime-native-contract`

`skiff-runtime-native-contract` 拥有 native binding 的纯描述 contract。它位于 linker
下方、native dispatch 上方，解决“linker 需要验证 native signature，但不能依赖 native
handler crate”的依赖问题。

职责：

- native binding key。
- `NativeSignature`。
- native generic type parameter mapping。
- argument type expressions 和 return type expression。
- `NativeRequiredContext` 的纯枚举描述。
- `NativeCallPlan` / `NativeBindingResolution` 这类不含 runtime argument value 的解析结果。
- `NativeCallPlan` 只保存 binding key、arg/return `RuntimeTypePlan` 和 required context；
  不保存 `BoundaryConversionPlan`，也不依赖 boundary contract。

候选类型：

```rust
struct NativeBindingSpec {
    key: NativeBindingKey,
    signature: NativeSignature,
    required_context: NativeRequiredContext,
}

struct NativeCallPlan {
    binding: NativeBindingKey,
    arg_plans: Vec<RuntimeTypePlan>,
    return_plan: RuntimeTypePlan,
    required_context: NativeRequiredContext,
}
```

规则：

- native contract 可以依赖 artifact-model 的 native signature DTO 和 `runtime-model` 中的
  type plan / value-independent descriptors。
- native contract 不能依赖 eval、host、activation、service DB、file runtime、router
  transport 或 native handler implementation。
- linker 和 eval 可以读取 `NativeCallPlan`；只有 `skiff-runtime-native` 可以把它绑定到
  handler dispatch。

### `skiff-runtime-activation`

`skiff-runtime-activation` 拥有：

```text
LinkedProgramImage + runtime config -> RuntimeActivation
```

职责：

- service runtime identity 和 activation identity。
- package config vector materialization。
- service dependency activation view。
- DB / actor / gateway / timeout / route binding activation view。
- activation-level validation：本机 config 是否满足 linked image 的 runtime requirements。
- activation cache key 定义。

规则：

- activation 可以依赖 linked image DTO contract 和 boundary contract。只要
  `LinkedImageActivationFacts` / linker errors 仍由 linker 产生，activation 可以依赖 linker 的
  这些行为输出，但不能把 linked DTO 的所有权绕回 linker。
- activation 不读取 artifact files，不做 link，不持有 router socket，不执行 request。
- activation 不拥有 route registry；它只产出 route registry 可消费的只读事实。
- activation 不持有 DB client。DB runtime 由 host 根据 activation facts 和平台配置构造。
- activation 不持有 package-test synthetic construction 逻辑。

目标态不定义 `RuntimeProgram`。任何同时包含 linked image、activation metadata 和
identity bits 的扁平聚合对象都不属于 architecture contract。production path 使用
`LinkedProgramImage` + `RuntimeActivation` + explicit identity。

### `skiff-runtime-capability-context`

`skiff-runtime-capability-context` 拥有 eval/native/effects 可见的 host capability context
契约。它定义“某个 capability 能看见什么”，但不拥有 host state。

职责：

- config、DB、file、time、HTTP/WebSocket effect、actor、spawn、service dependency、
  telemetry 等 capability context trait / DTO。
- `NativeCapabilityContexts` 的 projection contract。
- capability required-context 到实际 context projection 的校验规则。
- request-local resource / stream / outbound handle 的窄接口。

规则：

- capability context crate 可以依赖 runtime model、boundary contract 和
  `skiff-runtime-native-contract` 的 `NativeRequiredContext`。
- capability context crate 不能依赖 host、request runner、route registry、router socket、
  service DB concrete client、file backend concrete store 或 telemetry exporter。
- host/request 层提供 concrete implementation；eval/native 只依赖 trait / DTO。
- `NativeCapabilityContexts` 不能实现成“所有 optional handle 的大包”。它必须按
  `NativeRequiredContext` 做 projection，让 handler 只能拿到声明过的 context。

### `skiff-runtime-native`

`skiff-runtime-native` 拥有 native capability handler 注册和调度。native binding 的纯
signature / required-context metadata 归 `skiff-runtime-native-contract`；本层只负责把
contract 绑定到实际 handler。

职责：

- `NativeCapabilityRegistry`。
- native binding key 到 handler 的注册。
- native contract spec 到 handler implementation 的一致性校验。
- 每个 std capability 的 dispatch ownership：file、http、json、time、bytes、websocket、
  actor、telemetry、service dependency、test double 等。
- native call dispatch。

目标模型：

```rust
trait NativeCapability {
    fn handlers(&self) -> &'static [NativeHandlerBinding];
}

struct NativeHandlerBinding {
    key: NativeBindingKey,
    handler: NativeHandlerFn,
}

struct NativeCallContext<'a> {
    plan: &'a NativeCallPlan,
    args: &'a [RuntimeValue],
    contexts: NativeCapabilityContexts<'a>,
}
```

规则：

- signature metadata 必须有一个 source of truth：`skiff-runtime-native-contract`。运行时不能在
  `std_runtime_schema.rs`、`native_registry.rs`、`native_dispatch.rs` 三处分别维护同一个
  binding 的 contract。
- `skiff-runtime-native` 的 registry 只能用 binding key 引用 contract spec，并登记 handler；
  不能复制 signature、arg plan、return plan 或 required-context metadata。
- registry 启动时必须按 handler binding key 查 contract spec，并校验 handler 覆盖率、
  未知 handler key 和重复 handler key。
- dispatch branch 不能从当前 caller executable 的 params / return type 推断 native callee
  boundary。callee signature 必须先解析成不含 argument values 的 `NativeCallPlan`。
- capability implementation 只能接收它声明需要的 context。文件 native 不应拿到 HTTP
  context；JSON native 不应拿到 service DB runtime。
- native crate 可以依赖 runtime model 中的 value APIs 和 `skiff-runtime-capability-context`，
  但不能依赖 `skiff-runtime-eval` 或 `RuntimeHost`。
- 涉及 IO 的 handler 必须通过窄 host-provided context 调用，不直接持有全局 host。

### `skiff-runtime-eval`

`skiff-runtime-eval` 拥有解释执行核心。

职责：

- `EvalInvocation` / `EvalExecutableBody` 等 eval-owned narrow input DTO。
- executable body evaluation。
- expression / statement / flow completion。
- local env、call frame、request heap 协调。
- execution budget / cancellation check 的窄接口调用。
- effect 和 native call 的 dispatch request 构造。

规则：

- eval 不能依赖 `RuntimeHost`、route registry、artifact loader、activation manager 或 router
  session，也不能依赖 linker / `LinkedProgramImage`。
- request 的 `InvocationContextBuilder` 根据 `ExecutableAddr` 从 `LinkedProgramImage` 解析出
  executable body，并投影成 eval-owned `EvalInvocation` / `EvalExecutableBody`。
- eval 只消费 `EvalInvocation` 和 request heap / capability dispatcher 的窄接口，不能接收
  full `ServiceRuntimeContext` 或 `LinkedProgramImage`。
- eval 不直接访问 DB、file、HTTP/WebSocket、telemetry exporter、service dependency
  transport。它只能通过 capability dispatcher 接口发出请求。
- eval 可以使用 `RuntimeActivation` 的只读 facts，但不能修改 activation。
- eval 不拥有 boundary type interpretation。参数绑定、native args、return value
  encode/decode 通过 boundary contract。

### `skiff-runtime-transport`

`skiff-runtime-transport` 拥有 runtime/router protocol DTO、frame codec 和 router socket
adapter。它不拥有 request response event DTO；这些 DTO 属于 request 层，transport 只依赖
并编码它们。

职责：

- runtime protocol frame DTO：request start/cancel、response start/chunk/end/error、
  runtime register/capabilities、connection send、actor/spawn control frames。
- binary frame encode/decode；frame payload bytes 在 transport 中保持 opaque。
- `RuntimeTransportSession`：router WebSocket read/write loop 和 frame envelope decode。
- request-owned `ResponseEvent` / `ResponseStreamEvent` 到 protocol frame 的唯一映射。

规则：

- transport 可以依赖 runtime model、runtime protocol DTO 和 request-owned response event
  contract。
- request crate 不直接构造 `Response*FrameHeader`，也不直接调用 binary frame encoder。
- request crate 产出 `ResponseEvent` / `ResponseStreamEvent`；transport writer 负责把事件编码
  成 router frame。
- transport 不做业务 payload type decode；runtime binary payload 的 type-plan encode/decode
  由 request adapter 调 boundary contract，transport 只封装 opaque bytes。
- transport 不做 route lookup、artifact load、link、activation 或 eval。

### `skiff-runtime-request`

`skiff-runtime-request` 拥有单次 request execution 的生命周期。

职责：

- request envelope validation。
- `RequestEnvelope` / `IngressRequest` 入站 request payload contract types。
- invocation target lookup 结果的包装。
- invocation context 构造。
- execution budget、cancellation、active request tracking。
- `RequestOperationContext` contract type and request-owned execution inputs.
- `BoundaryResponse`、`ResponseEvent`、`ResponseStreamEvent` DTO 和 event 生成。
- request-level telemetry span 生命周期。

目标拆分：

```text
IngressDispatcher
  -> InvocationContextBuilder
  -> RequestSupervisor
  -> ResponseEventWriter / StreamResponseEventWriter
```

规则：

- ingress dispatcher 只选择 request mode，不执行 business logic。新增 mode 时通过
  `IngressModeAdapter` registry / table 注册 adapter，而不是扩大中央 match 分支。
- response event writer 只生成 response events，不编码 runtime protocol frame，不理解
  transport writer internals。
- package-test request wrapper 不进入通用 request runner 中央分支；它通过 package-test runtime
  adapter 产出普通 invocation。
- binary HTTP、typed JSON adapter、server stream、WebSocket receive、普通 runtime payload
  都必须被建模为 `IngressMode`，而不是在一个大函数中互相读取局部变量。
- request crate 不拥有 route registry、lazy artifact loading、`ServiceRuntimeContext` 或
  `ServiceOperationContext`；它接收 host 已投影的 `RequestOperationContext`。
- request crate 不直接消费 transport protocol frame；host/transport adapter 将 frame 投影成
  request-owned `RequestEnvelope` / `IngressRequest`。
- host-owned `ServiceRuntimeContext` / `ServiceOperationContext` 不跨入 request crate；host /
  `RouteRegistry` 构造它们，并投影出 request-owned `RequestOperationContext`。
- `RequestOperationContext` 字段只能使用 request 执行所需的只读 operation facts、service
  metadata 和 eval adapter，不能包含 host concrete client、route registry 或 `RuntimeHost`。

### `skiff-runtime-host`

`skiff-runtime-host` 是 runtime 二进制的 composition layer。它可以保留在现有 `runtime`
crate / bin 中，直到其它层稳定。

目标组成：

```text
RuntimeHost
  - ControlPlane
  - RouteRegistry
  - BuildRegistry
  - ActivationManager
  - RequestSupervisor
  - RuntimeTransportSession
```

职责：

- runtime process config 和 router WebSocket session。
- artifact roots / pointer reload 的 orchestration。
- artifact graph loader、linker、activation 和 cache instances 的 composition。
- route registry mutation。
- loaded build lifecycle、idle release、spawn worker lifecycle。
- service DB client、blob store、telemetry producer/exporter 的 host-level construction。
- request supervisor 的 top-level ownership。

规则：

- `RuntimeHost` 是 facade，不是所有状态的 owner of record。
- route index 归 `RouteRegistry`。
- loaded build、spawn worker、idle release 归 `BuildRegistry`。
- artifact pointer -> `ArtifactGraphLoader` -> linked image -> activation orchestration 归
  `ActivationManager`。
- `ActivationManager` 拥有本机 cache instance 和 eviction policy；cache 类型归各自层：
  artifact graph cache type 归 loader，linked image cache type 归 linker，activation cache
  type 归 activation。
- reload config 和 runtime register frame construction 归 `ControlPlane`。
- request cancellation / active request / outbound request registry 归 `RequestSupervisor`。
- host 可以组合各层，但不能让下层拿 `&RuntimeHost`。

### `skiff-runtime-package-test`

package-test synthetic service 生成不属于 production runtime 主路径。

短期目标可建独立模块或 crate：

```text
skiff-runtime-package-test
  - dispatch_selection
  - test_assembly_validation
  - synthetic_service_builder
  - link_policy
```

职责：

- package-test assembly 验证。
- test entrypoint / dispatch selection。
- test binding provider projection 校验。
- synthetic service artifact / service unit 构造。
- package-test 专属 link policy。

规则：

- production host 只能通过窄入口调用，例如：

  ```rust
  PackageTestRuntimeBuilder::load(...)
  ```

- package-test crate 可以依赖 artifact model、linked-program DTO contract、linker 行为 API 和
  activation 的公开 contract。
- package-test crate 不能修改 production route registry 或 loaded build registry 内部结构。
- package-test 生成物必须尽早变成普通 artifact graph / linked image / activation，不把
  “测试服务如何合成”的细节泄漏给 request runner。
- 长期更优目标是由 test runner 或 artifact 侧预先生成 synthetic service artifact，production
  runtime 只加载普通 service artifact。

## Boundary Objects

目标态必须显式存在以下边界对象或等价对象。

### `ArtifactGraphLoader`

表示 artifact root / pointer 到 typed artifact graph 的读取服务。

拥有：

- artifact root 和 pointer resolution policy。
- artifact path containment。
- artifact file IO。
- canonical DTO parse/schema validation。
- service/package/file artifact cache。

不拥有：

- linked executable addresses。
- native call validation。
- runtime activation config。
- route registry。
- request state。

### `ArtifactGraph`

表示从 artifact files 读取并完成 raw parse/schema validation 后的 canonical artifact 输入。

拥有：

- service unit。
- package units。
- service file units。
- package file units。
- artifact identity / content identity facts。

不拥有：

- linked executable addresses。
- route registry entries。
- runtime config。
- DB client。
- request state。

规则：

- `ArtifactGraph` 不保证 symbol/call target/native call 已经可执行；这些是 linker 的
  link-time semantic validation。
- `ArtifactGraph` 不携带 runtime config、activation view 或 host capability handle。

### `LinkedProgramImage`

表示 linker 输出的只读执行 image。

拥有：

- linked files。
- linked package export overlay。
- routes / operations / spawn routes / receiver dispatch index。
- linked type context。
- native call validation facts。

不拥有：

- package config values。
- runtime service DB connection。
- router registration state。
- active request state。
- package-test assembly construction state。

### `RuntimeActivation`

表示某个 linked image 在某个 runtime config 下可被本机激活的只读视图。

拥有：

- service/version/build identity projection。
- package config materialization。
- service dependency activation view。
- DB / actor / gateway / timeout / operation route binding facts。

不拥有：

- linked executable bodies。
- route registry mutable index。
- DB client/session。
- cancellation table。
- router socket writer。

### `ServiceRuntimeContext`

表示 host 已经把 `LinkedProgramImage` 和 `RuntimeActivation` 组装成可执行 service 后的
host-owned service runtime state。

Owner：

- 类型定义归 `skiff-runtime-host`。
- 实例由 host / `ActivationManager` / `BuildRegistry` 构造和发布。
- 它可以持有 host 组装请求所需的 concrete capability sources，但不得泄露给 eval、
  request、native、transport 等 lower crates。

拥有：

- service identity。
- linked image reference。
- activation reference。
- host 构造的 capability sources，例如 service DB、blob store、telemetry producer
  capabilities；对 lower crates 只能通过窄 adapter / trait / DTO 暴露。

规则：

- capability handles 必须以窄 context 暴露给 eval/native/effects。
- 不允许把整个 `ServiceRuntimeContext` 传给每个 native handler。
- `ServiceRuntimeContext` 到 `NativeCapabilityContexts` / eval capability bundle /
  `RequestOperationContext` 的 projection 由 host-owned adapters 负责；projection 只能包含目标
  handler 或 request execution 所声明/需要的 narrow context。

### `ServiceOperationContext`

表示 host route lookup 后命中的可执行 operation。

Owner：

- 类型定义归 `skiff-runtime-host`。
- 实例由 host / `RouteRegistry` 构造。
- request crate 不消费该类型；host 将它投影为 request-owned `RequestOperationContext`。

拥有：

- `service: Arc<ServiceRuntimeContext>`。
- `operation: RuntimeOperation` 或等价 operation metadata。
- `addr: ExecutableAddr`。
- ingress / adapter mode 所需的只读 operation facts。

不拥有：

- request payload bytes。
- response writer / response event sink。
- cancellation token。
- execution budget。
- artifact cache。
- route registry mutable state。

规则：

- `ServiceOperationContext` 由 host / `RouteRegistry` 产生。
- request crate 不能通过它反查或修改 route registry，因为该类型不跨入 request crate。
- `IngressMode` 根据 host 投影出的 `RequestOperationContext` 和 request envelope 选择 adapter；
  adapter 不得修改 service-level context。

### `RequestOperationContext`

表示 host 从 `ServiceOperationContext` 投影出的 request-owned operation input。

Owner：

- 类型定义归 `skiff-runtime-request` contract。
- 实例由 host 从 `ServiceOperationContext` 构造。
- request crate 只读消费实例，不通过它反查 host state。

拥有：

- service metadata。
- operation metadata。
- target executable address。
- request execution 所需的 eval adapter / mode facts。

不拥有：

- `ServiceRuntimeContext`。
- route registry mutable state。
- artifact cache。
- host concrete client。
- router socket writer。

规则：

- `RequestOperationContext` 是 request crate 的唯一 operation-level host input。
- 它不能提供返回 host/root state 的 escape hatch。
- 新增 request execution 依赖时，优先增加窄 field / trait，而不是传入 host context。

### `RequestEnvelope`

表示 transport/host 已经从 protocol frame 投影出的 request-owned inbound DTO。

Owner：

- 类型定义归 `skiff-runtime-request` contract。
- 实例由 host / transport adapter 从 protocol frame 构造。

拥有：

- request id、target、ingress mode。
- opaque payload bytes 或 stream handles。
- request headers / metadata。
- trace context、deadline、caller identity facts。
- HTTP/WebSocket/runtime payload mode 所需的 adapter facts。

不拥有：

- protocol frame header type。
- router socket writer。
- route registry。
- service activation lifecycle。

规则：

- request crate 只消费 `RequestEnvelope` / `IngressRequest`，不 import transport protocol
  frame DTO。
- typed body decode 由 request adapter 调 boundary contract；transport 只提供 opaque bytes /
  stream handles。

### `EvalInvocation`

表示 request 已经从 linked image 中解析出的单次 eval 输入。

Owner：

- 类型定义归 `skiff-runtime-eval`。
- 实例由 request `InvocationContextBuilder` 构造。

拥有：

- executable body 的只读 eval view。
- parameter binding plan。
- executable-local type/value refs needed by eval。
- request heap / capability dispatcher 的窄 handle。

不拥有：

- `LinkedProgramImage`。
- `ServiceRuntimeContext`。
- route registry。
- artifact cache。
- protocol frame writer。

规则：

- request 可以依赖 eval-owned `EvalInvocation` constructor / DTO。
- eval 不依赖 linker；linked executable body 必须在 request 层投影成 `EvalInvocation`。

### `RequestExecution`

表示单次 request 的短生命周期执行。

拥有：

- request id、target、deadline、trace context。
- request envelope / ingress payload handle。
- execution budget。
- cancellation token。
- request heap。
- response writer。
- request-local stream/resource/outbound handles。

不拥有：

- artifact cache。
- route registry。
- loaded build lifecycle。
- runtime control reload。

### `BoundaryResponse`

表示 request 层产出的 response event，而不是 encoded runtime protocol frame。

拥有：

- unary payload 或 stream event。
- HTTP/WebSocket adapter response metadata 的 typed event 形态。
- runtime error event。

不拥有：

- binary frame bytes。
- router socket writer。
- protocol header serialization policy。

规则：

- `BoundaryResponse` / response events 只由 request 层生成。
- eval/native 不能依赖 request-owned response event DTO；它们返回 `RuntimeValue`、
  eval completion、effect result、native result 或窄 stream/resource handle。
- request-owned `ResponseEventWriter` 负责把 eval/native 结果映射成 `BoundaryResponse` /
  response events。
- `skiff-runtime-transport` 负责把 response events 编码成 `response.start` /
  `response.chunk` / `response.end` / `response.error` 等 protocol frames。

## Forbidden Dependencies

以下依赖在目标态禁止：

- boundary codec -> eval / host / service DB client / router sender。
- loader -> linker / activation / host route registry / request runner / telemetry exporter。
- linker -> artifact file IO / host / request runner / route registry / telemetry exporter。
- activation -> artifact file IO / linker mutable state / route registry mutation。
- eval -> `RuntimeHost` / artifact loader / router session。
- native capability handler -> full `RuntimeHost` 或 full `ServiceRuntimeContext`。
- native capability handler -> undeclared capability context。
- request runner -> artifact pointer file reading、link phase internals 或 protocol frame encoder。
- transport -> boundary business type decode / route lookup / eval。
- package-test builder -> production request runner internals。
- artifact-model -> runtime linked types、identity hashing、compiler lowering 或 host config。

## `serde_json::Value` Policy

目标态判据：

- Skiff-defined schema 必须转成 typed artifact DTO、`RuntimeTypePlan`、`RuntimeValue` 或
  boundary plan。
- 第三方 JSON schema、user `Json` / `JsonObject`、opaque config extension payload 可以保留
  `serde_json::Value`。
- artifact raw file 入口可以短暂使用 `Value` 做 unknown-field rejection 或 error context，但必须
  尽快 parse 成 canonical DTO。
- typed pipeline 中不得为了跨层方便而使用 `serde_json::Value` 作为万能中间表示。

因此这些位置的 `Value` 只能是临时迁移债或明确 opaque JSON：

- runtime type descriptor / boundary descriptor。
- eval IR node、program invocation、runtime ops。
- activation package config。
- service DB business document command。

package config 要按层区分，不能一刀切：

- activation 注入的 **resolved config 载体**（部署方传入的整包配置 blob）是合法 opaque
  入口，保留 `Value`；这与 `runtime-compiler-shared-artifact-types.md` 的"activation
  阶段传入的 runtime config values"是同一类。
- config 的 **typed 字段读取**（`config.require<T>` / `config.optional<T>` 对 `string` /
  `number` / `bool` 字段的 decode）当前自带一套独立 `Value` decode，属迁移债，目标态应并入
  §`serde_json::Value` Policy 上游的 boundary contract，复用同一 `RuntimeTypePlan` 解释，
  而不是 config 层自决类型。
- config shape 中声明为 `Json` / `JsonObject` 的字段是显式 opaque 逃生舱，永久保留
  `Value`，不算债。

## Request And Response Flow

目标 flow：

```text
router frame
  -> RuntimeTransportSession
  -> HostRequestEntry
  -> RequestEnvelope / IngressRequest
  -> RouteRegistry lookup
  -> ServiceOperationContext
  -> RequestOperationContext
  -> RequestSupervisor
  -> IngressDispatcher
  -> InvocationContextBuilder
  -> Eval
  -> BoundaryResponse / ResponseEvent
  -> RuntimeTransportSession
  -> router frame
```

规则：

- `RuntimeTransportSession` 只解 runtime protocol envelope，不做业务类型 decode。
- `HostRequestEntry` 属于 host composition layer，负责区分 control frame 和 request frame，并在
  request frame 上执行 route lookup。
- `RouteRegistry` 只做 target lookup，不触发 artifact load。lazy load 由 host-level
  `ActivationManager` 明确处理，完成后再更新 registry。
- `RequestSupervisor` 只接收已投影的 `RequestOperationContext`，不持有、不读取
  `RouteRegistry`。
- `IngressDispatcher` 根据 gateway/runtime mode 选择 adapter。
- adapter 的 typed body / response decode encode 走 boundary contract。
- response event writer 负责 unary、server stream、HTTP stream、WebSocket connect response 的
  event sequencing；`RuntimeTransportSession` 负责 protocol frame serialization。
- cancellation 和 timeout 属于 `RequestSupervisor`，不能藏进 eval 或 native handler。

## Native Flow

目标 flow：

```text
CallIr native target
  -> linker validates target exists and type args are resolvable
  -> NativeSignatureRegistry resolves NativeCallPlan
  -> eval evaluates arg RuntimeValue
  -> boundary contract validates/materializes args
  -> native adapter builds NativeCallContext from plan + args + declared contexts
  -> NativeCapability dispatches
  -> boundary contract validates/materializes return
```

规则：

- native handler 不能重新查 caller executable signature 来决定自己的参数或返回类型。
- `NativeCallPlan` 是 signature resolution 的结果，不含 `RuntimeValue` argument；包含
  `RuntimeValue` argument 和 host-provided context 的 `NativeCallContext` 只在 dispatch 前构造。
- native handler 不能自己 materialize arbitrary JSON，除非该 native 的语义就是
  `std.json` / opaque JSON。
- capability context 必须可测试：单测可以只构造该 capability 所需 context，而不是完整
  request frame。

## Package-Test Flow

目标短期 flow：

```text
package-test artifact inputs
  -> PackageTestRuntimeBuilder
  -> synthetic ArtifactGraph
  -> runtime-linker
  -> runtime-activation
```

后续 host/request 集成 flow：

```text
LinkedProgramImage + RuntimeActivation
  -> ordinary ServiceRuntimeContext
  -> ordinary RequestExecution
```

目标长期 flow：

```text
test runner / artifact generation
  -> synthetic service artifact
  -> production runtime ordinary artifact loader
```

规则：

- package-test link policy 可以存在，但必须局限在 package-test builder 内。
- production request runner 不理解 package-test entrypoint discovery。
- production host 只看到普通 linked image / activation。
- package-test crate 不依赖 runtime-request；它的输出边界是 synthetic `ArtifactGraph`、
  `LinkedProgramImage` 和 `RuntimeActivation`，后续 request execution 由 host/request
  集成路径负责。

## Suggested Promotion Order

crate 拆分顺序应从低层到高层：

0. `skiff-runtime-model` 的最小 DTO / value / type-plan 子集。
1. `skiff-runtime-loader`。
2. `skiff-runtime-boundary`。
3. `skiff-runtime-native-contract`。
4. `skiff-runtime-linker`。
5. `skiff-runtime-activation`。
6. `skiff-runtime-capability-context`。
7. `skiff-runtime-native`。
8. `skiff-runtime-eval`。
9. `skiff-runtime-package-test`。
10. `skiff-runtime-request`。
11. `skiff-runtime-transport`。
12. `skiff-runtime-host` facade 瘦身。

原因：

- boundary 最能直接阻止类型转换行为漂移。
- boundary 独立前必须先抽出它依赖的最小 runtime model；不能让 boundary 反向依赖现有
  `runtime` crate。
- loader 必须在 linker/activation 前收口，否则 artifact file IO 和 cache policy 会继续泄漏到
  linked image / activation 层。
- native contract 必须先于 linker / native dispatch 固化，避免 linker 依赖 handler crate。
- linker 和 activation 依赖方向清晰，适合早期用 crate 边界固化。
- capability context 必须先于 native/request 收口，避免 native handler 反向依赖 host/request。
- native 需要先收敛 registry source of truth，再提升。
- eval 在 native、boundary、activation read-only facts 稳定后提升，避免 request crate 吞掉
  interpreter core。
- package-test 可以单独隔离 production runtime surface。
- request 必须先拥有 `BoundaryResponse` / `ResponseEvent` contract，transport 才能依赖它并
  实现 event -> protocol frame mapping。
- transport 在 request contract 稳定后提升，避免 request runner 继续直接 encode frames。
- host 依赖最多，应在底层边界稳定后再拆 crate，否则容易制造循环依赖。

## Acceptance Criteria

目标态满足以下检查：

- `runtime-boundary` 不依赖 host、eval、request、transport writer、service DB client。
- `runtime-loader` 输出 `ArtifactGraph`，不输出 linked image、activation 或 service runtime
  context。
- JSON、binary payload、HTTP typed body/response、native args/return、DB business value 对同一
  `RuntimeTypePlan` 的接受 / 拒绝规则来自同一 boundary contract。
- native binding 的 signature、type args、arg plans、return plan 和 required context 只定义在
  `skiff-runtime-native-contract`；handler 只定义在 `skiff-runtime-native`，并通过 binding
  key 引用 contract spec。
- `skiff-runtime-native` 不复制 signature、arg plan、return plan 或 required-context metadata。
- linker 不依赖 `skiff-runtime-native` handler / dispatch crate，只依赖
  `skiff-runtime-native-contract`。
- linker 输出 `LinkedProgramImage`，不构造 DB client、不注册 route、不执行 request。
- activation 输出 `RuntimeActivation`，不读取 artifact files、不修改 route registry。
- capability context 不暴露 full host/service context；native handler 只能获取
  `NativeRequiredContext` 声明过的 context。
- request crate 产出 response events，不直接构造 protocol frame header 或调用 binary frame
  encoder。
- transport crate 只做 protocol frame encode/decode 和 socket session，不做 business type decode
  或 route lookup。
- `RuntimeHost` 下层 API 不接收 `&RuntimeHost`。
- request crate 不依赖、不持有 `RouteRegistry`；它只消费 host 已解析的
  `ServiceOperationContext`。
- request runner 中没有一个大函数同时处理所有 ingress mode、response framing、package-test
  wrapper、telemetry、cancellation 和 eval dispatch。
- package-test synthetic service 构造不在 production host / request runner 主路径中。
- typed runtime pipeline 中的 `serde_json::Value` 只剩 opaque JSON、third-party schema 或
  raw artifact/config 边界。

## Relationship To Existing Documents

- `runtime-value-layout-and-type-erasure.md` 定义普通 runtime value、request-scope memory、
  nominal erasure 和 boundary 编解码原则；本文把这些原则放入 crate / 模块边界。
- `runtime-compiler-shared-artifact-types.md` 定义 artifact DTO 和 linked runtime overlay 的归属；
  本文进一步定义 artifact graph、linked image、activation、host 的长期依赖方向。
- `gateway-runtime-adapter-boundary.md` 定义 router gateway 和 runtime adapter 的边界；本文只
  定义 runtime 内部如何接收 adapter mode 并执行 request。
- `../implementation/runtime-architecture-boundaries.md` 是现状问题记录；本文是目标态契约。
- `../implementation/runtime-boundary-convergence-implementation.md` 是迁移计划；若它与本文冲突，
  以本文为准。
