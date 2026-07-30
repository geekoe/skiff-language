# Skiff Static Semantics Reference

本文负责：稳定描述 Skiff 类型身份、名字解析、import 解析、target typing、错误捕获、match narrowing、
函数 effect / suspension 推断、服务 API 类型边界、schema closure 和 stream 静态边界。interface
专门语义见 `interface.md`。

本文不负责：源码 token / AST 细节、运行时调度和取消算法、具体 wire 编码、manifest 字段表、标准库完整 API surface、测试 runner 发现规则。

## 1. Core Principles

Skiff 以名义身份优先。用户声明的 `type` 都创建独立身份；两个类型即使字段或 RHS 形状相同，也不能结构化互换。

显式构造优先于隐式包装。目标类型上下文不会自动把 RHS payload 包成某个 representation，也不会把一个名义值改标成另一个名义值。

匹配和捕获只依赖有限、可实现的运行时信息：名义 type id、显式 interface conformance、声明 discriminator 字段和普通 literal 值。

远程 API、跨服务 payload 和持久化 schema 必须能闭包成对应边界的静态 policy。普通 public schema 仍要求确定 schema；
跨 request / 持久边界还要求值满足可恢复值 contract。裸 interface、callback、`unknown` 和无法枚举的结构不能藏在
ordinary schema 边界深层；`any I` 只有在明确的 owner-internal recoverable boundary 中按可恢复值规则处理。

record / object / collection 采用可变引用语义。`const` 只约束 binding，不表示 deep immutable。

## 2. Nominal Representation Types

`type Name = R` 创建名义 representation。`Name` 的值在类型系统中不是 `R` 的透明 alias，也不与 `R` 建立赋值或传参兼容。

每个 representation 自动拥有系统 constructor 和 projection：`Name(value)` 构造，`Name.value(value)` 投影最外层 payload。用户不能覆盖这些系统成员。

构造合法的条件是参数静态类型可赋给 RHS `R`。若参数是另一个 representation，必须先显式 projection；编译器不会隐式拆解再构造。

赋值、传参、数组元素 target typing 和 record 字段 target typing 都不会隐式构造 representation。已有 representation 也不会被隐式 rewrap。

representation 值可继承 RHS 的字段、方法和 prelude receiver 能力用于 member lookup；这不改变 assignability。若 RHS 方法返回 `R`，通过 representation receiver 调用后返回类型仍按原签名计算，不自动 rewrap。

`==` / `!=` 只允许同一 representation 类型相互比较，比较其 payload。representation 与 RHS 直接比较、不同 representation 之间比较，即使 RHS 相同，也应报错。

ordering 只有在 RHS 支持 ordering 且两边是同一 representation 类型时允许。hashing 和 map key 类型同样保留名义身份。

序列化时，如果目标 schema 已唯一指定 representation 类型，wire payload 可以只编码 RHS payload；在 union 或无法唯一恢复的动态位置，schema 必须保留足够 discriminator。

literal pattern 可匹配 representation payload；nominal pattern 匹配 representation 的 type id。该能力只在 pattern 语义中成立，不等于表达式隐式 projection。

## 3. Alias Types

`alias Name = R` 是透明类型缩写，不创建 nominal type id，不生成 constructor / projection，也不参与 runtime discriminator。

alias 在 assignability、参数传递、字段检查、method set、schema closure 和 encode / decode 中按 RHS 展开。展开应发生在 IR 或 contract usage descriptor 生成前。

alias 声明可作为诊断、文档和 export metadata 保留 source spelling，但语义身份来自 RHS。

当前用户 alias 非递归、无 type parameter，使用点不能写 type args。递归值结构应使用名义类型或 compiler-known prelude 类型表达。

## 4. Named Union Identity

`type U = A | B | ...` 创建命名 union / representation 身份。运行时值同时保留 enclosing union context 和实际 branch identity。

concrete nominal branch 的 identity 是该具体名义 type id。anonymous discriminator record branch 的 identity 是编译器生成的稳定 synthetic branch id。literal branch 的 identity 由 enclosing type id 和 payload literal 共同决定。

带 `discriminator "field"` 的命名 union 中，每个 anonymous record branch 必须包含该字段，字段类型必须是唯一的 string literal。

anonymous discriminator record 的 synthetic branch id 由全限定 union type id、完全实例化后的 type args 和 discriminator 字段值派生。它不是用户可写类型名。

有目标类型 `U` 时，concrete nominal branch 值可进入 `U` context；discriminator record literal 由 discriminator 字段唯一选择 branch；string literal branch 由 literal payload 选择。

无目标类型时，object literal 不能自行推断为命名 union、record 或 map。需要返回类型、变量标注、参数类型或其他显式上下文。

