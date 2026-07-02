# Any Interface Value Architecture（含远程能力合并）

本文定义 `any I` first-class interface value 的长期内部架构契约。用户可见语义以
`../reference/any-interface.md` 为准；本文只规定 compiler、artifact、linker 和
runtime 如何承载该语义，以及它与现有静态 interface / conformance / type erasure 架构的边界。

**本版合并案（2026-06-24）**：`any I` 的载体从"本地 concrete 值"扩展到"本地或远程装箱源"，由此把
package capability binding 合并进 `any I`——远程能力是一个**可流动的 `any I` 值**，载体是
`operation_abi_id` 寻址而非进程内指针。本文据此取代两处既有定位：

- `package-capability-bindings.md`：binding 作为"发布期静态、单点绑定（每个 requirement 恰好一个实现、
  表达不了运行期变长异构集合）、不可流动的受控 root"的定位被合并案取代。binding 解决的问题（package
  抽象依赖能力、由 consumer 决定本地/远端）仍成立，但实现形态从"受控 root"改为"`any I` 参数 + 装箱源"。
  该文档已加指针指向本文。
- 本文旧版"`any I` 不跨 service / 不模拟 remote dispatch"的绝对排除被放开为"远程装箱源经显式
  `as I` 进入 `any I`，远程性由 `carrier = Remote` 表达"。

Skiff 尚未发布。本文目标态不要求兼容旧 parser、旧 File IR、旧 artifact schema 或旧 runtime value
layout。

## Scope

本文负责：

- `any I` 在 source type facts、File IR、linked runtime plan 和 runtime value 中的归属。
- `expr as I` 装箱点如何生成显式动态值，含**本地 concrete 值**与**远程 public instance 寻址源**两类装箱源。
- `any I` method call 如何经 interface method table（本地）或 `operation_abi_id`（远程）分派。
- 远程装箱的 fail-closed 锚点（装箱点锁定 callee protocol identity）。
- `any I` 与 ordinary object type erasure、package/public ABI、service boundary、DB 和 JSON 的边界。
- generic interface instantiation、object-safety、concrete receiver identity 和 method slot identity 的长期约束。

本文不负责：

- 用户语法的完整 reference。见 `../reference/any-interface.md`。
- 静态 interface / explicit conformance 的完整实现计划。见 `../implementation/interface-implementation.md`。
- 具体 Rust 模块拆分、迁移步骤和任务顺序；这些属于实现计划，不写入本 architecture 文档。
- durable / 跨 request 持有的远程能力句柄（remote capability transport）、service callback、downcast、
  reflection 或 marker interface runtime value。

## Position

`any I` 是 Skiff 第一种普通用户可见的显式动态分派值。它**吸收并取代**既有 package binding alias /
dependency public instance root 的"受控 root"形态——把它们从"不可流动的编译期 root"升格为"可流动的
`any I` 值"。它不是 actor ref 的重命名。

现有 interface 用途，合并后收敛为：

- compile-time contract：`type T implements I` 和 conformance checking。
- ABI metadata：public instance 和 publication metadata（binding requirement 退役，见 §Capability As Parameter）。
- `any I` first-class dynamic value：见下。

`any I` 的核心用途：

- publication-internal dynamic value：值可放入局部变量、普通内部函数参数/返回、不经任何 boundary schema closure
  投影的内部 record 字段（含具名 record 类型）和 collection；调用时按 interface method table（本地）或
  `operation_abi_id`（远程）分派。
- **本地与远程统一**：一个 `any I` 值的装箱源可以是本地 concrete record，也可以是远程 public instance
  寻址 root（如 `remoteLlm/managedLlm`）。两类装箱出的 `any I` 类型上不可区分，可混入同一个
  `Array<any I>`；区别只在值布局 `carrier` 是 `Local` 还是 `Remote` 分支（见 §Runtime Value）。
- **远程对象也是本地对象**：远程装箱值在持有它的进程里就是一份本地数据（载体是 `operation_abi_id`
  寻址坐标，不是函数指针），可传可存可进容器。"远程"只体现在**调用方法时**走 service dispatch，
  不体现在"值作为数据存在"时。

