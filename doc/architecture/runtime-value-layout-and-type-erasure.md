# Runtime Value Layout And Type Erasure

本文定义 Skiff runtime value layout、request-scope memory、边界编解码和 source
nominal type erasure 的长期内部契约。它面向 compiler/runtime 维护者，不是用户可见
reference，也不是迁移 checklist。迁移步骤记录在
`../implementation/runtime-type-erasure-implementation-plan.md`。

Skiff 尚未发布。本文目标态不要求兼容旧 artifact、旧 runtime payload、旧 DB fixture
或旧 `__skiffType` wire shape。

## Scope

本文负责：

- 普通 runtime value 的物理形态。
- 哪些对象可以进入 request-scope memory。
- compiler、artifact descriptor、linked plan 和 runtime value 之间的 nominal identity
  边界。
- JSON、runtime binary payload、DB、exception、interface/method dispatch 的编解码原则。
- `Duration` 这类 nominal representation 类型在运行时如何擦除。

本文不负责：

- 用户可见语法和完整标准库 API。
- artifact DTO ownership。该契约见
  `runtime-compiler-shared-artifact-types.md`。
- compiler publication pipeline 的完整阶段定义。该契约见
  `compiler-publication-pipeline.md`。
- 具体 Rust 模块拆分和迁移步骤。

## Core Rule

普通 runtime value 必须按物理形态执行，不携带 source-level nominal type identity。

source nominal identity 只允许存在于：

- compiler 的 source/type/expression facts。
- canonical artifact DTO 和 contract/runtime descriptors。
- runtime linking 之后的 `RuntimeTypePlan`、call target、boundary plan、method table 或
  interface adapter table。
- exception envelope、显式 tagged value、未来显式 interface/vtable value 这类主动建模的
  dynamic value。

source nominal identity 不允许存在于：

- 普通 `RuntimeValue` variant。
- 普通 `HeapNode::Object`、array、map、bytes。
- request heap node 中的 per-instance type name string。
- map key 的 runtime identity。
- DB business document。
- `std.json.encode` / HTTP JSON response 默认输出。

这意味着 `UserId("1")` 和 `TenantId("1")` 在静态类型检查之后可以拥有同一个 runtime
payload。union 是 compiler/link/boundary plan 层的 expected type fact，不是 runtime
value wrapper。`Map<UserId | TenantId, V>` 的 runtime key 是 plain string；
`UserId("1")` 和 `TenantId("1")` 就是同一个 key。只有当某个语言机制或边界必须在运行时
从 erased payload 反推出 nominal branch identity 时，才需要显式 tag，或在 compiler/link
阶段拒绝该用法。

## Runtime Value Layout

目标态 runtime value 只有两层：

```rust
enum RuntimeValue {
    Null,
    Bool(bool),
    Number(f64),
    Date(i64),
    String(RuntimeString),
    ActorRef(ActorRef),
    Heap(HeapHandle),
}

enum HeapNode {
    Bytes(RuntimeBytes),
    Array(Vec<RuntimeValue>),
    Object(RuntimeObject),
    Map(RuntimeMap),
    Resource(RuntimeResourceHandle),
}
```

上述 Rust-ish 类型只表达契约，不要求字段名和当前实现一致。

规则：

- `integer` 继续使用 `Number(f64)` 的 safe-integer 子集，除非未来引入独立 integer
  storage。
- `Date` 是 inline scalar，存 epoch milliseconds。
- `Duration` 是 nominal representation over integer milliseconds。运行时按 safe integer
  milliseconds 执行，不新增 heap wrapper。
- `type UserId = string`、`type TenantId = string` 等 representation 类型运行时擦成
  payload。
- record/object 值不保存 source type name。目标态 `RuntimeObject` 不得带任何承载 source
  type name 的字段，特别是不允许 `shape: Option<String>` 这种字段——无论它名义上叫
  shape、type、还是别的。decode 直接生成 unshaped object，由 expected plan
  materialize。
- runtime 不需要、也不应该为普通 object 保留 per-instance 的 source nominal identity。
  反射不是理由：shape 在编译期已知，反射信息由 compiler 写入 descriptor，不必挂在每个
  运行时 object 上。性能不是理由：本契约不为任何 hidden-class / shape-cache 优化预留口
  子。若将来确有此需要，它是一次独立的 runtime 内部优化提案，且必须满足：id 是整数
  newtype、不是 source name、不存在从该 id 反查 source name 的表、不参与任何 boundary
  输出或语言语义。在该提案落地前，目标态 object 没有 shape id。
