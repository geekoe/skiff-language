# P5-F445D Timeout syntax checkpoint result

状态：`COMPLETED`。syntax 边界能够形成稳定、完整 AST，没有触发停止条件。

本节点完整落地了 F445B-I1 的通用语言 surface；没有加入 Agine / WebSocket 特殊 spelling，
没有修改 compiler、artifact、runtime 或 std，也没有让普通 call/block 冒充 timeout/value/
concurrent/serial 节点。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| 任务指定 integration input | `d7596b4b` |
| 本 task 初始 HEAD | `42edd1b5` |
| implementation | `dd73e4bc` |

implementation 写集只有：

- `syntax/src/ast.rs`
- `syntax/src/lexer.rs`
- `syntax/src/parser.rs`
- `syntax/src/ast_utils.rs`
- `syntax/src/parser/tests.rs`
- `syntax/src/ast_utils/tests.rs`

implementation 提交后只新增本 result。没有 merge、rebase、push、stable、live、network 或
instance 操作。

## 2. Test-first 证据

先新增 parser/lexer/visitor/round-trip 测试，再运行任务指定的 syntax test。此时 production
尚未修改，命令真实 RED，`cargo test` 以 exit `101` 停止并报告 15 个预期缺口：

- `DurationUnit` 不存在；
- `TokenKind::Duration` 不存在；
- `Stmt::{Timeout,Concurrent,Serial}` 不存在；
- `Expr::{ValueBlock,ConcurrentValue,Timeout}` 不存在。

随后才实现 production 并转为 GREEN。

任务文件原命令使用共享
`/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`。首次 RED 和提示前的中间实现
迭代按该任务命令执行；随后 owner 提示该共享 target 会污染并行任务的 Cargo 依赖路径。本节点
没有清理或改动共享 target；从提示开始，所有 Cargo 命令均改用本任务独立路径：

```text
/Users/geek/workspace/skiff-p5-f445d-timeout-syntax/build/cargo-target
```

最终 GREEN、fmt check 都基于独立 target，不复用首次 RED 的共享构建结果。

## 3. 冻结的 syntax / AST surface

### 3.1 Duration literal

`TokenKind::Duration(DurationLiteral)` 是独立 token。`DurationLiteral` 保留：

- 原始正整数 `digits: String`，不经过 `f64`；
- `DurationUnit::{Milliseconds,Seconds,Minutes,Hours,Days}`；
- 精确 token `SourceSpan`。

`DurationLiteral::checked_milliseconds()` 使用整数 parse、checked multiplication 和
`9_007_199_254_740_991` safe-integer 上限；零、整数 parse overflow、单位乘法 overflow 和
safe-ms overflow 都 fail closed。`DurationUnit::suffix()` 与
`milliseconds_multiplier()` 为后继 consumer 提供唯一单位映射。

lexer 只在 digits 与 `ms|s|m|h|d` 连续时产生 duration token。小数加单位、未知连续单位和零
直接返回稳定 syntax diagnostic；负号或空格会使 timeout 参数不再是一个 duration token。
duration token 在普通 expression 位置显式拒绝。

### 3.2 Statement/value AST

| source | AST |
| --- | --- |
| `timeout(200ms) { ... }` | `Stmt::Timeout { duration, body }` |
| `concurrent { ... }` | `Stmt::Concurrent { body }` |
| `serial { ... }` | `Stmt::Serial { body }` |
| `value { ... tail }` | `Expr::ValueBlock(ValueBlock { body, tail })` |
| `concurrent value { ... tail }` | `Expr::ConcurrentValue(ValueBlock { body, tail })` |
| `timeout(200ms) value { ... tail }` | `Expr::Timeout { duration, value: ValueBlock }` |
| `timeout(200ms) concurrent value { ... tail }` | `Expr::Timeout { duration, value: ConcurrentValue }` |

`ValueBlock.body` 不包含 tail；`tail: Box<Expr>` 是独立且必需的 AST 字段。timeout value
始终是外层 wrapper，不能把 timeout 藏进 concurrent lane。`serial` 保留显式 body；
其只能作为 concurrent 直属 lane 的限制仍由 I2 判断。

parser 只接受 canonical modifier 顺序。value tail 可继续是普通 expression、嵌套 canonical
value expression、throw/rethrow expression，或括号包裹的 object literal。F445B 要求的
`catch<TimeoutError>(timeout(...) value { ... })` 括号形态已由正例覆盖。

### 3.3 Span、serde 与 AST utilities

新 AST 严格 round-trip。source span 布局冻结为：

- timeout expression：一个 child，即被包装的 value/concurrent-value expression；
- value/concurrent-value：一个 block，即不含 tail 的 body；一个 child，即 tail expression；
- timeout/concurrent/serial statement：一个 block，即 statement body；
- duration span 直接保存在 `DurationLiteral`。

