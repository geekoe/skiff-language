# Compiler Type Checking Architecture

本文定义 `compiler` 内部 type resolution / expression type checking 的长期
架构契约。它面向 compiler 维护者和验收 agent，不是用户可见语言规范，也不是迁移
计划。

用户可见语义仍由 `../reference/syntax.md` 和
`../reference/static-semantics.md` 定义。本文只规定 compiler 内部 facts 的 owner、
输入输出、key/provenance 和阶段边界。临时落地步骤放在 `../implementation/`。

## Scope

本文负责：

- `TypeResolutionModel` 和 `ExpressionTypeModel` 的事实归属。
- type checker 可以读取什么、必须产出什么、不得产出什么。
- expression facts 的 stable key 和 diagnostic provenance。
- constructor、representation constructor、nullable materialization 和 assignability
  的内部边界。
- lowering / projection 如何消费 type checking facts。

本文不负责：

- 完整用户语义描述。
- Parser token / AST 具体字段设计。
- Rust 模块拆分或迁移阶段。
- runtime decode、wire schema 或 artifact JSON 形状。

## Pipeline Position

Type resolution 和 expression type checking 都属于 `SourceCompileModel` 构建阶段。

```text
ParsedSourceSet
  -> NameResolutionModel
  -> TypeResolutionModel
  -> ExpressionTypeModel
  -> SourceCompileModel
  -> LoweredPublication
  -> CompiledPublication
  -> projections
```

`TypeResolutionModel` 和 `ExpressionTypeModel` 不是 `LoweredPublication` 的缓存，也
不是 projection 的派生结果。它们是 source-level typed facts 的 owner。

## Inputs And Outputs

`TypeResolutionModel` 可以读取：

- parsed source AST 和 source spans；
- source/module/package name resolution；
- current publication source set；
- direct package dependency public type surface；
- compiler-known std/prelude type registry；
- DB object type declarations and attachment metadata；
- external service type symbols already resolved by publication input.

`TypeResolutionModel` 产出 compiler-owned `ResolvedTypeRef` 和 provenance。它不得产出
File IR、runtime descriptor 或 artifact DTO。

`ExpressionTypeModel` 可以读取：

- parsed source AST 和 expression spans；
- name resolution facts；
- `TypeResolutionModel` facts；
- declaration context：function / impl / actor manager hook / const / test / DB source owner；
- package/service callable signature facts that are already typed compile inputs.

`ExpressionTypeModel` 产出 expression / statement boundary facts 和 diagnostics。它
不得生成 File IR，不得读取 final artifact JSON，不得通过 display string 重新解析类型。

## Type Resolution Contract

Type resolution 负责把 source type syntax 归一为 `ResolvedTypeRef`，并保留 source
spelling/provenance 用于诊断和 metadata。

`ResolvedTypeRef` 必须区分 source spelling、public API nameability、ABI type identity 和 runtime
address。显式 public type 和 closure-only ABI type 都可以有 canonical ABI type id；只有前者有外部源码可写
public path。完整 identity 分层见 `compiler-entity-and-identity.md`。

它必须覆盖：

- source-local named type, alias and interface lookup；
- alias expansion, including aliases inside anonymous records, unions and containers；
- generic type parameter binding and substitution；
- package alias / dependency public type lookup；
- std/prelude compiler-known type lookup；
- DB object nominal type lookup；
- external service type symbol lookup；
- nullable, union, literal, anonymous record and container type expressions.

Artifact-specific `TypeRefIr`、runtime descriptor、contract type key 和 package ABI type
都是 projection 结果，不是 source type facts owner。

## Expression Keys

Expression facts 必须使用 stable key，不得使用 Rust 引用地址、arena index 或临时 borrow
作为跨阶段协议。

建议形状：

```rust
struct ExpressionKey {
    module_path: ModulePath,
    owner: ExpressionOwnerKey,
    preorder_index: u32,
}

enum ExpressionOwnerKey {
    Function(String),
    ImplMethod { type_name: String, method: String },
    ActorHook { actor: String, hook: String },
    Const(String),
    Test(String),
    DbIndexWhere { db: String, index: String },
}
```

`preorder_index` 只在所有 facts consumer 使用同一个 traversal contract 时成立。如果
parser 会重写 AST、插入 synthetic expression 或允许多个 walker 顺序，必须改为 parser
node id 或 facts-sharing walker。

key 空间必须覆盖所有 source-level expression owner。新增 owner 时，先扩展
`ExpressionOwnerKey`，再允许 validator 消费对应 facts；不得为新增 owner 保留私有
AST walker 作为长期路径。