record literal target typing 要求未知字段报错，缺失的非 nullable 且无默认字段报错，缺失的 nullable 字段在源码 literal 构造中默认填入 `null`。该规则不自动改变 wire decode 的字段缺失语义。

不同命名 union 即使 branch 形状完全一致也不是同一类型。若同一 concrete branch 可进入多个命名 union，必须由目标类型决定 union context。

匿名 union 只存在于静态类型表达式，没有自身 runtime type id。编译器不能在无目标类型时把匿名 union 自动提升成某个命名 union。

## 5. Throw, Catch And Rethrow

`CatchLeaves(T)` 是类型 `T` 可作为 runtime catch payload 的 concrete nominal leaves 集合。

任意用户 `type` 声明都创建可抛出、可捕获的名义 runtime identity，包括名义 record、名义
representation 以及命名 union 中的 concrete / synthetic branch identity。透明 `alias` 不创建新 identity，
按 RHS 展开。union 的 leaves 是各 branch leaves 的并集，且所有 branch 都必须有可确定的 runtime identity；
命名 union 按其保留 enclosing union context 的 runtime branches 计算 leaves。

未被用户 `type` 包装的 interface、primitive、literal、anonymous record、container、`unknown`、`FnType`
和无约束类型参数本身不能成为 catch leaves。它们可以作为名义 representation 或名义 record 的内部
representation；这不取消外层 `type` 的名义 catch identity。

`throw expr` 合法当且仅当 `expr` 的静态类型所有可能运行时值都有可确定的名义 catch identity。运行时
envelope 记录实际 concrete leaf identity；对命名 union，它同时保留 enclosing union context。

`catch<E>` 合法当且仅当 `CatchLeaves(E)` 非空。捕获时按 envelope 的 actual payload type id 与 leaves 集合匹配，否则继续向外传播。

`catch<MyErrorInterface>`、`catch<unknown>`、`catch<string>` 和无约束类型参数捕获都应报错。不存在要求
错误类型实现的 `ErrorPayload` marker。

`rethrow exception` 要求操作数静态类型是 `Exception<E>` 且 `CatchLeaves(E)` 非空。它重新抛出同一 envelope，不创建新 throw site。

`Exception<E>` 和 `CatchResult<T,E>` 是 request-local 控制流结构，不是 boundary payload；不能出现在 service API、contract public type、跨服务 payload 或持久化 schema 中。

可抛出不等于可序列化。package内部throw不要求`SchemaClosed`。未捕获错误越过service boundary时，只有
在其owner Package中显式公开且`SchemaClosed`的名义类型才能保留原始类型；私有、非closed或编码失败的错误
由runtime替换为`std.service.InternalError`。具体错误envelope与逐跳异常栈语义见`runtime.md`和
`../architecture/package-service-contract-deployment.md`。

## 6. Match Typing And Narrowing

match 检查顺序是：检查 scrutinee，按 arm 顺序检查 pattern，在 arm body 中应用 narrowing 和 bindings，检查穷尽性与不可达 arm，表达式 match 再合并结果类型。

pattern 按顺序匹配。重叠 arm 运行时选择第一个匹配 arm；编译器对可证明完全覆盖后的 arm 报不可达。

表达式 match 有外层目标类型时，目标类型传播到每个非 `never` arm。所有正常产值 arm 都必须可赋给目标类型，整体类型为该目标类型。

无目标类型时，丢弃 `never` arm；剩余 arm 类型相同则结果为该类型，否则结果为规范化匿名 union。不会自动推断命名 union。

record literal arm 仍需要目标类型。不能依靠分支 union 反向推断匿名 record。

`never` 是 bottom type。有目标类型时总可接受；无目标类型 join 中被忽略；所有 arm 都是 `never` 时整体为 `never`。

literal pattern 匹配 literal 值；对 representation scrutinee 匹配 payload。wildcard `_` 匹配任何值且不绑定。

identifier pattern 绑定当前值，不引用外层变量。nominal pattern 匹配 concrete nominal type id；若 pattern 名是 interface，则按 `interface.md` 的 conformance-test 规则匹配显式 conformance。

record pattern 只检查列出的字段，允许额外字段。空 record pattern 可用于纯 nominal、interface conformance 或 representation test。

泛型 nominal pattern 必须能确定完整 type args。写出完整 type args 总是允许；省略只有在 scrutinee 静态类型唯一确定实例时允许。

当 scrutinee 是稳定访问路径时，pattern 在 arm 中收窄该路径。稳定路径包括局部名、参数名和字段路径，不包括动态下标和函数调用结果。

discriminator record pattern 可把命名 union 收窄到对应 anonymous branch。字段只存在于部分 branch 时，必须先通过能唯一确定 branch 的 pattern 收窄。

