# Skiff Interface Reference

本文负责：稳定描述 Skiff `interface` 的目标态用户语义，包括声明、显式 conformance、泛型
interface、method requirement、marker interface、public instance 和 boundary 限制。first-class
interface value（`any I`，含远程能力）见 `any-interface.md`；capability binding 已退役并合并进 `any I`。

本文不负责：parser 现状、artifact 字段表、runtime vtable 实现、package binding manifest 完整
schema 或 actor manager 内部流程。发布和 binding 投影见 `publication.md`。

## 1. 定位

interface 是名义能力契约，不是结构类型，也不是数据 shape。

```skiff
interface ManagedLlmService {
  function sendChat(self: Self, input: ManagedLlmChatRequest) -> Stream<LlmStreamEvent>
}

type ManagedLlmServiceImpl implements ManagedLlmService {
  model: string,
}
```

核心规则：

- conformance 必须由 nominal type 显式写 `implements`。
- compiler 不按字段或 method set 做 structural matching。
- interface 本身没有 runtime object layout，不是可序列化 payload。
- interface 可以作为 compile-time contract、Publication ABI metadata、public instance conformance 和 marker。
- **裸** interface 名不当作普通 first-class value 传递、存储、返回或放进容器；`function f(x: I)` 这类裸
  interface 参数不是合法 value signature。需要 first-class 动态值时用 `any I`（见 `any-interface.md`），
  它可在 publication 内部流动，装箱源可以是本地 concrete record 或远程 public instance。

## 2. Interface Declaration

interface 是顶层声明，可以有类型参数；是否成为外部源码可写 public name 由 publication public
API graph 决定：

```skiff
interface Repository<Id, Entity> {
  function get(self: Self, id: Id) -> Entity?
  function put(self: Self, id: Id, entity: Entity) -> void
}
```

interface body 只包含 method requirements。空 body 合法，用于 marker interface：

```skiff
interface Actor<Id> {}
```

第一版不定义 interface field requirement，也不在 interface declaration 内定义 default method body。
如果需要共享 helper 行为，应在后续单独设计 interface extension / default method 机制；不要让
interface 同时成为第二套 record shape。

## 3. Self And Method Requirements

interface method requirement 使用普通函数签名，receiver 参数写 `self: Self`：

```skiff
interface QueueSink<Item> {
  function push(self: Self, item: Item) -> void
}
```

`Self` 只在 interface method requirement 的 receiver 参数中有特殊含义。它表示“实现该 interface 的
concrete nominal type”。`Self` 不应出现在普通 type、alias、record field、service boundary payload
或非 receiver 参数位置。

concrete type 实现 interface 时，compiler 按 type 参数替换 requirement，然后检查该 concrete type 的
method namespace 中存在匹配 method：

```skiff
type NotificationSink implements QueueSink<Notification> {
  channelId: ChannelId,
}

impl NotificationSink {
  function push(self: NotificationSink, item: Notification) -> void {
    ...
  }
}
```

匹配必须基于 canonical method identity 和完整签名。参数数量、参数类型、返回类型、stream / effect /
throw metadata 和后续 ABI 相关 metadata 不匹配时，conformance 失败。compiler 不允许靠同名但不同签名
的方法“差不多匹配”。

第一版不支持同一 interface 内的 method overload。源码 method token 必须在该 interface 内唯一映射到
一个 canonical `method_abi_id`；重名 method 即使签名不同也必须报错。

## 4. Explicit Conformance

conformance 写在 nominal record type declaration 上：

```skiff
type ManagedLlmServiceImpl implements llm.ManagedLlmService {
  model: string,
}
```

规则：

- 只有 nominal record type 可以声明 `implements`。
- `type Name = R` representation、`alias`、anonymous record、primitive、union branch 和 literal
  不能直接声明 conformance。