“publication-internal” 是 ordinary public schema 的硬边界：`any I` **值**不进入 service public API payload、
ordinary JSON materialization、public instance operation signature、config schema 或 test double external fixture schema
的默认 wire shape。但 DB schema、`spawn`、queue / persistent work item 和 runtime 内部跨 request payload
已经由 `recoverable-value.md` 重新定义为 owner-internal recoverable boundary：`carrier = Local` 且 self payload 全可恢复时可恢复，
`carrier = Remote` 仍是 request-scope 正向远程引用、不可持久化。它**可以**作为同进程 package public 入口的参数类型
（package link 进 consumer 同一 runtime，`any I` 值不跨进程；远程性只在调用时 dispatch）——见 §Boundary Contract 与
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

规则：

- `interface` 必须是完整 `InterfaceInstantiationRef`。generic interface 必须带完整 canonical type args。
- 裸 interface instantiation 不能当普通 value type。`ToolProvider` 和 `ToolProvider<Ctx>` 在 value type 位置仍是错误；
  只有 `any ToolProvider` 和 `any ToolProvider<Ctx>` 合法。
- `any I` 可以被 `Nullable`、`Union`、`Array`、不经任何 boundary schema closure 投影的内部 record（含具名 record 类型）、
  `Map` value 和内部 function type 的参数/返回位置（如 `fn(any I) -> void`、`fn() -> any I`）等普通内部 type constructor
  包裹；`Map<any I, V>` 这类 map key 位置不允许。判据是闭包可达性，不是“临时 vs 具名 record”：具名 `type Foo { p: any I }`
  同样允许，只要 `Foo` 不被任何 boundary 的 schema closure 投影出去。function type 同理——含 `any I` 的函数类型当值传递在
  publication 内部允许，但同样不能进 ABI/DB/JSON，boundary walker 必须走 function type 的参/返闭包。
- Ordinary public schema projection 必须拒绝任何包含 `AnyInterface` 的 schema closure。Owner-internal
  recoverable boundary 使用 `recoverable-value.md` 的 boundary plan，不把 `AnyInterface` 当作 public schema field 展开。
- Object-safety 和 boundary-safety 是两层检查：object-safety 决定某个 interface 能否被 `any` 化；
  boundary-safety 决定某个 type graph 能否进入 public ABI / ordinary JSON，或是否需要 recoverable boundary plan。

`any I?` 按现有 postfix nullable 规则解释为 `(any I)?`。`Array<any I>`、`Map<string, any I>` 和
`any I | null` 是内部类型；是否能出现在某个位置由 boundary validator 决定。`Map<any I, V>`（map key 位置）
必须在 **type checker** 静态 fail closed——这是此特性 map-key 拦截的唯一权威定义点，与 §Runtime Value
"map key 是 type checker 静态拒绝"一致；不退到 runtime，也不依赖独立的 map-key shape validator 兜底。

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
        operations_plan: RemoteOperationPlanRef, // 选定 interface 方法集 → operation_abi_id 子集（plan，对应本地 method_table_plan）
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
dependency 调用路径（语法新、机制旧）。`const x = remoteLlm/managedLlm` 非法（装箱源不是值）。不给它 first-class
类型，是因为候选只有"裸 interface 名"（当类型违法）或"codegen 的 stub type"（不做 codegen）——寻址
靠 interface 类型 + `operation_abi_id` 已足够，类型出现在装箱**之后**，是 `any I`。

编译期要求（两类共通）：

- `I` 解析到 interface instantiation，不能是 concrete type、alias、primitive、anonymous record 或 `any I`。
- 装箱源必须显式 implements 同一个 interface instantiation；不做 structural matching。
- 目标 interface 必须 object-safe。
- marker interface 不允许装箱，因为没有可调用 method table，不能形成有意义的 dynamic dispatch value。
- **`as I` 不能省略**，即使装箱源只 expose 一个 interface，也即使赋值/参数已有 `any I` 目标类型。理由是
  **装箱可见性**，不是多 interface 消歧：`as I` 是装箱发生点——类型擦除、`carrier`（含寻址坐标）/`operation_abi_id`
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
- callee public instance 必须在其 `PublicationAbiUnit` 中显式 implements `as I` 选定的 interface
  （`InterfaceInstantiationRef` 一致），并发布选定 interface 方法派生的 `operation_abi_id` 集合。