or-pattern 的 narrowing 是各分支 narrowing 的 union；所有分支必须绑定同一组变量名且类型一致。

穷尽性检查至少覆盖 string literal union、bool、nullable 中的 `null`、discriminator record union、concrete nominal type union 和这些有限 union 上的 representation。

## 7. Nullable Control-Flow Narrowing

当前支持有限、保守的 `if` 条件 narrowing。它只作用于稳定访问路径。

`path == null` 在 then 分支中把 path 收窄为 `null`，在 else 分支和 then 必然早退出后的后继中收窄为 non-null。

`path != null` 反向处理。`!condition` 交换 true / false narrowing。

`a && b` 的 true 分支同时应用 `a` 的 true narrowing 和在该环境下 `b` 的 true narrowing。false 分支只应用编译器能安全证明的排除。

`a || b` 的 false 分支同时应用 `a` 的 false narrowing 和在该环境下 `b` 的 false narrowing。true 分支只应用可安全证明的包含。

若 then 或 else 分支必然以 `return`、`throw`、`rethrow`、`break` 或 `continue` 退出，后继代码可应用相反条件的 narrowing。

对某个稳定路径或其前缀赋值，会清除该路径及其子路径的 narrowing。

record / object 是可变引用。传参、callback、method call、host / package API 只要可能写入相关路径或其前缀，就必须清除旧 narrowing。

loop 中 narrowing 只在本次迭代控制流内有效，不自动归纳到下一次迭代。

编译器无法稳定证明时必须放弃 narrowing，要求代码使用 `match`、局部 non-null 绑定或更直接条件结构。

## 8. For-In And Collection Static Rules

`for item in iterable { ... }` 的单绑定形态按 iterable 静态类型确定 loop binding 类型：

- `iterable: Array<T>` 时，`item: T`。
- `iterable: Stream<T>` 时，`item: T`。
- `iterable: Map<K,V>` 时，`item: K`。
- 其他类型报错。

`for key, value in iterable { ... }` 的双绑定形态只允许 `iterable: Map<K,V>`。此时 `key: K`、`value: V`。`for a, b in array`、`for a, b in stream` 和对非 map 类型使用双绑定都必须报错。

loop binding 只在 loop body 内可见，离开 body 后恢复外层同名 binding。loop binding 不是 `let` 声明，不能作为 assignment target 重新绑定。双绑定形态中的两个名字不能重复；重复时按同一词法作用域的局部 binding 冲突报错。

对 `m: Map<K,V>`，`m.keys()` 的静态类型是 `Array<K>`。如果 `K` 是 string representation 类型，返回数组元素类型仍是该 representation 类型，不退化为 `string`。

当前合法 map key 类型只允许精确 `string` 或单一名义 representation over string。`Map<number,V>`、`Map<bool,V>` 和非 string payload representation key 都必须按既有 map key 规则拒绝；未来扩展非 string key 时，必须先定义 assignability、boundary encoding、runtime key representation 和 canonical ordering。

## 9. Name Resolution And Reserved Names

Skiff 使用 type namespace、value namespace 和 method namespace。`impl` 不绑定新顶层名字，只向 receiver 的 method namespace 添加方法。

类型名在 value 表达式的 member / call 位置也可作为 type namespace 使用，例如 representation constructor、projection 和 static method。

虽然 lookup 按上下文区分 type / value，当前仍要求同一模块顶层声明和 import local binding 的文本名不得重复。

关键字不能作为任何用户标识符。核心 prelude type 名、primitive 名、`Self`、`Array`、`Map`、`Stream`、`Json`、`JsonObject`、错误和 gateway 相关 prelude 类型都是 type namespace 保留名。

`std`、`root` 和 `config` 是 value namespace reserved root，用户不得声明、作为 alias 或局部绑定同名标识符。

`std.*` path 不是裸标识符保留名。用户可以局部使用 `http`、`json` 等普通名字，但不能 shadow `std` root 或已 import package local binding。HTTP helper 走 `std.http.*` 模块路径，例如 `std.http.json`。

局部 value binding 不能与当前词法作用域已有 value 名冲突；可在嵌套 block 中 shadow 外层局部 value 名。

局部 value binding 不能 shadow 顶层声明、import local binding、type / interface 名、primitive / prelude 名或 reserved root。pattern binding 按同样规则检查。

`integer` 是 compiler-known prelude refinement，运行时表示仍是 `number`。`integer` 可赋给 `number`；`number` 不能隐式赋给 `integer`，除非是可静态证明的整数 literal 或经过显式校验转换。

`Json` 和 `JsonObject` 是 compiler-known recursive prelude types，不是普通用户 alias。裸 JSON 不携带用户 representation、union 或 map-key identity。

## 10. Import Resolution

