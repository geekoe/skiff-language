# Any Interface Value Architecture（含远程能力合并）

本文定义 `any I` first-class interface value 的长期内部架构契约。用户可见语义以
`../reference/any-interface.md` 为准；本文只规定 compiler、artifact、linker 和
runtime 如何承载该语义，以及它与现有静态 interface / conformance / type erasure 架构的边界。

**本版合并案（2026-06-24）**：`any I` 的载体从"本地 concrete 值"扩展到"本地或远程装箱源"，由此把
package capability binding 合并进 `any I`——远程能力是一个**可流动的 `any I` 值**，载体是
service contract operation 寻址而非进程内指针。本文据此取代既有的 package capability binding
定位：

- binding 作为"构建期静态、单点绑定（每个 requirement 恰好一个实现、
  表达不了运行期变长异构集合）、不可流动的受控 root"的定位被合并案取代。它原本解决的问题（package
  抽象依赖能力、由 consumer 决定本地/远端）仍成立，但实现形态从"受控 root"改为"`any I` 参数 + 装箱源"。
- 本文旧版"`any I` 不跨 service / 不模拟 remote dispatch"的绝对排除被放开为"远程装箱源经显式
  `as I` 进入 `any I`，远程性由 `carrier = Remote` 表达"。

Skiff 尚未发布。本文目标态不要求兼容旧 parser、旧 File IR、旧 artifact schema 或旧 runtime value
layout。

## Scope

本文负责：

- `any I` 在 source type facts、File IR、linked runtime plan 和 runtime value 中的归属。
- `expr as I` 装箱点如何生成显式动态值，含**本地 concrete 值**与**远程 public instance 寻址源**两类装箱源。
- `any I` method call如何经interface method table（本地）或`ContractOperationId`（service carrier）
  分派。
- 远程装箱的 fail-closed 锚点（装箱点锁定 callee protocol identity）。
- `any I` 与 ordinary object type erasure、package/public ABI、service boundary、DB 和 JSON 的边界。
- ordinary aggregate snapshot、writable place 与 `inout` 对 interface dispatch 的硬限制。
- runtime interface target 如何归属 exact deployment `buildId` execution owner。
- generic interface instantiation、object-safety、concrete receiver identity 和 method slot identity 的长期约束。

本文不负责：

- 用户语法的完整 reference。见 `../reference/any-interface.md`。
- 静态 interface / explicit conformance 的完整语义；该契约归 `../reference/interface.md`。
- 具体 Rust 模块拆分、迁移步骤和任务顺序；这些属于实现计划，不写入本 architecture 文档。
- durable / 跨 request 持有的远程能力句柄（remote capability transport）、service callback 的 wire
  protocol、downcast、reflection 或 marker interface runtime value。Service callback 的 value/projection
  语义仍由本文约束；其 boundary/transport owner 见 `package-service-contract-deployment.md`。

### 当前实现边界

必须区分三条形似“回调”的路径，不能互相当作完成证据：

1. Package-local `any I`：例如 Agine 把本地 LLM/event adapter 传入 Agent package，adapter 内部再发普通
   Agine → AIHub service call。这始终是 `Local` method-table dispatch 加一条正向 service operation，不是
   callback capability，也没有把 `any I` 送过 Router。
2. Service operation 顶层 `any I` 的 boundary projection：当前 Runtime 已能在同一 runtime/execution
   assembly 内登记 opaque capability，并在 provider 调用时切回 owner；这是 `InProcessBoundary`。
3. 跨 runtime/Router 的反向 callback：当前 Router wire 没有 callback request/cancel/response family，
   capability 也不能经 JSON、recoverable 或 DB codec。`RemoteBoundary` 在新增 owner route、认证、lifetime、
   cancellation、deadline 与 backpressure 协议前必须 fail closed；它不是 bytecode VM 单独能补齐的能力。

## Position

`any I` 是 Skiff 第一种普通用户可见的显式动态分派值。它**吸收并取代**既有 package binding alias /
dependency public instance root 的"受控 root"形态——把它们从"不可流动的编译期 root"升格为"可流动的
`any I` 值"。它不是 actor ref 的重命名。

现有 interface 用途，合并后收敛为：

- compile-time contract：`type T implements I` 和 conformance checking。
- ABI metadata：public instance 和 Package Local ABI / ServiceContract metadata（binding requirement 退役，见 §Capability As Parameter）。
- `any I` first-class dynamic value：见下。

`any I` 的核心用途：

- linked-program-local dynamic value：值可放入局部变量、普通内部函数参数/返回、不经任何 boundary schema closure
  投影的内部 record 字段（含具名 record 类型）和 collection；调用时按 interface method table（本地）或
  `ContractOperationId`（远程）分派。
- **本地与远程统一**：一个 `any I` 值的装箱源可以是本地 concrete record，也可以是远程 public instance
  寻址 root（如 `remoteLlm/managedLlm`）。两类装箱出的 `any I` 类型上不可区分，可混入同一个
  `Array<any I>`；区别只在值布局 `carrier` 是 `Local` 还是 `Remote` 分支（见 §Runtime Value）。
- **远程对象也是本地对象**：远程装箱值在持有它的进程里就是一份本地数据（载体是`ContractOperationId`
  寻址坐标，不是函数指针），可传可存可进容器。"远程"只体现在**调用方法时**走 service dispatch，
  不体现在"值作为数据存在"时。

“linked-program-local” 是 ordinary public schema 的硬边界：`any I` **值**不进入 service public API payload、
ordinary JSON materialization、public instance operation signature、config schema 或 test double external fixture schema
的默认 wire shape。但 DB schema、`dispatch`、queue / persistent work item 和 runtime 内部跨 request payload
已经由 `recoverable-value.md` 重新定义为 owner-internal recoverable boundary：`carrier = Local` 且 self payload 全可恢复时可恢复，
`carrier = Remote` 仍是 request-scope 正向远程引用、不可持久化。它**可以**作为Package public入口的参数类型
（Package link进consumer同一linked program，`any I`值不跨service boundary；远程性只在调用时dispatch）——见§Boundary Contract与
§Capability As Parameter。

## Type Model

Source type resolution 必须把 `any I` 解析为结构化 type fact，而不是把字符串 `"any I"` 传给下游重新解释：

```rust
enum ResolvedTypeRef {
    // existing variants ...
    AnyInterface {
        interface: InterfaceInstantiation,
    },
}
```

