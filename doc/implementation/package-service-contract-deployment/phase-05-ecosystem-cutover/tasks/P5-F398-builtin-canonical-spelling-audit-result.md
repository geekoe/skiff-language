# P5-F398 Builtin canonical spelling audit result

状态：Complete（只读审计）。

## 结论

`TASK_EXECUTABLE`

唯一修复是 **compiler-owned source spelling → canonical FileIR builtin name**。当前
`std.db.ConflictError.retryable` 的首次分叉发生在 FileIR lowering：

- language reference规定 `bool` 是唯一 canonical 拼写；
- compiler source resolution 已把合法 source alias `boolean` 解析为 builtin `bool`；
- declaration lowering 没有消费该 resolved type，而是从原始 AST 再次 lowering，并把
  `boolean` 原样写入 `TypeRefIr::Builtin.name`；
- PackageSchema producer 随后按既有 producer-side normalization 正确写出 `bool`；
- Runtime linker 原样保留 FileIR 名称，并对两个 canonical artifact fact 做 exact name/arity
  比较，因而正确拒绝 `boolean != bool`。

不得通过修改 `std/db.skiff` 一处 source、放宽 linker、增加 alias compare、在 reader 中重写旧
artifact 或把 PackageSchema 改回 `boolean` 掩盖问题。`boolean` 仍可作为当前 compiler table
明确接受的 source alias，但不得越过 compiler artifact boundary。

修复不需要 artifact-model enum。FileIR builtin universe 包含 execution-only builtin，而
ContractTypeRef 只允许 boundary-safe subset；把 source alias policy 放进 artifact-model 会混淆两个
domain，并鼓励 artifact reader 做修复性 dual-read。唯一 owner 应保留在 compiler core builtin
registry，source semantic resolution 与 FileIR lowering 都消费同一 canonical-name helper。

## 审计边界与 fresh receipt

- production 基线 commit：
  `10f0ef53eaf77c7082115c544de35e20acc8af74`
- production 基线 tree：
  `536f3306915b7626539577d1ad7ff1b738daa799`
- Skiff/std/runtime production source 全部只读；没有访问 stable instance、MongoDB、Router、
  Runtime、live store 或外部服务。
- fresh artifacts 只写入隔离目录：
  `/tmp/skiff-p5-f398-audit.oabfZz`
- fresh std：
  - Package Build：
    `skiff-package-build-v8:sha256:1828acdba6f3745db377255fc759fac3b3e87ed987001af97c67fa72bbbe4796`
  - Package Local ABI：
    `skiff-package-local-abi-v6:sha256:c8be1d04060489a28f827a5313da12ae26891b1d3b21d1085b6e72884c9ab0ea`
  - PackageSchema index：
    `skiff-package-schema-index-v1:sha256:1f70d5626cddaab23d51d52db974a9292cf019cb0161d67ff560c599ed6fd7fe`
  - 16 个 FileIR records。
- representative ordinary package：
  `test-runner/fixtures/package-service-host/helper`
  - Package Build：
    `skiff-package-build-v8:sha256:ee6942065647352419e9129b26e5252a30a083784755c0ea1d801769e831f47a`
  - Package Local ABI：
    `skiff-package-local-abi-v6:sha256:4fa6887d7fb3ace10d60a51fdadcff2649f96a8db79b784a78707e8e06f23395`
  - PackageSchema index：
    `skiff-package-schema-index-v1:sha256:dd2d9ffdd407ec775e1f0375bae36830b7cfde49df7b2704bacedec7682fc6a7`

fresh generation 使用：

```bash
cargo build -p skiff-test-runner --bin skiff-package-service-smoke-fixture

build/cargo-target/debug/skiff-package-service-smoke-fixture \
  --bootstrap-only \
  --artifact-root /tmp/skiff-p5-f398-audit.oabfZz \
  --environment p5-f398-audit \
  --platform-source-root /Users/geek/workspace/skiff-p5-f398-builtin-spelling-audit

node scripts/skiff.mjs package build \
  test-runner/fixtures/package-service-host/helper \
  --artifact-root /tmp/skiff-p5-f398-audit.oabfZz \
  --environment p5-f398-audit \
  --json
```

## Source spelling 与 artifact canonical name

### Parser 与 primitive table

`syntax/src/type_expr.rs:37-87` 只把任意 named type 文本保存在
`TypeExpr::Named { name, args }`，不拥有 builtin identity，也不做 alias canonicalization。canonical
owner 从 parser 后开始。