`import` 只解析外部 package dependency，不导入 module 或 symbol。源码访问 package export 必须从 local binding 开始。

`std` 是 compiler-provided platform root，不需要 `packages` entry，也不需要 `import std` 才能使用。`import std` 只作为显式风格保留；`import llm`、`import gcloud` 等引入 manifest / package 配置中绑定到全局 package id 的 alias。

resolver 不接受 `import std.http`、`import ext.openai`、`import skiff.run/llm` 或 `import api.user.UserQuery`。

`skiff.run/std` 不能作为普通 package dependency 声明；用户 package、import alias、顶层声明和局部 binding 都不能 shadow `std`。

当前 package / service source set 的跨文件访问统一使用 `root.*` all-symbol index。普通 source file
没有public visibility marker；内部`root.*`可见性不受Package public API影响。

`root.<module>.<Symbol>` 解析到当前 source set 中对应模块的顶层 type / alias / interface / function / const 等声明。这里的 all-symbol 明确包含未进入 public API graph 的顶层声明；source file 不是包内 privacy 边界。`root.*` 不能解析局部变量、参数、pattern binding，也不能把 impl method 当作顶层符号访问；method 仍属于 receiver 的 method namespace。

Production build 的当前 source set 只包含 production source files。`*.test.skiff` 中的 helper、fixture type 和测试用 import 只在 test-runner 构造的测试编译模型中参与解析，不进入 production `root.*` index，也不影响 production root reference validation、artifact identity 或 public API graph。

外部 package 仍必须通过 import alias 访问其 published public API。`root.*` 不穿透 dependency 的 private 符号；如果某段共享代码需要被别的 package 使用，应进入该 package 的 public API，而不是依赖当前 package 的内部 root path。

唯一例外是`kind: test` service中dependency entry声明的`topLevelAlias`。普通`alias`始终按该dependency
的`api.yml` public path解析；`topLevelAlias`则按
`<top-level-alias>/<source-module-path>.<top-level-name>`解析同一直接dependency的精确implementation
symbol。两者没有fallback或precedence。该模式覆盖同一文件顶层type及附着到它的`db object`，因此
`db require subjectImpl/model.User(id)`中的target与`subjectImpl/model.User`类型引用选择同一个精确
provider type。

`topLevelAlias`只允许出现在`kind: test` service，必须是合法唯一identifier，并与所有package/service
alias和其它top-level alias不冲突。旧`access: topLevel`必须fail closed。top-level权限不传递：
dependency public ABI可以闭合其dependency公开类型，但consumer不能因此直接访问transitive dependency
top-level；需要时必须另行声明direct dependency及其`topLevelAlias`。

public alias与top-level alias仍表示同一个direct dependency edge。Source resolution保留顶层路径及
exact implementation expectation；lowering时package reference canonicalize回该entry的primary
`alias`，绑定同一个`expectedPackageBuild`，不得生成第二个`PackageRequirement`、code slot或collection
projection。

当表达式的静态receiver type来自该direct `topLevelAlias`视图时，精确implementation type还开放同一
artifact中该type现有的impl method namespace。解析必须同时满足：

- receiver为带完整`abiExpectation`的精确`PackageSymbol`，或以它为base并保留完整substitution的
  `AppliedNominal`；
- dependency是当前test service的direct edge，且调用点来自该entry的top-level view；
- method唯一解析为该精确PackageArtifact中的`PackageCallable`，其Local ABI/build expectation与
  receiver一致。

源码调用的显式参数arity与类型检查不计receiver；lowering沿用普通receiver-call形状，把receiver作为
执行参数的第一项，再追加源码中的显式参数。此规则不新增路径语法、调用关键字或动态method lookup。
普通public alias只开放API公开的public instance method，不能因为返回值保留名义类型就访问任意impl
method；service boundary对象不能获得provider package-local method；interface receiver仍只按interface
slot dispatch，不回退到concrete package method。任何view、ABI/build、generic substitution或method
identity不精确的情况都fail closed。

跨package DB target的符号约束与其它topLevel symbol相同：consumer保留dependency alias、完整
symbol path和ABI expectation；dependency requirement另行约束精确provider build。Linker必须把两者
解析到同一PackageArtifact及同一File IR type，且该File IR存在附着到该type的DB declaration。任何
缺失、ABI/build不匹配或用另一个artifact的同名type替换都fail closed。

当前禁止 import cycle。顶层 `const` 初始化按源码顺序检查，只能引用已声明的本模块顶层 `const` 或 import 进来的顶层符号。

顶层 `const` 初始化必须是纯的、不可变的、请求无关常量表达式。不得保存请求上下文、用户、trace、事务、临时缓存或随 request 改变的值。

