# Phase 5 结果：跨 crate 目录共享

日期：2026-08-03

直接父节点：`doc/implementation/linked-type-plan-refactor.md`（§5 Phase 5；落点决策：
共享目录放 `skiff-runtime-model`，不新增 crate）。

## 写集

- 新增 `runtime/model/src/type_plan/builtins.rs`：唯一共享目录，包含
  - 名称解析（`split_top_level` / `generic_text_parts` / `generic_root` /
    `type_name_root` / `bare_type_name`，从 boundary 下沉）
  - `RuntimeBuiltinShape`（名称 → 形状 + leaf 映射）
  - leaf/record 辅助（`builtin_plan`、`std_field`、`std_*_plan`、`std_http_*_plan`、
    `std_http_record_node`、`std_duration_plan`）
  - Db*Result 模板（`db_result_record_node` / `db_result_upsert_record_node`，
    递归字段仍由消费方注入）
  - 名称谓词（`is_builtin_named_type` / `is_builtin_concrete_type_name`）
  - artifact label 辅助（`artifact_type_ref_label` / `artifact_type_ref_named_type_name`）
- `runtime/model/src/type_plan.rs`：声明 `pub mod builtins` 并重导出目录。
- `runtime/boundary/src/type_descriptor.rs`：删除 5 个名称解析函数定义与
  test-support 段的 std.http 目录副本，改为 `pub use` model 重导出 + 薄委托；
  `std_runtime_builtin_node_from_descriptor` 改用 model 的 `std_http_record_node`。
- `runtime/boundary/src/db.rs`：删除本地 `artifact_type_ref_label` /
  `artifact_type_ref_named_type_name` 与 leaf match，改用 model 目录；行为不变
  （Db*Result 仍不进入 boundary 路径）。
- `runtime/linked-type-plan/src/type_plan/`：`builtins.rs` 瘦身为 PlanInput +
  递归接线 + 薄适配；`labels.rs` 删除 artifact label 副本；各模块改从 model 导入。

## 证据

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| model | `cargo test -p skiff-runtime-model` | 105 passed |
| boundary | `cargo test -p skiff-runtime-boundary --features test-support` | 169 passed |
| linked-type-plan | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | 31 passed |
| service-db（boundary/db.rs 消费方） | `cargo test -p skiff-runtime-service-db` | 144 passed |
| 近邻 | `cargo test -p skiff-runtime-eval --lib recoverable` | 14 passed |
| 纵向 | driver from_linked 三个过滤器 | 4 passed |
| 结构 | `rg 'fn std_http_client_request_plan'` / `fn artifact_type_ref_label` | 各 1（model） |
| 门禁 | line gate（1642 文件）/ rustfmt / `git diff --check` | PASS |

leaf 映射副本：runtime 计划构建路径只剩 model 目录 1 处；其余 `"JsonObject" =>`
属于 artifact-model 配置解析与 compiler 侧 `BuiltinShape`，不合并（设计非目标）。

## 遗留

- 完整 `pnpm verify` 未跑（聚焦验证 + 受影响消费方全绿）。
- `skiff-runtime-boundary::type_descriptor` 的 5 个名称解析函数保留为 re-export，
  外部调用点（db.rs、program_stream.rs、http_plan.rs、eval）无需改动。