File IR / artifact DTO 也需要显式承载该类型，用于内部函数签名、局部值和 collection element：

```rust
enum TypeRefIr {
    // existing variants ...
    AnyInterface {
        interface: InterfaceInstantiationRef,
    },
}
```

Package callable不能把existential退化成`Local`或display string。Package Local ABI必须保存结构化
interface target与generic arguments：

```rust
AnyInterface {
    interface: Box<PackageTypeRef>,
    arguments: Vec<PackageTypeRef>,
}
```

Package-owned interface target在PackageArtifact中使用精确Package Local ABI nominal identity；
Package-local target保持local identity。Raw `AnyInterface`不能作为ordinary ServiceContract
`ContractTypeRef`或PackageSchema。唯一例外是reference明确允许的operation顶层、非泛型位置：
boundary projection把它转换为opaque request-scope callback-capability plan，而不是把
`AnyInterface`本身写入contract schema。其它位置必须返回结构化Unavailable原因。
`Nullable`和container继续在existential外层保持结构，generic arguments不并入显示名称。

规则：

- `interface` 必须是完整 `InterfaceInstantiationRef`。generic interface 必须带完整 canonical type args。
- 裸 interface instantiation 不能当普通 value type。`ToolProvider` 和 `ToolProvider<Ctx>` 在 value type 位置仍是错误；
  只有 `any ToolProvider` 和 `any ToolProvider<Ctx>` 合法。
- `any I` 可以被 `Nullable`、`Union`、`Array`、不经任何 boundary schema closure 投影的内部 record（含具名 record 类型）、
  `Map` value 和内部 function type 的参数/返回位置（如 `fn(any I) -> void`、`fn() -> any I`）等普通内部 type constructor
  包裹；`Map<any I, V>` 这类 map key 位置不允许。判据是闭包可达性，不是“临时 vs 具名 record”：具名 `type Foo { p: any I }`
  同样允许，只要 `Foo` 不被任何 boundary 的 schema closure 投影出去。function type 同理——含 `any I` 的函数类型当值传递在
  linked program内部允许，也可以进入Package Local ABI；但不能进入PackageSchema、ServiceContract或
  ordinary JSON。DB等owner-internal recoverable boundary按`recoverable-value.md`判断。所有boundary walker
  都必须遍历function type的参数与返回闭包。
- Ordinary public schema projection 必须拒绝任何包含 `AnyInterface` 的 schema closure。Owner-internal
  recoverable boundary 使用 `recoverable-value.md` 的 boundary plan，不把 `AnyInterface` 当作 public schema field 展开。
- Object-safety 和 boundary-safety 是两层检查：object-safety 决定某个 interface 能否被 `any` 化；
  boundary-safety 决定某个 type graph 能否进入 public ABI / ordinary JSON，或是否需要 recoverable boundary plan。

`any I?` 按现有 postfix nullable 规则解释为 `(any I)?`。`Array<any I>`、`Map<string, any I>` 和
`any I | null` 是内部类型；是否能出现在某个位置由 boundary validator 决定。`Map<any I, V>`（map key 位置）
必须在 **type checker** 静态 fail closed——这是此特性 map-key 拦截的唯一权威定义点，与 §Runtime Value
"map key 是 type checker 静态拒绝"一致；不退到 runtime，也不依赖独立的 map-key shape validator 兜底。

## Value Semantics, Writable Places, And `InOut`

ordinary record / object / `Array` / `Map` / `JsonObject` 采用 value semantics。赋值、普通参数传递、
返回、container store 和 local `as I` 装箱都产生逻辑 snapshot。implementation 可以用 move、
shared backing 或 COW 避免 eager deep copy，但不得让一个 snapshot 的写入经 physical alias 被
另一 snapshot 观察。`InterfaceCarrier::Local.payload` 保存的也是这种逻辑 snapshot；
wrapper 赋值/传参/入容器不会创建 caller-writable payload alias。

绑定可写性是静态 place fact：

- 局部 `final`、普通 parameter、loop/pattern/`with` binding 及从它们派生的 path 不可写；
- 局部 `var` 可重绑，从它派生且未经 immutable/identity boundary 的精确 member/index path
  可写；
- 顶层 `const` 是 compiler-evaluated、request-independent 且 deeply frozen 的 value，不是局部
  binding 或可写单例。从 ConstantHeap 读取后若需可写副本，先放入 request-local `var`，
  首次写入按 value/COW 语义产生 request-owned node。

普通 aggregate 参数是 immutable snapshot。callee 要修改自己的副本时先写 `var local = parameter`；
该写入不影响 caller。只有显式 `inout` 是 caller-writable exclusive loan，且它必须同时满足：

- call site 传入从 `var` 派生的精确、exclusive place，并在实参处重复 `inout`；
- target 是已解析的 exact Package-local / package-direct concrete callable，`inout` mode 与读写
  path 进入 Package Local ABI；
- compiler 与 artifact verifier 都证明 target 是 `NoPending`（`maySuspend=false`），不信任未验证 summary。

`inout` 不得出现在 interface requirement（含 receiver）、interface method table、callback
signature、ServiceContract、gateway/external ingress、Actor external method、host effect 或 recoverable
payload/boundary。通过 `any I` 的 `InterfaceMethod` call 既没有静态 exact concrete target，又必须
保守为 `maySuspend=true`，因此绝不能作为 `inout` loan 的 callee。verifier 必须拒绝任何尝试借
same-process 部署、callback adapter 或 remote operation projection 绕过该限制的 artifact。

## Boxing

`expr as I` 是唯一装箱入口。装箱源（box source）有两类：**本地 concrete nominal value** 与
**远程 public instance 寻址源**。两类装箱出同一个 `any I` 类型。

```rust
struct InterfaceBoxingPlan {
    interface: InterfaceInstantiationRef,
    source: BoxSource,
}

enum BoxSource {
    // 本地 concrete record 值
    Local {
        concrete_type: ConcreteTypeRef,
        method_table_plan: InterfaceMethodTablePlanRef,
    },
    // 远程 public instance 寻址源（如 remoteLlm/managedLlm）
    Remote {
        dependency_ref: String,               // service dependency alias
        public_instance_key: String,          // callee public API graph 完整 path
        operations_plan: RemoteOperationPlanRef, // 选定 interface 方法集 → contract_operation_id 子集（plan，对应本地 method_table_plan）
        callee_protocol_identity: String,     // 装箱点锁进 dependency lock，见 §Remote Fail-Closed
    },
}
```

