# Leaf Task: D3 dispatch 表达式语法（parser + compiler，语法 / 类型 / plan）

## 引用链

- 权威设计：`doc/architecture/durable-task-dispatch.md`（完整阅读；compiler
  职责为证明 target 合法、return void/null、参数满足 recoverable boundary，
  生成精确 callable 与 payload plan，不选 Runtime、不理解 scheduler schema；
  提交前 timing/receiver/参数各只求值一次；db transaction 内禁止 task
  submission；目标态只有 dispatch 一种 surface）。
- 用户面契约：`doc/reference/dispatch.md`（`dispatch <call> [after(dur) |
  at(ts)]`；dispatch 是表达式，类型 `std.task.TaskRef`；单独成行是 expression
  statement、值可丢弃；`dispatch` 是完全保留名，`after` / `at` 只在 dispatch
  后缀位置保留；target 规则与 recoverable 参数）。
- 批次父节点：`doc/implementation/dispatch-d-batch.md`（集成 Agent
  `/root/dispatch_d_integration` 创建，位于集成分支 `dispatch-d-integration`
  commit `83f6aefb`；main 上尚未存在，按批次父文档引用）。
- 已合并基线代码：`syntax/src/parser/stmt.rs` 的 `Stmt::Dispatch`（现为
  statement）、`compiler/core/src/dispatch_targets.rs`（dispatchSubmit plan
  投影）、`runtime/eval/src/task_ops.rs`（只读参考，D4 才改）、
  `compiler/source/src/reserved_names.rs` 与
  `doc/reference/static-semantics.md` §9（保留名机制）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`16c177a099a3927eb9b89ce0afd61e419ad91ff7`（main HEAD，共享主
  worktree 干净）。
