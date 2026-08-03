# linked-type-plan 重构 Phase 0 — 基线锁定结果

日期：2026-08-03
状态：完成（聚焦验证）
Baseline：`c46d65b1`；分支 `refactor/type-plan-phase0`；
worktree `/Users/geek/workspace/skiff-type-plan-phase0`

## 1. 调用面审计（权威文档 2.3 表最终核对版）

审计范围：`runtime/linked-type-plan` 包外部全部调用点，含
`runtime/driver/value_codec/type_descriptor.rs` 临时 adapter。方法名计数基于
`rg`/`git grep` 对 c46d65b1 的精确匹配，排除 `type_plan.rs` 自身。

| trait 方法 | 外部调用数 | 明细 | 结论（与 2.3 一致） |
| --- | --- | --- | --- |
| `from_linked`（RuntimeTypePlan） | 32 处匹配行（31 实调用 + 1 注释） | driver 测试 5（program_execution.rs 4 + support/program.rs 1）；eval 22（actor_dispatch 2、actor_executor 3、actor_concurrent_continuation 1、actor_instance 1、ingress 1、invocation 1、invocation_builder 1、native_invocation 1、program_invocation 6 实调用 + 1 注释、http_gateway 2、websocket_connect 1、websocket_jsonrpc 1、spawn_ops 1）；同 crate http_plan 2、native_call_plan 3 | 保留 |
| `from_linked_nested_ref` | 12 | exceptions 2、program_invocation 3、http_gateway 2、websocket_connect 1、websocket_jsonrpc 1、type_projection 2（wrapper 内部）、http_plan 1；另有多处 eval 调用点经 type_projection 的 `plan_from_linked_nested_ref*` wrapper 间接调用 | 保留 |
| `from_artifact_type_ref` | 1 | `runtime/driver/eval/tests/program_execution.rs:2081`（stream 解码预算测试） | 保留 |
| `from_linked_ref`（RuntimeTypePlan） | 0 | — | 可私有化 |
| `from_linked_substituted` / `resolve_addr_or_bridge` / `from_linked_declaration` / `from_linked_descriptor` | 0 | — | 可私有化 |
| `builtin_node` / `artifact_builtin_node` / `artifact_builtin_node_in_program` | 0 | — | 可私有化 |
| `from_artifact_type_ref_in_program` / `_in_type_view` / `_in_program_ref` | 0 | 仅内部/tests | 可降级 |
| `RuntimeRecoverableExpectedTypePlan::from_linked` | 2 | recoverable_behavior.rs 1、recoverable_spawn_payload.rs 1 | 保留 |
| `RuntimeRecoverableExpectedTypePlan::from_linked_ref` | 2 | db_eval.rs L340/L349 | 保留 |

临时 adapter 事实：

- `runtime/driver/value_codec/type_descriptor.rs:12` 以
  `#[allow(unused_imports)]` 整体再导出 `RuntimeTypePlanLinkedExt` /
  `PlanContext` / `ProgramTypeView`（经 `runtime/driver/lib.rs` 的
  `pub(crate) use value_codec::type_descriptor`）。只 import trait 本身，
  不调用任何方法：Phase 4 缩小方法不破坏它，但删除方法时这里是盲区，审计
  已列入。
- 全仓库 `RuntimeTypePlanLinkedExt` / `RuntimeRecoverableExpectedTypePlanLinkedExt`
  导入面 = eval 19 文件 + driver 测试 6 文件 + linked-type-plan 2 模块 +
  value_codec adapter；`runtime/boundary/src/type_descriptor.rs:29` 仅注释
  提及，非消费者。

结论：设计文档 2.3/4.4 的行号、数量、调用点均与审计一致，无需事实性修正。
2.3 表中 `from_linked` 的「20+」精确为 31 个实调用点（不含注释）；无与设计
语义冲突的新事实，未改文档。

## 2. 差分测试

新增 `#[cfg(all(test, feature = "test-support"))]`
`differential_legacy_json_baseline_tests`（type_plan.rs 末尾，6 个测试）：