### 寻址语法 `/` 与装箱源

远程装箱源用 `/` 寻址，不用 `.`：`remoteLlm/managedLlm`。`/` 左边是 dependency alias，右边是 callee
public API graph 的 `public_instance_key`。`.` 是成员访问符，用它拼跨 service 路径会误导成"取字段"；
`/` 的既有含义是路径寻址，与"跨 service 寻址本质是路径"吻合，与成员访问 `.` 视觉上区分。`/` 跨 service
寻址是**新增语法**（现状跨 service 调用不是 `/` 形态），下面两条路径都随它一同引入。

裸 `remoteLlm/managedLlm` **不是值，没有 first-class 类型**。它是一个装箱源 / public instance 寻址 root，
只能出现在两种位置：`remoteLlm/managedLlm.method(...)`（直接 operation 调用）或 `remoteLlm/managedLlm as I`
（装箱）。这两种**语法**都是 `/` 一同引入的新写法；底层 outbound dispatch **机制**复用现状 service
dependency 调用路径（语法新、机制旧）。`final x = remoteLlm/managedLlm` 非法（装箱源不是值）。不给它 first-class
类型，是因为候选只有"裸 interface 名"（当类型违法）或"codegen 的 stub type"（不做 codegen）——寻址
靠 interface 类型 + `contract_operation_id` 已足够，类型出现在装箱**之后**，是 `any I`。

编译期要求（两类共通）：

- `I` 解析到 interface instantiation，不能是 concrete type、alias、primitive、anonymous record 或 `any I`。
- 装箱源必须显式 implements 同一个 interface instantiation；不做 structural matching。
- conformance只比较interface requirement拥有的调用形状，不比较concrete executable的推断
  suspension summary；装箱plan也不得把该summary复制成interface fact。
- 目标 interface 必须 object-safe。
- marker interface 不允许装箱，因为没有可调用 method table，不能形成有意义的 dynamic dispatch value。
- **`as I` 不能省略**，即使装箱源只 expose 一个 interface，也即使赋值/参数已有 `any I` 目标类型。理由是
  **装箱可见性**，不是多 interface 消歧：`as I` 是装箱发生点——类型擦除、`carrier`（含寻址坐标）/`contract_operation_id`
  填入都在此发生；这是有运行时表示成本、单向不可逆、对远程还锁跨 service protocol
  identity 的操作，必须可见。省略即隐式装箱，违反"装箱必须可见"。（与 Go/Java 隐式向上转型不同——skiff
  默认类型擦除，`any I` 是显式特例；多 interface 即使有目标类型也能消歧，所以消歧不是必写的理由。）
  一个 public instance expose 多个 interface 时，`as I` 顺带选定投影到哪个 interface（`any I` 的 `I` 必须是
  单一 instantiation），只填该 interface 方法集对应的 target/operation 子集。

本地装箱源额外要求：

- `expr` 的静态类型必须是 concrete nominal record instantiation。
- 装箱不改变 payload 的普通值语义；interface value 保存该 concrete value 的普通 runtime payload。无论
  具体优化如何，ordinary object 不得因此新增 per-instance source type 字段或隐式 vtable。

远程装箱源额外要求：

- `dependency_ref` 必须是已声明的 service dependency alias。
- callee public instance必须在`api.yml`中公开、被`service.yml.serviceCalls`选择，并在其
  `ServiceContract`中显式保留
  `as I`选定的interface conformance（`InterfaceInstantiationRef`一致）及选定interface methods派生的
  `ContractOperationId`集合。
- **选定 interface 的方法签名（参数与返回）不得含 `any I` 或任何 boundary-unsafe 类型**（**第一版约束**，
  非地基级永久禁令）。普通service operation的顶层`any I`可以在同runtime投影成
  `CallbackCapability`，但`Remote` carrier的方法表第一版必须保持placement-independent；在
  `RemoteBoundary` callback transport缺失时，不能把这类方法投影成可跨runtime的
  `ContractOperationId`。这是object-safety/boundary-safety之外，对
  "远程可装箱 interface"的第三个约束；本地装箱无此限制（本地方法可收发 `any J`，全程同进程）。解除该约束
  需要两块本 workstream 范围外的基建，见 §Evolution "远程方法返回 `any I`"。该约束的两道执行点：
  - **根因点 = callee ServiceContract projection**：含`any I`方法的interface无法生成service operation，
    因而没有对应`ContractOperationId`。
  - **派生点 = consumer装箱期**：consumer的`as I`在callee ServiceContract的
    `ContractOperationId`集合里找不到该方法时立即拒绝，不留到runtime verifier。consumer看到“选定
    interface的方法无可绑定operation”，根因仍是callee没有将它投影为service operation。
  - 实测佐证：agent 包现状三个 capability interface 的方法签名闭包都不含 `any I`（见 §Capability As
    Parameter 核对表），故第一版这条约束不咬任何主要场景。
- 装箱点必须把 callee 的 exact protocol identity 锁进 dependency lock，见 §Remote Fail-Closed。

Typed IR / artifact verifier 必须在 runtime execution 前保证：

- 本地装箱：`InterfaceBox.value` 的静态类型可验证，且 canonical concrete nominal type 等于 boxing plan
  `BoxSource::Local.concrete_type`；plan 的 `interface`、`concrete_type` 和 `method_table_plan` 严格对应
  同一个 `(interface instantiation, concrete receiver instantiation)` pair；`method_table_plan` 每个 slot
  target 来自该 pair 的 explicit conformance checker 结果。
- 远程装箱：`BoxSource::Remote` 的 `(dependency_ref, public_instance_key)` 必须解析到已声明 dependency 的
  callee public instance metadata；选定interface的每个方法都必须在callee ServiceContract的
  `ContractOperationId`集合中有匹配canonical signature的operation；`callee_protocol_identity`必须等于
  dependency lock 中锁定的 callee exact protocol identity。

runtime 不从 erased payload 反推 concrete type；runtime 只信任已经验证并 linked 的 plan。任何 malformed
artifact 破坏上述不变量都必须在 verifier/linker 阶段 fail closed。

## Runtime Value

