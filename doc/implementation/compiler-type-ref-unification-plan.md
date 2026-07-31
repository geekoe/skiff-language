# Compiler TypeRef 统一与 Source 类型模型拆分解构方案

日期：2026-07-31（审阅修订 v3，含 identity 兼容性决策）

状态：draft（尚未开始实现）

本文是阶段性实现方案，不是长期架构契约。目标是先把 compiler 里两套平行类型世界收敛成
“单一 canonical 类型 + 边界视图”，再按职责拆分 `compiler/source` 的两个 god 文件。
文件行数不是目标，抽象不足、代码重复、职责混杂才是要解决的问题。

## 1. 背景

`compiler/source/src/type_resolution_model.rs` 6533 行、
`compiler/source/src/expression_type_model.rs` 5605 行，是 Rust workspace 里最大的两个文件：

- `TypeResolutionModel` 一个结构体 20 个字段，混着索引构建、查询解析、interface conformance、
  assignability、依赖 ABI 五类职责。
- `OwnerChecker` 21 个字段、约 4000 行的 impl（648–4618 行），同时管 AST 遍历、DB typing、
  narrowing、call typing、assignability、test effects、object materialization、
  constructor validation 等至少八类关注点。
- 两个文件各自维护一份私有 type_ref 工具函数，存在逐字重复和近似重复；跨文件同一概念两份实现，
  且其中一份已经出现语义漂移。

两个文件旁边已经存在按职责抽取的子目录（`type_resolution_model/{catch_leaves,shape_assignability}`
、`expression_type_model/{contract_call_typing,db_projection,expression_assignability,
object_materialization}`），说明第一轮部分抽取已经完成，但 god impl 仍留在主文件里。
继续按职责拆而不是按长度拆。

## 2. Goals

- 建立单一 canonical 类型表示（`TypeRefIr`），所有类型遍历、格式化、字段提取、union 规范化、
  builtin shape 逻辑只写一份，落在共享层。
- `PackageTypeRef` / `ContractTypeRef` 收敛为边界视图；`ResolvedTypeRef` 收敛为 thin wrapper。
- 消除两个文件之间的逐字/近似重复，以及 crate 内同类重复。
- 修复同名 helper 的语义分叉（`union_type_ir`），用测试锁定唯一语义。
- 保留两种投影策略并明确边界归属：source 内部用折叠版（`PackageSchema→PackageSymbol`），
  ABI/export 边界用精确版（保留 `PackageSchema`）；`interface_abi_id` 统一到 canonical JSON，
  差分测试先行（见 Phase 4）。
- 类型层收敛后，再按职责拆分 `OwnerChecker` 与 `TypeResolutionModel` 的状态和 impl。
- 每个阶段可独立编译、测试、验收；除 union 语义对齐与 Phase 4 的 identity 序列化统一
  （已确认：不兼容历史 identity 串）外，其余阶段行为等价。

## 3. Non-Goals

- 不合并、删除或重命名 `artifact-model` 的三个 type enum；不动它们的序列化格式和 wire 格式。
  identity hashing 语义默认不动；Phase 4 的 identity 序列化统一（`serde_json` → canonical JSON）
  是唯一例外。用户已确认不需要兼容历史 identity 串（2026-07-31，skiff 尚未发布），差异经
  差分测试列明后直接更新 fixture golden，单独 commit。
- 不改变 `PackageTypeRef` 在 ABI / contract 边界的角色；它仍是子集视图，不是第二份递归实现。
- 不把 `shared::ast::TypeRef`（源码语法树输入）纳入统一；它是 parse 层输入，不是编译期类型模型。
- 不为了过行数门禁机械拆分文件；行数下降是职责拆分和重复消除的副产品。
- 不新增 legacy / compatibility adapter、dual path 或 fallback 来维持中间态可运行性。
- 不把 5 份 `PackageTypeRef → TypeRefIr` 拷贝压成单一函数；保留折叠/精确两种策略，按边界归属收敛。

## 4. Current Evidence

### 4.1 体量与职责

