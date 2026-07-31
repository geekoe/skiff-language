# Phase 1 叶子任务：core 层新增 TypeRef 纯操作与纯转换

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md` §5.2 目标 API + §6 Phase 1（commit `2431fd93c5c29daaa0997ac6430331a14aeb4aed`，reviewed v3；后经 `7c5a75e9e54aebcbd97321ae1d4f3379f838343e` 更新：明确 compiler-core 不依赖 `skiff_artifact_identity`，exact identity 用 `skiff_canonical_json::canonical_json_bytes`，输出与 `type_ref_abi_key` 逐字节一致）。
- 直接父节点：主 Agent `/root` 派发的 Phase 1 任务信封（`/root/skiff_dev_phase1`），及主 Agent 对依赖方向冲突的决策（批准最小闭合：不新增 artifact-identity 依赖，exact identity 直接用 canonical-json）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、`/Users/geek/workspace/multi-agent-development.md`。

## 任务范围（零消费变更）

在 `compiler_core::type_ref` 新增设计 §5.2 的纯操作与纯转换，配套单元测试；不改任何消费方。

### 写集

| 文件 | 操作 |
| --- | --- |
| `compiler/core/src/type_ref.rs` | 新增下述函数与 `BuiltinShape` |
| `compiler/core/src/type_ref/tests.rs` | 新增单元测试 |
| `doc/implementation/type-ref-phase1-leaf-task.md` | 本文件 |

`compiler/core/Cargo.toml` 与 `Cargo.lock` 不改动（主 Agent 决策：不新增 `skiff-artifact-identity` 依赖）。

### 禁止修改

- `compiler/source/src/type_resolution_model.rs`、`expression_type_model.rs` 及其子模块（继续使用私有副本）。
- `artifact-model` 三个 type enum（`TypeRefIr` / `PackageTypeRef` / `ContractTypeRef`）。
- wire 格式、identity 语义、任何消费方行为。

## 预检结论（零 worktree 只读，锚定 baseline `2431fd93`）

1. 入口确认：`compiler/core/src/type_ref.rs`（303 行）已有 `walk_type_ref` / `map_type_ref` / `substitute_type_params_in_type_ref` / `contains_*` / `type_ref_children`；测试布局为 `#[cfg(test)] mod tests;` + 同级 `type_ref/tests.rs`。
2. 依赖方向：实施验证中发现 `scripts/check-compiler-boundaries.mjs` 的 `compiler_core_no_forbidden_imports`（DENY）禁止 compiler-core 导入 `skiff_artifact_identity`（remove_when：compiler-core 仅含纯跨阶段支持）。已上报主 Agent；主 Agent 决策批准最小闭合：不新增依赖，exact identity 用 `skiff_canonical_json::canonical_json_bytes`（compiler-core 已依赖，输出与 `type_ref_abi_key` 逐字节一致）。设计文档已随 `7c5a75e9` 更新该约束。
3. `PackageTypeRef` / `ContractTypeRef` 由 artifact-model 根导出；`type_ref_abi_key` 的实现为 `skiff_canonical_json::canonical_json_bytes` 的 UTF-8 薄封装（`artifact-identity/src/semantic.rs`）。
4. 语义锚点：
   - `debug_text`：trm 6014 / etm 5339 逐字相同，仅 nominal base 格式化方式不同（trm 经 `nominal_base_type_ref` 中转，etm 直接 `nominal_base_debug_text`）；两者输出逐字节一致。core 版选择 etm 直接格式化实现，配测试锁定全部 nominal base 变体。
   - `normalize_union`：采用设计既定 canonical 语义 = trm `normalize_source_type_ref` + `normalize_source_union` + `collect_source_union_member`（递归 flatten + null 折叠 + sort/dedup，sort key 为 `debug_text`）。
   - `single_item`：etm `single_for_item_type` / `single_for_item_projection` 语义——Array/Stream/std.collection.Array/std.stream.Stream 且 1 参 → `args[0]`；Map/std.collection.Map 且 2 参 → `args[0]`（key）。
   - `map_entry`：etm `map_entry_types` / `map_entry_projections` 语义——Map/std.collection.Map 且 2 参 → `(args[0], args[1])`。
   - `exception_payload`：trm/etm `record_field_type_from_ir` 的 Exception 分支——`Exception` 且 1 参 → `args[0]`。
   - `catch_result_branches`：吸收 etm `discriminated_record_branches` + `catch_result_branch_types`（Phase 3.10 上移对象）——Union → items；CatchResult 2 参 → ok/err 两个 Record；Record → `vec![ty]`；其余 None。
   - `record_field_type`：trm/etm 两版并集——Record 字段、Union 递归合并（经 `normalize_union`）、CatchResult("tag")、DbUpsertResult("inserted"/"value")、Exception("error")。
   - `is_null_type`：两版相同——Builtin name=="null" 或 Literal Null。
   - `contains_type_param`：etm `type_contains_type_param` 全递归语义（trm 无对应私有实现，grep 确认）。
   - `package_type_ref_to_ir`（折叠）：type_projection `contract_type_ref_to_ir_from_package` / lowering `execution_type_ref` 的公共语义——Local 原样（不引入 etm `ordinary_package_local_type_ir` 重写，设计明确"折叠版不引入新重写"）；PackageSchema→PackageSymbol（PackageId + stable_schema_key，abi_expectation=None）；Container→Builtin；Nullable→Nullable；AnyInterface identity=`serde_json::to_string`。
   - `package_type_ref_to_ir_exact`（精确）：projection interfaces.rs `package_type_ref_to_ir` 语义——Local 原样；PackageSchema 保留；Container/Nullable 递归；AnyInterface identity=canonical JSON（`skiff_canonical_json::canonical_json_bytes` 内联，与 `type_ref_abi_key` 逐字节一致；不依赖 artifact-identity）。
   - `contract_type_ref_to_ir`：吸收 type_projection 双子（经 `package_type_ref_from_contract_type` 的等价直接映射，折叠策略）——Builtin→Builtin、Nullable→Nullable、AnyInterface→AnyInterface（serde_json identity）、PackageSchema→PackageSymbol、TypeParam→TypeParam、Record→Record、StructuralUnion→Union、Literal(String)→Literal(String)。
   - `BuiltinShape`：按设计枚举 16 变体；`of_name` 识别短名与 `std.*` 全名（`std.collection.Array` / `std.stream.Stream` / `std.collection.Map`，来自两文件 grep 快照）。