- `ActorRef` 是被祝福的例外（见 Core Rule 第二节白名单）。它是主动建模的 dynamic
  value，不是普通 erased value。它携带 actor type 和 actor id，承载真实运行时语义
  （method table lookup、actor manager routing），不是被擦除的 source nominal type。
  详见「Interface And Method Dispatch」。
- `HeapHandle` 是 request-local id，不是 ABI，不得跨 request、artifact、service 或
  actor method call 泄漏。

禁止的目标态形态：

```rust
HeapNode::Representation { type_name: String, payload: RuntimeValue }
RuntimeValueKey::StringRepresentation { type_name: String, value: String }
RuntimeObject { shape: Option<String>, ... } // 任何 String shape 字段都禁止，不论是否"仅用作优化"
```

## Request-Scope Memory

request-scope memory 是一条 request 生命周期内的 runtime-owned 内存集合。它至少包括：

- request heap nodes。
- call frames 和 slot values。
- concurrent lane / join state。
- request-local exception envelope 和 catch result materialization。
- request-local stream、resource、file、outbound handle。
- request-local mutation state。

request 结束时，这些 state 整体释放。任何 heap handle、stream handle、resource
handle、`Exception<E>`、`CatchResult<T, E>` 都不能逃逸到 request 结束之后。

可以放入 request heap：

- 变长值：array、map、object、bytes。
- 需要 mutation 的 collection/object。
- 需要共享引用或 cycle check 的 runtime graph node。
- request-local resource/stream/file handle 的显式 wrapper。

不应放入 request heap：

- source type name、artifact descriptor、`RuntimeTypePlan`、method table。
- alias/representation wrapper。
- config metadata、storage binding / schema plan、route map、package export overlay。
- 只为通过类型检查而存在的 nominal identity。
- 可以 inline 的 scalar：bool、number/integer、Date、Duration payload、string value
  reference。

memory accounting 必须按物理 runtime value 计费。`NODE_OVERHEAD_BYTES` 这类估算常量只
能描述 request heap node 管理开销，不能被解释为语言对象固定 ABI 大小。source type
name 不应出现在 per-instance memory estimate 中。

## Boundary Decode And Encode

所有边界编解码都必须 expected-type driven：调用方提供 expected type plan，codec 根据
该 plan 校验和解释 payload。codec 不应从 payload 中读取普通 source nominal type tag
来决定类型。

### Runtime Binary Payload

runtime binary payload 用于 router/runtime、runtime/runtime 或 service call 的业务
payload。它有 expected `RuntimeTypePlan`，因此 representation 不需要写 type name。

规则：

- `Alias` 和 `Representation` 编码成 payload type 的编码。
- `Nullable` 可以使用 null discriminant。
- `Union` 是 expected type plan，不是 runtime value wrapper。codec 可以在分支擦除后仍可
  明确区分时选择分支；如果边界语义要求恢复 erased nominal branch identity，必须使用显式
  tagged value 或在 compiler/link 阶段拒绝该边界。
- `Record` 编码字段，不编码 source record type name。
- `Map<UserId, V>` key 编码成 string payload。`Map<UserId | TenantId, V>` 的 key 也编码成
  plain string；擦除后的 key collision 是该类型的运行时语义，不通过 runtime key identity
  保存 nominal 差异。
- `Date` 编码 `i64` epoch milliseconds。
- `Duration` 编码 safe integer milliseconds。

runtime payload format 可以在本迁移中破坏并提升 version；Skiff 未发布，不保留旧
`TAG_REPRESENTATION` 兼容。

### JSON Boundary

JSON 边界包括 HTTP JSON request/response、`std.json.encode<T>`、
`std.json.decode<T>` 和显式 JSON config/fixture decode。它们仍然 expected-type
driven。

规则：

- `std.json.decode<UserId>("\"u1\"")` 返回 runtime string payload，并由 expected type
  校验。
- `std.json.encode<UserId>(id)` 输出 `"u1"`，不输出 `__skiffType`。
- `Json` / `JsonObject` 是 JSON value 语义，不是保存 Skiff nominal metadata 的通道。
- bytes 可以使用显式 bytes protocol envelope 或 bytes-only transport 计划定义的
  binary segment，但不能使用 nominal type envelope。
