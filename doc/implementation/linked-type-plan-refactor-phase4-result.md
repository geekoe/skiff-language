# Phase 4 结果：公共 trait 入口收敛

日期：2026-08-03

直接父节点：`doc/implementation/linked-type-plan-refactor.md`（§5 Phase 4 / §4.4；
调用面审计见 phase0 result）。

## 写集

- `type_plan/mod.rs`：`RuntimeTypePlanLinkedExt` 从 14 个方法瘦身为 3 个
  （`from_artifact_type_ref` / `from_linked` / `from_linked_nested_ref`）；
  `RuntimeRecoverableExpectedTypePlanLinkedExt` 保持 `from_linked` / `from_linked_ref`。
- `type_plan/linked.rs`：被移出 trait 的方法改为 `pub(crate)` 自由函数
  （`from_artifact_type_ref_in_program_ref`、`from_linked_impl`、`from_linked_ref`、
  `from_linked_substituted`、`resolve_addr_or_bridge`、`from_linked_declaration`、
  `from_linked_descriptor`、`builtin_node`、`artifact_builtin_node`、
  `artifact_builtin_node_in_program`）；trait 三个保留方法成为薄委托。
- 删除两个无调用点包装：`from_artifact_type_ref_in_program` /
  `from_artifact_type_ref_in_type_view`（等价于 `_in_program_ref` + `PlanContext::new` /
  `from_type_view`，Phase 0 审计确认全仓库 0 外部调用）。
- 移除全部 `#[allow(dead_code)]`（`PlanContext` struct/impl、`from_linked`、
  `builtin_node`、测试 mod 上的残留标注）。
- 跨模块调用改为直接 `use super::linked::{...}`，不再经 mod.rs 重导出。

## 证据

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| 包级 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | 31 passed，无警告 |
| 近邻 | `cargo test -p skiff-runtime-eval --lib recoverable` | 14 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_db_insert_one_decodes` | 2 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_decodes_nested_anonymous` | 1 passed |
| 纵向 | `cargo test -p runtime --lib runtime_type_plan_resolves_package` | 1 passed |
| 结构 | `rg 'from_artifact_type_ref_in_program\b|_in_type_view'` | 仅已删除，0 |
| 结构 | `rg 'allow(dead_code)' type_plan/` | 0 |
| 门禁 | line gate / rustfmt / `git diff --check` | PASS |

外部调用面复核：被移除方法在 workspace 内 0 调用点；唯一保留的
`RuntimeRecoverableExpectedTypePlan::from_linked_ref`（db_eval 2 处）属于
recoverable trait，未动。driver/value_codec 临时 adapter 只 import trait 名，
不受影响。完整 runtime 套件未跑，标注聚焦验证。

## 遗留

- Phase 5（跨 crate 目录共享）保持搁置，需先决策目录落点。
- `linked.rs` 的自由函数是 `pub(crate)` 而非私有，供 builtins/nominal/recoverable/
  tests 调用；若后续只有一个消费方，可再降级为 `pub(super)`。