- **选定 interface 的方法签名（参数与返回）不得含 `any I` 或任何 boundary-unsafe 类型**（**第一版约束**，
  非地基级永久禁令）。远程方法对应 callee 的跨进程 operation，而 `any I` 值不跨进程（§Boundary Contract），
  故含 `any I` 的方法无法 projection 成 `operation_abi_id`。这是 object-safety / boundary-safety 之外，对
  "远程可装箱 interface"的第三个约束；本地装箱无此限制（本地方法可收发 `any J`，全程同进程）。解除该约束
  需要两块本 workstream 范围外的基建，见 §Evolution "远程方法返回 `any I`"。该约束的两道执行点：
  - **根因点 = callee 发布期**：含 `any I` 方法的 interface 在 callee 侧根本无法发布成 remote operation
    （没有对应 `operation_abi_id`）。这是约束的真正来源。
  - **派生点 = consumer 装箱期**：consumer 的 `as I` 因为在 callee 发布的 `operation_abi_id` 集合里找不到
    该方法对应 operation 而在装箱点（编译期）即拒，不留到 R3 verifier 对账时才炸。consumer 端看到的是
    "选定 interface 的方法无可绑定 operation"，根因是 callee 没发布它。
  - 实测佐证：agent 包现状三个 capability interface 的方法签名闭包都不含 `any I`（见 §Capability As
    Parameter 核对表），故第一版这条约束不咬任何主要场景。
- 装箱点必须把 callee 的 exact protocol identity 锁进 dependency lock，见 §Remote Fail-Closed。

Typed IR / artifact verifier 必须在 runtime execution 前保证：

- 本地装箱：`InterfaceBox.value` 的静态类型可验证，且 canonical concrete nominal type 等于 boxing plan
  `BoxSource::Local.concrete_type`；plan 的 `interface`、`concrete_type` 和 `method_table_plan` 严格对应
  同一个 `(interface instantiation, concrete receiver instantiation)` pair；`method_table_plan` 每个 slot
  target 来自该 pair 的 explicit conformance checker 结果。
- 远程装箱：`BoxSource::Remote` 的 `(dependency_ref, public_instance_key)` 必须解析到已声明 dependency 的
  callee public instance metadata；选定 interface 的每个方法都必须在 callee 发布的
  `operation_abi_id` 集合中有匹配 canonical signature 的 operation；`callee_protocol_identity` 必须等于
  dependency lock 中锁定的 callee exact protocol identity。

runtime 不从 erased payload 反推 concrete type；runtime 只信任已经验证并 linked 的 plan。任何 malformed
artifact 破坏上述不变量都必须在 verifier/linker 阶段 fail closed。

## Runtime Value

`any I` 是 type erasure 架构白名单里的显式 dynamic value。目标态 runtime value 可以选择 dedicated
`RuntimeValue` variant 或 heap node；架构契约是必须有一个 request-scope interface value record。合并后，
"装箱源真实身份 + 方法分派 + payload" 三者不再是三个可独立取值的平铺字段，而是收敛进单个 `carrier`
enum——本地分支恰好携带 concrete type / method table / payload，远程分支携带 instance 寻址坐标 /
operation 寻址，且不带本地 payload。

```rust
struct InterfaceValue {
    interface: InterfaceInstantiationId,
    carrier: InterfaceCarrier,
}

// 本地 / 远程是"二选一整体"：source identity、dispatch、payload 的本地-远程一致性
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
}
```

命名约定：plan（artifact 侧）一律带 `_plan` 后缀（`method_table_plan` / `operations_plan`），linked overlay
id（runtime 侧）一律无后缀（`method_table` / `operations`）。本地与远程两条线对齐，避免出现"两个都叫
operations 却一个是 plan ref、一个是 linked id"的歧义。

字段含义：

