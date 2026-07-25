# P5-F320 Representation carrier combined probe result

状态：BLOCKED（representation targeted chain PASS；`skiff-runtime-eval --lib`仍被授权外fixture
漂移阻断）。

证据基线：`ab639e5f9b1e1b18125fdff982af364836b2c440`。

交付提交：本结果文档所在提交；精确HEAD由任务回报给出。

## 结果

- F318记录的五处首错已机械关闭：
  - `test_effect_registry.rs`为现有snapshot测试补入
    `runtime_to_wire_required_plan`的test-only import；
  - `ordinary/tests.rs`删除一处旧`TypeDeclIr.discriminator`与两处旧
    `PackageCallableSignature.throw_types`；
  - `spawn_ops.rs`删除一处旧`LinkedTypeDeclIr.discriminator`。
- 新增独立combined probe
  `assembly_execution::ordinary::tests::representation_combined_probe::
  compiler_wrap_continues_through_file_ir_linking_and_eval`：
  - 通过production package compiler从真实source constructor产生一个
    `ExprIr::RepresentationWrap`；
  - payload call在File IR中只出现一次，wrap精确引用已lowered child；
  - hydrated Package schema进入canonical linker后，target精确解析为同一type index的
    `Package(0) / LoadedFileIndex / TypeAddr`，child ref不变；
  - linked executable由assembly eval真实执行，raw value仍为`"payload"`。
- 没有修改F315/F316/F318 production、artifact/model/generation、compiler、linker、
  linked-program、linked-type-plan、request/host/transport/router/std或权威设计。

## 条款矩阵

| 条款 | 证据 | 结果 |
| --- | --- | --- |
| source constructor产生required wrap，payload一次 | 新combined probe检查一个真实`ExprIr::RepresentationWrap`、一个`ExprIr::Call`及child index；lowering selector中的`explicit_representation_constructors_preserve_wraps_order_and_throw_site` | PASS |
| generic/nested/external owner与ordered arguments | lowering的`representation_wrap_preserves_external_package_owner_in_ordered_arguments`；linked-program representation tests；linker representation-wrap conversion/assembly tests；linked-type-plan representation tests | PASS |
| eval raw不变，carrier为exact outer representation | 新combined probe真实执行后raw值不变；`representation_wrap_consumer`的`wrap_preserves_raw_value_and_replaces_only_the_outer_identity`与external ordered-argument test | PASS |
| direct throw/catch exact identity；其它nominal/argument miss | `representation_wrap_consumer::wrapped_throw_catch_and_rethrow_keep_the_actual_identity_and_exception_state`及`exceptions::fully_instantiated_generic_identities_are_exact_and_fail_closed` | PASS（targeted）；full eval gate另有fixture blocker |
| named-union只按目标上下文与exact concrete branch提升 | `representation_wrap_consumer::named_union_context_promotes_only_the_exact_concrete_nominal` | PASS |
| required throw site保留source/synthetic site，wrap不造site | lowering的`explicit_representation_constructors_preserve_wraps_order_and_throw_site`；linker的source/synthetic throw site tests；consumer的exception state test | PASS |
| wrong kind/arity/unresolved owner/payload conflict fail closed | linker `representation_wrap_rejects_every_non_representation_declaration_kind`与`representation_wrap_rejects_wrong_arity_owner_and_residual_type_params`；consumer `wrap_fails_closed_for_wrong_plan_missing_identity_and_payload_conflict` | PASS |
| generation保持v8/v6/v8，无旧generation恢复 | `source_file_lowering.rs`断言`skiff-file-ir-v8`和`skiff-file-ir-format-v6`；`file_ir/identity.rs`断言`skiff-file-ir-v8:sha256:`；lowering内旧v7/v5与`_erased_wrapper_type`反搜为零 | PASS |
| 无shape/display/static throw fallback、隐式wrap或compat path | lowering non-representation/implicit-wrap负例、linked applied-nominal side-channel负例及eval exact identity负例均通过 | PASS |

## Fixture反搜

`rg -n 'throw_types:|discriminator:'`在三份授权fixture中均为零：

