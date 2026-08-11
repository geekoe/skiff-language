# Skiff Syntax Reference

本文负责：稳定描述 Skiff 源码如何被词法分析、分行、解析成 AST；覆盖 source、import、声明、类型、语句、表达式、pattern 和消歧规则。

本文不负责：名字解析、类型兼容、schema closure、service protocol identity、runtime 调度、取消、effect、标准库 API 语义和测试替身行为。

## 1. Lexical Surface

Skiff 当前不使用分号。简单语句由逻辑行结束；在括号、方括号、大括号、参数列表、类型参数、object / array literal、pattern、block 等结构未闭合时，换行不结束语句。

行尾为二元运算符、`.`、`,`、`=>`、`|`、`(`、`[`、`{`、`<` 等明显需要继续读取的 token 时，下一行继续当前语法结构。

行注释以 `//` 开始，到行尾结束。块注释以 `/*` 和 `*/` 包围，不支持嵌套。注释通常等价于空白，但不会把未闭合结构强制结束。

普通标识符只使用 ASCII：首字符为 `_` 或 ASCII 字母，后续字符可含 `_`、ASCII 字母和 ASCII 数字。关键字不能作为普通标识符。

关键字包括声明、控制流、并发、导入导出和基础 literal 相关单词：`function`、`fn`、`native`、`static`、`type`、`alias`、`interface`、`impl`、`const`、`final`、`var`、`inout`、`if`、`else`、`match`、`value`、`for`、`while`、`return`、`break`、`continue`、`throw`、`rethrow`、`catch`、`with`、`emit`、`concurrent`、`serial`、`timeout`、`import`、`export`、`implements`、`in`、`as`、`true`、`false`、`null`、`Self`。

基础类型名、prelude 核心类型名和内建 value root 是保留名；它们不能被声明、import alias 或局部绑定 shadow。关键 root 包括 `std`、`root` 和 `config`。

字符串 literal 使用双引号，支持常见转义和 `\uXXXX`。当前没有多行字符串语法。

数字 literal 产生统一的 `number` 类型；整数位置由语义规则检查。`integer` 是 safe integer refinement，不是独立 literal 种类。

duration literal 是单个 token，由正整数和单位 `ms`、`s`、`m`、`h`、`d` 组成，只能出现在 `timeout(...)` 及平台 schema 明确允许的位置。

## 2. Source File Shape

源文件结构是 import 声明序列，后接顶层声明序列。`import` 必须位于所有非 import 顶层声明之前。

普通顶层声明不带 `export` 修饰。源码层 public API 不通过 `.skiff` re-export 声明表达；public API
是 source-layer metadata，不是 Skiff source file 语法的一部分。

顶层 item 包括 `function`、`type`、`alias`、`interface`、`impl` 和 `const`。`const` 只是
顶层声明；`final` 与 `var` 只允许出现在 block 内，局部 `const` 不是语法。三者都必须在
声明处提供 initializer：

```ebnf
TopLevelConstDecl = "const" Identifier (":" Type)? "=" Expr
LocalBindingDecl = ("final" | "var") Identifier (":" Type)? "=" Expr
```

`let` 已移除，不再作为局部 binding 语法；解析器只保留明确的迁移错误。

三个名称的职责不重叠：

- 顶层 `const` 表示 compiler-evaluated、request-independent、deeply frozen 值；
- 局部 `final` 表示运行时不可重绑且不可通过该 binding 修改的值；
- 局部 `var` 表示可重绑、可作为 writable access-path root 的值。

普通裸 block 不是 statement。block 只出现在函数体、控制流、`with`、`timeout`、`concurrent`、`serial`、`value` 等语法结构要求的位置。

## 3. Import And Export

源码 import 只引入外部 package 的本地 binding，不引入 module、symbol 或当前 service internal module。

import 语法只接受 simple local package name，例如 `import std`、`import billing`、`import llm`。复杂 package id、版本和本地 alias 在 manifest / package 配置中声明，源码只 import alias。`std` 是内建平台 root；`import std` 只是显式风格保留，不是使用 `std.*` 的前提。

`std` 不是普通 package dependency，也不能通过 manifest 声明或改名。`import std.http`、`import skiff.run/llm`、`import std as standard`、wildcard import 和 grouped import 都不属于当前语法。