- `Date` 输出 RFC3339 UTC string。
- `Duration` 初始 JSON 表示为 integer milliseconds。若以后改为 ISO-8601 duration，
  必须作为 reference/API 变更单独定义。

`__skiffType` 不属于目标态 JSON 边界。迁移期间如果仍保留识别逻辑，只能作为 fail
closed 或 legacy fixture 拒绝路径，不能作为生产语义。

### DB Boundary

DB document 不保存 Skiff 元数据字段。collection selection 来自调用的 operation /
collection binding，result decode 使用 operation result plan，不来自 document field。

规则：

- business document 不存 `__skiffType` 或任何其他 Skiff type metadata。
- insert/update 写入业务字段，不写 source type metadata，也不递归 strip nominal metadata。
- find/query result 按 operation result plan decode。
- collection key、indexes、cascade 行为由 storage binding / schema plan 决定，不由
  document 内嵌元数据决定。
- 查询和 update path 不允许依赖 `__skiffType` 或任何 `__skiff*` 字段。
- 不把 `__skiff*` 当成 Skiff 特殊字段前缀。除 storage layer 特殊处理 `_id` 之外，字段名是否
  允许只受普通业务 schema / storage backend 规则约束。

### Exception Boundary

exception 是需要 runtime nominal identity 的显式动态值。它必须使用 exception envelope
建模，而不是要求 payload object 自带 `__skiffType`。

目标态：

```rust
struct UserException {
    actual_payload_type: TypeIdentity,
    declared_payload_type: Option<TypeIdentity>,
    payload: RuntimeValue,
    source: ExceptionSource,
    stack: Vec<FrameInfo>,
}
```

`TypeIdentity` 是本契约要求新引入的 runtime 类型，当前代码尚不存在。它必须复用 link 阶段
已有的 type id 空间，由 compiler lowering 携带、runtime 直接写入，**不得**在 runtime
重新发明一套与 compiler `ResolvedTypeIdentity`、artifact descriptor type id 不一致的编号。
避免重蹈"同一 identity 三处手写、必须逐字节一致"的覆辙：catch 比较只在这一个 id 空间内
进行。

规则：

- compiler lowering 必须让 `throw expr` 携带 payload static type 或 linked type id。
- runtime 构造 exception 时把 type id 写入 exception envelope。
- payload 本身按普通 runtime value 保存，不带 source type field。
- `catch<E>` 比较 envelope 中的 `actual_payload_type` 和 catch leaves。
- rethrow 校验 operand 是 request-local exception envelope，不从 payload 推断类型。

如果 `Exception<E>` 或 `CatchResult<T, E>` 作为普通值暴露给用户，它是显式 request-local
runtime envelope，不是普通 record 的隐藏 metadata。

## Compiler Responsibilities

类型擦除把责任前移到 compiler。compiler 必须在 source/type/lowering 阶段完成以下
工作：

- `UserId` 和 `TenantId` 的 assignability、constructor、field access、generic binding
  和 API boundary descriptor 校验。
- `Duration` 的单位安全。`std.time.sleep` 应接收 `Duration`，用户通过
  `Duration.milliseconds`、`Duration.seconds` 或 duration literal 构造值。常量参数的
  overflow/safe-integer 校验也在编译期完成（见 Duration 章节，变量参数由 runtime 校验）。
- map key 按擦除后的 key identity 执行；`Map<UserId | TenantId, V>` 这类类型的 key 是
  plain string。只有 union branch、pattern match、dynamic boundary 等用法需要从 erased
  payload 恢复 nominal branch identity 时，compiler/link 才必须提前拒绝或要求显式 tag。
- 用户 `impl` receiver call 必须静态解析为 executable call，例如
  `user.displayName()` 降为 `User.displayName(user)`。
- `DynamicReceiver` 只允许用于 built-in shape methods、actor refs，以及未来显式
  interface/vtable value。
- `throw expr` lowering 必须携带 payload type identity，不依赖 runtime value introspection。
- 普通 `match value { User(...) => ... }` 这类 nominal type pattern 不能依赖普通
  object metadata。它要么被静态化，要么只允许匹配显式 tagged value。

## Interface And Method Dispatch

当前 interface/conformance 是 contract/static ABI 事实，不要求普通 runtime object
保存 interface identity。

如果未来支持 first-class interface value，目标形态应是显式 value：

```rust
struct InterfaceValue {
    interface_id: InterfaceId,
    value: RuntimeValue,
    vtable: VTableId,
}
```

