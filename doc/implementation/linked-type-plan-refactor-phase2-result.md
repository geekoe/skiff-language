# Phase 2 结果：builtin 目录归一

日期：2026-08-03

直接父节点：`doc/implementation/linked-type-plan-refactor.md`（§5 Phase 2 / §4.2）。

## 写集

- `type_plan/builtins.rs`：新增 `RuntimeBuiltinShape`（层 1：名称 → 形状 + `leaf_node`，
  经 `bare_type_name` 统一处理 full/alias 拼写）；`std.http.*`/`Duration` 记录目录保持原状
  （层 2）；`native_builtin_plan` leaf 分支改走目录，未知名仍 `Err(InvalidArtifact)`；
  删除 3 个 `std_runtime_builtin_node_from_*_parts` 包装。
- `type_plan/linked.rs`：3 个 leaf match（`builtin_node` / `artifact_builtin_node` /
  `artifact_builtin_node_in_program`）收敛为目录查找，fallback 仍为 `Unknown`；
  std.http 目录调用点直连 `std_runtime_builtin_node(name, args.len())`。
- `type_plan/recoverable.rs`：`recoverable_expected_builtin_node` leaf 分支改走目录，
  未知名仍 `Unresolved { diagnostic_label }`。
- `type_plan/tests.rs`：新增 `builtin_catalog_tests`（表驱动：shape 解析含别名、leaf 映射、
  结构/Db* 形状无 leaf）。

## 证据

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| 包级 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | 28 passed（26 + 2 目录测试） |
| 近邻 | `cargo test -p skiff-runtime-eval --lib recoverable` | 14 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_db_insert_one_decodes` | 2 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_decodes_nested_anonymous` | 1 passed |
| 纵向 | `cargo test -p runtime --lib runtime_type_plan_resolves_package` | 1 passed |
| 结构 | `rg '"JsonObject" =>'` crate 内 | 仅 builtins.rs 目录 1 处 |
| 结构 | `rg 'std_runtime_builtin_node_from_'` | 0 |
| 门禁 | line gate / rustfmt / `git diff --check` | PASS |

三种 fallback 语义按设计保留在调用方：`Unknown`（linked 三入口）、`Err`（native）、
`Unresolved`（recoverable）。完整 runtime 套件未跑，标注聚焦验证。