- `interface`：被擦除后保留的 interface instantiation identity（两类共有）。
- `carrier`：装箱源整体，本地 / 远程二选一。
  - `Local.concrete_type`：concrete nominal instantiation identity（供 runtime validation 和未来 downcast）。
  - `Local.method_table` / `Local.payload`：linked method table + 具体值本体。
  - `Remote.{dependency_ref, public_instance_key}`：即"是哪个远程实例"。它**不**指向 concrete type——
    callee 私有 receiver concrete type 不导出（见 `publication.md`），consumer 侧没有该 id 可填。远程
    装箱源的真实身份就是它的寻址坐标。这对坐标同时是 operation 寻址依据和 fail-closed 锚点（§Remote
    Fail-Closed）。诊断/工具若要标注"此处发生跨 service 调用"，直接判 `carrier` 是 `Remote` 分支即可，
    不引入独立 effect。
  - `Remote.operations`：`operation_abi_id` 集合 + service dispatch；远程分支无本地 payload。
- 因为 `carrier` 把 source identity / dispatch / payload 锁成一个 enum 分支，"本地必有 payload、远程必无
  payload"在类型层不可违反——这是把原平铺三字段合并的主要收益。

约束：

- Interface value 是 request-scope dynamic value。**本条的绝对排除已被 `recoverable-value.md` 部分取代**：
  `carrier = Local` 的行为值可经可恢复 codec 进 DB/spawn/persistent（self payload 全可恢复时）；跨 service 把 `any I`
  作 payload 传去对端、对端回拨这一恢复语义第一版 fail-closed（卡 service callback transport）。仍然成立的是：远程
  装箱的 `operation_abi_id`（正向 `Remote` carrier，consumer 主动调已发布公开实例）是 request-scope 寻址，不持久化
  重建——它是“指向远程实例的引用”，不是被恢复的值。能否进 DB/spawn 的权威判据见 `recoverable-value.md`。
- `carrier`（method table / operation 寻址）、type descriptor 和 artifact metadata 不计入 ordinary object
  payload，也不写入 DB / JSON。
- clone/materialize/debug 可以保留 interface wrapper 的运行时可执行性，但不能把它编码成 ordinary JSON。
- equality、map key、JSON encode、DB encode 默认不支持 `any I`；若未来要支持，必须先修改 reference
  明确定义语义。第一版 fail closed，但拦截层级不同：map key 与 JSON/DB encode 是 type checker 静态拒绝
  （等同“裸 interface 不能当值”级别的保证，不能退到 runtime）；equality 这类无法在 type checker 拦尽的残余情况由 runtime 兜底。
- 远程 `carrier::Remote` 的寻址坐标**不含 `interface`**：同一个远程 instance 被 `as I` / `as J` 装成两个
  不同 interface 的 `any I` 值时，两者 `carrier` 里的 `(dependency_ref, public_instance_key)` 完全相同，只有
  顶层 `interface` 与 `carrier.operations` 不同。故一个 `any I` 值的完整身份是 `(interface, carrier)`，
  `carrier` 单独不足以唯一标识。这是未来定义 equality / downcast 时的前置事实：远程值的相等性须按
  `(interface, 寻址坐标)` 而非仅寻址坐标判定。

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
    signature: CanonicalCallableSignature,
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
- slot signature 是 interface requirement 完成 type substitution 后的 canonical signature。
- target 是 conformance checker 选出的 concrete receiver method；linker 把 artifact target 解析为 executable address。
- method-level generic requirement 第一版不允许进入 object-safe method table。
- 同一 concrete type 对同一 interface symbol 第一版最多实现一次；若未来允许多 instantiation conformance，
  method table key 必须扩展为完整 receiver/interface instantiation，不得只按 symbol 名查表。

Method table 是 linked runtime plan，不是 ordinary artifact DTO 的可变字段。artifact 可以保存构建 method table
所需的 boxing/call targets，但 runtime linking 后的 executable address table 归 runtime overlay。

### Remote Operation Table

