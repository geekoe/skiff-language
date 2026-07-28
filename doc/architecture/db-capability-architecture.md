# Skiff DB Capability Architecture

本文定义 Skiff DB capability 在 compiler、artifact、runtime、router 和测试基础设施之间的长期内部边界。它不是用户语言参考，也不是迁移 checklist。用户可见规则见 `../reference/db.md`，实现步骤见 `../implementation/db-read-record-removal-implementation.md`。

## Goals

DB 架构目标：

- source-level `type` 是 object shape 的唯一类型声明。
- `db object` 只声明 storage attachment metadata。
- DB query / projection 是 compiler 可分析的语言结构，不是 Mongo JSON。
- runtime 接收已经规范化的普通 type descriptor，不理解 `ReadRecord`。
- Mongo 只存在于 service DB adapter 内，不进入 Skiff source、File IR result type 或 service API schema。
- service DB 连接能力由 router / platform activation 注入，业务源码不能选择 database 或读取连接串。

## Stage Boundaries

长期阶段流向：

```text
source AST
  -> DB attachment semantic model
  -> File IR DB operation
  -> package symbolic target
  -> exact provider File IR DB metadata
  -> runtime linked program image
  -> service DB command
  -> storage adapter
```

### Source And Semantics

Parser 只识别 DB surface grammar，不推断 runtime storage shape。Semantic model 负责：

- 建立 `db object` 到同模块 attached `type` 的关系。
- 验证 primary key、index field、query field、projection field 和 change field。
- 标记 read result 的 readonly provenance。
- 保留 projection field set，用于后续类型展开。

DB words such as `fields`, `where`, `order`, `limit`, `offset`, `unset`, `add` and `remove` are contextual. They must not become global reserved identifiers.

### Type Normalization

`ReadRecord` is not an architecture type. It must not appear in:

- source-visible type display;
- artifact-model `TypeRefIr`;
- runtime linked `LinkedTypeRef`;
- runtime descriptor JSON;
- boundary schema.

Compiler may use an internal helper concept such as `DbReadView { object, fields }` while typechecking, but that helper must normalize before File IR artifact emission.

Normalization target:

```text
DbReadView(User, full)
  -> User

DbReadView(User, fields { name, visits })
  -> { id: string, name: string, visits: number }
```

Full reads/writes use the attached nominal type. Projected reads generate anonymous records and remain readonly by binding provenance, not by a special type descriptor. Runtime only sees ordinary nominal or record plans.

### File IR

File IR DB operation carries:

- operation kind;
- a local target or an external `PackageSymbolRef`;
- selector or query;
- projection as DB execution plan data;
- body or change;
- normalized result type.

`result_type` must be a normal `TypeRefIr`: record, nullable, array, DB result builtin, primitive or another ordinary shape. It must not carry DB-origin markers.

Projection remains useful as DB execution data. Runtime store needs it to ask storage for selected fields. That is separate from result type.

For an external target, consumer File IR reuses the canonical package symbol shape:

```text
PackageSymbolRef {
  package: Dependency(alias),
  symbolPath,
  abiExpectation
}
```

The consumer must not copy the provider's collection, key, field, lease, index,
retention or recoverable metadata into its own File IR. `typeName` can be kept
for diagnostics, but it is never a lookup key.

### Package Link And Provider Metadata

The consumer's `PackageRequirement.expectedPackageBuild` constrains a test-only
dependency entry with `topLevelAlias` to one immutable implementation artifact.
The entry's ordinary `alias` still resolves its `api.yml` public paths. Source
resolution through `topLevelAlias` canonicalizes back to that primary alias, so
both names produce one dependency edge, one requirement and one binding.
Assembly resolution produces a `PackageBinding`; linker resolution then
follows one fail-closed chain:

```text
consumer DbTargetIr.PackageSymbol
  -> PackageRequirement(alias, expectedPackageBuild)
  -> PackageBinding
  -> exact PackageArtifactRef
  -> PackageArtifact.implementation_links.types[symbolPath]
  -> provider FileIrRef + typeIndex
  -> provider File IR declarations.types
  -> provider File IR declarations.db
```

The type export, provider type declaration and DB attachment must identify the
same File IR type. Missing links, missing files, missing types, missing DB
attachments, ABI/build mismatch and cross-artifact substitution are artifact
errors. There is no search by module suffix, type name or discovery order.