`doc/reference/std-surface.md:23-32` 定义 canonical prelude names，并在 `:27` 明确规定 `bool`
是布尔类型唯一 canonical 拼写。`compiler/core/src/prelude_registry.rs:5-7` 的当前 source table
接受以下 primitive spellings；`:182-190` 已明确给出 canonical symbol：

| legal source spelling | canonical FileIR builtin name |
| --- | --- |
| `string` | `string` |
| `number` | `number` |
| `integer` | `integer` |
| `bool` | `bool` |
| `boolean` | `bool` |
| `null` | `null` |
| `unknown` | `unknown` |
| `void` | `void` |
| `never` | `never` |

因此 `boolean` 是当前 implementation 接受的 source alias，不是第二个 artifact wire name。

### Compiler-owned builtin registry

`compiler/core/src/prelude_registry.rs:26-135` 同时保存：

- `symbol`：owner-qualified source lookup symbol；
- `name`：compiler/runtime 使用的 canonical `TypeRefIr::Builtin.name`；
- exact arity/kind。

`compiler_builtin_type` 在 `:137-142` 接受 bare `name` 或 qualified `symbol`。两种合法 source
spelling 必须在 FileIR boundary 前统一为 `name`：

| legal source spellings | canonical FileIR name | arity |
| --- | --- | ---: |
| `Actor`, `std.actor.Actor` | `Actor` | 1 |
| `bytes`, `std.bytes.bytes` | `bytes` | 0 |
| `Array`, `std.collection.Array` | `Array` | 1 |
| `Map`, `std.collection.Map` | `Map` | 2 |
| `Config`, `config.Config` | `Config` | 0 |
| `Date` | `Date` | 0 |
| `Json` | `Json` | 0 |
| `JsonObject` | `JsonObject` | 0 |
| `Stream`, `std.stream.Stream` | `Stream` | 1 |
| `Exception`, `std.error.Exception` | `Exception` | 1 |
| `CatchResult`, `std.error.CatchResult` | `CatchResult` | 2 |
| `SourceLocation`, `std.error.SourceLocation` | `SourceLocation` | 0 |
| `StackTrace`, `std.error.StackTrace` | `StackTrace` | 0 |
| `StackFrame`, `std.error.StackFrame` | `StackFrame` | 0 |
| `TimeoutError`, `std.error.TimeoutError` | `TimeoutError` | 0 |
| `CancelError`, `std.error.CancelError` | `CancelError` | 0 |
| `ClientSessionRef`, `std.session.ClientSessionRef` | `ClientSessionRef` | 0 |
| `ClientCapability`, `std.session.ClientCapability` | `ClientCapability` | 0 |

这个 domain 区分也与既有 consumers 一致：compiler expression semantics按 `Exception`、
`CatchResult` 等 bare names 匹配（`compiler/source/src/expression_type_model.rs:4542-4568`），runtime
platform errors按 `TimeoutError`/`CancelError` 匹配
（`runtime/model/src/service_error.rs:56-108`），ContractTypeRef normalization把 overlapping
`std.collection.*`/`std.bytes.bytes` spellings归一到 bare names
（`artifact-identity/src/contract/normalization.rs:154-199`），fresh artifacts 也只观察到这些 bare
canonical names。

这 15 个 non-identity bare/qualified pair 是与 `boolean` 同类的潜在影响面。当前 lowering 的
`canonical_builtin_std_type_name` 在
`compiler/lowering/src/type_lowering.rs:975-1005` 先调用
`prelude_registry().is_builtin_type_name(name)` 并原样返回输入：对 Array/Map/Stream/bytes 四组 pair，
这使后面的 explicit qualified mapping不可达；其余 registry pairs根本没有 canonical branch。
qualified spelling 因而可能泄漏到 FileIR。当前 fresh population 未触发这些 pair，但
implementation 不能只特判 `boolean`。

### 不是合法 end-to-end source alias 的其它 pair

以下名字出现在局部兼容/normalization 分支，但不属于上述 source tables，不应借本修复升级成新语言
surface：

| pair | 当前出现位置 | 分类 |
| --- | --- | --- |
| `String` → `string` | source resolver、contract normalizer | resolver-only/contract input spelling；lowering 不完整支持，不是合法 end-to-end source alias |
| `Bytes` → `bytes` | contract normalizer | ContractTypeRef producer input spelling，不是 source spelling |
| `std.date.Date` → `Date` | lowering、contract normalizer、局部 resolver helper | 不在 compiler-owned registry 的合法 source pair |
| `std.time.Duration` → `Duration` | contract normalizer | contract-only pair；不能与 source/package `Duration` ownership 混合 |