- `runtime/eval/src/test_effect_registry.rs`：零；
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`：零；
- `runtime/eval/src/spawn_ops.rs`：零。

这些删除只覆盖已移除的旧构造字段；没有恢复兼容默认或删除其它现行DTO的同名真实字段。

## 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-lowering --lib -- --list` | PASS，52 tests，非零 |
| `cargo test -p skiff-compiler-lowering --lib --no-fail-fast` | PASS，52/52 |
| `cargo test -p skiff-runtime-linked-program --lib --no-fail-fast` | PASS，34/34 |
| `cargo test -p skiff-runtime-linker --lib --no-fail-fast` | PASS，45/45 |
| `cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast` | PASS，17/17 |
| `cargo test -p skiff-runtime-eval --test representation_wrap_consumer --no-fail-fast` | PASS，6/6 |
| `cargo test -p skiff-runtime-eval --lib --no-fail-fast assembly_execution::ordinary::tests::representation_combined_probe::compiler_wrap_continues_through_file_ir_linking_and_eval -- --exact` | PASS，1/1 |
| `cargo test -p skiff-runtime-eval --lib -- --list` | PASS，154 tests，非零 |
| `cargo test -p skiff-runtime-eval --lib --no-fail-fast` | BLOCKED，131/153通过、22失败 |
| targeted `rustfmt --check` | PASS |
| `git diff --check` | PASS |

完整eval在fixture closure之后、新增test-only combined probe之前运行；失败全部进入测试执行而非编译。
随后只新增并单独验证combined probe，最终list为154。按任务要求没有重复运行完整eval，也没有继续运行其后的
更宽gate；没有运行workspace/root/stable/live。

## 剩余blocker

完整eval的22个失败分成三类，均不是representation production回归：

1. 19个fixture没有向`HydratedPackageCode`提供required Package schema index，统一在
   `runtime/eval/src/test_support.rs:31`以`MissingHydratedSchemaIndex`失败：
   - ordinary：
     `inline_effect_request_finalization_reports_and_clears_unused_setup`、
     `inline_effect_response_is_materialized_in_spawned_stream_producer_heap`、
     `inline_effect_setup_dispatch_reports_request_subset_mismatch`、
     `inline_effect_stream_is_consumed_in_buffered_event_order`、
     `inline_effect_typed_throw_is_caught_by_exact_linked_nominal_type`、
     `object_materialization_interpreter_heap_shape_distinguishes_construct_and_map_literal`、
     `package_constant_load_resolves_exact_dependency_implementation_address`、
     `package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation`；
   - projection：
     `assembly_database_type_and_recoverable_views_use_the_execution_image`、
     `assembly_database_type_view_rejects_missing_type_information`、
     `assembly_execution_projection_never_falls_back_to_legacy_service_units`、
     `assembly_execution_projection_resolves_image_owned_lookup_matrix`、
     `builtin_only_registered_errors_remain_native_without_package_guessing`、
     `canonical_assembly_resolves_every_std_package_error_address_to_its_builtin_identity`、
     `canonical_assembly_std_error_resolution_is_exact_and_nominal`；
   - spawn：
     `canonical_spawn_missing_execution_projection_fails_before_actor_capability`、
     `canonical_spawn_missing_metadata_fails_before_actor_capability`、
     `canonical_spawn_rejects_metadata_target_not_matching_linked_symbol`、
     `canonical_spawn_uses_admitted_projection_and_submits_exact_function_target`。
2. 两个source-inline effect fixture在
   `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs:54`及`:105`
   构造canonical std seed时，被四个generic websocket public schema declaration拒绝：
   `source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable`与
   `source_inline_service_effect_sequence_typed_throw_is_caught_then_responds`。
3. `test_effect_registry::tests::typed_throw_clones_the_exact_carrier_into_the_request_heap`
   在`runtime/eval/src/test_effect_registry.rs:588`仍断言两个独立heap的数值handle不相等；
   当前两边都合法分配为`HeapHandle { index: 0, generation: 0 }`。

这些是新暴露的同类fixture漂移或独立fixture断言，不在F320允许的五处机械closure内，因此只记录并停止，
不扩大写入范围修复。representation high-risk acceptance仍需等待上述eval lib gate blocker另行关闭。