`Array`、`Map` 等 mutable collection literal 不允许用于顶层 `const` 初始化。请求内共享临时状态应使用参数、局部变量或显式 request context 模型。

## 11. Callback Static Rules

`FnExpr` 只允许在 IIFE callee 或白名单 callback 参数位置出现。用户自定义函数不能声明 callback 参数；record 字段、type RHS 和容器元素也不能包含 `FnType`。

白名单 API schema 为 callback 参数提供 target function type。arity、参数类型、返回类型和 body 按该 target 检查。

具名函数不是一等值，但在 callback 参数位置可作为 callback reference。它不能被绑定到变量、返回、存储或传给非 callback 参数。

当前只允许 lane-local non-escaping callback profile。callback 不能被保存、跨请求持有、放进容器或延迟到 sibling lane 调用。

callback 捕获外层变量时，其读写集合并入承载 API 调用的 lane。跨 lane 冲突检查按普通访问路径规则处理。

## 12. Function Effect Metadata

函数 effect metadata 至少描述 Skiff 可见 read / write path、external effect target 和 conflict-key、返回值
provenance、boundary分析需要的throw payload provenance、callback profile、stream 生产 / 消费行为和
推断的 suspension summary。
它不包含对调用方公开的“可能抛出类型集合”；函数签名、Package ABI和ServiceContract都不声明或推导
operation-specific throw set。

跨模块调用使用被 import 模块发布的 metadata；resolver 不重新解析依赖模块实现来猜 effect。

递归和 mutual recursion 使用固定点推导。无法证明返回 root provenance 时，返回值视为 `opaque`。

返回 provenance 同时记录两层事实：`returnOrigins` 是从返回值可达的全部 origin（包含
返回值自身），`directReturnOrigins` 只表示返回值自身可能是哪一个 root。fresh 容器的
直接 root 与其 payload 中可达的 caller root 因而不会混为一谈。payload 包含 caller
值不等于容器本身是 caller alias；但控制流合并后若 direct root 可能同时为 fresh 和
caller-owned，write / identity 分析必须保留 caller 候选并保守拒绝不安全边界。

caller 参数的字段或容器元素投影必须保留为“参数序号 + 结构化 selector path”。path
只允许稳定字段名和 `containerElement`，不得使用源码位置、表达式遍历序号或 callable
名称。当前 heap 分析对字段内容是 field-insensitive 的：读取字段时只把接收者 root 的
一层直接 payload 作为该字段的保守 direct 候选，不得把任意深度可达 root 都提升为字段
自身，否则会制造不存在的 cycle。未知、非法或超过实现上限的 path 必须 fail closed。

`returnOrigins`、`directReturnOrigins` 和结构化 path 都必须跨 Package artifact
序列化，并参与 package build identity。它们是实现语义事实，不进入 Local ABI 或
service protocol identity。`maySuspend` 是下节规定的例外：它会被提升为 concrete public Package
callable 的 Local ABI summary，但仍不进入 interface requirement 或 service protocol。

`opaque` mutable root 不能在 `concurrent` sibling lane 中参与 mutation；若编译器无法证明 lane-local，必须报错。

metadata 改变不改变 service protocol identity，但会改变 code revision、编译缓存和并发诊断结果；
concrete public Package callable 的 `maySuspend` 改变还会改变其 Package Local ABI。

### 12.1 Inferred Suspension Summary

语言不提供 `async`、`suspending`、effect declaration 或显式 `yield` 关键字。每个有函数体的 concrete
executable 都由 compiler 对函数体、调用图和内建等待点执行 sound may-analysis，得到
`maySuspend`。递归与 mutual recursion 按固定点收敛；无法证明不会挂起的调用必须保守为
`maySuspend=true`。

调用传播规则是：

- 同一 Package 内静态解析到 concrete function / method 时使用该 executable 的固定点 summary。
- package dependency 的 concrete public callable 使用依赖 PackageArtifact 发布的精确 summary；
  compiler 不读取依赖源码重新推断。
- 已知 concrete 或 package-direct public-instance binding 使用其 concrete public callable summary。
- `any I`、未知 interface dispatch 或其它无法确定 concrete target 的调用保守视为可能挂起。
- service dependency call 无条件视为可能挂起，因为 service call 自身就是调用方的等待点；不得从
  ServiceContract 读取或推测 callee implementation 的内部 summary。
- compiler-known native / builtin 等 target 使用其 canonical callable summary；未知 target仍
  fail closed 为可能挂起。

interface method requirement不包含`maySuspend`，interface conformance也不比较该位。同一 requirement
可以同时有 suspending 和 non-suspending implementation；这不会制造一个共享的 requirement summary。
若 Host/runtime 需要 concrete callee summary选择内部执行、取消或调度机制，只能从
PackageArtifact / deployment implementation metadata取得，不能写入interface或ServiceContract identity。