`any I` 是 type erasure 架构白名单里的显式 dynamic value。目标态 runtime value 可以选择 dedicated
`RuntimeValue` variant 或 heap node；架构契约是必须有一个 request-scope interface value record。合并后，
"装箱源真实身份 + 方法分派 + payload/owner" 不再是可独立取值的平铺字段，而是收敛进单个 `carrier`
enum——本地分支恰好携带 concrete type / method table / payload，正向远程分支携带 published instance 寻址
坐标 / operation 寻址，callback 分支携带 request-scoped owner capability；后两者都不带本地 payload。

```rust
struct InterfaceValue {
    interface: InterfaceInstantiationId,
    carrier: InterfaceCarrier,
}

// 三个 carrier 是互斥整体：source identity、dispatch、payload/owner 的一致性
// 由 enum 分支天然保证，不存在 source=Local 配 dispatch=Remote、或 Local 分支缺 payload
// 这类非法组合。verifier 不需要再单独对账三个轴的配对。
enum InterfaceCarrier {
    Local {
        concrete_type: ConcreteRuntimeTypeId,    // 装箱源 concrete nominal instantiation identity
        method_table: InterfaceMethodTableId,    // linked overlay id；plan 侧是 method_table_plan
        payload: RuntimeValue,                    // 具体值本体（普通 erased runtime value，不自带 source type name）
    },
    Remote {
        dependency_ref: DependencyId,
        public_instance_key: PublicInstanceKeyId, // 与 dependency_ref 一起构成"是哪个远程实例"的寻址坐标
        operations: RemoteOperationTableId,        // linked overlay id；plan 侧是 operations_plan
        // 无本地 payload：self 由远端 instance 承载
    },
    CallbackCapability {
        owner_deployment_build_id: DeploymentBuildId,
        owner_runtime_route: CallbackOwnerRoute,
        request_identity: RequestIdentity,
        operations: CallbackOperationTableId,
        opaque_capability_id: CapabilityId,
        lifetime: RequestOrStreamLifetime,
        // 无本地 payload：self 留在 capability owner
    },
}
```

命名约定：plan（artifact 侧）一律带 `_plan` 后缀（`method_table_plan` / `operations_plan`），linked overlay
id（runtime 侧）一律无后缀（`method_table` / `operations`）。plan与linked id对齐，避免出现"两个都叫
operations 却一个是 plan ref、一个是 linked id"的歧义。

字段含义：

- `interface`：被擦除后保留的 interface instantiation identity（三种 carrier 共有）。
- `carrier`：装箱源/边界投影整体，本地、正向远程或 callback capability 三选一。
  - `Local.concrete_type`：concrete nominal instantiation identity（供 runtime validation 和未来 downcast）。
  - `Local.method_table` / `Local.payload`：linked method table + 具体值本体。
  - `Remote.{dependency_ref, public_instance_key}`：即"是哪个远程实例"。它**不**指向 concrete type——
    callee 私有 receiver concrete type 不导出（见 `../reference/api-yml.md`），consumer 侧没有该 id 可填。远程
    装箱源的真实身份就是它的寻址坐标。这对坐标同时是 operation 寻址依据和 fail-closed 锚点（§Remote
    Fail-Closed）。诊断/工具若要标注"此处发生跨 service 调用"，直接判 `carrier` 是 `Remote` 分支即可，
    不引入独立 effect。
  - `Remote.operations`：`ContractOperationId`集合与service dispatch；远程分支无本地payload。
  - `CallbackCapability`：只由 service boundary 的显式顶层 projection 产生，记录 exact owner、允许的
    operation 与 request/stream lifetime。同 runtime 调用查 capability table；remote route 只有
    `RemoteBoundary` transport capability 存在时才可执行。
- 因为 `carrier` 把 source identity / dispatch / payload-or-owner 锁成一个 enum 分支，"本地必有 payload、
  remote/callback 必无本地 payload"在类型层不可违反——这是把原平铺字段合并的主要收益。

约束：

- Interface value 是 request-scope dynamic value。**本条的绝对排除已被 `recoverable-value.md` 部分取代**：
  `carrier = Local` 的行为值可经可恢复 codec 进 DB/dispatch/persistent（self payload 全可恢复时）；跨 service 把 `any I`
  作 payload传去对端、对端回拨时，同runtime顶层projection可形成`CallbackCapability`，跨runtime
  RemoteBoundary仍fail closed。仍然成立的是：远程
  装箱的`ContractOperationId`（正向`Remote` carrier，consumer主动调用service-call public instance）是
  request-scope寻址，不持久化重建——它是“指向远程实例的引用”，不是被恢复的值。能否进DB/dispatch的权威
  判据见`recoverable-value.md`。
- `carrier`（method table / operation 寻址）、type descriptor 和 artifact metadata 不计入 ordinary object
  payload，也不写入 DB / JSON。
- clone/materialize/debug 可以保留 interface wrapper 的运行时可执行性，但 clone 仍是逻辑
  snapshot；可以共享 COW backing，不得共享可观察 mutable alias，也不能把 wrapper 编码成
  ordinary JSON。
- equality、map key、JSON encode、DB encode 默认不支持 `any I`；若未来要支持，必须先修改 reference
  明确定义语义。第一版 fail closed，但拦截层级不同：map key 与 JSON/DB encode 是 type checker 静态拒绝
  （等同“裸 interface 不能当值”级别的保证，不能退到 runtime）；equality 这类无法在 type checker 拦尽的残余情况由 runtime 兜底。
- 远程 `carrier::Remote` 的寻址坐标**不含 `interface`**：同一个远程 instance 被 `as I` / `as J` 装成两个
  不同 interface 的 `any I` 值时，两者 `carrier` 里的 `(dependency_ref, public_instance_key)` 完全相同，只有
  顶层 `interface` 与 `carrier.operations` 不同。故一个 `any I` 值的完整身份是 `(interface, carrier)`，
  `carrier` 单独不足以唯一标识。这是未来定义 equality / downcast 时的前置事实：远程值的相等性须按
  `(interface, 寻址坐标)` 而非仅寻址坐标判定。

## Execution Owner

每次 request、stream producer 或 callback invocation 都先 pin 一个 exact deployment `buildId`，并只在该
build 的 immutable `DeploymentExecutionImage` 内解析 executable、method table、type/restore plan 和
ConstantHeap。`InterfaceValue` 不需要在每个 wrapper 中重复保存 buildId；`Local` 分支的 linked id
由当前 execution frame 的 exact image 定界，不得在其它 build 的 overlay 中解析。

