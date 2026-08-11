# Skiff DB Capability Architecture

本文定义 Skiff DB capability 在 compiler、artifact、runtime、router 和测试基础设施之间的长期内部边界。它不是用户语言参考，也不是迁移 checklist。用户可见规则见 `../reference/db.md`。

## Goals

DB 架构目标：

- source-level `type` 是 object shape 的唯一类型声明。
- `db object` 只声明 storage attachment metadata。
- DB query / projection 是 compiler 可分析的语言结构，不是 Mongo JSON。
- runtime 接收已经规范化的普通 type descriptor，不理解 `ReadRecord`。
- Mongo 只存在于 service DB adapter 内，不进入 Skiff source、File IR result type 或 service API schema。
- service DB连接能力由Router connection bootstrap注入；database identity由operator选择的受信Mongo
  endpoint/storage domain、profile与serviceId共同定界，不引入`platformId`。业务源码和service配置
  不能选择database、namespace或连接串。
- 一个service只有一个数据库；同一service中的Package共享它，但每个DB target仍保留精确
  PackageArtifact/File IR/type identity。

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
While building an exact `DeploymentExecutionImage`, deployment-package-closure resolution
produces a `PackageBinding`; image-local linker resolution then follows one fail-closed chain:

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
not a link error. Physical collection ownership is validated by exact stable
`(packageId, declared logical collection identity)` and system encoding. The
`db object name` is this logical identity, not a physical name. Package
dependency, requirement, binding and configuration inputs do not provide
collection-name or database-namespace mappings.

A test can directly depend on a DB-metadata provider that is also reachable through
the subject package. These are two real graph edges, not two spellings of one
entry. Runtime admission merges them into one active collection projection and
one metadata owner only when they select the exact same PackageBuild and their
owner-relevant facts are canonically equal. One Package ID resolving to
different builds, missing/duplicate logical collection identity, and a system
physical-name encoding collision all fail closed. Different packages may use
the same bare collection name without sharing storage. The
test database remains the current generated test service database; test-only
foreign target authority never opens the provider service database.

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

### Transaction Boundary

`db transaction`在一次execution中只允许一个active DB transaction；nested transaction不支持。
Compiler与artifact verifier拒绝静态可见的嵌套，Runtime在helper等动态路径重入时必须先拒绝，
不能开启第二个session transaction，也不能把内层`db transaction`静默折叠进外层边界。

Actor method（含`create`）中的transaction是DB-only：commit/abort只作用于DB，不为Actor arena建立
snapshot overlay。Transaction body禁止直接或经callee写Actor field；直接赋值和以Actor field为
receiver的原地修改都属于field write。Compiler effect summary必须闭合同包helper与可静态解析的
package-direct call；unknown/dynamic target不能证明无Actor field write时保守拒绝。普通local仍使用
`final`或`var`，不存在local `const`。

### Exact Service DB Index Plan

Runtime loader为每个exact `DeploymentExecutionImage`从该deployment自己的exact package closure和
linked DB metadata派生一份完整`ServiceDbIndexPlan`。该plan是image-local derived metadata，
不是新artifact，也不从运行中request、current release pointer、其它loaded image或Mongo反向
推断。它按以下顺序形成：

1. 从exact deployment package closure收集DB metadata；
2. 按系统physical collection identity合并collection；
3. 同一stable logical index identity且field、顺序、方向、unique和collation完全相同时去重；
4. 同一stable logical index identity的定义不同、collection metadata owner冲突、physical encoding
   collision或同Package ID解析到不同exact build时，在任何Mongo mutation前拒绝image；
5. 不同stable logical index identity形成当前image所需的并集；不试图从全局
   “当前版本集”生成plan。

受管index的stable identity是
`(packageId, logical collection identity, logical index identity)`；Package version/build、
service version、dependency alias、edge path和runtime replica都不参与该identity。因此多个build可以
共享同一stable identity，但只有canonical definition完全相同时才兼容。

每个受管index的physical name由系统对
`(packageId, logical collection identity, logical index identity)`做稳定、无碰撞、满足Mongo限制的编码。
源码index name不能直接成为physical name。`_id_`是Mongo内建主键索引，不进入受管plan，也不参与
removed检查。

Index field path必须复用canonical DB field policy和physical field mapper，不能另写一套dot-path parser。
encrypted、recoverable-envelope内部、动态shape及其它不允许query/order的path同样不允许index。所有受管
index固定使用Mongo simple/binary collation；collation是canonical definition的一部分。

每个Runtime replica在把exact build的`DeploymentExecutionImage`记为loaded并对业务请求可执行
之前，必须独立reconcile该image-local完整plan：

- 缺少的受管index：additive、幂等创建，创建后复读并验证canonical definition；
- 已有的同一stable identity受管index与canonical definition精确一致：通过；
- 已有的同一stable identity受管index与keys、方向、unique或collation不一致：fail closed；
- 数据库中存在但当前image plan未声明的受管index：保留且忽略；
- 非Skiff受管index：保留且忽略，不能自动drop；
- Mongo `_id_`：保留且忽略。

多replica及多build并发load各自执行同一幂等算法；Skiff不建立跨replica/build的等待或协调协议，
storage backend自身的DDL串行化不改变该契约。
重复的exact create视为幂等成功；任一replica观察到同一stable index identity定义冲突或创建后
复读不一致，都拒绝当前image load。另一build遗留但当前image未声明的受管index不是drift，
不得因其存在而拒绝load。
Runtime不在image load中drop、rename或background rebuild受管index。索引变更与删除必须先由
显式migration完成，再发布能通过exact load verification的新deployment。

创建unique index时发现历史duplicate，以及业务写入触发duplicate key，统一映射为脱敏、不可重试的
`std.db.ConstraintError`分类；不得泄漏Mongo code/message、database、collection、physical index、key
pattern或业务值。业务request中的约束冲突可被用户捕获；deployment image load中的冲突只作为sanitized
load rejection。其它Mongo错误继续映射到既有平台/内部错误边界，不能伪装成constraint。

Partial index不属于当前架构。Compiler遇到index `where`必须报静态错误；File IR、runtime projection、
linked DB metadata和store command均不得携带raw source AST或Mongo predicate。未来若支持partial index，
必须先设计可类型检查、canonical identity稳定、可在artifact boundary验证的封闭typed predicate IR。

## Router Bootstrap And Deployment Load

Router connection bootstrap injects `serviceDb.mongoUrl`. Source files and service config do not contain the real DB URL.

Router不解析index metadata，也不协调prepare/commit/abort。不存在为service DB汇总多个
release/build的global candidate generation或activation prepare。Runtime按exact deployment build构建image时，
必须在业务storage mutation前验证service DB identity和完整index plan；同replica内的同build并发load按
per-build临界区收敛。已有某些index不能跳过其余plan验证，失败不得把半可执行image记为loaded或注册。

`package.yml state`、`PackageRuntimeRequirements.state`、`StateBinding`和deployment state binding都不是DB
capability的一部分。Runtime仅在精确deployment package closure包含DB metadata时按需创建service DB handle；没有DB
metadata的service不需要创建空数据库。跨service DB访问禁止。未来Redis、queue或第三方数据库必须定义
独立capability，不能复用一个通用state namespace配置面。

Local dev examples and service-level live tests should discover DB configuration from dev `router.yml` through the same
Router bootstrap path. Low-level runtime crate tests may stay opt-in through an environment variable when they are testing adapter internals, but user-facing examples should not teach direct env-only DB setup.

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
- Test-runner / service tests: end-to-end DB behavior using dev router transport config and a database identity derived
  from `(testRunId, generatedTestServiceId)`.

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