| 文件 | 体量 | 关键结构 |
| --- | --- | --- |
| `compiler/source/src/type_resolution_model.rs` | 6533 行 | `TypeResolutionModel` 20 字段；`impl TypeResolutionModel` 约 3410 行；`ResolvedTypeRef` 定义于此 |
| `compiler/source/src/expression_type_model.rs` | 5605 行 | `OwnerChecker` 21 字段；`impl OwnerChecker` 约 3970 行；10 个 >100 行函数，最长 `check_expr_with_field_diagnostics` 499 行 |

`TypeResolutionModel` 的 20 个字段可明确分组：

- 索引构建（约 12 个）：`modules`、`source_types`、`source_interfaces`、`package_types`、
  `package_callables`、`package_constants`、`package_interfaces`、`package_type_slots`、
  `package_type_source_paths`、`package_aliases`、`external_type_symbols`、`local_impl_methods`。
- 依赖与 ABI（4 个）：`package_dependencies`、`package_dependency_views`、
  `package_dependency_canonical_refs`、`package_artifact_identities`。
- interface conformance（2 个）：`interface_semantics`、`interface_conformances`。
- assignability（1 个）：`package_public_to_internal`。
- service schema（1 个）：`service_api_schemas`。

### 4.2 具名重复清单（均已核对）

| 重复对 | 位置 | 性质 |
| --- | --- | --- |
| `type_ref_debug_text` ×2 | trm 6014 / etm 5339 | 两份都是 72 行，71 行逐字相同，唯一差异是 nominal base 的格式化调用方式 |
| `record_field_type_from_ir` ×2 | trm 5895 / etm 4925 | 同名同概念；共享 Record/Union 逻辑逐字相同，但 etm 版额外处理 `CatchResult`/`DbUpsertResult` 并包装 `ResolvedTypeRef`，语义已分叉 |
| `single_for_item_type` / `single_for_item_projection` | etm 4776 / 4827 | 同一逻辑按 `ResolvedTypeRef` / `PackageTypeRef` 各写一遍 |
| `map_entry_types` / `map_entry_projections` | etm 4808 / 4842 | 同上，双表示副本 |
| `map_key_type_ir` / `map_value_type_ir` | etm 5094 / 5102 | 同一形状逻辑的两种取法 |
| `function_callable_resolution` / `operation_callable_resolution` | trm 5658 / 5631 | 只差 AST 节点类型和字段名 |
| `insert_function_signature` / `insert_operation_signature` | etm | 同上 |
| `package_type_resolution` / `package_interface_fact` | trm 3080 / 3227 | 回退查找逻辑完全同构，只差 map 与返回类型；各有一份 `_for_view` 变体（3124 / 3271） |
| `generic_type_params_from_text` / `generic_type_params` | trm 5690 / etm 4757 | 逐字相同（仅函数名与 `crate::` 前缀差异） |
| `is_null_type_ir` / `type_ir_is_null` | trm 6236 / etm 5573 | 9 行完全相同，只差函数名 |
| `union_type_ir` ×2 | trm 5915 / etm 5202 | 同名但语义分叉：trm 版递归 flatten + null 折叠 + sort/dedup；etm 版只做顶层 sort/dedup |

块级扫描还发现 ≥6 行精确重复块 114 组（其中 ≥8 行几十组、聚类后约十几组内容簇）。
具体“8 组逐字 / 14 组 0.9+”的数字取决于扫描粒度，但重复存在且不止于上述清单。

### 4.3 更深层证据