concrete public Package callable 的 summary 是 Package Local ABI fact。从 non-suspending 变为
suspending（或反向）保持 stable `PackageCallableId`，但改变 Local ABI与Package build，并要求直接
package依赖方重编译。interface requirement / conformance不因此改变；service operation的请求、响应、
stream/callback形状和开放错误通道不变时，callee内部summary改变也不改变stable
`ContractOperationId`或`ServiceProtocolIdentity`。

`maySuspend=true` 只表示执行可能到达真实等待，不表示 runtime 在函数入口、任意指令之间或 interface
dispatch处主动切换。stream next、service call、timer等操作只有在执行到该点且实际需要等待时才释放
actor执行权；保守分析本身不产生调度点。

## 13. Recursive Types

用户自定义 `type Name = Type` 不允许直接或间接递归。用户自定义 alias 当前也非递归。

用户自定义 recursive record 当前暂不进入服务边界 schema，直到 runtime projection 发布可解析 schema definitions graph。

未来开放 recursive record 时，cycle 必须经过 guard，例如 nullable 或 collection 容器；无 guard 的无限展开仍非法。

`Json` / `JsonObject` 的递归来自 compiler-known prelude descriptor，不是普通 alias 例外。

## 14. Service API Static Boundary

Service API roots只来自`service.yml.serviceCalls`按public path显式选择的Package API callable roots。

ServiceContract projection先要求每个被选择root对应的`BoundaryCallableProjection`为`Available`，再派生
operations；被选择但Unavailable必须报出完整结构化原因，不能静默排除。未选择callable只是Package
API，即使技术上boundary-available也不会成为service operation。普通const不直接成为service operation；
被选择的public instance可作为receiver root，由其显式listed interface methods派生operations。public
interface仍只是conformance contract，不直接成为service operation。`serviceCalls`只引用`api.yml`
public path，不重复source selector或signature；重复、unknown或non-callable path均fail closed。

HTTP、WebSocket等external ingress不属于service-call projection。它们分别由`http.yml`和
`websocket.yml`选择当前Package中的handler/pre/guard；这些callable不要求public，也不生成service
operation。Compiler按其linked signature与gateway adapter source检查参数/返回，生成独立gateway entry
identity。Ingress不得使用或伪造`ContractOperationId`。

source module path是组织方式，不是协议身份。service protocol identity由显式service-call public path、
operation name、canonical signature、public instance receiver root metadata、schema closure和
cross-service dependency identity计算。

API operation signature 中的用户自定义类型必须来自当前 source set 的 public API graph 或 schema closure、
其他服务 / package 发布的 public schema，或 lang / platform `std` / package schema 中标记为
schema-stable 的类型。HTTP schema-stable platform types 写作 `std.http.HttpRequest`、
`std.http.HttpResponse`、`std.http.HttpClientRequest`、`std.http.HttpResponseStreamEvent` 等模块路径名。

上述类型都由声明它们的Package拥有。service operation签名保留owner Package的`PackageSchemaTypeId`；
service id、service version label和provider implementation build不能重新定义类型identity。跨service导入只把
ServiceContract operation与其精确Package schema requirements materialize为同一Package类型视图，不生成
service-owned nominal type。

未进入 public API graph 的 declarations 可在内部通过 `root.*` 使用，但第一版不能进入ABI/schema
boundary closure。Public operation或public type字段引用到的named type必须也在owner Package的
`api.yml`中显式公开；否则projection fail closed。compiler不得从内部模块路径、文件路径、遍历序号或
发现路径为它生成隐藏的boundary identity。

service-call callable必须按source identity解析，不能按短名或字符串后缀匹配。同一public path下
derived operation name重复是compile error。

第一版不允许generic service-call method或static service-call method。runtime-owned receiver /
handler fields是构造依赖，不是request payload。

`api.yml`中的public generic declaration可以供package linkage使用，但其声明本身没有第一版
PackageSchema投影。未被选择的generic public symbol不能让整个Package失败；若其public path被
`service.yml.serviceCalls`选择，其不可投影原因必须作为结构化compile error报告，不能静默排除。External ingress的
handler第一版不能是generic function declaration；其concrete signature仍可包含fully instantiated的
generic platform types，由linked signature和专用adapter处理，这些类型不因此获得ordinary service-call
schema。

跨 service 调用必须静态解析到已声明 callee API，并绑定发布时记录的 exact protocol identity。业务代码不通过字符串 service id 或 service locator 发起远程调用。

## 15. Static Stream Boundary