`Remote` carrier 保存 service requirement / public-instance / protocol operation 坐标，不把某个 provider
implementation build 永久写入该值。每次真正进入 provider 时按 release/dependency contract 解析并
pin 当次 exact provider deployment `buildId`；调用、stream 和 callback 在其生命内继续使用该
owner，不会被后续 pointer 更新迁移。callback capability 除 runtime route 外必须固定
`ownerDeploymentBuildId`；route 只是 transport coordinate，不是 owner identity。

这个模型不存在 deployment activation generation 或全局 `RuntimeAssembly` owner。Actor activation
identity/generation 和 transport socket generation 可以保留为它们各自子系统的生命周期事实，
但不得参与 deployment execution owner、interface target lookup 或 fallback。

## Method Table

每个 method table 对应一个 fully substituted pair：

```text
(interface instantiation, concrete receiver instantiation)
```

slot 顺序以 interface declaration 中的 method requirement 顺序为唯一来源；canonical `method_abi_id`
用于校验 slot 身份和 artifact identity，不用于排序：

```rust
struct InterfaceMethodTable {
    interface: InterfaceInstantiationRef,
    concrete_type: ConcreteTypeRef,
    slots: Vec<InterfaceMethodSlot>,
}

struct InterfaceMethodSlot {
    method_abi_id: String,
    source_method_name: String,
    signature: InterfaceRequirementSignature,
    target: LinkedInterfaceMethodTarget,
    receiver_call_abi: ReceiverCallAbi,
}
```

规则：

- `method_abi_id` 保留为 canonical string 而非新 newtype，是刻意沿用现有 interface method ABI identity 的既有表示
  （package public instance / remote operation projection 已用同一份 `method_abi_id`），避免为 `any I` 再引入第二套方法身份。
  （论据不引用已退役的 binding projection——见 §Capability As Parameter；远程 `as I` 复用的是 binding 的 lock
  *数据形态*而非 binding 机制本身。）
  其余字段用结构化 ref，唯独这里是 string，原因即此；它必须包含 generic interface type args。
- slot signature 是 interface requirement 完成 type substitution 后的 canonical调用形状，不包含
  `maySuspend`，也不允许任何 `inout` parameter mode。
- target 是 conformance checker 选出的 concrete receiver method；linker 把 artifact target 解析为
  executable address。concrete target自己的推断summary保留在executable/Package callable metadata，
  不进入method table的requirement signature。
- method-level generic requirement 第一版不允许进入 object-safe method table。
- 同一 concrete type 对同一 interface symbol 第一版最多实现一次；若未来允许多 instantiation conformance，
  method table key 必须扩展为完整 receiver/interface instantiation，不得只按 symbol 名查表。

Method table 是 linked runtime plan，不是 ordinary artifact DTO 的可变字段。artifact 可以保存构建 method table
所需的 boxing/call targets，但 runtime linking 后的 executable address table 归 runtime overlay。

### Remote Operation Table

远程装箱值（`carrier = InterfaceCarrier::Remote`）不走本地 method table，而走一张 **remote operation
table**：slot → `ContractOperationId`的映射。它与本地method table分属`InterfaceCarrier`的两个分支，
共享同一套slot身份规则，只是最终target不同（本地是executable address，远程是`ContractOperationId`+
outbound dispatch）：

```rust
struct RemoteOperationTable {
    interface: InterfaceInstantiationRef,
    dependency_ref: DependencyId,
    public_instance_key: PublicInstanceKeyId,
    slots: Vec<RemoteOperationSlot>,
}

struct RemoteOperationSlot {
    method_abi_id: String,            // slot 身份，与本地 method table 同源
    signature: InterfaceRequirementSignature, // substituted requirement调用形状，对账callee operation
    contract_operation_id: ContractOperationId, // service寻址：callee公开的精确operation
    // 不带 source_method_name：远程 slot 不解析本地 receiver method，无需源方法名（与本地
    // InterfaceMethodSlot 的差异仅此一处，是有意省略而非遗漏）。
}
```

规则（与本地 method table 对齐，差异仅在 target）：

- slot 顺序以 interface declaration method requirement 顺序为**唯一来源**，与本地 method table 完全一致；
  `method_abi_id` 用于校验 slot 身份和 artifact identity，不用于排序。同一个 `(interface, slot)` 在本地表和
  远程表里指向同一个 method requirement。
- 每个slot的`signature`是requirement完成substitution后的canonical调用形状，不含suspension summary；
  verifier用它对账callee ServiceContract中`ContractOperationId`对应的canonical调用形状
  （见§Boxing远程装箱verifier要求）。
- `contract_operation_id`字段取自callee `ServiceContract`中该方法对应的operation；一个public instance expose
  多 interface 时，只填 `as I` 选定 interface 方法集对应的 operation 子集（见 §Boxing `as I` 顺带选投影）。
- 远程表同样是 linked runtime plan / overlay（`RemoteOperationTableId`），不写回 ordinary artifact DTO；
  artifact 侧保存的是 `BoxSource::Remote` 的 symbolic 寻址信息（dependency_ref / public_instance_key /
  operation 集合 / callee_protocol_identity），linker 解析成 `RemoteOperationTable`。
- `method_abi_id` 复用与本地 method table、package public instance / remote operation projection 同一份
  canonical interface method ABI identity，不引入第二套方法身份。

## Dynamic Dispatch

对 `any I` 值调用 method：

```skiff
final out = provider.execute(ctx, call)
```

compiler 必须把它识别为 interface method call，而不是普通 field access 加动态 object lookup。

目标 lowering 形态：

```rust
enum CallTargetIr {
    // existing variants ...
    InterfaceMethod {
        interface: InterfaceInstantiationRef,
        method_abi_id: String,
        slot: u32,
    },
}
```

执行规则：

