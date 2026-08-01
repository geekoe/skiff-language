# Callable effects 大型测试模块重构：开发叶子合同

日期：2026-08-01

状态：in progress

直接父节点：[`rust-large-test-module-refactor-stage.md`](./rust-large-test-module-refactor-stage.md)。父节点继续引用唯一权威设计
[`rust-large-test-module-refactor.md`](./rust-large-test-module-refactor.md)，引用链完整。调度信封曾误写为
`doc/implementation/rust-large-test-module-refactor-tasks/callable-effects*.md`；协调 owner 已裁决以冻结 baseline
父节点规定的本文件及同目录 result 文件为准，不创建第二套证据文档。

## 节点与代码状态

- DAG 节点：A / callable-effects dev；前置依赖仅为已批准设计和冻结 baseline。
- 完成后解除 C / integration checkpoint 对本 lane 的阻塞；不直接写集成分支。
- baseline：commit `805426f2249ca24d7c3b46439ac5a60be2ca3ae2`，tree
  `5db1c89a0c47e3ccf84cb564610b17f68a916c0e`。
- repo：`/Users/geek/workspace/skiff`；worktree：`/Users/geek/workspace/skiff-rust-test-callable`；
  branch：`codex/rust-test-callable`；集成 owner：`/root/rust_test_integrator`。
- 当前成熟度为开发实现检查点；本节点不得声明稳定候选或阶段完成。
- callable-effects 生产模块边界与 API 已稳定；本节点只移动/整理 test-only 代码。兄弟 service-db lane 与本节点
  没有共享写入 owner。

## Owner、入口与写集

真实入口是 `compiler/source/src/callable_effects/mod.rs` 中的 `#[cfg(test)] mod tests;`，focused selector 为
`callable_effects::tests`。本节点唯一代码 owner 为：

- `compiler/source/src/callable_effects/tests.rs`；
- 新建 `compiler/source/src/callable_effects/tests/{analysis_resolution,heap_provenance,escape_boundaries,native_functions,receiver_builtins,dependencies_contracts,support}.rs`；
- 本文件及 `rust-large-test-module-refactor-callable-result.md`。

禁止修改 `runtime/service-db/src/tests*`、`scripts/check-rust-file-lines.mjs`、任何生产源码/API/可见性、Cargo
manifest/lockfile、依赖、配置或共享集成分支。不得启动 runtime/router/Mongo，不得 push 或清理本一级 worktree。

## 实现与完成标准

1. 根 `tests.rs` 只声明七个子模块，六个领域模块严格按权威设计拥有测试，`support.rs` 不拥有测试。
2. 86 个测试函数的名称、属性及完整显式 Skiff 行为 fixture 不变；旧全名各自唯一映射到一个新增领域段。
3. `support.rs` 以单一 `AnalysisFixture` builder 统一单源、多源、dependency analysis/artifact、module/package
   配置和 platform/prelude 初始化；删除被替代的多入口与 `too_many_arguments`。
4. dependency fixture 与窄断言由 `support.rs` 唯一拥有；不引入万能参数矩阵，不把源码样例模板化。
5. 实际 diff 只命中上述 owner，提交 clean；结构移动、support 抽象和 result 证据保持可审阅提交边界。

风险为中等，验收组为 callable-effects + 后续联合集成。最早风险探针是静态测试函数名/属性双射和
`cargo test ... -- --list`；它在完整 focused 执行前发现漏移、复制或路径归属错误。

## Baseline 身份

零 worktree 预检通过 `git show` / `git grep` 锚定上述 commit/tree：测试属性共 86 个，函数名如下，且
`#[ignore]` 为 0 个：