- 双表示转换自身也在复制：`PackageTypeRef → TypeRefIr` 转换在 compiler 里出现 5 次，且不是
  同一策略（见下表）。4 份把 `PackageSchema` 折叠成 `PackageSymbol`，1 份（projection export
  links）保留精确 `PackageSchema`；etm 版还会递归重写 `Local` 内部；`AnyInterface` identity
  有两套生成方式。因此 Phase 4 必须保留"折叠/精确"两个函数，禁止单一函数吸收全部 5 份。

  | 位置 | 层/用途 | PackageSchema | Local 内部 | AnyInterface identity |
  | --- | --- | --- | --- | --- |
  | etm `package_type_ref_ir` 5420 | source：表达式 typing 的 contract 输出 | 折叠 | 递归重写 PackageSchema→PackageSymbol | `serde_json::to_string` |
  | `contract_call_typing/type_projection.rs` `contract_type_ref_to_ir` 261 | source：ContractTypeRef→IR | 折叠 | 原样 | `serde_json::to_string` |
  | `contract_call_typing/type_projection.rs` `contract_type_ref_to_ir_from_package` 303 | source：PackageTypeRef→IR | 折叠 | 原样 | `serde_json::to_string` |
  | `projection/.../public_instances/interfaces.rs` `package_type_ref_to_ir` 294 | ABI identity / export links | 保留精确 | 原样 | canonical JSON（`type_ref_abi_key`，按键排序） |
  | `lowering/executable_type_projection.rs` `execution_type_ref` 5 | File IR 执行表示 | 折叠 | 原样 | `serde_json::to_string` |

- `AnyInterface.interface_abi_id` 两套生成方式输出并不必然相同：canonical JSON 按键排序，
  `serde_json::to_string` 按字段声明序。对带参数 Builtin 等多字段 IR，两者产生不同 identity 串；
  收敛前必须先差分测试（见 Phase 4）。
- `TypeRefIr → PackageTypeRef` 是有损、依赖上下文的投影：需要 `SourceDependencyAnalysisInput`
  解析 `ServiceSymbol`/`PackageSymbol`。其中 `PackageSymbol` 查找失败回退 `PackageTypeRef::Local`；
  `ServiceSymbol` 解析失败、以及 `Record`/`Function`/`Union` 内嵌 contract symbol 时直接
  `Err`，不是统一回退 Local。且 `Record`/`Function`/`Literal` 在 `PackageTypeRef` 没有对应变体，
  因此不能把三个 enum 简单合并。
- 反向投影已有实现：`contract_call_typing/type_projection.rs` 的
  `package_type_ref_from_resolved_ir`（118 行）已包含上述解析与 Local 兜底，Phase 4 是迁出合并，
  不是新写。
- 重复溢出到 crate 其他位置：`root_refs/mod.rs` 的 `visit_stmt` 两份（0.933）、
  `callable_effects/transfer.rs` 与 `execution_semantics/collectors.rs` 的 `pattern_bindings`
  （0.933）、`contract_call_typing` 双表示转换（0.902）、`expression_model.rs` 的
  `assert_no_remaining_block_children` / `assert_no_remaining_stmt_blocks`（0.900）。
- builtin 名字符串字面量在两个文件里按常见名字统计约 47 处，另有 16 处 `std.*` 全名
  （`"Array"`/`"Stream"`/`"Map"`/`"Exception"`/`"CatchResult"`/`"DbUpsertResult"`/
  `"Json"`/`"JsonObject"`/`"null"`/`"void"` 等，含构造点与匹配点；数字随统计口径浮动，
  Phase 6 开工前以 rg 快照为准），没有任何 shape 抽象。
- 行数门禁按“当前最差文件”反推：`scripts/check-rust-file-lines.mjs` 的
  `MAX_FILE_LINES = 6533` 恰好等于 `type_resolution_model.rs` 行数；
  `clippy.toml` 的 `too-many-lines-threshold = 534`，而 `check_expr_with_field_diagnostics`
  已经 499 行。8336c2ad 已把测试搬出源文件，测试文件又长成新的巨块
  （`callable_effects/tests.rs` 4839 行）。机械拆分只会把峰值搬到下一个文件。
- `compiler_core/src/type_ref.rs` 已有共享层雏形（`walk_type_ref`、`map_type_ref`、
  `substitute_type_params_in_type_ref`、`contains_*`），但格式化、字段提取、union 规范化、
  builtin shape 谓词都不在层里，导致消费方各自私有复制。`compiler/projection` 与
  `compiler/lowering` 均已依赖 `compiler_core`，转换下沉后三个 crate 可共享。

## 5. 目标形态

### 5.1 单一 canonical + 视图

