# Phase 1 结果：type_plan.rs 拆分为目录模块

日期：2026-08-03

直接父节点：`doc/implementation/linked-type-plan-refactor.md`（定稿，§5 Phase 1 / §4.1 / §4.5）。

## 写集

- `runtime/linked-type-plan/src/type_plan.rs` → `runtime/linked-type-plan/src/type_plan/mod.rs`
  （git mv，内容裁剪为 imports + trait 契约 + 模块声明 + 重导出）
- 新增子模块：`context.rs`（ProgramTypeView / PlanContext）、`linked.rs`
  （`RuntimeTypePlanLinkedExt` impl 全量搬迁）、`nominal.rs`、`recoverable.rs`
  （含 identity helpers 与 sorted_json）、`labels.rs`、`address.rs`、`builtins.rs`
  （std builtin 目录与 db result 目录原样搬迁）、`tests.rs`（3 个 test mod +
  `test_runtime_package`）
- `lib.rs` 与 crate 内既有路径（`crate::type_plan::native_builtin_plan`、
  `crate::type_plan::test_runtime_package`）保持不变：mod.rs 以
  `pub(crate) use` / `#[cfg(test)] pub(crate) use` 重导出。

## 执行决策（不改变设计语义）

- `RuntimeTypePlanLinkedExt` 的 600 行 impl 整体保留在 `linked.rs`（未拆成薄入口 +
  自由函数委托）；这是 Phase 1“纯搬迁”的最小形态，Phase 4 再做 trait 入口收敛。
- nominal ↔ linked 的双向依赖维持现状（nominal 调用 `from_linked_ref` /
  `from_linked_descriptor`），按设计 §4.1 留到 Phase 2/3 决定回调注入或显式承认。
- `test_runtime_package` 经 `#[cfg(test)] pub(crate) use tests::test_runtime_package`
  保留 `crate::type_plan::test_runtime_package` 路径，兄弟测试模块无需改动。

## 证据

| 层级 | 命令 | 结果 | 覆盖范围 |
| --- | --- | --- | --- |
| 包级 | `cargo test -p skiff-runtime-linked-type-plan --features test-support` | 26 passed | 20 既有 + 6 差分 |
| 近邻 | `cargo test -p skiff-runtime-eval --lib recoverable` | 14 passed | recoverable/nested_ref 探针 |
| 纵向 | `cargo test -p runtime --lib runtime_program_db_insert_one_decodes` | 2 passed | from_linked 端到端 |
| 纵向 | `cargo test -p runtime --lib runtime_program_decodes_nested_anonymous` | 1 passed | from_linked 端到端 |
| 纵向 | `cargo test -p runtime --lib runtime_type_plan_resolves_package` | 1 passed | DbUpsertResult/DbObjectSymbol |
| 格式 | `cargo fmt -p skiff-runtime-linked-type-plan -- --check` | PASS | 本 crate |
| 门禁 | `node scripts/check-rust-file-lines.mjs` | PASS（1641 文件 ≤3151） | 全仓 |
| 写集 | `git diff --check` | PASS | 本 commit |

生产行为零改动：`git diff` 仅文件搬迁 + 可见性修饰（`pub(super)`/`pub(crate)`）+
`sorted_json_string` 改为 `pub(crate)`（测试消费）。完整 runtime 套件未跑，标注聚焦验证。

## 遗留

- 行数余量：`type_plan/tests.rs` 现为全仓最大文件之一（约 940 行），Phase 2/3 会在
  builtins.rs 内部继续消减生产代码；tests.rs 行数如触顶再按测试域拆分。
- Phase 0 的三条 non-blocking 发现（driver 探针缺口、行数余量、legacy 无 depth cap）
  继续有效。