该 interface value 可以是 inline pair 或 heap node，但它是显式动态值，不是所有 object
共有的隐藏字段。

method dispatch 分层：

- user `impl` method：compiler/link 静态解析到 executable address。
- built-in receiver method：按 physical variant 分派，例如 `Array`/`String`/`Date`/
  `Bytes`。"physical variant" 指 runtime value 的物理 enum tag，不是 name string——
  dispatch 直接 match variant，不查任何类型名。
- actor method：按 `ActorRef` 分派，等价于按 actor type 查 RuntimeProgram 的 actor method
  table，并按 actor id 交给 actor manager 路由。`ActorRef` 的 actor type 和 id 因此是必须
  保留的运行时 identity，不是被擦除的 source nominal type。这是 Core Rule 白名单允许的
  dynamic value nominal identity，不构成普通值带 source type name。
- interface method：未来按显式 interface value/vtable 分派。

普通 object 不参与 source type name lookup。

### Built-in Record-Shaped 类型

`std.http.HttpRequest`、`std.http.HttpResponse` 这类 built-in record-shaped 类型没有
独立的 `RuntimeValue`/`HeapNode` physical variant，物理上就是 `HeapNode::Object`。它们
**同样不得**用 `RuntimeObject` 上的 source name shape 字段来标识自己。这些类型的语义由
boundary plan / built-in descriptor 在边界处提供（HTTP runtime 知道自己在构造
`HttpRequest`），不靠运行时 object 自带的 type name。也就是说，built-in record-shaped
类型与用户 record 在 runtime value layout 上无差别：都是 unshaped object，type identity
来自 expected plan，而不是 per-instance 字段。

## Duration

`Duration` 的目标语义：

```skiff
type Duration = integer

impl Duration {
  native static function milliseconds(value: integer) -> Duration
  native static function seconds(value: integer) -> Duration
  native function toMilliseconds() -> integer
}

native function std.time.sleep(duration: Duration) -> void
```

规则：

- runtime payload 是 safe integer milliseconds。
- `Duration.seconds(n)` 的 overflow/safe-integer 校验分两处，互补而非二选一：
  - compiler 对常量参数（`Duration.seconds(7200)` 这类编译期可知值）在常量折叠时校验，
    溢出直接报 compile error。
  - runtime 对变量参数（`Duration.seconds(userInput)`）在 native 构造时校验，溢出抛运行时
    异常。
- `std.time.sleep` 只读取 milliseconds payload。
- `integer` 不能隐式传给 `Duration` 参数，除非 compiler 明确定义并校验该 coercion。目标态
  不提供该隐式 coercion。
- JSON/API 边界初始使用 integer milliseconds，由 descriptor/contract 表明逻辑类型是
  `Duration`。

## Audit Targets

目标态达成后，以下搜索应只命中文档、legacy rejection tests 或显式 exception/tagged
value 代码。本清单应与 implementation plan 的 Phase 8 audit 集合保持一致（互为超集），
新增反模式时两处一起更新：

```bash
rg "__skiffType" runtime/src compiler/src
rg "HeapNode::Representation|alloc_representation|TAG_REPRESENTATION" runtime/src
rg "StringRepresentation" runtime/src
rg "DynamicReceiver" compiler/src runtime/src
rg "runtime_nominal_type" runtime/src/interpreter
# RuntimeObject 不得有 String shape 字段，也不得用 source name 构造 object：
rg "shape:\s*Option<.*String|ObjectShapeId|RuntimeObject::new\(" runtime/src
```

预期：

- 普通 decode/encode/coerce 不分配 representation heap node。
- runtime payload codec 不写 representation type name。
- `RuntimeObject` 没有 source name shape 字段；没有任何生产路径把 source type name 当
  per-instance object identity 写入或读出（`nominal_type()` 等不再从 object 拿 shape）。
- DB business documents 不存 Skiff metadata field；`__skiff*` 不作为 Skiff 特殊字段前缀
  特殊处理，只有 `_id` 属于 storage layer 特殊字段。
- `throw` 不通过 payload object 的 field 推断 type。
- 普通 user impl method call 不走 runtime dynamic receiver dispatch。
- request heap estimate 不包含 source type name per-instance cost。
- 唯一保留 nominal identity 的运行时值是白名单内的 dynamic value：exception envelope、
  显式 tagged value、`ActorRef`、未来的显式 interface value。
