# Skiff `any I` —— First-Class Interface Value（动态分派）

本文负责：稳定描述 `any I`（first-class interface value / 动态分派）的目标态用户语义——`any I` 类型、`as I` 装箱、裸 interface 名的使用规则、值布局、与单态化泛型的边界、以及 publication 内部适用范围。

本文不负责：parser/lowering/runtime vtable 的实现细节、artifact 字段表。这是 `interface.md §7` 末尾点名的“另立设计”——它在 §7 已声明“如果未来需要 first-class interface value，必须另立设计，显式定义 value layout”。本文即该设计。

状态：语言设计输入（未实现）。日期：2026-06-23。

## 1. 动机与定位

`interface.md §7` 禁止把 interface 当普通值（不能传参/存储/放容器），只允许 compile-time 受控的 receiver root。这使 skiff 完全静态：每个 interface 方法调用在编译期解析到唯一目标，无运行时分派。

但这挡住了一类真实需求：**一个集合里持有多个、可能是不同实现类型的 interface 值，并统一调用**（异构集合）。典型场景：一个对话挂载若干 tool provider（host / db / api 不同实现），运行期数量任意、种类可由使用方扩展，驱动方对它们一视同仁地调用。

这是 universal（参数化多态）解决不了、只有 existential（存在类型 / 动态分派）能解决的形态——见 §6。`any I` 引入这第二种多态。

**定位：`any I` 是显式的、opt-in 的动态分派。静态分派（单态化泛型）仍是默认。** skiff 不重蹈 Rust 早期的错误（裸 trait 名一名两用、动态分派是无标记默认、装箱隐形）：skiff 从一开始就规定裸 interface 名不能当类型，动态值必须显式 `any I`，装箱必须显式 `as I`。

## 2. 语法

### 2.1 裸 interface 名不能当类型

interface 名 `I` 单独出现在类型位置是**编译错误**。必须二选一：

```skiff
function f(p: ToolProvider)            // ❌ 非法：裸 interface 名不能当类型
function f<T: ToolProvider>(p: T)      // ✅ 约束：静态分派，单态化，同质
function f(p: any ToolProvider)        // ✅ 存在类型：动态分派，可异构
```

这条规则是与 Rust 的关键分野：Rust 早期允许裸 `Trait` 既作约束又作类型，`dyn` 是事后补的标记。skiff 让“一名两用”从类型语法上不可能——值类型位置只能写 `any I`；裸 `I` 只在 `<T: I>` 这类约束位置，或 `expr as I` 这类装箱右侧的 interface selector 位置出现。

### 2.2 `any I` 作类型

`any I` 是一个类型，表示“某个实现了 `I` 的值，其具体类型已擦除”。它只可用于 publication 内部普通值位置：
内部函数参数、内部返回值、不经任何 boundary schema closure 投影的内部 record 字段（含具名 record 类型）、
collection element、map value，以及内部 function type 的参数/返回位置（如 `fn(any I) -> void`）。

```skiff
type ThreadConfig {
  providers: Array<any ToolProvider>,   // 异构集合
  defaultProviderByName: Map<string, any ToolProvider>,
}

function listAll(providers: Array<any ToolProvider>) -> Array<ToolDefinition> { ... }
```

`any I` 不能作为 map key——这由 **type checker 静态拒绝**（等同 §2.1 "裸 interface 不能当值"级别的保证，
不退到 runtime，也不依赖独立的 boundary validator 兜底；权威定义点见
`../architecture/any-interface-value.md §Type Model`）。本版也不定义 equality/hash/ordering/JSON 编码语义；
其中 JSON/DB encode 同样是 type checker 静态拒绝，equality 这类无法在 type checker 拦尽的残余情况由 runtime
fail closed。

### 2.3 `as I` 装箱（显式类型擦除）

从一个实现了 `I` 的**装箱源**得到 `any I`，必须显式写 `as I`。装箱源有两类：**本地 concrete nominal
record 值**，或**远程 public instance 寻址源**（见 §2.5）。

```skiff
const p: any ToolProvider = HostProvider { ... } as ToolProvider   // 本地装箱源
providers.push(DbProvider { ... } as ToolProvider)
const llm: any LlmClient = remoteLlm/managedLlm as LlmClient           // 远程装箱源（见 §2.5）
```

- 本地装箱源：`expr` 的静态类型必须是 concrete nominal record instantiation，且 `implements I`（显式
  conformance，遵循 `interface.md §4`）。