implementation 应删除或隔离这些脱离 canonical owner 的 dead/local maps；至少不得把它们加入新的
source alias registry。fresh artifact 中的
`std.websocket.WebSocketConnection` 是唯一 observed qualified builtin，属于独立、无 bare alias 的
canonical WebSocket execution type，不是 spelling duplicate。

## `std.db.ConflictError.retryable` 逐跳追踪

source 在 `std/db.skiff:6-10` 写作 `retryable: boolean`。

| hop | exact fact | owner/evidence | 判定 |
| --- | --- | --- | --- |
| Source | `boolean` | `std/db.skiff:9` | 合法 source alias。 |
| Parser | `TypeExpr::Named { name: "boolean" }` | `syntax/src/type_expr.rs:37-87` | parser 原样保存，无分叉责任。 |
| Semantic resolution | `TypeRefIr::Builtin { name: "bool", args: [] }` | `compiler/source/src/type_resolution_model.rs:2579-2612,5710-5727`；primitive table `compiler/core/src/prelude_registry.rs:182-190` | 已 canonical。 |
| FileIR declaration | `TypeRefIr::Builtin { name: "boolean", args: [] }` | `compiler/lowering/src/declaration_lowering.rs:214-271` 从 raw AST field 调 `lower_type_ref`；`compiler/lowering/src/type_lowering.rs:775-855,954-962` 接受后原样写 `service_name` | **首次分叉；错误 producer。** |
| Package Artifact copies | `boolean` | `implementationLinks.types`、Local ABI `implementationSymbols` 和 `publicSymbols` 都复制 FileIR descriptor | FileIR drift 扩散三处。 |
| PackageSchema projection input | `boolean` | `compiler/projection/src/package_artifact/schema.rs:492-500` 先复制 FileIR name | 尚未成为 canonical record。 |
| PackageSchema canonical record | `bool` | `schema.rs:315-345` 调 producer-side normalization；`artifact-identity/src/contract/normalization.rs:154-199` 映射 `boolean` → `bool` | producer 正确。 |
| Linked type | `LinkedTypeRef::Native { name: "boolean" }` | `runtime/linker/src/linker/file_conversion.rs:1257-1262` 原样复制；`assembly_execution/code_linker.rs:335-340` 只链接 nested args | linker 不拥有 rename。 |
| Linker schema validation | exact `linked_name == schema_name` 且 exact arity | `runtime/linker/src/assembly_execution/service_error_index.rs:469-489` | 正确 fail closed；不得放宽。 |

fresh exact records：

- std.db FileIR：
  `skiff-file-ir-v8:sha256:bb39d35baa25cbfb50a1d146e21a18a2ad088940d34304b877e13e348543b069`
  - `target: string`
  - `message: string`
  - `retryable: boolean`
- `std.db.ConflictError` PackageSchema：
  `skiff-package-schema-type-v1:sha256:dd893e08035a093080419ff2c04beda67c1dab2e95ddcc23dec12f9ce6d8bdd0`
  - `target: string`
  - `message: string`
  - `retryable: bool`

## Fresh artifact 全量 builtin scan

扫描每个 JSON subtree 中所有 `{ "kind": "builtin", "name": ... }`，不是只扫 public
surface：

| artifact surface | observed names/counts |
| --- | --- |
| fresh std 16 FileIR records | `Array×18`, `Json×7`, `JsonObject×5`, `Stream×3`, `bool×3`, `boolean×1`, `bytes×14`, `integer×18`, `number×1`, `std.websocket.WebSocketConnection×1`, `string×116`, `void×14` |
| fresh std Package Artifact | `Array×50`, `Json×16`, `JsonObject×10`, `Stream×7`, `bool×7`, `boolean×3`, `bytes×39`, `integer×51`, `number×3`, `std.websocket.WebSocketConnection×3`, `string×322`, `void×34` |
| fresh std PackageSchema records | `Array×10`, `Json×1`, `bool×1`, `bytes×6`, `integer×12`, `string×58` |
| helper FileIR | `Stream×1`, `string×5`, `void×1` |
| helper Package Artifact | `Stream×2`, `string×20`, `void×3` |
| helper PackageSchema records | `string×5` |

fresh population 中唯一同时出现的同语义 duplicate 是 `bool`/`boolean`。唯一 FileIR
`boolean` 是 `std.db.ConflictError.retryable`；Package Artifact 的三个 exact path 是：