远程装箱值（`carrier = InterfaceCarrier::Remote`）不走本地 method table，而走一张 **remote operation
table**：slot → `operation_abi_id` 的映射。它与本地 method table 分属 `InterfaceCarrier` 的两个分支，
共享同一套 slot 身份规则，只是最终 target 不同（本地是 executable address，远程是 `operation_abi_id` +
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
    signature: CanonicalCallableSignature, // substituted requirement signature，对账 callee operation
    operation_abi_id: String,         // 远程寻址：该方法对应 callee 发布的 operation
    // 不带 source_method_name：远程 slot 不解析本地 receiver method，无需源方法名（与本地
    // InterfaceMethodSlot 的差异仅此一处，是有意省略而非遗漏）。
}
```

规则（与本地 method table 对齐，差异仅在 target）：

- slot 顺序以 interface declaration method requirement 顺序为**唯一来源**，与本地 method table 完全一致；
  `method_abi_id` 用于校验 slot 身份和 artifact identity，不用于排序。同一个 `(interface, slot)` 在本地表和
  远程表里指向同一个 method requirement。
- 每个 slot 的 `signature` 是 requirement 完成 substitution 后的 canonical signature；verifier 用它对账
  callee 发布的 `operation_abi_id` 的 canonical signature（见 §Boxing 远程装箱 verifier 要求）。
- `operation_abi_id` 取自 callee `PublicationAbiUnit` 中该方法对应的 operation；一个 public instance expose
  多 interface 时，只填 `as I` 选定 interface 方法集对应的 operation 子集（见 §Boxing `as I` 顺带选投影）。
- 远程表同样是 linked runtime plan / overlay（`RemoteOperationTableId`），不写回 ordinary artifact DTO；
  artifact 侧保存的是 `BoxSource::Remote` 的 symbolic 寻址信息（dependency_ref / public_instance_key /
  operation 集合 / callee_protocol_identity），linker 解析成 `RemoteOperationTable`。
- `method_abi_id` 复用与本地 method table、package public instance / remote operation projection 同一份
  canonical interface method ABI identity，不引入第二套方法身份。

## Dynamic Dispatch

对 `any I` 值调用 method：

```skiff
const out = provider.execute(ctx, call)
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
     `self`，再追加用户参数，调用 concrete receiver executable（本进程）。本地分支必有 payload，由 enum
     保证。
   - `Remote`：从 `carrier.operations` 取该 slot 对应的 `operation_abi_id`，按 `carrier.dependency_ref` 走
     service dependency dispatch（与 `remoteLlm/managedLlm.method(...)` 直接 operation 调用走同一条 outbound
     dispatch 机制——该机制复用现状 service dependency 调用路径，`/` 写法本身是新增语法）；远程分支结构上
     无本地 payload，self 由远端 instance 承载。
4. 返回值按 ordinary runtime value 返回；如果返回 `any J`，它必须是被显式装箱过的 interface value。
   注意：**远程**方法不可能返回 `any J`——远程方法对应 callee 的跨进程 operation，而 `any I` 值不跨进程
   （§Boundary Contract），故选定 interface 的方法签名含 `any I`（参/返）时该 interface 在 callee 发布期
   根本无法发布成 remote operation，consumer 装箱点随之因找不到 operation 而拒（§Boxing 远程额外要求的两道
   执行点）。下面这条只对本地装箱值成立。

无论本地远程，调用 lowering 选 slot 的逻辑相同；只有 §3 的最终 target 解析按 `carrier` 分支分流。

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
3. 把 callee 的 exact `serviceProtocolIdentity`、`public_instance_key`、选定 interface 派生的
   `operation_abi_id` 集合写入 dependency lock。

dependency lock entry 复用现状远端 binding 形态（`package-capability-bindings-implementation.md` Stage 7
的 `serviceProtocolIdentity` + `operations` + provenance 数组），只是触发点从 service.yml binding
entry 改为源码里的远程 `as I`。binding 机制退役后，provenance 字段名一并从 `bindingProvenance` 改为
`remoteBoxProvenance`（不保留死语义字段名；skiff 无兼容包袱，见 implementation R4）。

fail-closed 语义：callee 改了选定 interface 方法签名、撤了 conformance、或移除该 public instance，都会
改变锁进 lock 的 `serviceProtocolIdentity`，consumer **编译失败**，不退化为运行时才炸。校验集中在
`as I` 这一个可见点；同一个 `any I` 值后续在多处调用不重复锁定。

## 远程性的可见性（无 effect）