- 远程装箱源：见 §2.5。
- **`as I` 不能省略**，即使装箱源只 expose 一个 interface，也即使赋值/参数已有 `any I` 目标类型。理由是
  **装箱可见性**，不是消歧：`as I` 是类型擦除发生点（产生 fat 值 / 对远程还含寻址填入与 protocol identity
  锁定），这是有运行时表示成本、单向不可逆、对远程还有跨 service ABI 后果的操作，必须在
  源码可见；省略即隐式装箱，违反 `interface.md §7` "不能把 concrete object 隐式装箱成 interface value"。
  这与 Go/Java 的隐式向上转型不同——那里"接口值"是无处不在的隐式默认，skiff 的值默认类型擦除、`any I`
  是显式抠出的特例。
- 一个装箱源 implements 多个 interface 时，`as I` 顺带选定投影到哪个 interface（`any I` 的 `I` 必须是单一
  interface），装箱后只能调该 interface 的方法。注意：多 interface 本身**不是** `as I` 必写的理由——即使
  只有一个 interface、或目标类型已指定，仍要写 `as I`，理由如上（装箱可见）。
- `as I` 是 expression postfix/conversion operator：在 field/call/generic/constructor 之后解析，优先级高于二元/逻辑运算。
  因此 `a + b as I` 等价于 `a + (b as I)`；若要装箱整个复合表达式，应写 `(a + b) as I`。

> `as` 复用说明：`as` 已是 skiff 关键字，用于引入绑定名（`with expr as name`、`db claim ... as obj`）。`expr as I`（`as` 后跟 **interface 类型**而非新名字）与之语境隔离，且与主流语言 `as` 类型转换/断言惯例一致，不产生歧义。Rust 的真错误是裸 trait 名歧义（已由 §2.1 的 `any I` 规则消解），与 `as` 复用无关。

### 2.4 方法调用

对 `any I` 值调用 `I` 的方法，语法与普通调用相同；编译器 emit 间接调用。分派路径由该值的装箱源决定，
对调用方透明：

- 本地装箱值：经该值携带的 method table 调用本地实现（本进程）。
- 远程装箱值：经该值携带的 operation 寻址走跨 service 调用（带 remote 语义，见 §2.5、§6）。

```skiff
const p: any ToolProvider = ...
const tools = p.listTools(ctx)     // 间接调用：本地或远程，由 p 的装箱源决定
const out = p.execute(ctx, call)
```

只能调用 `I` 声明的 method requirement；`any I` 不暴露具体类型的其它成员。

### 2.5 远程装箱源（`/` 寻址）

远程能力是一个**可流动的 `any I` 值**：一个实现某 interface 的远程 public instance，装箱后可像本地
`any I` 一样赋值、传参、进 `Array<any I>`，调用方法时走跨 service 调用。

```skiff
const llm: any LlmClient = remoteLlm/managedLlm as LlmClient
const providers: Array<any ToolProvider> = [
  remoteLlm/remoteTools as ToolProvider,   // 远程
  localTools as ToolProvider,           // 本地
]
```

- 远程装箱源用 `/` 寻址：`<dependencyAlias>/<publicInstanceKey>`，如 `remoteLlm/managedLlm`。`/` 左边是已声明
  service dependency 的 alias，右边是 callee 公开的 public instance。跨 service 寻址用 `/`，与成员访问 `.`
  区分（`.` 会误导成"取字段"）。
- 结合性：`/` 寻址先于 `as` 装箱结合，即 `remoteLlm/managedLlm as I` 解析为 `(remoteLlm/managedLlm) as I`——先把
  `remoteLlm/managedLlm` 寻址成装箱源，再 `as I` 装箱。`/` 寻址整体优先级与 `.`/call 等 postfix 同级，高于
  §2.3 所述 `as` 的转换运算优先级。
- 裸 `remoteLlm/managedLlm` **不是值**，没有可赋值的类型。它只能出现在 `.method(...)`（直接调用）或 `as I`
  （装箱）左边。`const x = remoteLlm/managedLlm` 非法。
- 远程装箱源必须显式 implements `as I` 选定的 interface，并公开该 interface 方法对应的 operation。
- 远程对象也是本地对象：装箱后的 `any I` 值在本进程内自由流动；"远程"只体现在**调用方法时**——那一刻
  发生跨 service 调用（带 remote 语义）。值传递本身不跨进程。
- 远程能力是 request-scope 正向远程引用，不能持久化后跨 request 重建（见 §7）。若进入
  DB/spawn/queue/persistent payload 或显式 recoverable slot，按可恢复值规则以
  `recoverable_remote_carrier_not_persistable` fail closed。

## 3. 值布局

`any I` 的值布局显式定义。它承载两类装箱源（本地 / 远程），布局据此泛化：

