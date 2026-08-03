# Phase 3 结果：db result 三份合一 + 结构分支收敛

日期：2026-08-03

直接父节点：`doc/implementation/linked-type-plan-refactor.md`（§5 Phase 3 / §4.3）。

## 写集

- `type_plan/builtins.rs`：新增 `PlanInput` 三变体输入视图（`Artifact` /
  `ArtifactInProgram` / `Linked`），depth 规则内聚在各变体：
  artifact 不加深、artifact-in-program 不加深、linked 统一 `deeper_by(2)`；
  `db_result_node` 单份目录替代原三份 `db_result_node_from_*`；
  `structural_builtin_node` 收敛 Array/Map/Stream 分支，并保留历史差异：
  linked 入口只精确匹配 `Array`/`Map`，artifact 入口按 `bare_type_name` 匹配。
- `type_plan/linked.rs`：三个入口方法（`builtin_node` /
  `artifact_builtin_node` / `artifact_builtin_node_in_program`）改为构造
  `PlanInput` 后依次走结构分支、db result、std 目录、leaf fallback。
- `type_plan/tests.rs`：新增 `plan_input_forms_tests`（三输入形式 db result /
  结构形状差分 + 全拼写容器匹配的历史差异锁定）。

## 证据

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| 包级 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | 31 passed（28 + 3 差分） |
| 包级（无 feature） | `cargo test -p skiff-runtime-linked-type-plan` | 25 passed |
| 近邻 | `cargo test -p skiff-runtime-eval --lib recoverable` | 14 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_db_insert_one_decodes` | 2 passed |
| 纵向 | `cargo test -p runtime --lib runtime_program_decodes_nested_anonymous` | 1 passed |
| 纵向 | `cargo test -p runtime --lib runtime_type_plan_resolves_package` | 1 passed |
| 结构 | `rg 'db_result_node_from_'` | 0 |
| 结构 | `rg 'RuntimeTypeNode::Array(Box::new'` linked.rs | 0 |
| 门禁 | line gate / rustfmt / `git diff --check` | PASS |

预期差异清单已锁定：linked 可解析 Address/LocalType/ServiceSymbol/PackageSymbol/
DbObjectSymbol，artifact-in-program 只桥接 PackageSymbol，artifact 对 AppliedNominal
报错；容器全拼写匹配规则差异由 `full_spelling_container_matching_*` 测试锁定。
完整 runtime 套件未跑，标注聚焦验证。