1. runtime 先求值 receiver，结果必须是 `InterfaceValue`。
2. receiver 的 `interface` 与 call target interface 一致这一不变量由 linker 静态保证；runtime 不承担生产校验，至多在 debug build 做 assert。runtime 只信任已经验证并 linked 的 plan，不退回字符串比较的兜底路径。
3. 按 `carrier` 分支分流：
   - `Local`：从 `carrier.method_table.slots[slot]` 取 linked target，以 `carrier.payload` 作为 explicit
     `self`，再追加用户参数的逻辑 snapshot，调用 concrete receiver executable（本进程）。
     `self` 和普通参数在 callee 中都是 immutable value binding；本地分支必有 payload，由 enum 保证。
   - `Remote`：从`carrier.operations`取该slot对应的`ContractOperationId`，按`carrier.dependency_ref`走
     service dependency dispatch（与 `remoteLlm/managedLlm.method(...)` 直接 operation 调用走同一条 outbound
     dispatch 机制——该机制复用现状 service dependency 调用路径，`/` 写法本身是新增语法）；远程分支结构上
     无本地 payload，self 由远端 instance 承载。
   - `CallbackCapability`：校验 request/stream lifetime、owner build/route 与 operation slot；同 runtime
     通过 capability table 切回 owner execution context。跨 runtime 只有 transport 明确声明 reverse-callback
     capability 时才可发起，否则稳定返回 `CapabilityUnavailable`，不能退化成本地 method table 或普通正向
     service call。
4. 返回值按 ordinary runtime value 返回；如果返回 `any J`，它必须是被显式装箱过的 interface value。
   `Local`与`CallbackCapability`分支可按各自owner/boundary plan返回；`Remote`分支第一版的
   operation table因上述placement-independent约束不会含返回`any J`的slot，consumer装箱点会因
   找不到完整`ContractOperationId`集合而拒绝。

三种 carrier 的调用 lowering 选 slot 逻辑相同；只有 §3 的最终 target 解析按 `carrier` 分支分流。
静态suspension分析不能从`any I` requirement取得concrete summary，因此所有`InterfaceMethod`调用都保守为
`maySuspend=true`。`Remote`/`CallbackCapability`分支还因boundary call而属于caller-side suspension；三种分支都
不会仅因保守summary在runtime自动插入`yield`。
任何 `InterfaceMethod` call 携带 `inout` argument mode，或任何 method table / remote operation slot 声称
接受 `inout`，都是 verifier 必须拒绝的 malformed artifact。

禁止路径：

- 不按 source method name 在 runtime 搜索 object field。
- 不从 ordinary object 读取 source type name 或 `implements` 列表。
- 不允许 `p.method` 作为 first-class method value；第一版只支持直接 call expression。

（原"不通过 binding/remote dispatch 模拟 `any I`"的禁令在合并案中放开：远程能力**就是**经显式
`as I` 装箱出的 `any I` 值，远程性由 `carrier = Remote` 表达，不是"模拟"。）

## Remote Fail-Closed（装箱点锁定）

远程装箱在编译期 fail-closed，锚点是装箱点 `remoteLlm/managedLlm as api.LlmClient`。`as I` 对远程装箱源
**额外承担**一次 service dependency 声明确认 + protocol identity 锁定：

1. 确认 `dependency_ref`（如 `remoteLlm`）是已声明的 service dependency。
2. 校验 callee public instance 显式 implements `as I` 选定的 interface（`InterfaceInstantiationRef`
   一致，遵循 `interface.md §4` 显式 conformance）。
3. 把callee的exact `serviceProtocolIdentity`、`public_instance_key`和选定interface派生的
   `ContractOperationId`集合写入dependency lock。

dependency lock entry保存`serviceProtocolIdentity`、选定interface methods对应的
`ContractOperationId`集合与`remoteBoxProvenance`。这些事实由源码里的远程`as I`装箱点产生，不恢复已退役
的service.yml binding机制，也不保留`bindingProvenance`等旧字段名。

fail-closed 语义：callee 改了选定 interface 方法签名、撤了 conformance、或移除该 public instance，都会
改变锁进 lock 的 `serviceProtocolIdentity`，consumer **编译失败**，不退化为运行时才炸。校验集中在
`as I` 这一个可见点；同一个 `any I` 值后续在多处调用不重复锁定。

callee只改变concrete implementation的内部suspension summary时，interface调用形状、conformance与
ServiceProtocol identity都保持不变，因此不会使remote装箱lock失效。新的provider build/deployment仍由
implementation identity精确选择。

## 远程性的可见性（无 effect）

远程性**不**引入用户声明的独立 effect。`carrier = Remote` 已经在值布局里携带"这是远程装箱值"这一事实，
工具/诊断要标注"此处发生跨 service 调用"直接读它即可，不需要在 `static-semantics.md` 的 effect 体系里
挂一个空壳。尽管没有声明或protocol位，remote method call仍按service call种类推断为
`maySuspend=true`；callee内部summary不参与这项判断。

不强制并发上下文：对运行期变长 `any I` 集合的并发 fan-out 依赖 `concurrent`，而当前 `concurrent` 只接
静态平铺 lane、不接 `for`（见 §Evolution 开放项）。在该缺口解决前，远程调用与现状 service dependency
调用一致——不强制 `concurrent` / `timeout`，不引入 `async`/`await` 染色。是否将来要求并发上下文，留待
动态并发缺口解决后再议；届时若需要，再讨论是用 effect 还是别的机制承载，本版不预先固化。

## Capability As Parameter（binding 退役）

合并后，package "我需要某能力但不指定来源"不再用 binding requirement，而是声明一个吃 `any I` 的入口参数：

```skiff
// package 源码：只 import 定义 interface 的包，不依赖具体实现
function run(input: AgentInput, llm: any api.LlmClient) -> Stream<api.LlmStreamEvent> {
  return llm.streamChat(toLlmRequest(input))
}
```

consumer 在调用点装箱后传入，由此决定本地/远端：

```skiff
agent/run(input, remoteLlm/managedLlm as api.LlmClient)   // consumer A：远端 remoteLlm
agent/run(input, localLlm as api.LlmClient)            // consumer B：本地实现
```

binding 整套退化为普通参数 + 类型检查：

| binding 机制（退役） | 新形态 |
| --- | --- |
| `requires.bindings`（requirement） | 参数类型 `any I` |
| requirement `alias`（受控 root） | 普通形参名 |
| service.yml `bindings` entry（resolution） | 调用点传参 |
| `BindingRequirementResolution` | 普通类型检查（实参 `any I` 匹配形参） |
| "恰好绑一次/不漏/不重复" | 参数必填，语言天然保证 |
| 单点绑定（一 requirement 一实现，装不下运行期变长异构集合） | 无——参数是值，可传任意多个、进 `Array<any I>` |