```text
any I = {
  interface_id        // 装箱后保留的 interface（本地 / 远程共有）
  carrier             // 装箱源整体，本地 / 远程二选一（一个 enum 分支，不是三个独立字段）：
                      //   Local  { concrete_type, method_table, payload }
                      //     concrete_type: 具体 type id（保留，供未来 downcast；本版不暴露）
                      //     method_table:  I 的 method requirement 的具体实现地址表
                      //     payload:       具体值本体
                      //   Remote { dependency, public_instance, operations }
                      //     dependency/public_instance: (依赖, 远程实例) 寻址坐标，即"是哪个远程实例"
                      //     operations: I 方法 → operation 寻址 + service dispatch
                      //     （远程分支无本地 payload，self 由远端实例承载）
}
```

本地装箱值是一个 fat 值（数据 + 方法表）；远程装箱值不携带本地 payload，只携带寻址坐标与 operation
寻址。"本地必有 payload、远程必无 payload" 由 `carrier` 是单个 enum 分支天然保证——`source_identity` /
`dispatch` / `payload` 不是三个可独立取值的字段，拼不出非法组合。无论哪种，都不是把普通 object 的
per-instance shape 偷偷扩成隐式 vtable（`interface.md §7` 明确禁止后者）。该布局只在显式 `as I` 装箱点
产生。完整内部契约（含 `carrier` enum 定义、fail-closed、远程性可见性）见
`../architecture/any-interface-value.md`。

## 4. 与单态化泛型的边界（为什么两者都要）

| | `<T: I>`（约束，单态化） | `any I`（存在类型，动态） |
| --- | --- | --- |
| 多态种类 | universal / 参数化 | existential / 存在 |
| 类型信息 | 保留（`T` = 具体类型） | 擦除（只剩“满足 `I`”） |
| 分派 | 编译期静态 | 运行时：本地经 method table / 远程经 operation 寻址 |
| 值表示 | 原生具体类型 | 本地 fat 值（含 method table）/ 远程寻址值（含 operation 坐标） |
| `Array<它>` | 同质（一次调用一个 `T`） | **可异构**（混装不同实现） |
| `-> 它` | 可表达“进出同型” | 不可（已擦除） |
| 成本 | 零运行时间接，代码膨胀 | 间接调用，无膨胀 |

要点：异构集合（一对话多种 provider）**只有 `any I` 能表达**，`<T: I>` 的 `Array<T>` 是同质的，装不下不同实现。两者不可互相替代，故都保留；默认用 `<T: I>`（静态），需要异构/类型擦除时显式 `any I`。

## 5. object-safety（哪些 interface 可被 `any` 化）

只有满足 object-safety 的 interface 才能用于 `any I`（method table 的每个槽必须是定长可寻址的实现地址）：

- method requirement 必须有 `self: Self` receiver。
- method requirement 不得带 method-level 泛型参数。
- `Self` 不得出现在非 receiver 参数、返回值、record 字段、container element 或 function type 中。
- method requirement 不得是 `static`、`native` 或 provider-only declaration。
- marker interface（无 method requirement，`interface.md §6`）`any` 化无意义（无可调方法），本版不允许。

（`interface.md §3` 已禁 interface method 的 method-level 泛型/`static`/`native`，故大多数 interface 天然 object-safe；
`any I` 仍要独立诊断 object-safety，不能依赖 runtime/linker 才发现。）

## 6. 与现有禁令/boundary 的关系

- **`interface.md §7` 的切割**：§7 的"interface 不能当普通值"被本文以 `any I` 形态**解禁**——值类型位置可
  写 `any I`（裸 `I` 仍不行，见 §2.1）。§7 的"不能**隐式**装箱 concrete object 成 interface value"本文
  **继续坚持并强化**为 `as I` 强制（见 §2.3）。即：§7 禁的是"裸值化 + 隐式装箱"两件事，本文只解前者、保后者。
- **吸收 binding**：原 binding alias / dependency public instance 受控 root 被 `any I` 合并取代——package
  能力依赖改为入口吃 `any I` 参数，consumer 在调用点用 `as I`（本地或远程）装箱传入（见 §8）。`any I`
  是显式的、可流动的 first-class 形态。
