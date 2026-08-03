# P1-1 叶子任务：游标抽取 + 快照 + 终止符 helper

- 直接父节点（权威设计，唯一事实源）：`doc/implementation/parser-rs-refactor.md` @ commit `7e032073`。
- 集成 Agent：`/root/skiff_parser_integration`，集成分支 `parser-refactor`（唯一写入者）；本节点不合并、不 push。
- DAG 位置：Phase 1 步骤 1（cursor.rs 抽取）+ 步骤 2（快照替换 Parser 克隆）+ 步骤 5（`match_statement_terminator` 部分）。
- 并行边界：`syntax/src/parser/tests.rs` 与 `syntax/src/parser/tests/spawn.rs` 归 P0，本节点不触碰；公共 API 仅保留
  `parse_source` / `parse_source_metadata` / `parse_source_with_bodies_tolerant`，不变。
- 分支名偏差：任务要求 `parser-refactor/p1-1-cursor`，但 git 无法在既有分支 `parser-refactor`
  （`refs/heads/parser-refactor` 为文件）下创建同名子命名空间分支；改用合法等价名
  `parser-refactor-p1-1-cursor`，其余流程不变。

## 写入范围

- `syntax/src/parser/cursor.rs`（新增）
- `syntax/src/parser.rs`
- 本文件（叶子任务记录）

## 预检结论（锚定 7e032073）

- `parser.rs` 4073 行；全文件唯一 Parser 克隆位于 `looks_like_generic_call_suffix`（2696–2705）。
- `parse_generic_args` 路径（经 `parse_type` 链）不写 `self.source_spans`，快照替换与克隆语义一致。
- 终止符模式：8 处精确 `self.match_symbol(";") || self.match_symbol(",")`（4 处 `let _ = …;` 形式 +
  4 处 if 条件形式）；另有 2 处同型反序 `self.match_symbol(",") || self.match_symbol(";")`
  （均在 `parse_db_projection_fields`）。因单个 token 的 Symbol 值互斥，`;`/`,` 两个检查不可能同时为真，
  两序返回值/副作用完全等价；属本节点机械范围，按“同类残留可自行闭合”一并纳入。
- 同类游标残留：`parse_primary_pattern`（2456/2461/2475）与 `parse_callable_decl_body_tolerant`
  （1357/1371）直接 save/restore `self.current`，改用 `snapshot()/restore()` 闭合。
- 域相关 lookahead（`check_db_field_entry`、`check_dependency_source_address_suffix`、
  `self.tokens.get(self.current + 1)` 于 db 表达式处）不属于纯游标层，保持原位。

## 实施内容

1. 新增 `syntax/src/parser/cursor.rs`：`use super::Parser`，`impl Parser` 块内方法统一 `pub(super)` 可见性
   （设计 §3.3）：
   - 移动：`peek` / `advance` / `previous` / `match_ident` / `match_symbol` / `check_ident` /
     `check_symbol` / `check_function_start` / `check_provider_capability_start` / `expect_ident` /
     `expect_ident_value` / `expect_string` / `expect_positive_integer` / `expect_symbol` /
     `is_at_end` / `peek_binary_op` / `skip_balanced_block` / `import_tail_is_terminated`。
   - 新增：`snapshot() -> usize`（返回 `self.current`）、`restore(usize)`（`self.current = snapshot`）、
     `match_statement_terminator() -> bool`（`self.match_symbol(";") || self.match_symbol(",")`）。
2. `parser.rs` 声明 `mod cursor;`；`looks_like_generic_call_suffix` 改为 `&mut self`，用
   `snapshot()/restore()` 替换克隆式试探解析，删除 Parser 克隆。
3. 10 处 `;`/`,` 终止符表达式（8 处精确 + 2 处同型反序）全部替换为 `match_statement_terminator()`。
4. 两处直接 `self.current` save/restore 改用 `snapshot()/restore()`。

## 完成标准

- 游标方法全部位于 `cursor.rs`；无 Parser 克隆式 lookahead；终止符表达式全部走 helper；
  `parser.rs` 行数只减不增（当前 4073，门禁上限 4073）。
- `cargo test -p skiff-syntax` 全绿。

## 聚焦验证命令

```bash
cargo test -p skiff-syntax
node scripts/check-rust-file-lines.mjs
cargo fmt --check -p skiff-syntax
cargo clippy -p skiff-syntax --all-targets
```

不运行完整 verify / pnpm verify。

## 停止条件

- 需要修改 `parser/tests.rs` / `parser/tests/spawn.rs`（P0 所有）→ `TASK_SCOPE_EXPANDED`。
- 需要改变公共 API、AST、span、错误文案、门禁配置或解析语义 → `TASK_NOT_EXECUTABLE`。
- 与权威设计冲突 → 停止并上报。

## 交接

- result commit 为该 worktree 最后一次提交；报告 branch / worktree 路径 / result commit/tree /
  实际写集 / 自验收矩阵给 `/root/skiff_parser_integration` 与 `/root`。
- 不自行合并、不 push。