- worktree：`/Users/geek/workspace/skiff-d3-grammar`，branch `dispatch-grammar`。
- 集成 Agent：`/root/dispatch_d_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

在 `skiff` 仓库实现阶段 D3（syntax + compiler，不改 runtime/wire/router）：

1. parser：`dispatch` 从 statement-only 改为表达式（`Expr::Dispatch`）：
   `dispatch <call> [after(<expr>) | at(<expr>)]`，可出现在任意表达式位置；
   单独成行按 expression statement 处理。移除 `Stmt::Dispatch` 与旧错误
   “dispatch is a statement and cannot be used as an expression”。
2. `dispatch` 完全保留名：现有“关键字不能作为用户标识符”的落实层是
   `compiler/source/src/package_rules/reserved_validation.rs`
   （`is_reserved_root`，被 `root_validation.rs` 的 top-level 声明检查与
   block binding 检查实际调用）；`reserved_names.rs`（经
   `prelude_registry().is_reserved_name()`）是同一机制的独立层但当前无生产
   调用方。两处都补 `dispatch`：`is_reserved_root` 覆盖 function / const /
   type / alias / interface / impl target / 局部与 pattern binding；
   `RESERVED_ROOT_NAMES`（compiler-core）覆盖 `is_reserved_name` 路径。
   import alias：当前语法本身不支持 `import X as Y`（parser 只接受单名
   import），`import std as dispatch` 在 parser 层按 import name 规则拒绝。
   `after` / `at` 只在 dispatch 后缀位置保留（parser 上下文关键字，不做
   全局保留）。
3. 类型：`std.task.TaskRef` 作为 compiler-known 类型进入 prelude（加到
   `COMPILER_BUILTIN_TYPES`，symbol `std.task.TaskRef`、OpaqueHandle、
   arity 0，与 `std.session.ClientSessionRef` 同机制）；dispatch 表达式类型
   为 TaskRef；target 返回 void/null 检查保留；参数 recoverable boundary
   检查保留（EscapeLane::Dispatch + runtime encode fail closed，D3 不改
   runtime）；`dispatch` 在 db transaction 内禁止（ExpressionTypeModel
   增加 transaction depth，检查 dispatch 表达式与 timing 表达式）；
   actor create 内 `dispatch self.method(...)` 禁止（既有 source + IR 校验
   同步到表达式形态）；receiver / 参数 / timing 各只求值一次（plan 中每个
   参数与 timing 都是单一 expression ref）。
4. lowering/IR：dispatch 表达式 → task-submit plan（沿用 dispatchSubmit
   机制），dispatchSubmit metadata 增加 `timing` 字段：
   `{kind:"immediate"}` / `{kind:"after"|"at", expr:<u32>}`（expr 为 executable
   body 表达式表中的 timing 表达式索引，D4 runtime 求值一次）；既有
   callable/payload plan 形状（targetKind / target / args）不变。语句位置的
   dispatch 继续 lower 成 `StmtIr::Dispatch { call }`；表达式位置的
   dispatch lower 成带 dispatchSubmit metadata 的 `ExprIr::Call`（D4 消费：
   遇到该 metadata 即 submit 并返回 TaskRef，本任务不改 runtime）。
5. 测试（reference 矩阵 1–4）：立即 / after / at 正例（表达式位置、赋值、
   参数、单独成行丢弃值）；负例（保留名、target 非 call、负 duration /
   类型错误、db transaction、create 内 self、非 void、不可恢复参数）；
   一次 source operation 各只求值一次（plan 级计数 + parser AST 计数）；
   既有 parser / compiler 测试同步到新语义。

## 预检结论（只读，锚定 baseline 16c177a0）

- parser：`syntax/src/parser/stmt.rs` `parse_statement` 用
  `match_ident("dispatch")` 解析 `Stmt::Dispatch { call }` 并强制 call；
  `syntax/src/parser/expr.rs` `parse_primary` 对 `dispatch` 直接报
  “dispatch is a statement and cannot be used as an expression”。
- 保留名：`compiler/core/src/prelude_registry.rs`
  `RESERVED_ROOT_NAMES = [service, std, connect, config, root]`；
  `compiler/source/src/reserved_names.rs` 检查 top-level 声明、import
  alias、参数与 block binding；`package_rules/root_validation.rs` 检查
  type/alias/interface/function/const/impl target。`dispatch` 不在任何
  列表，需补 enforcement。
- prelude：`COMPILER_BUILTIN_TYPES`（compiler-core）安装到
  `builtin_type_names` / `type_symbols` / `file_ir_builtin_source_spellings`
  / `prelude_types`（保留名）。`std.task.TaskRef` 目前不存在。
- dispatchSubmit plan：`compiler/lowering/src/function_lowering.rs`
  `lower_task_stmt` 把 call lower 进 body 表达式表，把 metadata
  （targetKind / target）插到 `CallIr.metadata["dispatchSubmit"]`，包成
  `StmtIr::Dispatch { call }`；`compiler/core/src/dispatch_targets.rs`
  `service_task_targets_with_packages` 只投影 function target（actorMethod
  留 assembly 链接）。metadata 是 `BTreeMap<String, MetadataValue>`，可
  承载嵌套 object / number，无需改 `CallIr` 结构（runtime linked `CallIr`
  无 `deny_unknown_fields`，不解析新 metadata 键也不受影响）。
- 保留名现状补查：全 pipeline 实际生效的 enforcement 是
  `package_rules/reserved_validation.rs::is_reserved_root`（仅
  std/ext/connect/config/root），`reserved_names.rs` 无生产调用方；D3 两处
  都补 `dispatch`。
- `std.task.TaskRef` 作为 qualified std type 会经过
  `package_rules/type_name_validation.rs` 的 std root allowlist；`std.task`
  不在 std 包 api.yml 导出中，需允许 compiler-owned builtin 的 qualified
  spelling（`compiler_builtin_type(bare)` + `known_type_symbol` 双检查）。
- 既有编译器检查：`expression_type_model.rs` `check_task_stmt` 校验 target
  return void/null；`callable_effects/transfer/statement.rs` 记录
  `EscapeLane::Dispatch`；`actor_method_validation.rs` source（create 内
  self method 调用）与 IR（create executable 内同 actor 方法调用）两层
  校验。db transaction 内禁止 dispatch 的静态检查当前不存在，本任务新增。
- timing 类型：`Duration` 是 std 包类型（`std/time.skiff`：
  `type Duration = integer`，`Duration.milliseconds` 等 native static，
  prelude type symbol `std.time.Duration`）；`Instant` 在 std 中尚不存在，
  `at(t)` 类型检查按 `std.time.Instant` 或裸 `Instant` 解析名接受（测试
  fixture 自行声明 `type Instant = Date`；D4 / 后续 std API 节点再定稿
  std.time.Instant 拼写，本任务不改 std 源码）。
- duration literal：`TokenKind::Duration` 目前只在 `timeout(...)` 合法且
  `checked_milliseconds` 拒绝 0；dispatch `after(200ms)` / `after(0ms)`
  需要新路径。决策：parser 在 `after(...)` 内把 duration literal 脱糖为
  `Duration.milliseconds(<ms>)` 普通调用（0 自然合法；`-1ms` 在 parser
  拒绝），复用既有 call 类型检查 / lowering / runtime 求值。
- 与兄弟节点无重叠：D1 已合入集成分支（transport/task-control/router 机械
  闭合）；D2 worktree `skiff-d2-router` 只改 `task-control/src/*`；本任务
  只改 syntax / compiler / doc 新叶子，不碰 runtime、router、task-control、
  wire。

## 实现顺序（实际修改集见提交交接）

1. `compiler/core/src/prelude_registry.rs`：`RESERVED_ROOT_NAMES` 增加
   `dispatch`；`COMPILER_BUILTIN_TYPES` 增加 `TaskRef`
   （symbol `std.task.TaskRef`，OpaqueHandle）。
2. `syntax/src/ast.rs`：删除 `Stmt::Dispatch`；新增
   `Expr::Dispatch { call: Box<Expr>, timing: Option<DispatchTiming> }` 与
   `DispatchTiming::{After(Box<Expr>), At(Box<Expr>)}`。
3. `syntax/src/parser/*`：`parse_primary` 接 `dispatch`，新增
   `parse_dispatch_expression`（call + 可选 timing clause；after 内 duration
   literal 脱糖；重复 timing clause 报错）；`parse_statement` 删除
   statement 分支。
4. `syntax/src/ast_utils.rs`：语句 visitor 删除 Dispatch 分支；表达式
   visitor / contains / import collector 增加 Dispatch（call + timing）。
5. `compiler/source`：所有 `Stmt::Dispatch` 分支按新语义迁移；所有穷尽
   `Expr` match 增加 `Expr::Dispatch`；`expression_type_model` 实现
   `check_dispatch_expr`（TaskRef 类型、void/null、timing 类型、db
   transaction depth、diagnostics）；`expression_model` 处理 children
   （call + timing）。
6. `compiler/lowering`：`function_lowering.rs` 语句位置 dispatch 保持
   `StmtIr::Dispatch`，表达式位置 lower 成带 metadata 的 call；metadata
   增加 `timing`；`actor_method_validation.rs` source/IR 校验表达式形态；
   其它穷尽 match 同步。
7. `compiler/core/src/dispatch_targets.rs`：dispatchSubmit metadata 校验
   timing 字段形状（kind 三态、after/at 必须带 expr）。
8. 测试：`syntax/src/parser/tests/dispatch.rs` 重写；compiler-source /
   compiler-lowering / compiler-core 增加聚焦用例；既有用例同步。
9. 自验收：受影响 crates `cargo check` + 聚焦 `cargo test`，按矩阵记录。

## 禁止

- 不改 runtime（eval/host）行为与 wire（`runtime/**`、`task-control/**`、
  `router/**` 不写）。
- 不改 `doc/reference/dispatch.md` 与 `doc/architecture/durable-task-dispatch.md`
  既有内容；`doc/reference/syntax.md` 是否补 expression statement 枚举由
  本任务自行判断，若改则列入交接。
- 不改 `doc/implementation/**` 既有文件（本叶子为新增）。
- 不 push、不写共享集成分支、不动共享主 worktree。

## 自验收矩阵（提交后与交接报告一致）

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| dispatch 是表达式（任意位置 + 单独成行） | `syntax/src/ast.rs` 有 `Expr::Dispatch`；parser `parse_dispatch_expression`；无 `Stmt::Dispatch` | `rg -n "Stmt::Dispatch\|dispatch is a statement" syntax compiler` 为空 | parser dispatch 测试 + `cargo test -p skiff-syntax parser::tests::dispatch` |
| 保留名 dispatch（function/const/type/alias/interface/import/binding） | `RESERVED_ROOT_NAMES` 含 `dispatch`；`reserved_names.rs` 既有路径生效 | `rg -n "\"dispatch\"" compiler/core/src/prelude_registry.rs` 命中 | compiler-source 保留名负例测试 |
| TaskRef prelude + dispatch 类型 | `COMPILER_BUILTIN_TYPES` 含 TaskRef；`check_dispatch_expr` 返回 TaskRef | prelude registry 测试断言 TaskRef symbol | compiler-source / core 测试 |
| timing plan immediate/after/at | `function_lowering.rs` metadata `timing` 字段；`dispatch_targets.rs` 校验 | `rg -n '"timing"' compiler` | lowering plan 断言 + dispatch_targets 测试 |
| target void/null、recoverable、db transaction、create self 保留/新增 | `check_task_stmt` 语义迁入 `check_dispatch_expr`；`db_transaction_depth`；actor_method_validation | 负例测试逐条命中诊断 | compiler-source / lowering 测试 |
| 每项只求值一次 | plan 中 args / timing 各为单一 expression ref；AST 单节点 | 副作用计数测试 | parser / lowering 测试 |

## 实际写集

```text
compiler/Cargo.toml                                  # 新增 dispatch_grammar 集成测试 target
compiler/core/src/prelude_registry.rs                # RESERVED_ROOT_NAMES + TaskRef builtin
compiler/core/src/prelude_registry/tests.rs
compiler/core/src/dispatch_targets.rs                # dispatchSubmit timing 校验
compiler/core/src/dispatch_targets/tests.rs
compiler/lowering/src/function_lowering.rs           # 表达式 lowering + timing plan
compiler/lowering/src/actor_method_validation.rs
compiler/lowering/src/executable_declaration_lowering.rs
compiler/lowering/src/suspend_analysis.rs
compiler/lowering/src/type_inference.rs
compiler/lowering/src/source_file_lowering/tests.rs
compiler/source/src/callable_effects/transfer/{statement,expression}.rs
compiler/source/src/callable_effects/tests/escape_boundaries.rs
compiler/source/src/config_usage/ast.rs
compiler/source/src/execution_semantics/{collectors,mutation,owner}.rs
compiler/source/src/expression_model.rs
compiler/source/src/expression_type_model.rs          # check_dispatch_expr + db transaction depth
compiler/source/src/expression_type_model/db_typing.rs
compiler/source/src/package_rules/{reserved_validation,type_block_validation,type_expr_validation,type_name_validation}.rs
compiler/source/src/resolved_call_targets/builder.rs
compiler/source/src/source_name_resolution.rs
compiler/source/src/source_rules/function_type_validation.rs
compiler/source/src/source_rules/stream_emit/{coverage,mod,statements,types}.rs
compiler/source/src/tests.rs
compiler/source/src/tests/dispatch_source_semantics.rs
compiler/tests/dispatch_grammar.rs
syntax/src/ast.rs
syntax/src/ast_utils.rs
syntax/src/lexer.rs
syntax/src/parser/{cursor,expr,mod,span,stmt}.rs
syntax/src/parser/tests/dispatch.rs
syntax/src/parser/tests/data/fixture-parse-output-baseline.txt
doc/implementation/dispatch-d3-grammar-leaf.md
```

不写：`runtime/**`、`router/**`、`task-control/**`、wire、共享主 worktree。

## 验证记录

- `cargo test -p skiff-syntax`：162 通过（parser dispatch 7 例 + baseline
  重新生成后全绿）。
- `cargo test -p skiff-compiler-core --lib`：70 通过（prelude TaskRef +
  dispatch_targets timing 校验）。
- `cargo test -p skiff-compiler-source --lib`：367 通过（新增
  dispatch_source_semantics 10 例 + callable_effects Dispatch escape 表达式
  形态）。
- `cargo test -p skiff-compiler-lowering --lib`：77 通过（timing plan
  断言、actor create dispatch self 负例）。
- `cargo test -p skiff-compiler`：全集成套件通过（含新增 dispatch_grammar
  3 例：full pipeline 表达式正例 + 保留名五类声明 + 局部/pattern binding）。
- `cargo test -p skiff-test-runner --test test_service_flow`：16 通过
  （含含 dispatch 语句的 checked-in test services 编译装配）。
- `cargo check --workspace` 通过；`git diff --check` 通过。