`AstVisitor` / `AstVisitorMut` 新增 duration hook，并完整遍历 timeout/concurrent/serial body、
value body 和 tail。`expr_contains`、`stmt_contains_expr`、type-ref/dotted-root collector
也覆盖全部新路径；只读和 mutable visitor、body/tail 顺序、dotted-root 收集均有直接测试。

## 4. Syntax 负例闭包

聚焦测试稳定拒绝：

- `0s`、`-1s`、`1.5s`、`15 s`、`1x`；
- exact safe-ms 上界以上和超出 `u64` 的纯 digits；
- duration 出现在 timeout 之外；
- `timeout()`、`timeout {}`、`timeout(1s)`；
- 空 value body、只有 statement 而没有 tail；
- 未加括号的 object-literal tail；
- `concurrent timeout ...`、`value concurrent ...`、
  `timeout(...) value concurrent ...`、`timeout(...) serial value ...` 和
  `serial value ...` 等非 canonical modifier。

作用域、`return` / `break` / `continue` 限制、concurrent surface 合法项、sibling const、
lane DAG、mutation/effect/cancel-safety 不在 parser 伪实现，继续由 I2 fail closed。

## 5. I2 exhaustive consumer handoff

I2 不能只在主 type checker 增加几处 match arm。以下是当前
`compiler/source/**` 中直接匹配 `Stmt` / `Expr` 或消费其 traversal 的 production inventory，
必须逐项补齐或明确证明由 syntax visitor 自动覆盖：

```text
compiler/source/src/callable_effects/transfer/expression.rs
compiler/source/src/callable_effects/transfer/statement.rs
compiler/source/src/config_usage/ast.rs
compiler/source/src/config_usage/validation.rs
compiler/source/src/contract_type_resolution/validation.rs
compiler/source/src/expression_model.rs
compiler/source/src/expression_type_model.rs
compiler/source/src/expression_type_model/expression_assignability.rs
compiler/source/src/package_db_schema/field_paths.rs
compiler/source/src/package_rules/type_block_validation.rs
compiler/source/src/package_rules/type_expr_validation.rs
compiler/source/src/prelude_registry/mod.rs
compiler/source/src/provider_rules.rs
compiler/source/src/resolved_call_targets/builder.rs
compiler/source/src/root_projection_validation/mod.rs
compiler/source/src/root_refs/mod.rs
compiler/source/src/semantic/interface.rs
compiler/source/src/source_name_resolution.rs
compiler/source/src/source_rules/function_type_validation.rs
compiler/source/src/source_rules/stream_emit/coverage.rs
compiler/source/src/source_rules/stream_emit/mod.rs
compiler/source/src/source_rules/stream_emit/statements.rs
compiler/source/src/source_rules/stream_emit/types.rs
compiler/source/src/type_resolution_model.rs
```

此外，`compiler/source/src/alias_resolution.rs`、
`compiler/source/src/callable_effects/{analysis.rs,transfer/call.rs}` 和
`compiler/driver/pipeline/mod.rs` 分别是 visitor rewrite、call/effect 汇总和 pass 注册 owner，
也必须纳入 I2 closure。

I2 的最低逐层合同是：

1. `expression_model.rs` 必须按第 3.3 节精确消费 `ExprSourceSpans`，为 body/tail 建立稳定
   preorder expression key；duration 本身不伪造 expression key。
2. `source_name_resolution.rs`、`root_refs/mod.rs` 与 resolved-target collector 必须建立
   value/timeout/concurrent 的词法 scope，并只让 concurrent 直属前序 `const` 成为
   sibling-visible binding。
3. `expression_type_model.rs`、`type_resolution_model.rs` 和 assignability/semantic consumer
   必须实现 statement 无值、tail expected type、timeout 类型透明、value control-flow 限制和
   serial/concurrent shape 检查。
4. callable effect/provenance、root projection 与 mutation consumer 必须完整透传 body/tail 的
   call、throw、mutation、root provenance 和 `maySuspend`；timeout 不得清空这些事实。
5. concurrent pass 必须建立 lane source order/dependency DAG，检查 outer-root mutation、
   effect/conflict-key/cancel-safety，并拒绝所有当前不允许的 concurrent surface。
6. config、function-type、stream-emit、package/type、DB-field-path、provider/prelude 和 contract
   validation 必须遍历 body/tail；任何未知新路径都 fail closed，不能用 wildcard 静默跳过。

本节点没有编译或修改这些 I2 consumer，也没有提前决定其 semantic/lowering/runtime 行为。

## 6. 验证

| 命令 | 结果 |
| --- | --- |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445d-timeout-syntax/build/cargo-target cargo test -p skiff-syntax --no-fail-fast` | PASS：124 tests，doc-tests 0 |
| `CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445d-timeout-syntax/build/cargo-target cargo fmt --check` | PASS |
| `git diff --check` | PASS |

没有运行 workspace/full gate、compiler/runtime tests、stable/live/network。