```text
implementationLinks.types.std.db.ConflictError.descriptor.fields.retryable
packageLocalAbi.implementationSymbols.std.db.ConflictError.descriptor.fields.retryable
packageLocalAbi.publicSymbols.std.db.ConflictError.descriptor.fields.retryable
```

PackageSchema 与 representative ordinary package 均没有 noncanonical alias。上节 registry 的 15
组 pair 是 code-defined latent surface，虽未出现在本次 population，也必须由同一表驱动测试覆盖。

## 唯一 production repair boundary

### Production owners

1. `compiler/core/src/prelude_registry.rs`
   - 把 source spelling 与 canonical FileIR name 作为一份显式、可枚举的 registry fact；
   - primitive helper 固定 `boolean`/`bool` → `bool`；
   - compiler builtin helper固定 `builtin.name` 与 `builtin.symbol` → `builtin.name`；
   - 校验 source spelling 无碰撞、canonical name 唯一、arity/kind 不因 alias 改变；
   - unknown spelling 返回 `None`，不做 suffix 或大小写 fallback。
2. `compiler/source/src/type_resolution_model.rs`
   - 删除本地 `builtin_type_name`/qualified canonicalization 重复表；
   - semantic resolution 只消费 compiler-core owner；
   - 不保留 `String` 等未在 source table 声明的隐式 alias。
3. `compiler/lowering/src/type_lowering.rs`
   - 所有从 AST/text 产生 `TypeRefIr::Builtin` 的入口先消费同一 helper；
   - bare/qualified、generic arguments、nullable/union/record/function nested refs 都递归 canonical；
   - 删除当前 “recognized then return input unchanged” 分支和重复 qualified maps。

若 lowering 能直接消费 `TypeResolutionModel` 的 exact resolved type，优先减少第二套解析；但不应为本
blocker 重写无关 lowering pipeline。无论采用 resolved fact 还是 shared helper，验收事实都是：
compiler 生成的每个 `TypeRefIr::Builtin.name` 已 canonical。

### 明确不修改

- `std/db.skiff`：可在后续 style cleanup 改写为 `bool`，但这不能替代 legal alias 的 producer
  canonicalization。
- `compiler/projection/src/package_artifact/schema.rs` 与
  `artifact-identity/src/contract/normalization.rs`：当前输出正确；保留 producer-side canonicalization
  与 unknown contract builtin fail-closed。
- `artifact-model::TypeRefIr`：继续作为 typed DTO；本任务不引入把 source aliases 带入公共 artifact
  model 的 enum/helper。
- `runtime/linker/src/assembly_execution/service_error_index.rs`：保留 exact name、arity、owner identity
  比较，不增加 `bool|boolean`、normalization 或 fallback。
- stable/live artifacts：不原地修复、不重写旧 hash、不做 reader compatibility。

现有 compiler projection、HTTP projection和 runtime type-planning 中仍可搜索到若干
`"bool" | "boolean"` artifact-side tolerant branch。它们不是 canonical owner，也不能作为保留
noncanonical FileIR 的理由，也不属于关闭 F397 blocker 的最小 production owner。实现任务不得新增
任何这类 branch，并必须增加 emitted-artifact recursive invariant；后续 hard-cut cleanup若删除既有
branch，只能严格拒绝 alias，不得改成 reader normalization，也不得扩大到无关 runtime行为。

## Identity 影响与 rebuild DAG

canonical `boolean` → `bool` 会改变 immutable execution artifact，不允许保留旧 identity：

1. `std.db` FileIR payload 改变，因此 FileIR identity 改变。
2. std Package Local ABI preimage包含完整 `publicSymbols`
   (`artifact-identity/src/package_artifact.rs:28-35`；
   `package_artifact/projection.rs:24-50`)；公开 `ConflictError` descriptor 改变，因此 std Local ABI
   identity 改变。
3. std Package Build preimage包含 Local ABI identity、implementation symbols、FileIR refs、
   implementation links及 requirements
   (`artifact-identity/src/package_artifact.rs:37-59`；
   `package_artifact/projection.rs:53-145`)；因此 std Package Build ID 改变。
4. `std.db.ConflictError` PackageSchema canonical descriptor 已经是 `bool`，所以其
   PackageSchemaTypeId 和 std PackageSchemaIndex identity **应保持 bit-identical**。若改变，说明修复
   误触 schema owner。
5. 仅引用该 PackageSchema identity 的 ServiceContract operation/type requirement不因本修复改变；
   Service protocol identity也应保持稳定，除非另有真实 contract delta。