远程性**不**引入独立 effect。`carrier = Remote` 已经在值布局里携带"这是远程装箱值"这一事实，
工具/诊断要标注"此处发生跨 service 调用"直接读它即可，不需要在 `static-semantics.md` 的 effect 体系里
挂一个空壳。

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

**canonical example（example.com/agent 现状迁移）**：现状 `agent` 包用三个 binding——`agentTools:
ToolExecutor`、`agentLlm: LlmClient`（可能绑远端 remoteLlm）、`agentEvents: AgentEventReceiver`，在
`drain.skiff` 里直接 `agentTools.execute(input)` / `agentLlm.streamChat(input)`。合并后改为入口参数
`tools: any ToolExecutor` / `llm: any LlmClient` / `events: any AgentEventReceiver`，调用 `tools.execute(...)`
不变。`agentLlm` 绑 remoteLlm 这条恰好覆盖远程场景：consumer 传 `remoteLlm/managedLlm as LlmClient`，本地
`tools`/`events` 传 `localImpl as ...`，三者混在同一组入口参数——本地 + 远程 + 异构全覆盖（三者签名第一版
均可远程，见下方核对表）。现状之所以是 binding 而非 `any I`，仅因 `any I` 尚未实现；它一旦实现，binding 退役。

`any I` 作为 package public 入口参数不违反 publication-internal 边界：package link 进 consumer 同一
runtime，`any I` 值从 consumer 流到 package 入口全程同进程，远程性只在调用时 dispatch。见 §Boundary
Contract。

### 三个 capability interface 远程可装箱性核对

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
- method requirement 不得带 method-level type params。
- `Self` 不得出现在非 receiver 参数、返回值、record field、container element 或 function type 中。
- method requirement 不能是 `static`、`native` 或 provider-only declaration。
- marker interface 不允许 `any` 化。

现有 `interface.md` 已把大部分非 object-safe 形态排除；`any I` 实现仍要在 type checking 阶段集中诊断，
不能依赖后续 lowering/runtime 崩溃。

## Boundary Contract

`AnyInterface` 是 schema-open runtime type。边界判据是**值进入哪类 boundary policy**，不是"是否离开当前函数"。
以下位置默认 fail closed（`carrier` 里的 method table / `operation_abi_id` 在对端无意义）：

- service operation 参数或返回值。
- public instance operation signature。
- public API type schema closure。
- ~~service DB schema、queue/spawn/persistent work item payload~~ —— **此条已被 `recoverable-value.md` 取代**：
  `carrier = Local` 行为值可经可恢复 codec 进 DB/spawn/persistent（self 全可恢复时）；跨 service 恢复语义第一版
  fail-closed。DB/spawn/persistent 能否进的权威判据见 `recoverable-value.md`，不再由本条绝对排除。
- cross-service ordinary payload、ordinary JSON materialization。runtime binary payload 若是 owner-internal 跨 request
  lane，按 recoverable boundary 处理；若带 cross-service / external trust boundary，第一版行为节点 fail closed。
- config schema、test double external fixture schema。

以下位置**允许** `any I`（值不跨进程）：

- 内部 helper function、内部 record、局部变量、transient collection。
- **package public 入口的参数 / 返回类型**——package link 进 consumer 同一 runtime，`any I` 值在同进程内
  从 consumer 流到 package，远程性只在调用时 dispatch。这是 binding 退役后 package 抽象依赖能力的承载点
  （见 §Capability As Parameter）。注意：这是 package（同进程 link 单元）入口，**不是** service operation
  （跨进程 ABI）；后者仍然 fail closed。

判据收敛为：ordinary public schema 不承载 `any I` 默认 wire shape；owner-internal DB/spawn/queue/persistent/runtime
lane 按“值必须可恢复”处理；离开 owner service trust domain 的行为值第一版 fail closed。boundary walker 区分
"package 入口签名"（同进程，允许）、"service operation 签名"（ordinary 跨进程 ABI，拒绝）和
"owner-internal recoverable boundary"（按 carrier/self recoverability 判定）。

## Relationship To Type Erasure