- implements list 中必须写完整 interface type arguments。
- 同一个 nominal type 对同一个 interface symbol 第一版最多实现一次；不能同时实现
  `Partitioned<UserId>` 和 `Partitioned<TenantId>`。
- conformance 是 public / package ABI fact。删除 conformance 或改变 type arguments 必须改变相关
  publication identity，并让依赖方 fail closed。

## 5. Generic Interface Identity

所有 public、binding、operation 和 conformance contract 都使用完整 interface instantiation，而不是裸
interface declaration id：

```text
InterfaceInstantiationRef {
  interface_abi_id: AbiInterfaceId,
  canonical_type_args: [AbiTypeRef]
}
```

non-generic interface 的 `canonical_type_args` 是空数组。generic interface 必须 fully substituted 后
才能进入 Publication ABI、`any I` 类型 fact、operation projection、conformance fact 和 ABI identity。
`Actor<ThreadId>` 和 `Actor<UserId>` 是不同 interface instantiation；equality、hash、sort 和
compatibility check 都必须比较完整 `InterfaceInstantiationRef`。裸 `interface_abi_id` 只能表示
interface declaration 自身，不能单独表示 public/binding contract。

generic interface 不提供 associated type inference。需要的类型关系必须在 `implements` 中显式写出。
这让 `Partitioned<Id>` 这类 marker 能稳定定义资源 id 类型。

## 6. Marker Interfaces

marker interface 没有 method requirement，只表达 compile-time fact：

```skiff
interface Partitioned<Id> {}

type ChatPartition
  implements Partitioned<ChatId>
{
  id: ChatId,
}
```

marker 仍然是显式 conformance，不是注解字符串。compiler 和 artifact 可以读取该 conformance，用于：

- 资源分区或 shard id type binding。
- platform error marker，例如 `ErrorPayload`。
- publication / package capability metadata。
- 后续安全能力标记。

marker 不应该引入 runtime wrapper value。

## 7. Interface-Typed Access

裸 interface 名不作为普通 first-class value：

- 不能声明普通函数参数或返回值为裸 interface 并在 runtime 传递（用 `any I`，见 `any-interface.md`）。
- 不能把裸 interface 值存入 record 字段、DB schema、public API payload 或 collection。
- 不能手写 interface literal。
- 不能把 concrete object 隐式装箱成 interface value（必须显式 `as I`）。

first-class interface value（动态分派）是 `any I`，见 `any-interface.md`：`any I` 类型 / `as I` 显式
装箱（装箱源可以是本地 concrete record 或远程 public instance）/ 裸 interface 名不能当类型。一个能力
可以作为 `any I` 值赋值、传参、放入 `Array<any I>`。进入 DB/spawn/queue/persistent 等跨 request
边界时按 recoverable boundary policy 判定；ordinary public schema 仍不承载 `any I` 默认 wire shape。见
`any-interface.md` 与 `recoverable-value.md`。

> **已更新（2026-06-24）**：原本"interface-typed access 只能是不可流动的受控 receiver root（binding
> alias / dependency public instance），不能传参/存储/进容器"的限制已被 `any I` 合并案取代。能力现在
> 是可流动的 `any I` 值。跨 service 寻址用 `/`（如 `remoteLlm/managedLlm`），不用 `.`。直接调用
> `remoteLlm/managedLlm.method(...)` 与装箱 `remoteLlm/managedLlm as I` 都合法；裸 `remoteLlm/managedLlm` 仍不是
> first-class 值（是装箱源 / 寻址 root），只能在 `.method()` 或 `as I` 左边出现。详见 `any-interface.md`。

pattern / narrowing 可以用 interface conformance 做有限类型测试，但不创建 interface value。当前
publication 的普通 public const 不会自动成为可调用 root。

`any I` 的 value layout 见 `any-interface.md §3` 和 `../architecture/any-interface-value.md`（含本地 /
远程两类装箱源的泛化布局）。不能把 ordinary object 的 per-instance runtime shape 偷偷扩展成隐式 vtable。