5. 无并行 ownership 冲突：当前协作树仅有本 Agent 与集成 Agent `/root/skiff_integration_phase1`。
6. baseline `2431fd93` 即主工作区 main HEAD，worktree 创建于 `/Users/geek/workspace/skiff-phase1`（分支 `impl/type-ref-phase1`）。

## 实现约束

- 新函数全部为 `pub`，位于 `compiler/core/src/type_ref.rs`；不导出到 lib 之外的新模块。
- `package_type_ref_to_ir_exact` 的 canonical identity 使用 `skiff_canonical_json::canonical_json_bytes`（私有 helper `canonical_json_key`）；不新增 `skiff-artifact-identity` 依赖（compiler-boundaries DENY）。
- clippy `too_many_lines`（阈值 534）与 rustfmt 无特殊配置，保持现有风格。
- 不改 `compiler/source` 任何文件。

## 验证命令（证据 owner：`/root/skiff_dev_phase1`）

```bash
cd /Users/geek/workspace/skiff-phase1
cargo test -p skiff-compiler-core type_ref
cargo fmt --check -p skiff-compiler-core
cargo clippy -p skiff-compiler-core --all-targets
node scripts/check-compiler-boundaries.mjs
```

注：clippy `-D warnings` 会因依赖 crate（syntax/artifact-model/artifact-identity）的既有 advisory lint 失败（baseline 同样复现，与本任务无关）；本 crate 自身无新 warning。边界检查器必须 DENY 通过。

提交要求：不提交 `.skiff-instance/`、`target/`、`node_modules/` 等忽略目录。

## 交接目标

完成提交后交接给集成 Agent `/root/skiff_integration_phase1`，并通知主 Agent `/root`。