`any I` 不推翻 runtime type erasure。它是主动建模的 dynamic wrapper，与 `ActorRef` 和 exception envelope 同属
白名单机制：

- ordinary record/object 仍然 unshaped，不携带 source nominal type。
- 装箱源身份（`carrier` 里的本地 concrete type id / 远程 instance 坐标）只保存在 interface wrapper 内，不反写 payload。
- method dispatch 本地使用 linked method table、远程使用 `operation_abi_id`，都不查 ordinary object shape。
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
- ~~**运行期实例级跨进程句柄**（为运行期临时实例铸造可寻址坐标 + "句柄→活实例"注册表 + 生命周期/GC）~~
  **（已否定，2026-06-27，据 `recoverable-value.md §Cross-Service Interface Value`）**。这块基建当初的设想是"用
  坐标机制去寻址运行期临时实例"，但**坐标与临时对象机制不匹配**：坐标只能指**有稳定身份的顶层符号**（顶层 const /
  public instance），临时对象没有这种身份，硬要给它坐标就得造注册表把它**伪装**成稳定实例——这是用错机制。正解是
  按对象类别分两条互不替代的机制：
  - **顶层符号**（单例，不可复制）→ **传坐标**。其中**已发布 public instance** 坐标 `(service, public_instance_key)`
    复用现状 service dispatch、无需注册表/GC；而**未发布私有顶层 const** 的内部路径坐标需要跨 service 寻址层**新增能力**
    （非“复用现状”）——第一版**跨 service 寻址单元只有 `api.yml` 显式发布的 public instance**（`publication.md`：跨
    service 调用按 `operation_abi_id` 寻址，只对已发布 public instance method 存在；未进 public API graph 的 symbol
    不进 service remote contract），故私有顶层 const 第一版**寻址层不认**，其坐标方案属演进（见
    `recoverable-value.md §Cross-Service`）。
  - **运行期临时（局部）对象** → **直传可恢复字节**，对端持有、回拨带回、构造侧按 build id 无状态重建等价副本，
    不铸句柄、不持有活实例、无注册表/GC。
  两条都不需要"实例级句柄 + 注册表"，故本块基建不再认领。**进恢复边界（值被传去对端、对端回拨）的只有局部对象直传
  这一条，其卡点是下方的 service callback transport**；顶层符号那条第一版要么是正向 `Remote`（consumer 主动调、不进
  恢复机制）、要么寻址层不认（演进），不在“进恢复边界”之列。
- **service callback transport**（consumer 拿句柄回调打回 callee 特定活实例）。现状 outbound dispatch 单向
  （runtime 主动连 router，业务流量 consumer→callee）；反向入站路由到一个特定活对象实例是相反方向的传输层
  新增。
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
  坐标**到 consumer 可寻址的地址系，不是原样透传。这是纯坐标重映射，不涉及临时实例。（未发布的私有顶层 const 第一版
  跨 service 寻址层不认，不在此列——见 `recoverable-value.md §Cross-Service` 演进项。）
- callee 返回的 `any J` 装箱源是**运行期临时（局部）实例**（`localImpl as J`）：没有顶层符号身份，坐标无处可指，
  改走**直传**——callee 把它的可恢复字节进 wire，consumer 持有，回拨时带回、由 callee 按 build id 无状态重建等价
  副本。**不铸句柄、不维护注册表**（那块基建已否定，见上）。

两类**都不需要"运行期实例级句柄 + 注册表"**。真正的卡点是 **service callback transport**：consumer 之后对这个
`any J` 调方法要反向打回 callee，而现状 outbound dispatch 单向。该通道落地后，本约束方可解除；在此之前，远程方法
签名含 `any I` 在装箱点编译失败。（顶层符号坐标本身的寻址复用现状 service dispatch，不依赖 callback transport；
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
  fan-out（动态得靠 `spawn` 手搓）。toolprovider 场景闭环需要"对 Array 并发 map"原语。这是将来是否要求
  远程调用出现在并发上下文（§远程性的可见性）的前提。
- **`as I` 是否隐式引入 service dependency**：§Remote Fail-Closed 要求远程装箱源是已声明 dependency。是否
  允许远程 `as I` 隐式引入 dependency，待 implementation 定。