1. `analysis_pending_is_an_explicit_diagnostic_seed_not_the_production_default`
2. `simple_detached_wrapper_is_safe_and_direct_transitive_calls_resolve`
3. `nested_local_calls_preserve_exact_effects_and_provenance`
4. `module_constant_return_keeps_exact_constant_provenance_through_local_call`
5. `unsupported_and_cyclic_module_constants_remain_fail_closed`
6. `unresolved_global_and_non_constant_zero_arg_return_are_not_constant_shortcuts`
7. `root_qualified_and_catch_wrapped_helpers_keep_exact_local_targets`
8. `typed_catch_tag_narrowing_keeps_success_and_error_provenance_separate`
9. `typed_catch_does_not_sanitize_unknown_success_provenance`
10. `relay_shaped_cross_module_root_calls_keep_exact_targets`
11. `publication_wide_call_graph_closes_effects_and_provenance_across_files`
12. `missing_and_ambiguous_cross_file_targets_remain_fail_closed`
13. `concrete_interface_implementation_call_uses_exact_impl_method_target`
14. `generic_local_receiver_call_target_carries_exact_receiver_instantiation`
15. `interface_conformance_accepts_non_suspending_and_suspending_implementations`
16. `actor_receiver_call_uses_actor_method_target_and_exact_local_effects`
17. `ordinary_receiver_call_does_not_use_actor_method_target`
18. `post_construction_store_taints_fresh_return`
19. `post_construction_store_then_nested_mutation_fails_closed`
20. `aliased_fresh_holder_store_taints_original_return`
21. `fresh_store_taint_propagates_through_callers_and_scc`
22. `direct_parameter_field_store_has_write_without_identity_observation`
23. `fresh_alias_helper_loop_and_suspend_keep_relay_shaped_state_local`
24. `nested_heap_store_remains_fail_closed_and_direct_reference_store_is_precise`
25. `mutated_fresh_root_can_enter_acyclic_local_containers_but_database_escape_fails_closed`
26. `conditional_map_lookup_tracks_distinct_fresh_and_formal_candidates`
27. `helper_map_projection_can_be_mutated_and_reinserted_without_becoming_the_map_root`
28. `helper_field_projection_keeps_parent_edge_and_rejects_real_cycle`
29. `scalar_field_projection_does_not_invent_a_heap_cycle_in_relay_state_updates`
30. `fresh_json_root_stays_distinct_from_caller_reachable_payload`
31. `dependency_container_projection_can_be_mutated_and_reinserted_into_fresh_map`
32. `dependency_fresh_wrapper_keeps_payload_reachable_without_becoming_caller_owned`
33. `helper_parameter_store_distinguishes_field_projection_from_root_cycle`
34. `recursive_scc_reaches_alias_fixed_point`
35. `recursively_growing_projection_path_fails_closed_at_the_wire_limit`
36. `normal_return_and_wire_detached_throw_remain_independent`
37. `throw_and_rethrow_preserve_operand_effects_but_detach_emitted_provenance`
38. `stream_spawn_database_and_callback_escape_lanes_are_explicit`
39. `database_queries_and_detached_writes_do_not_escape_caller_values`
40. `persisting_caller_owned_mutable_values_remains_a_database_escape`
41. `database_value_transactions_transfer_the_exact_final_value`
42. `database_writes_detach_static_field_projections_but_not_direct_or_unknown_values`
43. `exact_context_free_native_uses_shared_callable_semantics`
44. `date_from_epoch_milliseconds_wrapper_uses_exact_native_semantics`
45. `map_empty_materialization_accumulator_uses_exact_native_semantics`
46. `json_decode_materialization_uses_exact_detached_semantics`
47. `json_merge_materialization_uses_exact_detached_semantics`
48. `optional_date_parse_wrapper_uses_exact_native_semantics`
49. `bytes_from_base64_wrapper_uses_exact_native_semantics`
50. `bytes_from_hex_wrapper_uses_exact_native_semantics`
51. `bytes_concat_openai_multipart_shape_uses_exact_native_semantics`
52. `exact_http_request_natives_transfer_through_local_helpers`
53. `exact_http_client_stream_is_fresh_detached_and_suspending_through_raw_request`
54. `exact_http_client_sse_is_fresh_detached_and_suspending_through_raw_request`
55. `exact_http_response_stream_event_constructors_are_fresh_and_effect_free`
56. `exact_http_response_stream_emit_escapes_and_suspends_only_for_caller_event`
57. `std_exact_native_matrix_uses_shared_callable_semantics`
58. `exact_package_boundary_callables_transfer_canonical_effects_and_provenance`
59. `receiver_effects_are_contextual_to_caller_reachable_values`
60. `local_call_transfer_maps_alias_and_identity_to_exact_formal_actuals`
61. `json_object_set_effects_are_contextual_to_caller_reachable_values`
62. `config_intrinsics_are_exact_detached_sources`
63. `exact_date_and_duration_receiver_targets_use_sparse_semantics`
64. `date_add_milliseconds_keeps_v1_proxy_expiry_detached`
65. `date_diff_milliseconds_keeps_interaction_duration_shape_detached`
66. `nullable_date_compare_keeps_upstream_status_shape_detached`
67. `exact_string_contains_target_is_read_only_detached_and_non_suspending`
68. `exact_bytes_to_hex_target_is_read_only_detached_and_non_suspending`
69. `exact_json_object_has_target_is_read_only_detached_and_non_suspending`
70. `exact_json_object_delete_mutates_caller_receiver_but_discharges_fresh_receiver`
71. `json_object_delete_semantics_do_not_generalize_to_map_delete`
72. `exact_json_object_get_preserves_nested_alias_but_fresh_codec_shape_is_detached`
73. `exact_map_get_preserves_caller_alias_but_discharges_fresh_accumulator`
74. `exact_map_has_and_set_keep_contextual_receiver_semantics`
75. `formal_indexed_receiver_writes_ignore_unrelated_caller_actuals_through_helpers_and_scc`
76. `formal_indexed_stream_escape_ignores_unrelated_caller_actuals_through_helpers_and_scc`
77. `missing_dynamic_mutable_and_capability_semantics_remain_fail_closed`
78. `exact_file_creation_wrappers_are_fresh_and_only_suspend`
79. `exact_dependency_callee_does_not_poison_known_target`
80. `dependency_exact_signature_controls_caller_suspension`
81. `exact_dependency_field_callee_does_not_poison_known_target`
82. `exact_contract_field_callee_uses_detached_descriptor`
83. `dependency_field_first_class_value_remains_fail_closed`
84. `detached_contract_target_uses_descriptor_effect_guarantees`
85. `missing_detached_error_or_other_guarantee_remains_fail_closed`
86. `unknown_contract_member_fails_with_source_location_and_stable_key`

## 验证与证据有效性

本节点唯一拥有以下验证，使用独立
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff-rust-test-callable/build/cargo-target`：

- 前后 `cargo test --manifest-path compiler/source/Cargo.toml callable_effects::tests -- --list`；
- `cargo test --manifest-path compiler/source/Cargo.toml callable_effects::tests -- --test-threads=1`；
- 静态函数名双射、测试属性及 ignore 审计；
- `cargo fmt --manifest-path compiler/source/Cargo.toml -- --check`；
- 本 crate Clippy/编译结果；
- `git diff --check`、写集审计和 clean 提交审计。

不运行 full verify。证据仅对最终 implementation/result commit tree 和上述隔离 target/当前工具链环境有效；
任何测试源码、manifest/依赖、编译器输入或测试环境变化都会使相应证据失效。

## 停止条件

若实现要求修改生产契约/源码、Cargo 配置或依赖、兄弟 service-db owner、line gate、共享集成分支，或出现多个
会改变六域归属/单一 support 方向的未知量，则以 `TASK_SCOPE_EXPANDED` 停止并报告。若冻结 baseline 不可达、
86 项身份无法建立双射或规定验证无法执行且无 baseline 归因，则以 `TASK_NOT_EXECUTABLE` 停止。