1. `builtin_directory_matches_legacy_json_descriptors`：leaf（Json/JsonObject/
   bytes/Date/string/bool/boolean/integer/number/null/void）、Array/Map/Stream、
   DbInsert/Update/Delete/Upsert result、std.http 三记录、未知 builtin fallback
   Unknown——全部与 legacy `from_descriptor` 逐字段等价。
2. `inline_record_union_nullable_and_literal_match_legacy_json_descriptors`：
   内联 Record/Union/Nullable/Literal 等价。
3. `address_resolved_descriptors_match_legacy_nodes_but_owner_context_is_outer_path_only`：
   Representation/Alias 的 node 等价；label/named_type_name 列预期差异
   （owner context 由外层 JSON 路径应用，`from_descriptor` 桥本身不应用）。
4. `type_param_substitution_resolves_bound_ref_but_legacy_bridge_has_no_substitution_pass`：
   绑定 T 解析为 string 并等价；裸 typeParam descriptor → Unknown、未绑定
   TypeParam 报错均列为预期差异。
5. `depth_32_cap_truncates_linked_walk_but_legacy_bridge_recurses_uncapped`：
   16 层 Array（最内层 depth=32）两路径完整等价；17 层（depth=34）linked 截断
   为 Unknown、legacy 桥无 cap 不截断——预期差异已注释。
6. `recoverable_expected_structural_shapes_match_legacy_shape_only_bridge`：
   recoverable expected 对 string/Array/Map/Record 与
   `from_runtime_type_plan_shape_only_for_diagnostics(from_descriptor(..))`
   等价。

## 3. 基线证据

| 层级 | 命令 | owner | commit/代码状态 | 结果 | 覆盖范围 |
| --- | --- | --- | --- | --- | --- |
| 包级（改动前） | `cargo test -p skiff-runtime-linked-type-plan` | phase0 | c46d65b1 | 20 passed | 既有包测试（无差分测试） |
| 包级（改动前） | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | phase0 | c46d65b1 | 20 passed | 确认 legacy trait 可经 feature 启用 |
| 包级（改动后） | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | phase0 | 本 commit | 26 passed（20 既有 + 6 差分） | 差分覆盖 §2 全部形状 + 预期差异 |
| 包级（改动后，无 feature） | `cargo test -p skiff-runtime-linked-type-plan` | phase0 | 本 commit | 20 passed | 差分模块 cfg 门控正确，不污染默认测试 |
| 文件行数门禁 | `node scripts/check-rust-file-lines.mjs` | phase0 | 本 commit | PASS（1633 文件，limit 3151；type_plan.rs = 3133） | 全仓库 .rs |
| rustfmt | `cargo fmt --check --package skiff-runtime-linked-type-plan` | phase0 | 本 commit | PASS | 包内格式 |
| 纵向探针（driver/eval） | `cargo test -p runtime --lib runtime_program_db_insert_one_decodes` | phase0 | 本 commit | 2 passed | from_linked + boundary decode 端到端 |
| 纵向探针（driver/eval） | `cargo test -p runtime --lib runtime_program_decodes_nested_anonymous` | phase0 | 本 commit | 1 passed | from_linked + nullable 嵌套 record |
| 纵向探针（driver/eval） | `cargo test -p runtime --lib runtime_type_plan_resolves_package` | phase0 | 本 commit | 1 passed | DbUpsertResult + DbObjectSymbol 解析 |
| 近邻探针（eval recoverable/nested_ref） | `cargo test -p skiff-runtime-eval --lib recoverable` | phase0 | 本 commit | 14 passed | recoverable_behavior、recoverable_spawn_payload、projection（nested_ref wrapper） |

未跑完整 runtime 套件与 `pnpm verify`：本结果为「聚焦验证」，不声称全绿。

## 4. 纵向探针清单（后续每个 Phase 的固定可观察探针）

`runtime/driver/eval/tests/program_execution.rs`（经 `cargo test -p runtime
--lib <filter>` 运行，crate 内路径 `eval::tests::program_execution::*`）：