## 8. Public Instance（capability binding 已退役）

> **已被取代（2026-06-24）**：package capability binding 机制已合并进 `any I`。下文保留 public instance
> 的 `api.yml` 公开规则与 operation identity（这些仍有效），但 `requires.bindings` manifest 机制退役——
> package 抽象依赖能力改为"入口吃 `any I` 参数 + consumer 调用点用 `as I`（本地/远程）装箱传入"。详见
> `any-interface.md §8`。

public instance 是 `api.yml` 显式公开的、可被装箱/调用的 receiver root（见 `publication-api-yml.md`）。
它不是 interface 自身，也不是普通 public const 自动派生的能力。`public_instance_key` 是完整 API graph
public path，例如嵌套 `llm.managed` 的 key 就是 `llm.managed`，不是 leaf/display name。consumer 跨
service 引用它用 `/`：`remoteLlm/managed`。

public instance method 的 operation identity 是 `operation_abi_id`。dependency compiler 使用
`PublicationAbiUnit.sourceCallOperationIndex` 从完整 source-call path
`<public_instance_key>.<method>` 解析到唯一 `OperationAbiRef`；runtime/linker/provider 只按
`operation_abi_id` 执行或路由，不按 public instance 名、source method name、interface id + method name
做动态派发。private receiver concrete type 只属于 implementation/runtime validation，不进入 public
contract identity。远程 `as I` 装箱正是把这个 `(public_instance_key, operation_abi_id)` 寻址包进一个
`any I` 值；fail-closed 在装箱点锁 callee protocol identity（见 `any-interface-value.md §Remote
Fail-Closed`）。

## 9. Boundary Rules

裸 interface 不是 schema-closed 类型。以下位置禁止出现 interface：

- service operation 参数或返回 payload。
- public API type 的 schema closure。
- service DB schema。
- cross-service payload。
- record 字段、collection element、persistent work item payload。
- ordinary JSON materialization。

interface 可以出现在 publication metadata、public instance conformance metadata、`any I` 类型
fact（含 package 入口的 `any I` 参数）、远程 `as I` 装箱锁定的 callee conformance metadata 和 compiler
内部 type-check facts 中。

## 10. Pattern And Narrowing

interface pattern 是 conformance test，不是 structural match：

```skiff
match value {
  ManagedLlmService {} => ...
  _ => ...
}
```

该 pattern 只在 compiler 能证明 scrutinee 的可能 runtime values 都携带 concrete nominal identity 时
合法。匹配依据是 concrete type 是否显式 implements 目标 interface instantiation。

record field destructuring 不通过 interface pattern 完成。需要按字段处理数据时，应使用 nominal record
type 或命名 union，而不是 interface field requirement。

## 11. Artifact Requirements

compiler / artifact 必须保存足够的 interface metadata：

- interface symbol identity。
- interface type parameters。
- method requirement signatures。
- nominal type 的 explicit conformance list，包括完整 type arguments。
- public instance 使用到的 implemented interface identities。
- `any I` 装箱点的 interface identity；远程 `as I` 装箱还需保存 callee `(public_instance_key,
  operation_abi_id)` 寻址与锁定的 protocol identity（见 `any-interface-value.md §Remote Fail-Closed`）。

runtime 执行 concrete method 时不需要给 ordinary object 附加 interface identity。interface metadata
只用于 compile / link / publish 阶段的静态解析、ABI identity 和 compatibility check。runtime 不依据
interface id + method name 做动态派发；实际 call target 必须已经链接成 local executable、package
operation target 或 service `operation_abi_id`。

## 12. Non-Goals

第一版不实现：

- structural interface matching。
- interface field requirements。
- interface declaration 内 default method body。
- ordinary first-class interface values。
- implicit boxing / unboxing。
- runtime service locator 或按字符串选择 provider。
- interface object 存储、序列化或跨请求持有。