当前 package / service implementation 自身不通过 import 引入。跨文件访问当前 source set 使用内建 `root.<dotted-module-path>.<Symbol>`，其中 dotted module path 由相对源码路径去掉 `.skiff` 后转换得到。

`root.*` 是当前 Package内部的绝对限定名，不是import语法，也不表示外部API。source file是模块命名和
代码组织单位，不是Package内privacy边界；顶层声明的外部可见性只由`api.yml`的Package API graph决定。

import local binding 不能与保留 root、prelude 名、本文件顶层声明或局部绑定冲突。`config` 是内建 value root，不通过 import 引入。

public API metadata 不是 package import。它不能从 `.skiff` 源码引用，不参与 `root.*` module
namespace，也不生成 wrapper 或普通 type alias。

## 4. Declaration Syntax

函数声明由可选 `native` / `static` 修饰、函数头和函数体组成。普通函数有 block body；`native` 函数可以无 body。

函数头包含 `function`、名称、可选类型参数、参数列表和返回类型。参数必须写类型。普通参数
写作 `name: Type`；显式 write-through 参数写作 `inout name: Type`。`inout` 是参数 mode，
不是类型修饰符；它的 Package-local exact-target 限制由 static semantics §12.2 检查。
类型参数写在 `<...>` 中，允许尾随逗号。

`static function` 是 type namespace 成员，不绑定 `self`。普通 receiver method 通过 impl block 引入，并通过 method call 调用。

`type Name = Type` 创建名义 representation 或命名 union。它不是透明 alias。当前用户源码中的 `type Name = Type` 不允许包含 `FnType`。

record 形态 `type Name { fields }` 创建名义 record。只有 record 形态可以带 `implements` conformance list。

`type Name discriminator "field" = ...` 只用于命名 union 的 anonymous record branch 判别字段。判别字段细节由 static semantics 检查。

`alias Name = Type` 是透明类型缩写。当前 alias 无类型参数，非递归，使用点不能写 type args。

`interface Name<TypeParams?> { ... }` 声明名义能力契约。目标态 interface body 只包含 method
requirement；空 body 可作为 marker interface。interface 的 conformance、`Self` 和 boundary 规则见
`interface.md`。

`Self` 的 receiver 规则由 interface reference 定义。语法层只保留 `Self` 作为 type primary。

`impl Target { function ... }` 给 receiver type 的 method namespace 增加 method。target 可以是
qualified type name 和可选 type args。interface default / extension method 不是第一版目标态。

`impl` method 不是 top-level symbol，第一版不能直接作为 public API source target。需要公开 receiver
能力时，通过公开 receiver type、public instance 或后续单独设计的 method API机制表达。

## 5. Type Syntax

类型表达式的顶层是 union。`A | B | C` 表示 union type；语法结合性不影响语义集合。

`T?` 等价于 `T | null`，且 `?` 绑定强于 `|`。

primary type 包括 qualified name、可选 type args、`Self`、`Array<T>`、`Map<K,V>`、record type、`fn(...) -> T`、string literal type 和括号类型。

当前只有 string literal type。没有 number literal type 或 bool literal type。

record type 写作 `{ field: Type, ... }`，主要作为类型表达式。边界位置是否允许 anonymous record 由 static semantics 检查。

`FnType` 是语法存在但受限的类型形态，只能出现在标准 API 或 package metadata 明确允许 callback 的签名元数据中。

`Self` 只用于 interface method requirement 的 receiver 参数；其他上下文是语义错误。

## 6. Statement Syntax

block 是 `{ Stmt* }`。statement 包括声明、赋值、控制流、stream 输出、错误控制、resource / timeout / concurrent 结构和受限 expression statement。

赋值是 statement，不是 expression。左侧 place 由名字、member 和 index 组成；语法不把任意
place 当作可写。其语法形状为：

```ebnf
AssignmentStmt = Place "=" Expr
Place = Identifier (MemberSuffix | IndexSuffix)*
MemberSuffix = "." Identifier
IndexSuffix = "[" Expr "]"
```

Static semantics 只接受从局部 `var`、当前有效的 `inout` loan，或 Actor method
中允许直接写入的 `self.field` 派生的精确 writable path；Actor DB transaction body 还有额外禁写规则。

`if` 只作为 statement，不作为 expression。需要产值时使用 ternary、`match` expression 或
`value` block。