| 探针 | 测试名 | 覆盖 |
| --- | --- | --- |
| P1 | `runtime_program_db_insert_one_decodes_business_json_through_ordinary_result_plan` | `from_linked` 内联 Record → boundary 解码 |
| P2 | `runtime_program_db_insert_one_decodes_db_object_symbol_result_plan` | `from_linked` + DbObjectSymbol 地址解析 → 解码 |
| P3 | `runtime_program_decodes_nested_anonymous_record_result_plan_with_nullable_nested_record` | `from_linked` + Nullable/嵌套 Record |
| P4 | `runtime_type_plan_resolves_package_db_object_symbol_from_file_declarations` | `from_linked` + DbUpsertResult + 包内 DbObjectSymbol |
| P5 | `runtime_program_create_from_stream_items_use_request_heap_budget` | `from_artifact_type_ref` Stream（L2081 唯一外部调用点） |

基线缺口（如实记录）：

- `runtime/driver/eval/tests` 没有直接覆盖 `from_linked_nested_ref` 与
  `RuntimeRecoverableExpectedTypePlan::from_linked*` 的端到端用例。最近可替代
  探针在 `runtime/eval` crate：
  - `assembly_execution::projection::tests::assembly_database_type_and_recoverable_views_use_the_execution_image`
    （经 `plan_from_linked_nested_ref` wrapper 调用 nested_ref，L127/L253）；
  - `recoverable_behavior::tests::duplicate_package_id_*`（recoverable 行为层）；
  - `cargo test -p skiff-runtime-eval --lib recoverable`（14 项，本 commit 全绿）。
- 建议后续 Phase 在 driver/eval/tests 补一个覆盖 nested_ref/recoverable 的
  端到端探针（或由集成 Agent 决策改用 eval crate 探针作为固定探针）。

## 5. 自验收矩阵

| 设计条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| Phase 0 验收：聚焦测试全绿 | 26 passed（含 6 差分） | `rg differential_legacy_json_baseline_tests` 仅 type_plan.rs 1 处 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` |
| Phase 0 验收：调用面清单入库 | 本文件 §1；与 2.3 表逐项一致 | `rg -l RuntimeTypePlanLinkedExt` = 预期 28 文件 | — |
| Phase 0 验收：纵向探针清单可执行 | 本文件 §4 | `rg from_linked runtime/driver/eval/tests` 5 处 | `cargo test -p runtime --lib <P1..P5 过滤器>`（P1-P4 已跑） |
| 不改生产行为 | type_plan.rs 仅新增测试 mod；`git diff` 生产段 0 行 | `git diff HEAD --stat` 仅 type_plan.rs + 2 文档 | `cargo test -p skiff-runtime-linked-type-plan`（20 passed） |
| 文件行数门禁 | type_plan.rs = 3133 ≤ 3151 | `node scripts/check-rust-file-lines.mjs` PASS | — |
| 差分断言行为真实 | 6 差分测试含等值断言与预期差异断言 | 无 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` |
| 未跑完整套件不声称全绿 | 本文件 §3 标注「聚焦验证」 | — | — |

## 6. 需要主 Agent/集成 Agent 知晓的发现

1. **driver/eval/tests 缺 recoverable/nested_ref 直测探针**（见 §4 缺口）：
   建议在 Phase 1–3 批次内补一个端到端用例，或把 eval crate 的近邻探针
   正式纳入固定探针清单。这不阻塞 Phase 0，但影响「每个 Phase 同一条纵向
   探针」的落实。
2. **legacy `from_descriptor` 自身无 depth cap**：cap 在外层 JSON walk
   （`resolve_program_descriptor_refs`），差分测试把该差异钉为预期差异；
   Phase 3 归一化 depth 语义时不得拿裸 `from_descriptor` 当深度基准。
3. **行数余量收紧**：type_plan.rs 当前 3133/3151（余 18 行）。Phase 1–3
   必须按「先降峰值再收敛」的顺序搬迁，任何中间 commit 一旦在 type_plan.rs
   增加超过 18 行都会触顶；建议 Phase 1 首先搬走两个内联 test mod
   （约 450 行）腾出空间。
4. **`from_linked` 精确调用数为 31**（文档 2.3 写作「20+」）：语义一致，
   无需改文档；Phase 4 做入口收敛时以本清单为唯一事实源。