不新建第三个递归类型。`TypeRefIr`（artifact-model，15 个变体）已经是三个表示里的超集，作为唯一
canonical；其余表示降级为视图：

```text
artifact-model
  TypeRefIr          canonical，唯一递归类型，所有逻辑的事实来源
  PackageTypeRef     ABI / wire 子集视图（Container/Nullable/AnyInterface/PackageSchema/Local）
  ContractTypeRef    contract descriptor 子集视图

compiler_core::type_ref
  纯操作 + 纯转换（walk/map/substitute 已有，新增 debug/field/normalize/shape/转换）
        ▲
compiler/source
  上下文相关投影（TypeRefIr → PackageTypeRef 需要 DependencyAnalysis）
  消费方（OwnerChecker、TypeResolutionModel 等）
```

注意 `PackageTypeRef::Local` 内嵌完整 `TypeRefIr`，"子集视图"仅对
Container/Nullable/AnyInterface/PackageSchema 成立。

依赖方向固定为 `artifact-model ← compiler_core ← compiler/source`。
`compiler_core` 已经依赖 `artifact-model`；但 `SourceDependencyAnalysisInput` 在
`compiler/source`，core 不能反向依赖，因此上下文相关投影留在 source crate 的单一模块。

### 5.2 目标 API（`compiler_core::type_ref` 扩展面）

```rust
// 纯操作——收编两个文件里的私有副本
pub fn debug_text(ty: &TypeRefIr) -> String;                   // 吸收两处 type_ref_debug_text
pub fn record_field_type(ty: &TypeRefIr, field: &str) -> Option<TypeRefIr>;
pub fn normalize_union(ty: TypeRefIr) -> TypeRefIr;            // 唯一语义，见 Phase 2
pub fn single_item(ty: &TypeRefIr) -> Option<&TypeRefIr>;      // Array/Stream/Map item
pub fn map_entry(ty: &TypeRefIr) -> Option<(&TypeRefIr, &TypeRefIr)>;
pub fn exception_payload(ty: &TypeRefIr) -> Option<&TypeRefIr>;
pub fn catch_result_branches(ty: &TypeRefIr) -> Option<Vec<TypeRefIr>>;
pub fn is_null_type(ty: &TypeRefIr) -> bool;                   // 吸收 is_null_type_ir/type_ir_is_null
pub fn contains_type_param(ty: &TypeRefIr) -> bool;

// builtin shape 抽象——替换两文件中的名字字面量（口径见 Phase 6）
pub enum BuiltinShape {
    Array, Stream, Map, Exception, CatchResult, DbUpsertResult,
    Json, JsonObject, Null, Void, Never, Unknown,
    String, Integer, Number, Bool, // json assignability 等处的原始类型匹配
}
impl BuiltinShape {
    pub fn of_name(name: &str) -> Option<BuiltinShape>;
}

// 纯转换（context-free，可以进 core）
pub fn package_type_ref_to_ir(ty: &PackageTypeRef) -> TypeRefIr;         // 折叠策略：PackageSchema→PackageSymbol（source 内部）
pub fn package_type_ref_to_ir_exact(ty: &PackageTypeRef) -> TypeRefIr;   // 精确策略：保留 PackageSchema，identity 用 canonical JSON（ABI/export 边界）
pub fn contract_type_ref_to_ir(ty: &ContractTypeRef) -> TypeRefIr;       // 吸收 type_projection 双子
```

5 份拷贝不是同一策略（见 4.3 表），因此不提供"单一函数吸收 5 份"：
- source 内部（etm、type_projection、lowering 执行表示）用折叠策略；`Local` 内部除 etm 现有
  `ordinary_package_local_type_ir` 重写外均原样，折叠版不引入新重写。
- ABI/export 边界（projection export links）用精确策略；`interface_abi_id` 统一以 canonical
  JSON（`type_ref_abi_key`）为准，变更需差分测试 + fixture golden（见 Phase 4；已确认不需
  兼容历史 identity 串）。注意 compiler-boundaries 规则禁止 compiler-core 依赖
  `skiff_artifact_identity`；core 内直接用 `skiff_canonical_json::canonical_json_bytes`
  （输出与 `type_ref_abi_key` 逐字节一致），不引入该依赖方向。
