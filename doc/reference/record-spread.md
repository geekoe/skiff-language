# Skiff Record Spread Reference

> 状态：语言设计输入（未实现）。日期：2026-08-05。
> `spread` 语法本身尚未在 compiler / runtime 实现；本节描述目标语义。跨包类型引用的斜杠拼写
> （`alias/module.Symbol`）已实现（见 §2）。

本文负责：稳定描述 record 声明中的 `spread` 字段复制特性——语法、编译期展开语义、快照与漂移行为、与 db object / interface / impl 的交互边界和错误清单。

本文不负责：db object 存储契约（契约声明、宿主实现、链接/激活期覆盖校验）由 `db.md §1.3` 定义；一般类型兼容性规则的变更；跨包类型可见性规则（由现有 boundary / api.yml 规则决定）。

## 1. 设计目标与原则

`spread` 用于在 record 声明中把另一个 record 类型的字段按原名、原类型复制进当前字段集。它解决**声明复用**：产品侧扩展引擎类型字段时，不需要逐字段重写引擎字段。

三个不可变原则：

- **无类型关系**：展开后的类型是独立名义 record，与源类型之间不存在子类型、赋值、传参或字段兼容关系（§3.4）。
- **只复制字段**：不复制 `impl` 方法、interface conformance、type namespace 成员；展开后的类型按普通 record 使用 nominal construct 构造（§5）。
- **快照语义**：展开在编译期一次性完成，源类型后续修改不传播；一致性靠显式校验（§4）。

## 2. 语法

`spread` 是 record 字段列表中的关键字条目：

```skiff
type Thread {
  spread agent/model.AgentThread,
  ownerUserId: string,
  pinnedAt: string?,
  agentId: string?,
  createCommandId: string,
}
```

同包最小示例：

```skiff
type Base {
  id: string,
  title: string?,
}

type Thread {
  spread Base,
  pinnedAt: string?,
}
```

规则：

- `spread` 后跟单个 qualified type name。字段列表与普通 record 声明一致：字段与 `spread` 条目统一按**逗号**分隔（`parse_field_block` 的既有规则），条目之间逗号必需。
- 跨包源类型引用与其它 package symbol 引用一致：**斜杠是命名空间分隔符**，`alias/module.Symbol`
  与点形式 `alias.module.Symbol` 同义（斜杠在解析期归一化为 dependency alias + public path，不是
  字段访问语法）；同包跨 module 用 `root.<module>.<Symbol>`。`root.*` 只解析当前 source set，
  不穿透 dependency 的 private 符号。
- `spread` 条目可以出现在字段列表的任意位置；一个 record 允许任意多个 `spread` 条目。
- 多个 `spread` 条目与显式字段之间按**字段名**合并：重名即 compile error（见 §8），不存在覆盖或优先级。
- `spread` 只在 record 形态 `type Name { ... }` 的字段列表内是 contextual keyword；**`spread` 后跟 `:` 时按普通字段名解析**（与 db block 内 `where` 等 contextual keyword 的处理先例一致）。type 表达式和其余上下文不受影响。
- record type（anonymous `{ field: Type }`）不参与 spread 声明；spread 是声明级特性。

## 3. 展开语义

### 3.1 展开时机与产物

展开的权威位置是 source 语义层（AST 到类型解析之间）。语义层中消费 record 字段集的机制——db 附着校验
（`db object` 附着、primary key 必须在字段上、storage mapping 声明）、类型解析的 record field facts、
表达式字段访问与 record literal target typing——一律以**展开后的字段集**为准。展开后进入 File IR 的
是普通具体 record，IR 中不存在 spread 节点；后续机制（type plan、构造、pattern、schema closure、
encode / decode）按现有路径工作，零运行时成本。

### 3.2 源类型的合法形态

- 源解析后必须是 record 形态：名义 record 直接合法；透明 alias 展开为 record 的合法；representation、命名 union、interface 不合法。
- 泛型源：允许 fully instantiated 的泛型 record（`spread agent/model.AgentThread<agent/model.Metadata>`，实参闭合、不引用目标类型参数）。**第一版不允许字段集依赖未绑定类型参数的源**。
- 源类型必须对该模块可见。字段本身的可见性沿用现有跨包类型引用规则，spread 不新增或放宽可见性。
- 空字段集：源类型展开后字段集为空（零字段 record，或 spread 链展开后字段集为空）时，展开结果为零字段 record；该 record 仍合法，但不能附着 db object（无字段可作 primary key）。

### 3.3 字段复制