fail-closed 位置更合理：锁在装箱点（产生远程引用的 consumer），package 那行 `run(llm: any api.LlmClient)`
完全不碰 fail-closed，只认 `any api.LlmClient` 这个类型。package 依赖的是"能力的形状"（import 定义
interface 的包），不是"能力的实现"——这正是 binding 当初要的解耦，用普通 package dependency 即达成。

**canonical current example（Agine/Agent）**：Agine已经把本地`any LlmClient`、`any ToolExecutor`和
`any AgentEventReceiver` adapter作为runtime bindings传给Agent package；Agent通过local method table调用这些
adapter。Agine的Llm adapter内部再调用`aihub/managedLlm.streamChat`，那一跳是普通正向service operation。
因此这条生产链证明`Local` carrier与Package capability parameter可行，但不证明`Remote`装箱或
`CallbackCapability`经过Router。目标写法`remoteLlm/managedLlm as LlmClient`仍必须由对应source/lowering/
runtime路径单独验收。

`any I` 作为 package public 入口参数不违反 linked-program-local 边界：package link 进 consumer 同一
runtime，`any I` 值从 consumer 流到 package 入口全程同进程，远程性只在调用时 dispatch。见 §Boundary
Contract。

### 三个 capability interface 的目标远程可装箱性核对

"本地 + 远程 + 异构全覆盖"这一宣称要求三个 capability interface 的方法签名都满足 §Boxing 的远程额外约束
（签名闭包不含 `any I`）。实地核对结果——三者签名闭包全部干净，第一版即可全部远程装箱：

| interface | 方法签名 | 闭包含 `any I`？ | 第一版可远程？ |
| --- | --- | --- | --- |
| `LlmClient` | `streamChat(LlmRequest) -> Stream<LlmStreamEvent>` | 否（全 string/number/Json/嵌套 record/union） | ✅ |
| `ToolExecutor` | `execute(ToolExecutionInput) -> ToolExecutionOutput` | 否（string/`ToolCall`/`Json`/`ToolResult`/`ToolError`） | ✅ |
| `AgentEventReceiver` | `receive(AgentEvent) -> void` | 否（string/`LlmStreamEvent`/`ToolResult`/`Json`/`void`） | ✅ |

结论：§Boxing 的"远程方法不得含 `any I`"第一版约束**不咬**任何主要场景，"全覆盖"在第一版成立。该约束是
对将来设计的防御性边界，不是当前障碍。若某 capability interface 将来要在签名里挂 `any I`（如返回一个
子能力句柄），解除路径见 §Evolution。

## Object Safety

`any I` 只允许 object-safe interface：

- method requirement 必须有 `self: Self` receiver。
- receiver 和其它 requirement 参数都不得使用 `inout`。
- method requirement 不得带 method-level type params。
- `Self` 不得出现在非 receiver 参数、返回值、record field、container element 或 function type 中。
- method requirement 不能是 `static`、`native` 或 provider-only declaration。
- marker interface 不允许 `any` 化。

现有 `interface.md` 已把大部分非 object-safe 形态排除；`any I` 实现仍要在 type checking 阶段集中诊断，
不能依赖后续 lowering/runtime 崩溃。

## Boundary Contract

`AnyInterface` 是 schema-open runtime type。边界判据是**值进入哪类 boundary policy**，不是"是否离开当前函数"。
以下位置默认 fail closed（`carrier` 里的 method table / `contract_operation_id` 在对端无意义）：

- service operation 中除上述显式顶层callback-capability projection以外的参数或返回值。
- public instance operation signature。
- public API type schema closure。
- ~~service DB schema、queue/spawn/persistent work item payload~~ —— **此条已被 `recoverable-value.md` 取代**：
  `carrier = Local` 行为值可经可恢复 codec 进 DB/dispatch/persistent（self 全可恢复时）；跨 service 恢复语义第一版
  fail-closed。DB/dispatch/persistent 能否进的权威判据见 `recoverable-value.md`，不再由本条绝对排除。
- cross-service ordinary payload、ordinary JSON materialization。runtime binary payload 若是 owner-internal 跨 request
  lane，按 recoverable boundary 处理；若带 cross-service / external trust boundary，第一版行为节点 fail closed。
- config schema、test double external fixture schema。

以下位置**允许**`any I`（值不跨service boundary）：

- 内部 helper function、内部 record、局部变量、transient collection。
- **Package public入口的参数/返回类型**——Package link进consumer同一linked program，`any I`值不经过
  service boundary，远程性只在调用时dispatch。这是binding退役后Package抽象依赖能力的承载点
  （见§Capability As Parameter）。注意：这是Package local-link入口，**不是**service operation；后者即使
  物理同进程也经过service boundary，仍然fail closed。

判据收敛为：ordinary public schema 不承载 `any I` 默认 wire shape；owner-internal DB/dispatch/queue/persistent/runtime
lane 按“值必须可恢复”处理；离开 owner service trust domain 的行为值第一版 fail closed。boundary walker 区分
"Package入口签名"（local link，允许）、"service operation签名"（只允许显式顶层
callback-capability projection，其它拒绝）和
"owner-internal recoverable boundary"（按 carrier/self recoverability 判定）。

上述任何 boundary 都不能携带 `inout` loan。顶层 `any I` 若按显式规则投影为
request-scope callback capability，其 operation 也只接收 detached value snapshot，不保留 caller-writable
origin。same-process fast path 不得改变这一 ABI 事实。

## Relationship To Type Erasure

`any I` 不推翻 runtime type erasure。它是主动建模的dynamic wrapper，与actor句柄的Runtime内部表示和exception envelope同属
白名单机制：

- ordinary record/object 仍然 unshaped，不携带 source nominal type。
- 装箱源身份（`carrier` 里的本地 concrete type id / 远程 instance 坐标）只保存在 interface wrapper 内，不反写 payload。
- method dispatch 本地使用 linked method table、远程使用 `contract_operation_id`，都不查 ordinary object shape。
- JSON/DB/HTTP boundary 仍然 expected-type driven；`any I` 没有默认 wire shape。

这条规则避免把 `any I` 实现成“所有 object 都附带 vtable”，也避免重新引入已被 runtime value layout 文档禁止的
per-instance source type metadata。

## Evolution

第一版完成后仍不提供：