- `single_for_item_projection` 不能假设"无损包回"：`from_ir` 对 Record/Function/Literal 有损，
  且 `Local` 包装的容器现有行为是返回 `None`。改接 `single_item` 时用测试锁定该行为。

反向投影保持 `Result`：

```rust
// compiler/source/src/type_projection.rs（迁出合并 contract_call_typing/type_projection.rs
// 已有的 package_type_ref_from_resolved_ir；错误分支与 Local 兜底逐一保留）
pub fn package_type_ref_from_ir(
    ty: &TypeRefIr,
    dependency_analysis: &SourceDependencyAnalysisInput,
) -> Result<PackageTypeRef, String>;
```

### 5.3 `ResolvedTypeRef` 的收敛路径

```rust
pub struct ResolvedTypeRef { ir: TypeRefIr, source_text: String }  // 现状

// 第一步：集中构造入口，行为不变
impl ResolvedTypeRef {
    pub fn new(ir: TypeRefIr) -> Self;                 // source_text = debug_text(&ir)
    pub fn with_text(ir: TypeRefIr, text: String) -> Self; // 保留少数手拼文本
}

// 第二步：读点迁移完成后删字段
pub struct ResolvedTypeRef(TypeRefIr);                 // impl Display = debug_text
```

`source_text` 不能直接删：当前构造点约 66 处（排除测试；含测试约 70）、`.source_text` 读点
约 64 处（含测试约 73；全仓库 87），数字随近期提交漂移，开工前以 rg 快照为准。其中大部分是
`type_ref_debug_text(&...)`，但也有手拼的 `format!("Stream<{}>", ...)`、`"Json"`、
`String::new()`，测试里还有 `"otherRole.LlmRole"`、`"{ value: string }"` 等断言字符串。
注意 `CanonicalInterfaceSelectorResolution`（type_resolution_model.rs:188）也有独立的
`pub source_text` 字段，删字段范围仅限 `ResolvedTypeRef`。先统一构造入口、再迁移读点、
最后删字段，保证诊断输出逐字节不变。

## 6. 分阶段实施

每个阶段独立提交、独立验证；上一阶段验收通过后才细化下一阶段。

### Phase 1：core 层新增 API（零消费变更）

在 `compiler_core::type_ref` 增加 5.2 中的纯操作与纯转换，配套单元测试；两个文件继续使用私有副本。
行为零变化。

验证：`node scripts/verify.mjs --only compiler,rust-quality`。

### Phase 2：union 规范化语义对齐（允许行为变化的步骤之一）

先写差分测试枚举输入（嵌套 union、带 null 的 union、`Nullable` 套 union、空 union、
Record/Literal/Function/AnyInterface 成员、重复成员、`PackageTypeRef::Local` 包装的容器），
对比 trm 递归版与 etm 顶层版的输出差异，选定 canonical 语义
（当前倾向以 trm 版为准：递归 flatten + null 折叠 + sort/dedup，更接近语言语义；最终以现有
测试期望为准）。然后两个文件统一调用新 `normalize_union`，删除私有实现。

验证：差分测试 + `node scripts/verify.mjs --only compiler,rust-quality` + `--only skiff-tests`
（union 语义影响 `.skiff` 编译结果）。

### Phase 3：逐对替换私有副本

每替换一对一个 commit，每个 commit 都保持行为等价（Phase 2 已把唯一语义分歧修掉）：

1. `type_ref_debug_text` ×2 → `debug_text`（先做 core 版与两个私有副本的逐输入差分对照，
   确认 nominal base 输出一致，以测试为 golden）。
2. `is_null_type_ir` / `type_ir_is_null` → `is_null_type`。
3. `generic_type_params_from_text` / `generic_type_params` → core 解析函数。
4. `contains_type_param` 相关私有实现 → `contains_type_param`。
5. `record_field_type_from_ir` ×2 → `record_field_type` + `CatchResult`/`DbUpsertResult` shape 分支。
6. `single_for_item_type` / `single_for_item_projection` → `single_item` + 两个薄包装。
7. `map_entry_types` / `map_entry_projections` / `map_key_type_ir` / `map_value_type_ir`
   → `map_entry`。
