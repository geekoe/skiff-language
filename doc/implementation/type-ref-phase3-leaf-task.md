# Phase 3 叶子任务：逐对替换私有 type-ref 副本

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md`（已合入 main，
  baseline `126804ad784b92c3c8584d23e27fc0cd82bee48a`；Phase 3 条款见 §6 Phase 3 与 §5.2）。
- 直接父节点：Phase 1（`f9a745b0`，core 已含 5.2 全部纯操作）与 Phase 2（`5dc9bce8`，
  union 已统一到 core `normalize_union`），均已合入 main；Phase 2 验收 follow-up #1
  （normalize_union 注释溯源引用改“former private implementation”）纳入本任务第 10 项。
- 必读规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。

## 任务范围

按设计 §6 Phase 3 的 10 对清单逐对替换私有副本；每对独立 commit（commit message 标注
“Phase 3 第 N 对”），每 commit 行为等价（第 5 项 trm 调用点按设计获得
CatchResult/DbUpsertResult 分支，是设计明示的并集语义，必须出现在 diff 并有测试覆盖）。
禁止：改 artifact-model、identity/wire、lowering 的 `union_type_ir`、push、写 main、
承接其他阶段。

## 只读预检结论（锚定 baseline 126804ad，零 worktree）

1. `debug_text`：core（type_ref.rs:174）与 trm 私有（type_resolution_model.rs:5915）、
   etm 私有（expression_type_model.rs:5335）对全部 15 个变体输出逐字节一致。两私有副本唯一
   差异是 AppliedNominal base 格式化路径：trm 经 `nominal_base_type_ref`（6315，仅此一处
   消费 debug_text，其余 10+ 处消费方保留该 helper）再 `type_ref_debug_text`；etm 与 core
   直接 `nominal_base_debug_text`（etm 5527，仅 debug_text 消费，可删除）。两条路径对全部
   5 种 nominal base 输出相同串。差分对照可行，需临时对拍测试 + 记录证据。
2. `is_null_type`：core（488）与 trm `is_null_type_ir`（6136）、etm `type_ir_is_null`
   （5568）逐字相同。trm 消费点 1822/5865/5866/5872、shape_assignability 768；
   etm 消费点 5132/5561、expression_assignability（import 6、888、943、945，
   `type_ir_is_nullable` 保留自身语义，只换底层谓词）。
3. `generic_type_params`：core 尚无解析函数，需在 `compiler_core::type_syntax`（现有
   `pub use skiff_syntax::type_syntax::*` 模块）新增 `generic_type_parameter_names`
   （设计 §6 Phase 3 第 3 项明确要求 core 解析函数）。4 个私有副本（trm 5690、etm 4759、
   alias_resolution 545、prelude_registry/validation 353）与 source `shared::type_syntax`
   的 `generic_type_parameter_names`（3）行为逐字节一致（`split_top_level` 已 trim，
   alias_resolution 版无显式 trim 不影响结果）。同簇闭合：把实现移到 core，source shared
   改为 re-export，4 个私有副本删除，6 个消费点改调
   `generic_type_parameter_names`（etm 384/4649、trm 5598、alias_resolution 177、
   validation 170、callable_effects/analysis 209、contract_type_resolution/executables 71）。
   排除：lowering `executable_declaration_lowering.rs:40` 的同名函数签名不同
   （额外 `type_indices` map，语义不同），不在本簇。
4. `contains_type_param`：core（501）与 etm `type_contains_type_param`（4852）、
   trm `type_contains_unresolved_param`（6345）、projection
   `type_ref_contains_type_parameter`（interfaces.rs:111）逐分支相同。三个私有实现均删除，
   消费点改 core（etm 3410/3562/3614；trm 1641/1680；projection interfaces.rs:75）。
   projection 文件与 Phase 4 的转换拷贝（interfaces.rs:294）同文件但不同函数；预检确认
   Phase 4 agent 尚未启动，无并行写入冲突，按自主闭合纳入并在本文件记录。
5. `record_field_type`：core（265）= trm 版（Record/Union/Exception）+ etm 版
   （Record/Union/CatchResult/DbUpsertResult/Exception）并集，Union 合并走 canonical
   `normalize_union`。trm 消费点（shape_assignability 1207）从“无 CatchResult/DbUpsertResult
   分支”变为有（设计明示并集语义；需测试覆盖）；etm 保留薄包装
   （`record_field_type(..).as_ref().map(resolved_type_from_ir)`），etm 测试
   tests.rs:1301/1310 改调包装函数（行为不变）。
6. `single_item`：core（401）与 etm `single_for_item_type`（4778）名称/元数匹配完全一致
   （Array/Stream/std.collection.Array/std.stream.Stream 1 参；Map/std.collection.Map 2 参）。
   `single_for_item_projection`（4829）必须保留 `PackageTypeRef::Container` 前置匹配：
   `Local` 包装容器仍返回 None（设计 §5.2 与 Phase 4 第 5 步；本阶段测试锁定）。
   排除：lowering type_inference.rs:23/33/38 只匹配短名（File IR 语义），替换会扩大匹配，
   不在本簇。
7. `map_entry`：core（419）与 `map_entry_projections`（4844）、`map_key_type_ir`（5105）、
   `map_value_type_ir`（5097）一致（Map/std.collection.Map 2 参）。但 etm `map_entry_types`
   （4810）只匹配短名 `"Map"`（现状分叉）：包装器必须保留该行为（std.collection.Map → None），
   测试锁定；其余三者直接薄包装 core。排除 lowering suspend_analysis.rs:817 与
   source_rules/stream_emit/types.rs:422 的 &str 文本版（不同层，不在本簇）。
8. `function_callable_resolution`/`operation_callable_resolution`（trm 5658/5631）与
   `insert_function_signature`/`insert_operation_signature`（etm 4691/4718）：AST 两侧同用
   `Param`/`Option<TypeRef>` implicit_self/`TypeRef` return_type，可各收敛为一个参数化实现
   + 两个薄包装（跨文件输出类型不同，不能合并为单一函数；本项为一个 commit 内两处收敛）。
9. `package_type_resolution`（3080）/`_for_view`（3124）/`package_interface_fact`（3227）/
   `_for_view`（3271）：4 步回退链（direct → canonical refs → package_id → reversed deps）
   与 2 步链（direct → package_id）两种形状；收敛为一个带 map 参数 + full/view 标志的
   查找函数 + 4 个薄包装。`package_callable_resolution`（3208）与 view 变体逐字同构，
   同簇机械闭合为同一函数。注意 5564 的独立自由函数 `package_callable_resolution`
   （package callable 语义解析）同名不同物，不触碰。
10. `catch_result_branches`：core（440）与 etm `discriminated_record_branches`（5159）+
    `catch_result_branch_types`（5170）一致（`record_type_fields<N>` 与
    `BTreeMap::from` 构造相同 map）。`narrow_type_by_tag` 改调 core，删除两私有函数；
   其余 etm 形状函数（`array_item_type_ir` 5086、`stream_chunk_type` 4801）名称集合比
    core 窄，替换会扩大匹配，保留并记录为 non-blocking follow-up。core normalize_union
    注释（type_ref.rs:307）溯源引用改为 “former private implementation”。

## 写集（预期；实际以 commit diff 为准）

- `compiler/core/src/type_ref.rs`（第 10 项注释；第 3 项不在本文件而在
  `compiler/core/src/type_syntax.rs`）
- `compiler/core/src/type_syntax.rs`（第 3 项新增 `generic_type_parameter_names`）
- `compiler/core/src/type_ref/tests.rs`（如需补矩阵）
- `compiler/source/src/type_resolution_model.rs`（1/2/3/4/5/8/9）
- `compiler/source/src/expression_type_model.rs`（1/2/3/4/5/6/7/8/10）
- `compiler/source/src/expression_type_model/expression_assignability.rs`（1/2 import 与消费）
- `compiler/source/src/type_resolution_model/shape_assignability.rs`（1/2/4/5/9）
- `compiler/source/src/alias_resolution.rs`（3）
- `compiler/source/src/prelude_registry/validation.rs`（3）
- `compiler/source/src/shared/type_syntax.rs`（3，改 re-export）
- `compiler/source/src/callable_effects/analysis.rs`、`contract_type_resolution/executables.rs`
  （3，import/消费）
- `compiler/source/src/expression_type_model/tests.rs`、`type_resolution_model/tests.rs`
  （5/6/7 行为锁定与包装函数测试）
- `compiler/projection/src/package_artifact/export_links/public_instances/interfaces.rs`
  （4，`type_ref_contains_type_parameter` → core）
- `doc/implementation/type-ref-phase3-leaf-task.md`（本文件）

## 验证矩阵（每对 commit）

| 层级 | 命令 | 范围 |
| --- | --- | --- |
| 编译 | `cargo check -p skiff-compiler-source -p skiff-compiler-core -p skiff-compiler-projection` | 受影响 crate（按对裁剪） |
| 聚焦测试 | `cargo test -p skiff-compiler-core`（3/4）；`cargo test -p skiff-compiler-source`（每对，可加过滤）；`cargo test -p skiff-compiler-projection`（4） | 对应 crate |
| 格式 | `cargo fmt --all -- --check` | workspace |
| 差分证据 | 第 1 对：临时对拍测试（core vs trm vs etm，全 15 变体 + 5 nominal base + 嵌套组合），通过后记录证据再替换删除 | 见 commit 1 |
| 行为锁定 | 6/7：Local 包装容器 → None；std.* 全名矩阵（含 map_entry_types 短名-only 分叉） | etm tests.rs |
| 阶段自验收 | `node scripts/verify.mjs --only compiler,rust-quality` | 全部 10 对完成后 |

## 禁止与边界

- 不改 artifact-model/identity/wire；不动 lowering `union_type_ir` 与
  `executable_declaration_lowering.rs`/`type_inference.rs`/`suspend_analysis.rs` 私有副本
  （不同语义，排除）。
- 不 push、不合并 main；完成后把 branch/worktree/commit 清单交接
  `/root/skiff_integration_phase1` 并通知主 Agent。
- 遇到差分不一致或影响面超出设计时停止，返回 TASK_NOT_EXECUTABLE。