6. 所有直接依赖 std、其 `PackageRequirement.expected_local_abi` 指向旧 std ABI 的 package必须重新
   compile；requirement 本身进入 Package Build preimage。若 edge 设置
   `expected_package_build`，也必须更新 exact build。
7. downstream 只沿 identity-bearing edge 传播：
   - dependency Local ABI 改变；
   - exact package build pin 改变；
   - 或 caller public symbol实际嵌入已变化的 ABI expectation。
   普通 public dependency若 provider Local ABI保持不变且没有 build pin，不应无条件扩大为全生态重建；
   每层仍须重新判定 exact requirement。
8. 引用任何新 PackageArtifactRef 的 ServiceDeployment 必须重新 projection/assign deployment
   identity；RuntimeAssembly 的 resolved packages、code slots、package links和activation templates
   含 exact package build refs，必须重新组装并取得新 Assembly identity。

按 dependency-topological order 执行：

```text
compiler fix
  -> fresh std FileIR
  -> fresh std Local ABI + Package Build
  -> direct std consumer packages
  -> identity-bearing downstream consumers
  -> affected ServiceDeployments
  -> affected RuntimeAssemblies / activation generation
```

所有记录写成新 content-addressed artifacts；不 patch 旧 record，不伪造 identity，不使用
compatibility pointer。

## Implementation tests 与 focused commands

最小 test ownership：

1. compiler-core table test：
   - primitive 全表；
   - `bool` 与 `boolean` 同归 `bool`；
   - 18 个 compiler builtin 的 bare/symbol forms同归 `name`；
   - exact arity/kind不变、无 alias collision、unknown fail closed。
2. compiler source/lowering integration：
   - source alias 与 canonical spelling 在 nested record、nullable、union、function和 generic args 中
     产生相同 canonical builtin refs；
   - `ConflictError.retryable` FileIR 精确为 `bool`；
   - `String` 等非 source-table spelling 不被隐式接受。
3. artifact projection conformance：
   - 递归扫描 FileIR、implementation links、Local ABI、boundary projection和 PackageSchema；
   - 所有 overlapping builtin names exact 相等；
   - fresh std 中 `boolean` 为零，`bool` 存在；
   - schema type/index identities保持上述 fresh receipt。
4. linker strict negative：
   - 人工构造 FileIR `boolean` / schema `bool` 仍失败；
   - canonical `bool` / `bool` 成功；证明没有 linker alias。
5. fresh isolated std activation 与 representative package build，随后重跑 F397 isolated suite。

focused commands：

```bash
cargo test -p skiff-compiler-core prelude_registry
cargo test -p skiff-compiler-source prelude_registry
cargo test -p skiff-compiler --test builtin_canonical_spelling
cargo test -p skiff-compiler --test prelude_std_schema prelude_builtin_schema_is_typed_in_file_ir
cargo test -p skiff-compiler --test prelude_std_schema builtin_types_reach_the_package_boundary_projection
cargo test -p skiff-runtime-linker service_error_index
cargo test -p skiff-test-runner --test canonical_std_seed_bootstrap -- --test-threads=1
```

再用 fresh isolated artifact root 执行上文 fixture/package commands，并对所有 JSON 运行：

```bash
jq -r '.. | objects | select(.kind? == "builtin") | .name' <artifact-json>
```

验收必须证明：

- emitted FileIR/package artifact 中没有 `boolean` 或其它已声明 alias spelling；
- `std.db.ConflictError.retryable` 在 FileIR、linked type、Local ABI与 PackageSchema全部为 `bool`；
- PackageSchema IDs/index保持不变；
- fresh isolated activation越过 F397 的 ServiceErrorTypeIndex gate；
- linker strict mismatch test仍拒绝 noncanonical pair。

## 本次只读验证结果

通过：

```text
cargo test -p skiff-compiler-core prelude_registry
# 6 passed

cargo test -p skiff-compiler-source prelude_registry
# 21 passed

cargo test -p skiff-runtime-linker service_error_index
# 4 passed

cargo test -p skiff-compiler --test prelude_std_schema \
  prelude_builtin_schema_is_typed_in_file_ir
# 1 passed

cargo test -p skiff-compiler --test prelude_std_schema \
  builtin_types_reach_the_package_boundary_projection
# 1 passed
```

审计也运行了完整 `cargo test -p skiff-compiler --test prelude_std_schema`：9/10 通过，既有
`stream_type_is_explicitly_boundary_unavailable` expectation 失败。该 test 不含
`boolean`/`bool`、不改变上述 first-divergence 或 owner 判定；本只读任务没有修改或掩盖该独立
baseline failure。