- 复制字段名与字段类型。字段类型在**源类型的声明上下文**中解析（与源类型内部分辨一致）。
- 泛型源必须先在字段类型上按实参替换源 type_params，再复制进目标字段集；不得把引用源 type_params 的游离 `TypeParam` 带入目标类型。
- 复制不改变字段语义：可空性、默认形态、recoverable 行为等字段级事实随类型表达式保留；但 db object 的 storage mapping（如 `storage ... using encrypted`）不在 type 上，不随 spread 传播（见 §6）。
- 目标 record 自身的字段与复制字段共同构成最终字段集。字段顺序不在语言语义内（IR 按字段名排序），展开顺序只影响字段名冲突检测的诊断顺序。

### 3.4 展开后的类型

- 目标是普通名义 record，拥有独立 type id。
- 与源类型**没有任何兼容关系**：不能互相赋值、传参、用作记录字段 target typing 或 pattern 匹配替代。
- 展开后的类型可以执行普通 record 的一切操作：声明 `implements`、附着 db object、作为泛型实参、被其他 spread 引用。

## 4. 快照与漂移

spread 复制是编译期一次性快照：

- 源类型所在 package 后续发布新版本并修改字段时，已展开的类型不自动跟随。
- 如果两个类型之间的字段一致性是某种契约（例如存储契约要求产品类型覆盖引擎类型），必须由显式的链接/激活期校验发现漂移并 fail closed（见 `db.md §1.3` 的覆盖校验）。spread 本身不提供、也不承诺该校验。
- 快照语义是刻意的：隐式跟随会让存储形态（索引、storage mapping）在宿主不知情时静默变化；显式失败要求宿主审视新字段的存储语义。

## 5. 与 interface / impl 的关系

- **方法不复制**：源类型的 `impl` 方法、method namespace 不进入目标类型。目标类型的方法由自身的 `impl` 声明提供。
- **conformance 不复制**：源类型 `implements` 的 interface 不传播。conformance 由显式 `implements` 声明与 method namespace 匹配决定（`interface.md`），与字段集无关；目标类型需要 conformance 时必须显式声明。
- **type namespace 不复制**：`static function` 等 type namespace 成员不进入目标类型。
- 行为共享不是 spread 的职责：需要跨类型共享行为时使用 interface 与泛型机制，而不是 spread。

## 6. 与 db object 的交互

- 含 `spread` 的 record 可以附着 db object：展开后是具体 record，满足 db object 附着约束（非泛型 concrete record，见 `db.md §1`）。spread 复制的字段与显式字段一样可以作为 primary key 或索引路径候选。
- **storage mapping 不随 spread 传播**：`storage ... using encrypted` 等物理存储声明属于 db object declaration（`db.md §10`），由声明 db object 的一方自己写出。spread 只复制 type 层面的字段名与类型。
- **recoverable lane 不因 spread 改变**：展开后的字段集若含 recoverable-envelope lane 字段，跨 request 边界的 fail-closed 约束照常生效（`db.md §11`）；spread 不是绕开 recoverable 检查的手段。
- 设计上下文：存储契约特性（`db.md §1.3`）允许产品类型用 `spread` 复用引擎字段。契约的字段覆盖校验在链接/激活期验证的是**展开后的字段集**，与 spread 无关——spread 只是声明复用手段。

## 7. 与泛型的关系

- 泛型 record 的声明体可以包含 `spread`：被 spread 的源与类型参数无依赖时合法；依赖类型参数的 spread 第一版不允许（源必须可独立确定字段集）。
- 已展开的 record 可以作为泛型类型实参或被其他 spread 引用。

## 8. 错误清单

以下均为 compile error：

- `spread` 出现在非 record 形态（representation / union / alias / interface 声明体）。
- `spread` 源解析后不是 record 形态。
- `spread` 源是裸泛型类型（未 fully instantiated）。
- `spread` 源的字段集依赖未绑定的类型参数（fully instantiated 但实参引用目标类型参数）。
- 多个 spread 条目之间、spread 条目与显式字段之间字段名冲突。
- 自 spread（`type A { spread A ... }`）与循环 spread 链（`A spread B` 且 `B spread A`）。循环检测在展开前按 source 引用图完成，不依赖展开后的 IR。
- `spread` 源类型不可见或不存在。
- `spread` 后接的不是 qualified type name。

## 9. 开放问题

- 漂移校验的落点：`db.md §1.3` 定义了存储契约场景的覆盖校验；非存储场景下两个类型字段一致性是否也需要通用机制，待需求出现再定。
- 展开后的类型是否保留源字段的 source span 用于诊断：实现细节，不影响语义。
- 与 `implements` conformance 列表的书写顺序约束：不引入，conformance 由目标类型独立声明。