8. `function_callable_resolution` / `operation_callable_resolution` 与
   `insert_function_signature` / `insert_operation_signature` → 一个参数化实现。
9. `package_type_resolution` / `package_interface_fact`（含 `_for_view` 两对）→
   一个带 map 参数的查找函数。
10. 把 `catch_result_branch_types`、`discriminated_record_branches` 等 etm 私有形状函数
    上移到 core shape 层。

验证：每步 `cargo check` + 聚焦测试 + rustfmt；Phase 3 全部完成后跑
`node scripts/verify.mjs --only compiler,rust-quality`。

### Phase 4：转换收敛（先决策，后合并）

1. identity 差分测试先行：枚举带参数 Builtin、PackageSymbol/PackageSchema 等输入，对比
   `serde_json::to_string` 与 `type_ref_abi_key`（canonical JSON）输出；差异清单作为 golden。
   决策（2026-07-31 已确认）：统一以 canonical JSON 为 `interface_abi_id` 唯一算法；与现状
   `serde_json` 输出的差异不需要兼容历史（skiff 尚未发布），差分测试列明差异后直接更新
   fixture golden，单独成 commit（Non-Goals 第 3 条的唯一例外）。
2. core 提供 `package_type_ref_to_ir`（折叠）与 `package_type_ref_to_ir_exact`（精确）两个纯函数，
   按 4.3 表的边界归属替换 5 份拷贝；etm 的 `ordinary_package_local_type_ir` 行为保持现状。
   exact 版的 canonical identity 在 core 内用 `skiff_canonical_json::canonical_json_bytes`
   实现（compiler-boundaries 禁止 core 依赖 `skiff_artifact_identity`；输出与
   `type_ref_abi_key` 逐字节一致）。
3. `contract_type_ref_to_ir` 与 `contract_type_ref_to_ir_from_package` 合并为直连
   `ContractTypeRef → TypeRefIr` 的一版，identity 按第 1 步决策执行。
4. 反向投影：把 `contract_call_typing/type_projection.rs` 的 `package_type_ref_from_resolved_ir`
   迁出合并到 `compiler/source/src/type_projection.rs`（新文件，内容来自现有实现）；`ServiceSymbol` 失败 Err、
   `PackageSymbol` 查找失败回退 Local、Record/Union/Function 内嵌 contract symbol Err
   三个分支逐一保留并加测试。
5. `single_for_item_projection`/`map_entry_projections` 改接 `single_item`/`map_entry` 后，
   对 `Local` 包装容器的现有 `None` 行为用测试锁定。

验证：identity 差分测试 + compiler 域测试 + contract fixture 测试 + `--only skiff-tests`。

### Phase 5：`ResolvedTypeRef` 瘦身

按 5.3 推进：先加 `new`/`with_text` 并替换构造点（当前约 66 处，含测试约 70），再迁移
`.source_text` 读点（当前约 64 处，含测试约 73）到 `Display`，最后删 `source_text` 字段。
范围仅限 `ResolvedTypeRef`，不动 `CanonicalInterfaceSelectorResolution`。诊断断言（测试文件里的
`expected.source_text == "..."`）是 golden 基线，任何输出变化都要先解释再接受。

验证：compiler 域测试 + rustfmt + clippy；重点跑 diagnostics 相关 fixture；merge 前补
`--only skiff-tests`。

### Phase 6：builtin shape 替换

用 `BuiltinShape::of_name` 替换两个文件里的名字字面量（约 47 处匹配/构造点 + 16 处 `std.*`
全名，数字以开工前 rg 快照为准）。`of_name` 同时识别 `std.collection.Array`/
`std.stream.Stream` 等全名。放在 Phase 3 之后，因为 `single_item`/`map_entry`/`catch_result`
收编后匹配已集中。