`Stream<T>` 可以作为service operation，或adapter kind明确允许stream的external gateway entry返回类型。
Service operation的chunk类型`T`必须通过Package schema closure。当前HTTP ingress中，`typedJson`
handler必须是unary并拒绝任意`Stream<T>`返回；只有`rawHttp`允许精确返回
`Stream<std.http.HttpResponseStreamEvent>`，其event按linked handler signature与raw HTTP adapter验证，
不要求仅为ingress进入PackageSchema。

显式 stream-producing native std / package API 也可返回 `Stream<T>`，作为 request-local external source handle。平台 std 也可以把 `Stream<T>` 放在 runtime-owned handle record 字段里，例如 `std.http.HttpClientStreamHandle.body`；这类 handle 仍是 request-local 值，不是可持久化 schema。除非调用方把 chunk `emit` 到服务边界，或写入其他边界 payload，否则该 `T` 不因 handle 本身进入 boundary closure。

普通 Skiff package / local 函数不能通过源码 body 创建本地 `Stream<T>`。它们可以在同一 request 内返回或
转发从service operation、external gateway entry或特权source API获得的`Stream<T>` handle；这是
request-local pass-through，不创建新的 stream sink，也不能跨请求持有。当前只有 server / source stream，
不支持 bidirectional stream、stream 参数、半关闭、resume 或 cursor。

`Stream<T>` 不能作为用户 operation 参数、用户 record 字段、持久化字段、collection 元素或普通 public API type 字段。平台 std 的 runtime-owned handle 字段和 native host operation 参数是特权例外；例如 `std.file.createFromStream(source: Stream<bytes>, ...)` 只在同一 request 内消费 source，不把 stream 作为远程 API 或持久化值暴露。

返回 `Stream<T>` 的 service operation 是 server-stream producer。函数体内 `emit expr` 要求 `expr` 可赋给 `T`，并写入当前 stream sink。

`Stream<T>` operation 不需要 `return expr`。自然结束或裸 `return` 表示 normal end；带值 return 是编译错误。

helper 若使用 `emit`，其 effect metadata 必须标记 `emits T`，并且只能在兼容当前 stream sink 的上下文调用。

`emit` 不能出现在普通 unary request、顶层 const 初始化、escaping callback、后台清理路径或 request 结束后。`emit` 不允许出现在 `concurrent` surface 内，包括 sibling lane、`serial` lane 和 `concurrent value` tail lane。

消费 stream 的 `for` loop 是一次性顺序消费。break、return、外层 timeout 或 ancestor cancel 必须向 source 传播 cancel；消费后不能再次迭代或复制到多个 lane。

## 16. Boundary Schema Closure

必须通过 schema closure 的位置包括 service API operation 参数和返回、public API closure 中 public
type 字段图、跨服务 payload、跨请求 / 入库 / 落盘 payload，以及平台 schema 标记的边界 payload。

schema closure由逐类型`PackageSchemaTypeRecord`闭合。每个命名root以`PackageSchemaTypeId`引用owner
Package中的canonical descriptor；descriptor引用的每个named child也必须是其owner Package的显式public type。
ServiceContract只记录实际可达的package/type ids，不记录整包schema index identity，也不复制字段定义。
缺type record、owner/key/identity不匹配或依赖闭包不完整都不是“结构相同即可”的兼容情形，必须fail closed。

第一版schema named-type依赖图必须无环；自递归和相互递归类型都不closed。projection先拒绝SCC，再按拓扑序
计算逐类型identity。descriptor中的named child引用使用child的package id、stable schema key和已计算
`PackageSchemaTypeId`，不能引用待计算的自身identity。

模块内部可以使用 interface conformance test、本地 `any I` 能力值和 public instance receiver
root；第一版不把裸 interface 当作普通 runtime value。package 抽象能力通过显式 `any I` 参数传递，
不再有旧式 manifest receiver root。anonymous union 和进程内类型可以作为内部临时类型，
只要不进入上述边界。

`SchemaClosed(T)` 对 primitive、`integer`、string literal、`null`、所有分支 closed 且可枚举的 union、元素 closed 的 `Array<T>` 成立。

`Map<K,V>` 要求 `V` closed，且边界 key 类型当前只允许精确 `string` 或单一名义 representation over string。key identity 不能退化成普通 string。

名义 record 要求类型本身可见且字段 closed。用户自定义递归 record 当前不 closed。

名义 representation `N = R` 要求 `R` closed 且不含裸 interface、callback 或 unknown。

命名 union 要求所有 branch closed。anonymous discriminator record branch 只允许出现在带 discriminator 的命名 union 内。

prelude schema-stable 类型如 `Json`、`JsonObject` 在其参数 closed 时 closed。`Exception<E>` 和 `CatchResult<T,E>` 明确不是 boundary schema 类型。