- **Boundary**：`any I` 是 publication 内部普通值，但进入边界时按边界 policy 判定。
  service public API payload、public instance operation signature、ordinary JSON materialization、config schema 或 test
  double external fixture schema 的默认 wire shape 不承载 interface value，也不会隐式生成 recoverable envelope。
  DB schema、
  `spawn` 参数、queue / persistent work item payload 和 runtime 内部跨 request payload 是 owner-internal
  recoverable boundary：`carrier = Local` 且 self payload 全可恢复时可恢复；`carrier = Remote` 是 request-scope
  正向远程引用，不可持久化。跨 service 行为值第一版 fail closed，目标态依赖 sealed opaque payload 与 callback
  transport。权威规则见 [`recoverable-value.md`](../architecture/recoverable-value.md) 和
  [`any-interface-value.md`](any-interface-value.md)。
  `any I` **可以**作为同进程 **package public 入口参数**（package link 进 consumer 同一 runtime，值不
  跨进程；远程性只在调用时 dispatch）。这是 binding 退役后 package 抽象依赖能力的承载点（见 §8）。
- **§4 显式 conformance**：`as I` 装箱要求装箱源显式 `implements I`，本地远程一致，与 §4 一致。

## 7. 非目标（本版）

- **不支持 downcast**：`any I` 是单向擦除，不能取回具体类型（本地装箱保留 concrete type identity 供未来；
  远程装箱保留的是 `(dependency, public instance)` 坐标，本就不可能 downcast 回本地 concrete type）。本版
  不暴露 narrowing/downcast。
- **跨 service 行为值第一版 fail closed**：owner-internal DB/spawn/queue/persistent payload 按可恢复值规则处理，
  不再由 `any I` 绝对排除；但跨 service 把行为值交给对端后回拨构造侧的形态第一版仍 fail closed，直到 sealed
  opaque payload 与 callback transport 落地。远程**装箱**（远程能力作为 request-scope `any I` 值，§2.5）是支持的；
  不支持的是把这个 `carrier = Remote` 值序列化进跨 request / 持久 payload，或 durable 化为远程能力句柄。
- **不改默认**：默认仍静态分派；`any I` 必须显式。
- **不支持隐式装箱**：必须 `as I`。
- **不支持 marker interface 的 `any` 化**。

## 8. Capability as parameter（binding 退役）

package "我需要某能力但不指定来源"不再用 capability binding，而是声明一个吃 `any I` 的入口参数：

```skiff
// package 源码：只 import 定义 interface 的包，不依赖具体实现
function run(input: AgentInput, llm: any LlmClient) -> Stream<LlmStreamEvent> {
  return llm.streamChat(toLlmRequest(input))
}
```

consumer 在调用点装箱后传入，由此决定本地/远端：

```skiff
agent/run(input, remoteLlm/managedLlm as LlmClient)   // 远端 remoteLlm
agent/run(input, localLlm as LlmClient)           // 本地实现
```

- package 依赖"能力的形状"（import 定义 interface 的包），不依赖"能力的实现"。本地 vs 远端由 consumer
  在调用点选 `remoteLlm/...`（远程装箱）还是 `local...`（本地装箱）决定。
- `any I` 参数可以传任意多个、混入 `Array<any I>`——不像 binding 那样每个 requirement 只能绑一个实现、
  装不下运行期变长的异构集合。
- 远程能力一旦改了签名 / 撤了 conformance，consumer 的装箱点编译失败（fail-closed 锁在产生远程引用的
  装箱点，package 不碰）。

`requires.bindings` manifest 机制本版退役，不再支持旧的 binding 写法。（是否在后续版本以更轻的"能力
requirement"声明形态回归——让 consumer 用 `as I` 满足——是一个开放设计项，见
`../architecture/any-interface-value.md §Evolution`。）

## 9. 实现要点（仅提示，详见架构/implementation 文档）

落地涉及：parser 加 `any` 类型前缀、`as I` 装箱表达式与 `/` 远程寻址；类型检查在 §2.1 规则下解禁
“interface 当值”但限定为 `any I`，并加 object-safety 检查；lowering 在 `as I` 处构造值（本地 fat 值 /
远程寻址值）、在 `any I` 方法调用处 emit 间接调用（本地 method table / 远程 operation dispatch）；
runtime 执行间接调用。**本地** `any I` 不影响 ABI/identity；**远程**装箱在装箱点把 callee protocol
identity 锁进 dependency lock（fail-closed）。远程性由值布局 `carrier` 是 `Remote` 分支表达，不引入
独立 effect。DB/spawn/queue/persistent 等跨 request 边界是否接受该值由 recoverable boundary plan 与 runtime
carrier check 决定，本文不展开。

内部架构契约见 `../architecture/any-interface-value.md`；可恢复边界契约见
`../architecture/recoverable-value.md`；落地阶段计划见
`../implementation/any-interface-completed.md` 和 `../implementation/any-interface-todo.md`。