`for name in expr { ... }`、`for key, value in expr { ... }` 和 `while expr { ... }` 是循环语句。`break` / `continue` 只在循环内合法。

`for` 绑定语法是：

```ebnf
ForStmt = "for" Identifier ("," Identifier)? "in" Expr Block
```

`while` 循环语法是：

```ebnf
WhileStmt = "while" Expr Block
```

单绑定形态可用于 `Array<T>`、`Stream<T>` 和 `Map<K,V>`；在 map 上绑定 key。双绑定形态只用于 `Map<K,V>`，第一个名字绑定 key，第二个名字绑定 value。`for (key, value) in map`、`for key: value in map` 和 `for key => value in map` 不属于当前语法。

`return` 可带表达式，也可裸返回。`throw` / `rethrow` 同时有 statement 和 expression 形态；expression 形态类型为 `never`。

`emit expr` 是 statement，只能在存在当前 stream sink 的上下文中使用；sink 建立、元素类型和 boundary 行为不在语法层定义。

`with expr as name { ... }` 引入 scoped binding。资源生命周期由语义和 runtime 定义。

`timeout(200ms) { ... }` 是 statement，不产值。产值形态写作 `timeout(200ms) value { ... }`。

`concurrent { ... }` 是 statement。`serial { ... }` 在语法上是 statement，但只能作为 `concurrent` block 的直属子语句。

expression statement 的最外层表达式必须是普通函数调用、method call、std API call、service call、IIFE call 或 dispatch 表达式。裸 literal、name、object / array literal、`match` expression 和 `value` expression 不能作为独立 statement。

## 7. Expression Syntax

表达式优先级从高到低：call / member / index / nominal construct，unary `!` / `-`，乘除模，加减，关系比较，相等比较，`&&`，`||`，ternary。

同一优先级的普通二元运算符左结合。ternary 单独按下文规则右结合。

关系比较和相等比较不允许链式写法。`a < b < c` 应写成显式布尔组合。

postfix 表达式由 primary 加任意数量的 member、index 和 call suffix 组成。index suffix 的
canonical source spelling 是 `object[index]`；方括号内是一个完整 expression，后缀可以链式组合：

```ebnf
PostfixExpr = PrimaryExpr PostfixSuffix*
PostfixSuffix = MemberSuffix | IndexSuffix | CallSuffix
IndexSuffix = "[" Expr "]"
InOutArg = "inout" Place
```

call suffix 可带 type args。哪些 receiver/selector 类型支持 index，以及 read、assignment 和
`inout` path 的求值顺序与失败语义，由 `static-semantics.md` 和 `runtime.md` 定义，不由 parser 猜测。

普通 call argument 是 expression。与 `inout` 参数对应的 argument 必须显式写作 `inout place`，
`place` 只能是从局部 `var` 派生的 name/member/index access path；`inout` argument 不是可单独
使用的 expression，Actor `self.field` 也不因本身可写而成为合法 `inout` argument。

generic call 只有在 postfix 后出现可成功解析的 `<...>` 且随后直接进入 `(` 时成立。`>` 和 `(` 必须在同一逻辑行，中间只允许普通空白或注释。

primary expression 包括 literal、name、object literal、array literal、match expression、value expression、catch expression、throw / rethrow expression、anonymous function、nominal construct 和括号表达式。

`cond ? thenExpr : elseExpr` 是 ternary 表达式，优先级低于 `||`，右结合；只求值选中的分支，
两分支必须存在可共同赋值的类型（`A | B` 不作为结果类型，除非两分支本身是联合类型）。
statement header slot 中若 ternary 分支以构造形态表达式结尾，整个 ternary 需要加括号（见 §9）。

object literal 是 target-typed。语法允许 `{ name: expr }` 和 `{ [expr]: expr }` entry；它最终解释为 record literal、map literal 或 json literal由目标类型决定。`{ "name": value }` 不是合法 entry。

record literal 中字段名是编译期字段名。map / json literal 中裸 `Name` entry 表示同名 string key，computed key 表达式必须是 `string`。

array literal 写作 `[expr, ...]`。空数组需要目标类型上下文；mixed element 的 union 推断或报错由类型规则决定。