- downcast / narrowing from `any I` to concrete type。
- interface method value。
- implicit boxing。
- marker interface runtime value。
- **durable / 跨 request 持有的远程能力句柄**（remote capability transport）。此条专指**正向 `Remote` carrier**
  （指向已发布远程公开实例的 `any I`，consumer 主动调）的持久化重建——它是 request-scope 引用，本版不允许持久化后
  重建。注意区分：**局部值**（`carrier = Local`）跨 service 的“持有后重建”是 `recoverable-value.md` 的直传模型
  （第一版 fail-closed，卡 callback transport），不属本条；本条说的是“把一个远程引用坐标 durable 化”这件**另一回事**。
- ~~**运行期实例级跨service句柄**（为运行期临时实例铸造可寻址坐标+"句柄→活实例"注册表+生命周期/GC）~~
  **（已否定，2026-06-27，据 `recoverable-value.md §Cross-Service Interface Value`）**。这块基建当初的设想是"用
  坐标机制去寻址运行期临时实例"，但**坐标与临时对象机制不匹配**：远程调用坐标只能指向
  `api.yml` 显式公开并被 `service.yml.serviceCalls` 选择的 public instance root。普通顶层
  `const` 是 deeply frozen value，不是 identity-bearing singleton；它的 logical snapshot 可按 value/COW
  语义复用，不得为它隐式铸造 remote receiver identity。临时对象同样没有 public-instance 身份，
  硬要给它坐标就得造注册表把它**伪装**成稳定实例——这是用错机制。正解是
  按对象类别分两条互不替代的机制：
  - **已发布 public instance root** → **传坐标**。`(service, public_instance_key)` 复用现有
    service dispatch，调用按 `ContractOperationId` 寻址，无需注册表/GC。未进入 ServiceContract
    的 symbol（包括普通顶层 `const`）不在该寻址面中。
  - **运行期临时（局部）对象** → **直传可恢复字节**，对端持有、回拨带回、构造侧在当次
    exact deployment `buildId` owner 中无状态重建等价副本，
    不铸句柄、不持有活实例、无注册表/GC。
  两条都不需要"实例级句柄 + 注册表"，故本块基建不再认领。**进恢复边界（值被传去对端、对端回拨）的只有局部对象直传
  这一条，其卡点是下方的 service callback transport**；已发布 public-instance 坐标那条第一版要么是正向 `Remote`（consumer 主动调、不进
  恢复机制）、要么寻址层不认（演进），不在“进恢复边界”之列。
- **RemoteBoundary service callback transport**（consumer 跨 runtime 拿 capability 回调 owner 的特定
  request/stream）。同 runtime 的 capability table 已有实现；这里缺的是 Router 上的反向 request/cancel/
  response、认证、lifetime 与 backpressure。现状 outbound dispatch 单向（runtime 主动连 router，业务流量
  consumer→callee），不能把普通 service call 反过来当成该协议。
- method-level generic interface requirement 的 dynamic dispatch。
- structural matching 或按 method set 自动 conformance。

注意：合并案已支持 `any I` 的**远程装箱**（远程能力作为 request-scope `any I` 值，见 §Boxing /
§Remote Fail-Closed），所以"`any I` 跨 service"不再是排除项——被排除的只是上面那些 durable / 句柄 /
callback 形态。

### 远程方法返回 `any I`（约束解除路径）

§Boxing 的"远程方法签名不得含 `any I`"是**第一版约束**，本节给它的解除路径。远程方法返回 `any J`（或收
`any J` 参数）要做的，是让 callee 把那个 `any J` 值在 wire 上表示成 consumer 可寻址的远程坐标：

按 `recoverable-value.md §Cross-Service Interface Value` 的二分，这个 `any J` 也分两类，机制不同：

- callee 返回的 `any J` 装箱源是**已发布 public instance**（再下游的远程公开实例）：其坐标是
  `(service, public_instance_key)`，但 `dep_in_callee` 是 **callee 侧的 alias**，consumer 侧无此 alias——须**重映射
  坐标**到 consumer 可寻址的地址系，不是原样透传。这是纯坐标重映射，不涉及临时实例。
  普通顶层 `const` 没有 public-instance receiver identity，不在此列。
- callee 返回的 `any J` 装箱源是**运行期临时（局部）实例**（`localImpl as J`）：没有 public-instance receiver identity，坐标无处可指，
  改走**直传**——callee 把它的可恢复字节进 wire，consumer 持有，回拨时带回、由 callee 在当次
  exact deployment `buildId` owner 中无状态重建等价
  副本。**不铸句柄、不维护注册表**（那块基建已否定，见上）。

两类**都不需要"运行期实例级句柄 + 注册表"**。真正的卡点是 **service callback transport**：consumer 之后对这个
`any J` 调方法要反向打回 callee，而现状 outbound dispatch 单向。该通道落地后，本约束方可解除；在此之前，远程方法
签名含 `any I` 在装箱点编译失败。（public-instance 坐标本身的寻址复用现状 service dispatch，不依赖 callback transport；
依赖它的是"反向回拨"这个动作。）

downcast 仍未提供。未来若支持：本地装箱值复用 `carrier::Local.concrete_type` 回到 concrete type；
远程装箱值**不可能**回到本地 concrete type（consumer 侧不存在该类型），最多回到"它是
`(dependency_ref, public_instance_key)` 这个远程实例"这一事实。`carrier::Remote` 存坐标而非 concrete
type，在布局层就表达了"远程不可 downcast 成本地类型"。任何 downcast 都必须先新增 reference 语义、pattern 规则和
boundary 限制，不能把保留字段解释成已支持用户可见 downcast。

### 开放项（合并案）

- **package 抽象依赖的最终形态**：§Capability As Parameter 给的是"入口吃 `any I` 参数"。是否再保留一个
  更轻的"能力 requirement"声明（让 consumer 用 `as I` 满足），还是完全靠参数，待定。
- **动态并发**：当前 `concurrent` 只接静态平铺 lane、不接 `for`，无法对运行期变长 `any I` 集合并发
  fan-out（动态得靠 `dispatch` 手搓）。toolprovider 场景闭环需要"对 Array 并发 map"原语。这是将来是否要求
  远程调用出现在并发上下文（§远程性的可见性）的前提。
- **`as I` 是否隐式引入 service dependency**：§Remote Fail-Closed 要求远程装箱源是已声明 dependency。是否
  允许远程 `as I` 隐式引入 dependency，待 implementation 定。