## Diagnostic Provenance

每个 expression fact 至少携带：

- `ExpressionKey`；
- expression `SourceSpan`；
- declaration / owner context；
- resolved type provenance；
- diagnostics emitted while deriving the fact.

需要字段级诊断的 facts 必须携带字段自己的 span。例如 constructor duplicate /
unknown field 诊断应使用 field name span；field value type mismatch 应能指向 field
value expression span。不能从 `Vec<(String, Expr)>`、`BTreeMap` key 或 serialized JSON
反推 source location。

## Constructor Facts

`QualifiedName TypeArgs? { fields }` 是名义 record constructor sugar，在当前 AST 中是
`Expr::Record`。它的 shape validation 属于 expression type checking。

Constructor fact 至少包含：

```rust
struct ConstructorValidation {
    key: ExpressionKey,
    target: ResolvedConstructorTarget,
    provided_fields: Vec<ResolvedConstructorField>,
    materialized_fields: Vec<MaterializedConstructorField>,
    missing_required_fields: Vec<ResolvedField>,
    unknown_fields: Vec<SourceFieldName>,
    type_mismatches: Vec<FieldTypeMismatch>,
}

enum ConstructorFieldValueSource {
    Provided(ExpressionKey),
    SyntheticNull,
}
```

Checker 必须处理：

- constructor target resolution；
- generic field substitution；
- duplicate field；
- unknown field；
- missing required field；
- field value assignability；
- nullable field materialization.

Lowering 只能消费 constructor fact。它不得重新解析 target type syntax，不得只读取 AST
provided field map，不得 fallback 到 string inference。

## Nullable Field Materialization

Canonical static semantics 已定义：record literal target typing 中，缺失的 nullable
字段默认填入 `null`。因此 compiler 内部 contract 是：

- missing nullable field is valid；
- constructor fact records `SyntheticNull` for that field；
- lowering projects `SyntheticNull` to an explicit null expression / construct field；
- runtime construct execution must not be expected to infer missing fields from schema.

Wire decode / runtime schema 对 optional nullable 字段的处理是 boundary behavior，不改变
source constructor 的 compile-time materialization 责任。

## Representation Constructors

Representation constructor 是 `Name(value)` call 形态，不是 `Expr::Record`。它属于 call
/ assignability checking。

Checker 必须把 `Name(value)` 解析为 representation constructor only when `Name`
resolves to a representation type namespace item. It must then check argument
assignability to the representation RHS. It must not run record shape validation,
missing-field checks or field materialization for representation constructors.

## Assignability

Assignability is a shared type-checking service, not per-feature ad hoc logic.

It must support the source-level rules needed by:

- constructor field values；
- `let` initializer annotations；
- return expressions；
- call arguments and receiver methods；
- representation constructor arguments；
- target-typed object / record literals；
- pattern bindings and narrowing where applicable.

The service must operate on `ResolvedTypeRef`, not display strings. It must handle
nominal identity, representation identity, transparent alias expansion, anonymous
record shape, union membership, nullable, literals, containers and generic
substitution according to reference semantics.

## Source Rule Consumers

DB rules、stream emit validation、suspend analysis、config usage collection and future
source rules should consume `ExpressionTypeModel` facts when they need expression
types, call facts, effect facts or shared traversal metadata.

During migration, a source rule may keep a private walker only as a documented
temporary adapter. New long-term source-rule behavior must not introduce another
expression type inference path.

## Lowering And Projection Boundaries

Lowering may project `ResolvedTypeRef` and expression facts into File IR. It must
not:

- infer expression types from AST；
- call type syntax lowering helpers to discover source facts；
- use `TypeExpr::parse_lossy` or display strings as source-of-truth；
- silently continue when an expression fact required for lowering is missing.

Projection may project `CompiledPublication` typed facts into contract/runtime/package
outputs. It must not:

- import AST expression types to recover type facts；
- call lowering helpers to reconstruct type resolution；
- parse display strings or artifact JSON to recover source typing decisions.

## Verification Contract

Architecture-level verification should check:

- constructor diagnostics for missing/unknown/duplicate/type mismatch fields；
- nullable omitted field materializes to explicit null in lowered construct；
- representation constructor is checked as call / assignability, not record shape；
- expression keys are stable and cover non-executable owners such as DB index
  `where` expressions；
- field-name diagnostics use field-name spans；
- lowering fails when required expression facts are missing；
- production paths no longer use string inference helpers after their migration
  stage is complete；
- projections do not recover type facts from AST, display strings, lowering helpers
  or artifact JSON.
