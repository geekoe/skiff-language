# Phase 0：parser 重构测试基线（DAG 节点 P0-tests）

## 定位与父节点

- 权威设计（唯一事实源）：`doc/implementation/parser-rs-refactor.md`，基线 commit
  `7e032073e980c70def1f293219997d37606ae3df`（main HEAD）。
- DAG 位置：parser-rs-refactor 执行 DAG 的 Phase 0 测试基线节点。并行兄弟节点 P1-1
  （`parser-refactor-p1-1-cursor`）拥有 parser.rs 改动，本节点不得触碰 parser.rs。
- 集成分支：`parser-refactor`（集成 Agent `/root/skiff_parser_integration` 独占写入，
  本节点不合并、不 push）。
- 本节点分支：`parser-refactor-p0-tests`，worktree `/Users/geek/workspace/skiff-parser-p0`。

## 机械闭合记录

- 任务信封要求分支 `parser-refactor/p0-tests`，但 `refs/heads/parser-refactor` 已作为分支
  文件存在，git 不允许同名 ref 同时作为目录前缀（`cannot lock ref
  'refs/heads/parser-refactor/p0-tests'`）。采用与并行兄弟节点一致的连字符风格
  `parser-refactor-p0-tests`。集成时按 DAG 语义（Phase 0 测试基线）归属，无需改名。
- `test_default_run_span` 与 `source_spans` 均为 `#[serde(skip)]`
  （`syntax/src/ast.rs`），本节点用 serde round-trip 断言锁定该契约。
- parse 输出差量载体在 baseline 数据文件中的失败差量用逐条对比输出，便于 Phase 3 直接
  复用并 diff。
- 预检发现现有 production 缺陷（不修改）：`advance()` 在 EOF 时不前进并返回
  `previous()`（parser.rs:4032），因此 `skip_balanced_block` 的 `TokenKind::Eof`
  分支不可达；tolerant 模式下未闭合函数体会在 `parse_callable_decl_body_tolerant`
  回退路径死循环。Phase 0 是 test-only 节点，无法在不改 production 的情况下覆盖该
  用例，已从测试集中移除并记录，交由 Phase 1 游标抽取时处理。
- 仓库 `.cargo/config.toml` 配置 `build/cargo-target` 为 target-dir；carrier walker
  与 line-gate 扫描口径一致，跳过 `target`/`cargo-target`/`.git`/`node_modules`。

## 写入范围（全部 test-only）

- `syntax/src/parser/tests.rs`：仅新增 `mod` 声明，不改现有测试。
- `syntax/src/parser/tests/binary_precedence.rs`：二进制表达式结合性与优先级 AST 形状。
- `syntax/src/parser/tests/type_golden.rs`：类型文本 golden corpus（含控制字符用例）。
- `syntax/src/parser/tests/span_sensitive.rs`：functions/impl_methods/consts/
  db_index_wheres 的 `SourceSpanTable` 内容与 `test_default_run_span`。
- `syntax/src/parser/tests/tolerant_recovery.rs`：tolerant 模式失败后游标/剩余 token
  精确断言。
- `syntax/src/parser/tests/parse_output_carrier.rs` 与
  `syntax/src/parser/tests/data/fixture-parse-output-baseline.txt`：Phase 3“全部 .skiff
  fixtures parse 输出差量”的最小可复现载体（syntax crate 内 `#[test]` + 基线数据，
  不新增顶层脚本/配置）。

禁止：parser.rs 与任何 production 代码；CI/门禁/配置；顶层集中式工具；触碰 P1-1 的
parser.rs 改动。

## 完成标准

1. 二进制表达式结合性与优先级：`a - b - c`、`a / b / c`、`a + b * c`、`a && b || c`、
   `a < b == c` 的 AST 形状断言。
2. 类型文本 golden corpus：泛型、record、fn 类型、`any I`、`/` 依赖路径、string literal
   类型、nullable、union 的“输入源 → 期望 `TypeRef.name` 文本”表；含控制字符用例
   （`quote_string_type` 与 `serde_json` 转义差异，对照 `TypeExpr::parse(name)
   .to_type_string()`）。
3. span 敏感用例：`SourceSpanTable` 的 functions/impl_methods/consts/db_index_wheres
   内容；另覆盖 `test_default_run_span`（局部变量收集 + `#[serde(skip)]`）。
4. tolerant 模式回退精确行为：失败后游标恢复到 body 起始并 `skip_balanced_block`，
   断言后续声明的精确 offset、被跳过 token 不回流、以及不可恢复路径的精确错误位置。
5. parse 输出差量载体：`#[test]` 枚举仓库全部 `.skiff` fixtures，计算 `parse_source`
   的确定性输出并与 committed baseline 对比；支持 `UPDATE_PARSER_PHASE0_BASELINE=1`
   显式再生成。Phase 3 重构后重跑同一测试即可得到差量。
6. parser.rs 行数保持 4073 不变；本节点聚焦验证全部通过。

## 聚焦验证命令（本节点自行运行）

```bash
cargo test -p skiff-syntax
node scripts/check-rust-file-lines.mjs
cargo fmt --check -p skiff-syntax
cargo clippy -p skiff-syntax --all-targets
```

不运行完整 `verify` / `pnpm verify`。

## 停止条件

- 需要修改 parser.rs、公共 API/契约或设计口径：停止并返回
  `TASK_SCOPE_EXPANDED`/`TASK_NOT_EXECUTABLE` 精确报告。
- 必须新增顶层 scripts/ 或集中式配置才能满足需求：先停止并上报主 Agent，不擅自决定。
- 触碰并行兄弟节点 P1-1 的写范围：停止并上报。