nominal construct 写作 `QualifiedName TypeArgs? { fields }`，不依赖名称大小写。expression slot 中
`Path { ... }` 总是按 nominal construct 解析，类型名是否成立由语义阶段验证；statement header slot
是例外，见 §9。

anonymous function 写作 `fn(params) -> Type { ... }`。语法允许它作为 primary，但语义只允许 IIFE callee 或白名单 callback 参数位置。

`value { ... tailExpr }` 是表达式 block，必须有 tail expression。`concurrent value`、`timeout(...) value` 和 `timeout(...) concurrent value` 是当前 canonical modifier 顺序。

`value` block 内禁止 `return`、`break` 和 `continue`。tail expression 如果是 object literal，必须加括号以避免和 block 混淆。

`catch<E>(expr)` 捕获 expression。`catch<E> value { ... }` 是简写形态。若要捕获带 modifier 的 value expression，使用括号形态。

`match expr { pattern => body }` 在 expression slot 中是 expression；复杂分支应使用 `value { ... }`。表达式 match 的分支体不能使用普通 statement block。

## 8. Match Statement And Patterns

statement slot 中以 `match` 开头解析为 match statement；每个 arm 的 body 必须是 block，整体不产值。

expression slot 中以 `match` 开头解析为 match expression；每个 arm 的 body 是 expression 或 value expression，整体产值。

pattern 包括 literal pattern、wildcard `_`、nominal pattern、record pattern、or-pattern 和裸 name binding。

裸 `Name` pattern 永远是 binding，不表示引用外层变量，也不表示类型测试。类型或 interface 测试必须写 `TypeName {}` 或 `TypeName { ... }`。

nominal pattern 写作 `QualifiedName TypeArgs? RecordPattern`，record pattern 即使为空也要写 `{}`。

record pattern 只列出需要匹配或绑定的字段。字段形态可以是 `name: pattern` 或短写 `name`。

or-pattern 用 `|` 连接 primary pattern。所有分支必须绑定同一组变量名；一致性由 static semantics 检查。

literal pattern 可包含 string、number、bool 和 `null`。对 representation scrutinee 的 payload 匹配是语义规则，不改变 pattern 语法。

## 9. Disambiguation

statement slot 和 expression slot 是最重要的消歧边界。`match`、object literal 和 nominal construct 的解析都依赖所在 slot。

statement header slot（`if`、`while`、`for ... in`、`match` 的 scrutinee，以及 db `where if`
的条件）是第三种边界：头部表达式后的 `{` 一律是 body 或 arms，不按 nominal construct 消费。
头部内需要 nominal construct 或 patch construct 时用括号包裹：`if (Point { x: 1 }) { ... }`。
嵌套 expression slot（调用参数、括号内、字段值、value block 内等）不受此限制。

在 statement slot 中，裸 `{ ... }` 是语法错误；在 expression slot 中，`{ ... }` 是 object literal，随后由目标类型解释。

表达式 `match` 分支若要返回 object literal，写作 `=> ({ ... })`。`value` block tail expression 若要返回 object literal，也写作 `({ ... })`。

`QualifiedName TypeArgs? { ... }` 在 expression slot 中解析为 nominal construct（名称大小写不参与
消歧；是否指向 record 类型由语义验证）。statement slot 中的 `Name { ... }` 不作为 statement。

generic call 和比较表达式按“`<...>` 后是否直接进入 call”消歧。不能把 `a < b > (c)` 误解析为泛型调用。

`throw` / `rethrow` 在 statement slot 是 statement，在 expression slot 是 expression。二者的类型、可捕获性和错误 payload 由 static semantics 定义。

`serial { ... }` 即使在语法上是 statement，也不通过 parse 阶段判断其父节点是否合法；该限制留给 semantic pass。

## 10. Syntax-Only Boundaries

object literal、array literal、nominal construct、match arm body 和 catch type 都依赖 target typing 或 name resolution；parse 阶段不尝试决定它们的最终类型。

import 只建立 local package binding token，不解析 package artifact、版本或导出符号。

`api.yml`定义源码层public path；最终service operation、Package Local ABI和protocol identity仍由
Package/service projection、schema closure和linkage policy决定。

concurrent、timeout、stream、with 和 callback 的调度、取消、effect、资源释放和 non-escaping 行为都不是语法规则。

schema closure、wire identity、platform error mapping 和测试替身匹配不属于 syntax reference。