验证：compiler 域测试；验收限定范围：目标文件中除 `BuiltinShape` 定义、shape 构造器与
显式序列化点外，`rg '"Array"|"Stream"|"Map"|"Exception"|"Json"|"CatchResult"|"null"|"void"'`
应 0 命中。

### Phase 7：状态拆分（职责，不是行数）

类型层收敛后，`OwnerChecker` / `TypeResolutionModel` 的拆分才是纯职责移动：

- `OwnerChecker` 的八个输出集合收敛为 `CheckOutputs` 结构体；`build()` 与 `check_source()`
  的长参数列表改为传 `CheckOutputs`。
- `OwnerChecker` 的 impl 按关注点拆进子模块（narrowing、db typing、call typing、
  assignability、test effects、materialization），参照 `contract_type_resolution/` 的
  callables/types/interfaces/validation 模式（主文件 321 行）。
- `TypeResolutionModel` 拆成 index 与 query 两个状态面；索引构建相关字段和方法与查询/解析
  方法分离。
- 超长方法（`check_expr_with_field_diagnostics` 499 行、`check_stmt` 448 行、
  `call_type` 440 行等）按 match 分支提取，而不是包一层函数。

主文件压到 1000 行内是这一阶段的可能副产品，不是验收标准。

## 7. 验证与验收

- 每个阶段：`node scripts/verify.mjs --only compiler,rust-quality`（compiler 测试 +
  boundaries + rustfmt + 文件行数门禁；注意 `rust-quality` 不含 clippy，需要时单独跑
  `cargo clippy`）。
- Phase 2 额外：`node scripts/verify.mjs --only skiff-tests`。
- Phase 4 额外：identity 差分测试 + contract fixture / boundary 相关测试
  （`contract_dependency_test_fixture`、`contract_call_typing` 测试）+ `--only skiff-tests`。
- Phase 5 merge 前：diagnostics fixture + `--only skiff-tests`。
- 重复收编的验收：目标私有函数名在两个文件里 `rg` 应为 0 命中（除薄包装）。
- 语义等价的验收：除 Phase 2 的差分测试外，任何诊断文本或类型输出变化都必须先出现在
  diff 里并有测试覆盖，不允许静默变化。
- 合并与 push：按阶段顺序逐个合并本地 `main`，每阶段独立 commit 并保持全绿；Phase 2、4
  的 commit 可单独回退。push 等用户明确要求。

## 8. 风险与边界

- `TypeRefIr → PackageTypeRef` 是有损投影：`ServiceSymbol` 失败与 contract-symbol 内嵌走
  `Err`，`PackageSymbol` 查找失败回退 `Local`，各分支必须保留，文档写明这是子集投影而不是
  互逆转换。
- 5 份 forward 拷贝存在三种策略差异（Local 重写、PackageSchema 折叠或保留、identity 序列化），
  Phase 4 保留折叠/精确两个函数；禁止单一函数吸收全部 5 份。
- `AnyInterface.interface_abi_id` 存在 canonical JSON 与 serde_json 两套输出；统一到 canonical
  是 identity 语义变更，必须先差分测试 + fixture golden；用户已确认不需要兼容历史 identity 串，
  但仍不能混进等价重构，单独 commit。
- `single_for_item_projection`/`map_entry_projections` 对 `Local` 包装容器的 `None` 行为
  必须测试锁定；from_ir 对 Record/Function/Literal 有损。
- `source_text` 存在手拼文本与测试断言依赖，删除字段前必须先统一构造入口；范围仅限
  `ResolvedTypeRef`，勿误伤 `CanonicalInterfaceSelectorResolution`。
- union 规范化语义选择会影响编译结果，必须单独成阶段、单独跑 skiff-tests，
  不能混进“等价重构”步骤。
- 行数门禁与 clippy 阈值都由当前最差文件决定（6533 / 534），机械拆分只会搬家；
  本方案的验收只看职责与重复是否消失。
- `artifact-model` 的 enum 定义和 wire 格式不动；identity 序列化统一是本方案唯一允许的
  identity 语义变更，需单独 commit（已确认：不需要兼容历史 identity 串）。第一阶段只加函数，
  不改类型。