Provider File IR is the single owner of DB storage metadata: object kind,
collection, key, stored fields, retention, leases, indexes and recoverable
plans are read from the resolved declaration exactly once. Consumer File IR,
PackageArtifact and linked executable must not duplicate those facts.

Two dependencies may contain the same module path and type name. Their exact
PackageArtifactRef keeps their DB target identities distinct; name collision is
not a link error. Physical collection projection remains service-owned and is
validated separately through each dependency edge's collection-name mapping.

A test can directly depend on a stateful provider that is also reachable through
the subject package. These are two real graph edges, not two spellings of one
entry. Runtime admission merges them into one active collection projection and
one metadata owner only when they select the exact same PackageBuild and their
fully resolved source-to-target mappings and owner-relevant facts are
canonically equal. The same build with different mappings, different builds
targeting one physical collection, and dependency/root collection collisions
all fail closed. `config.skiff-test.yml` remains the sole test-activation state
binding owner.

### Linked DB Target Identity

After linking, every DB target has the canonical identity:

```text
DbObjectTargetId {
  packageArtifactRef,
  fileIrRef,
  typeIndex
}
```

This identity is used by DB operation dispatch, `DbQuery`, lease claim, lease
state read and lease write guards. `typeName` is diagnostic text only and must
not select metadata. A transaction has no target identity of its own; each DB
operation inside it carries its linked target.

### Runtime Linked Program

Runtime linked program owns dispatch maps, linked File IR, linked type descriptors and DB metadata. It does not reconstruct source typing decisions from `ReadRecord`.

When executing a DB operation, runtime:

1. resolves the linked `DbObjectTargetId` to the already-admitted provider File IR declaration;
2. evaluates query/body/change expressions into wire JSON values;
3. sends a typed command to the service DB store;
4. receives business JSON from the store;
5. decodes it through the already-normalized ordinary result plan.

If runtime sees an unsupported type descriptor, a target whose exact provider
artifact/file/type is not admitted, or a metadata lookup that would require
`typeName`, that is an artifact error. `readRecord` should never be a possible
label.

### Service DB Store

The service DB store owns storage metadata parsing, business value to document mapping, projection compilation, query compilation, update compilation, transaction handling and adapter IO.

Mongo-specific responsibilities stay below this boundary:

- `_id` mapping for primary key;
- BSON coercion;
- Mongo filter / sort / projection / update document construction;
- session and transaction execution;
- duplicate key and write result mapping.

Skiff runtime above the store talks in service DB commands and business JSON, not Mongo documents.

## Router And Activation

Router / platform activation injects `serviceDb.mongoUrl`. Source files and service config do not contain the real DB URL.

Local dev examples and service-level live tests should discover DB configuration from dev `router.yml` through the same path as runtime activation. Low-level runtime crate tests may stay opt-in through an environment variable when they are testing adapter internals, but user-facing examples should not teach direct env-only DB setup.

## Testing Boundary

Tests belong at the lowest layer that can prove the contract:

- Parser tests: DB block grammar, especially `fields { where, name }`.
- Compiler tests: full DB result normalization to nominal type refs, projection normalization to anonymous record result types and readonly diagnostics.
- Compiler/linker tests: test-only top-level external DB targets, exact
  PackageBinding/type-link resolution, same-name targets in two packages and
  fail-closed missing/mismatched/cross-artifact cases.
- Runtime non-Mongo tests: ordinary record result plans decode DB business JSON.
- Runtime target tests: all DB operations, `DbQuery`, lease claim/read/guard use
  `DbObjectTargetId` and provider metadata without consumer copies.
- Service DB adapter tests: Mongo mapping, projection document, transaction and BSON coercion.
- Test-runner / service tests: end-to-end DB behavior using dev router config or explicit test config.

Core runtime tests should not depend on a user service example. User service examples should not be the only coverage for compiler/runtime DB contracts.

## Non-Goals

This architecture does not add:

- cross-service DB access;
- top-level DB visibility for ordinary package dependencies, transitive dependencies or production services;
- relation / load semantics;
- cursor / continuation semantics;
- schema migration workflow;
- automatic dirty tracking;
- Mongo API exposure in Skiff source;
- runtime support for `ReadRecord`.