不 closed 的类型包括裸 interface、`unknown`、`FnType` / callback、request-local control flow 类型、边界根位置的 anonymous record、无法枚举 branch 的 union、不可被边界 schema 解析到的未导出名义类型。

## 17. Wire Identity Requirements

schema closure 不规定具体编码，但要求接收方能按 expected schema 无歧义恢复静态类型需要的名义身份。

字段 schema 已唯一指定 concrete representation 时，可以只传 payload。discriminator record union 使用声明字段做 discriminator；concrete nominal union、representation union 或同 payload 形状的 branch 必须携带 type id、branch id 或 schema discriminator。

anonymous discriminator branch 的 identity 是 enclosing named union 加 discriminator 字段值。

裸 `Json` / `JsonObject` 只保留 JSON 数据形状，不携带用户 representation、union 或 map-key identity。需要保留身份时应使用 typed schema decode，而不是把值降入裸 JSON。

## 18. Boundary Policy And Recoverable Values

边界 closure 不再只有“能否编码成 public schema”一个问题。compiler 必须先识别边界种类，再验证该边界对应的
policy：

- ordinary service/public API schema：必须 schema-closed，不允许 `any I` 默认 wire shape，也不隐式调用
  recoverable codec。
- DB stored field、`spawn` target 参数、queue / persistent work item payload、runtime 内部跨 request payload：
  是 owner-internal recoverable boundary，底线是“值必须可恢复”。详见
  [`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。
- service/public API 的显式 recoverable slot：只有 ABI/schema 明确标记时才调用 recoverable codec；第一版离开 owner
  service trust domain 时只允许 plain data envelope，行为节点 fail closed。
- package public 入口不是跨进程边界。`any I` 可在同 runtime request 内流动；若该值再进入 DB/spawn/queue/persistent
  payload，才按 recoverable boundary 检查。

Recoverable boundary 的静态检查对象是 recoverable state plan，而不是所有 runtime raw field 的图。compiler 必须拒绝
明显不可恢复的 request-local 类型，例如 callback/function value、`Stream`、`Exception`、transaction handle、live
connection、file descriptor 或没有 durable adapter 的 native handle。`any I` 在 owner-internal recoverable boundary
中静态允许，但 artifact 必须标记 runtime carrier/self recoverability check：`carrier = Local` 且 self payload 全可恢复时
可进入边界；`carrier = Remote` 是 request-scope 正向远程引用，进入 recoverable boundary 时 fail closed。

### 18.1 Recoverable Compatibility Contract

第一版 recoverable decode 采用精确身份匹配。没有列在这里的兼容都不存在，不能靠字段名相似、顺序相近、runtime
duck typing 或宽松 coercion 放行。

- plain data：`value_kind`、nullable shape、primitive canonical domain 和 expected type plan 必须匹配；不做
  string/number/bool 之间的 coercion。
- record/default nominal fields：field identity 必须逐项匹配写入时 concrete restore plan。field rename、删除、
  拆分或合并属于 DB schema migration 或 custom restore version，不由 recoverable decode 自动处理。
- nominal expected type：恢复出的 concrete type identity 必须等于当前 expected nominal identity；expected type 是
  interface 或 union 时分别进入 interface / union 规则。
- interface expected type：interface identity 和 projection identity 来自当前 expected type plan；`InterfaceValueState`
  不保存这两项。payload 中的 stable `LocalConcreteRestoreKey` 必须能在当前 linked program 中唯一定位 concrete type，
  且当前 metadata 必须证明该 concrete type 实现 exact interface/projection。
- union expected type：envelope 中的 `union_identity` 和 `branch_identity` 必须与当前 expected union/branch 精确匹配。
  没有 branch identity 时，只允许 compiler 证明 payload shape 在当前 union 中唯一。
- custom restore / native adapter：custom restore 由当前 `LocalConcrete` restore entry 定位，runtime wrapper 不保存
  `restore_schema_version`；native adapter 仍按 `adapter_identity`、`adapter_schema_version` 和 `native_type_identity` 精确定位。
  第一版默认不做 schema migration，除非 concrete type 或 adapter metadata 显式声明可接受版本。

示例：

```text
accept: stored LocalConcrete restore key C still maps to a current concrete type that implements exact current interface I and exact method projection P.
fail: field `displayName` was renamed to `name`; recoverable decode does not infer rename.
fail: union branch `Pending` was renamed to `Queued`; branch identity mismatch.
fail: interface method projection changed from P1 to P2, even if source method names look similar.
fail: string "1" is not coerced to number 1 for a plain data expected number.
```

Fail closed 必须发生在 decode 边界，错误类别至少能区分 expected type mismatch、state invalid、artifact unavailable、
native adapter missing、interface conformance missing、remote carrier not persistable、cross-service behavior unavailable 和
untrusted behavior payload。
