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
  -> service unit DB metadata
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
- target type metadata;
- selector or query;
- projection as DB execution plan data;
- body or change;
- normalized result type.

`result_type` must be a normal `TypeRefIr`: record, nullable, array, DB result builtin, primitive or another ordinary shape. It must not carry DB-origin markers.

Projection remains useful as DB execution data. Runtime store needs it to ask storage for selected fields. That is separate from result type.

### Service Unit DB Metadata

Service unit DB metadata is the runtime storage contract. It includes:

- module path and source role;
- object kind;
- attached type reference;
- canonical type name;
- collection name;
- key metadata;
- stored field metadata;
- retention and indexes.

Package DB metadata must be merged into service unit DB metadata before runtime linking. Runtime packages do not own service databases independently at execution time.

### Runtime Linked Program

Runtime linked program owns dispatch maps, linked File IR, linked type descriptors and DB metadata. It does not reconstruct source typing decisions from `ReadRecord`.

When executing a DB operation, runtime:

1. evaluates query/body/change expressions into wire JSON values;
2. sends a typed command to the service DB store;
3. receives business JSON from the store;
4. decodes it through the already-normalized ordinary result plan.

If runtime sees an unsupported type descriptor, that is an artifact error. `readRecord` should never be a possible label.

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
- Runtime non-Mongo tests: ordinary record result plans decode DB business JSON.
- Service DB adapter tests: Mongo mapping, projection document, transaction and BSON coercion.
- Test-runner / service tests: end-to-end DB behavior using dev router config or explicit test config.

Core runtime tests should not depend on a user service example. User service examples should not be the only coverage for compiler/runtime DB contracts.

## Non-Goals

This architecture does not add:

- cross-service DB access;
- relation / load semantics;
- cursor / continuation semantics;
- schema migration workflow;
- automatic dirty tracking;
- Mongo API exposure in Skiff source;
- runtime support for `ReadRecord`.
