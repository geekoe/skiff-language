# Phase 1 步骤 3：span 装配 helper（DAG 节点 P1-2）

## 定位与父节点

- 权威设计（唯一事实源，只读）：`doc/implementation/parser-rs-refactor.md`（commit
  `7e032073`，§4 Phase 1 步骤 3；§2.4 重复模式）。与设计冲突时以设计为准并停止上报。
- DAG 位置：parser-rs-refactor 执行 DAG 的 Phase 1 第二步。直接父节点：
  P1-1 cursor 抽取（`b68dc645` 已集成）；并行兄弟节点 P1-3（死状态删除）、P1-4
  （impl methods 合并），本节点不得触碰其边界。
- 集成分支：`parser-refactor`（集成 Agent `/root/skiff_parser_integration` 独占写入，
  本节点不合并、不 push、不改共享主 worktree）。
- 本节点分支：`parser-refactor-p1-2-span`，worktree
  `/Users/geek/workspace/skiff-parser-p1-2`（基于 baseline `d32aef8f`）。

## 行号锚定说明（预检记录）

- 任务信封给出的 20 处行号（148 及 2020/2034/2053/2066/2081/2100/2111/2122/2141/2150/
  2182/2215/2238/2263/2288/2331/2357/2374/2404）对应设计稿基线 `af7aada1`
  （parser.rs 4073 行）。实际集成分支 baseline `d32aef8f` 在 P1-1 抽取后 parser.rs 为
  3894 行，同一 20 处构造位于 150/1988/2002/2021/2034/2049/2068/2079/2090/2109/2118/
  2150/2183/2206/2231/2256/2299/2325/2342/2372。两清单数量一致（20 处），本节点以
  baseline `d32aef8f` 实际行号为准，全部替换。

## 预检结论（零 worktree 只读，`git grep/git show/git ls-tree`）

- `StmtSourceSpans {` 构造恰 20 处（上表），全部在本节点替换范围内。
- `children.push(x.spans.clone())` 恰 14 处：parser.rs:3296/3321/3337/3535/3536/3543/
  3559/3563/3605/3630/3638/3653/3661/3669（db 表达式 children 累积）。
- `.spans.clone()` 恰 24 处：上述 14 处 + 1170/1325/1345/1666/2289/2291/3142/3184/
  3768/3777。全部可在不改变 span 值的前提下用部分 move 消除（持有者均为局部
  `ParsedExpr`/`ParsedBlock`，其 `expr`/`block` 与 `spans` 各只消费一次）。
- 同类机械项（自行闭合并记录）：`ParsedExpr { expr, spans: expr_source_spans(...) }`
  包装约 15 处、带 `blocks`/`record_fields` 的内联 `ExprSourceSpans { ... }` 构造 6 处、
  else-if 分支把 `ParsedStmt` 手工包成 `ParsedBlock` 1 处。均为同一 span 装配链路上的
  机械样板，纳入本节点闭合并按零行为变化处理。
- 未发现需要改 cursor.rs、tests.rs 或公共契约/设计语义的情况。

## 写入范围

- 仅 `syntax/src/parser.rs`（新增 helper 放 parser.rs 内，Phase 2 才拆 span.rs）。
- 新增叶子任务文件本文件。不改测试、不改门禁/配置、不改 ast.rs/cursor.rs。

## 实现设计

新增（放在既有 `parsed_leaf_expr`/`expr_source_spans` 附近）：

- `impl ParsedExpr`：
  - `new(expr, span, children)`：`expr_source_spans` 包装；
  - `into_parts(self) -> (Expr, ExprSourceSpans)`：供所有“AST 与 spans 分别消费”的点，
    消除 `.spans.clone()`；
  - `with_children_and_blocks(expr, span, children, blocks)`；
  - `with_children_and_record_fields(expr, span, children, record_fields)`。
- `impl ParsedBlock`：
  - `from_stmt(stmt)`：把 `ParsedStmt` 包成单语句块（else-if 分支）；
  - `into_parts(self) -> (Block, BlockSourceSpans)`。
- `impl ParsedStmt`：
  - `expr(expression)`（替换 `parsed_expression_statement` 全部调用点后删除该自由函数）；
  - `leaf(stmt, span)`；
  - `with_expression(stmt, span, expression)`；
  - `with_expressions(stmt, span, expressions)`；
  - `with_block(stmt, span, block)`；
  - `with_expression_and_block(stmt, span, expression, block)`；
  - `with_expression_and_blocks(stmt, span, expression, blocks)`。

替换清单（baseline 行号）：20 处 `StmtSourceSpans` 内联构造全部走 `ParsedStmt`
helper；14 处 `children.push(x.spans.clone())` 与其余 10 处 `.spans.clone()` 改为
`into_parts`/部分 move；`ParsedExpr`/`ExprSourceSpans` 内联包装与构造改走上述 helper。

约束：AST、span 字节区间、children/blocks 顺序、错误文案、公共 API 一律不变；
`parser.rs` 行数只减不增（当前 3894，门禁 4073）；每个新函数远低于 534 行函数门禁。

## 完成标准与聚焦验证（worktree 内自跑）

1. `cargo test -p skiff-syntax` 全绿（含 P0 新增 149 个测试）。
2. `node scripts/check-rust-file-lines.mjs` 通过且 parser.rs 行数 ≤ 3894。
3. `cargo fmt --check -p skiff-syntax`、`cargo clippy -p skiff-syntax --all-targets` 通过。
4. 残留搜索：`StmtSourceSpans {` 仅出现在 helper 定义内；`.spans.clone()` 与
  `children.push(...spans.clone())` 在 parser.rs 内为零（或仅剩不可归并项并记录）。
5. 不跑完整 verify，不做 gate/live 操作。

## 停止条件

- 需要改 cursor.rs、tests.rs、ast.rs 公共契约、错误文案或设计语义 → 停止并上报
  TASK_SCOPE_EXPANDED/TASK_NOT_EXECUTABLE。
- 触碰 P1-3（死状态删除）/P1-4（impl methods 合并）明确边界 → 停止上报。
- 发现与设计冲突 → 停止上报。

## 交接

完成后把 branch、worktree 路径、result commit/tree、实际写集、自验收矩阵直接报告
集成 Agent `/root/skiff_parser_integration`，并通知主 Agent `/root`。不自行合并、不 push。
