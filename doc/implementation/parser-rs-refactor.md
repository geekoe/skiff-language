# parser.rs 职责重构设计

日期：2026-08-03

状态：proposed（待实施）

基线：commit `af7aada1`（此前 `69413059` 已移除 too_many_lines allowlist 机制；
`5c350fe7` 时点该机制尚未移除）。本文是 parser.rs 重构的唯一权威设计，阶段拆分、提交顺序和完成标准以此为准。

## 2026-08-03 审阅修订

本修订吸收独立审阅结论，只修正事实与口径，不改变目标架构和阶段划分：

- 热点函数行数统一按“`fn` 签名行到闭括号行（不含尾随空行）”统计，原表整体多计 1 行；
- `is_old_db_dotted_operation` 实际 19 行（39–57），不是 99 行，const 表化收益相应修正；
- `looks_like_generic_call_suffix` 克隆的是 `tokens` 与 `provider_capability`，
  `source_spans` 为 `SourceSpanTable::default()`，并非克隆；
- “provider 死表面”收缩为状态收集本身，provider 关键字错误分支是可达路径，删除边界明确；
- 补充跨文件 `impl Parser` 的可见性约定、`test_default_run_span` 链路、类型 golden 控制字符
  用例和 parse 差量载体；
- 验证命令按 AGENTS.md 口径修正为 `tests` selector。

## 1. 背景与门禁现状

`syntax/src/parser.rs` 当前为 4073 行，是全仓库最长的 Rust 文件，恰好等于
`scripts/check-rust-file-lines.mjs` 的 `MAX_FILE_LINES = 4073`。该门禁明确无例外、无 allowlist，
且提示“不要只拆文件”。这意味着当前文件任何净增一行都会立即失败，既是重构动因，也是每个中间提交
必须遵守的硬约束。

函数级门禁（`clippy.toml` `too-many-lines-threshold = 534`，`Cargo.toml` 中
`clippy::too_many_lines = "deny"`，无白名单）目前不触发：全文件 132 个函数中最长的是
`parse_db_operation_expr`（195 行）。重构时仍须保证任何新函数不超过 534 行，避免把文件级问题
转成函数级问题。

## 2. 现状诊断

### 2.1 职责清单

`parser.rs` 实际是“一个结构体 + 六个职责域”：

1. **Token 游标**：`peek/advance/previous/match_*/check_*/expect_*`、`skip_balanced_block` 等
   机械操作（约 140 行）。
2. **语法域**：顶层声明（import/type/actor/alias/interface/impl/db/const/function/test）、类型
   文本、语句与控制流、表达式、pattern、db 声明与 db 表达式、test effects。
3. **AST 与 span 双通道构造**：大部分解析返回 `ParsedStmt/ParsedExpr/ParsedBlock` 包装，但
   functions/impl_methods/consts/tests/db_index_wheres 的 span 又通过 `self.source_spans`
   副作用收集。
4. **解析策略状态机**：`ParseMode`（Full/Metadata/BodiesTolerant）×
   `CallableParseOptions`/`CallableBodyPolicy`/`NativeBodyPolicy`，同一策略在
   `parse_source_file`、`parse_impl`、`parse_callable_body` 多处重复展开。
5. **跨声明校验**：`validate_actor_declarations`、`validate_type_decl_discriminator`
   （约 160 行）。
6. **测试**：2630 行在 `parser/tests.rs` + 76 行 `parser/tests/spawn.rs`。

公共 API 极小：模块只暴露 `parse_source`、`parse_source_metadata`、
`parse_source_with_bodies_tolerant` 三个函数，其余全部私有。内部可大幅重组而不影响任何下游
（下游通过 `skiff_syntax::parser::parse_source` 调用，`pub mod parser` 路径保持不变）。

### 2.2 行数占比

| 职责域 | 行区间 | 行数 | 占比 |
|---|---|---:|---:|
| 入口 / ParseMode / Parser 状态 / 基础辅助 | 1–170 | 170 | 4% |
| 跨声明校验（actor/discriminator） | 171–332 | 162 | 3% |
| 声明解析 + 类型文本 + callable 策略 | 333–1661 | 1329 | 32% |
| test 声明与 effects | 1662–1949 | 288 | 7% |
| block / statement / pattern | 1950–2494 | 545 | 13% |
| 表达式核心（binary/unary/postfix/primary/value block 等） | 2495–3139 | 645 | 15% |
| db 表达式 | 3140–3734 | 595 | 14% |
| catch / record 构造 / object / patch | 3735–3895 | 161 | 3% |
| token 游标与运算符表 | 3896–4039 | 144 | 3% |
| 文件级辅助 + `mod tests` | 4040–4073 | 34 | 0% |

“声明解析 + 类型文本 + callable 策略”一块占 32%，是最大的单一责任堆叠；它把声明分派、类型字符串
拼接、函数体三种模式策略混在同一段。

### 2.3 函数级热点

| 函数 | 行数 | 行区间 | 问题 |
|---|---:|---|---|
| `parse_db_operation_expr` | 195 | 3269–3463 | 10 种 db operation 分支全在一个 match 里，深浅嵌套，`children` 手动累积 |
| `parse_primary` | 186 | 2736–2921 | 约 20 个关键字 arm 的巨型 match，含内联构造与错误提示 |
| `parse_statement` | 180 | 1977–2156 | 大量关键字分派，每个分支内联构造 `StmtSourceSpans`（全文件 20 处） |
| `parse_source_file` | 154 | 344–497 | 顶层分派 + 三种 ParseMode 的 function 分支内联展开 |
| `parse_postfix` | 138 | 2544–2681 | 后缀循环内混入 field/call/generic/`as`/patch/record 构造 |
| `parse_test_effects` | 111 | 1708–1818 | 与 `parse_test_effect_sequence`（97 行）重复约 60 行 |
| `parse_value_block_expression` | 110 | 2977–3086 | tail 判定逻辑复杂 |
| `is_old_db_dotted_operation` | 19 | 39–57 | 一个 `matches!` 列表占 19 行，改 const 表即可 |
| `parse_db_decl` | 96 | 856–951 | 十多个 db entry 关键字的 if-else 链 |
| `parse_db_change_block` | 74 | 3652–3725 | 五类 change op 的手动分支 |

行数口径：`fn` 签名行到闭括号行（不含尾随空行）。

### 2.4 重复模式

- `self.match_symbol(";") || self.match_symbol(",")` 表达式共出现 8 次（其中
  `let _ = …;` 形式 4 次，其余为 `if` 条件形式）→ 抽 `match_statement_terminator()`。
- `parse_impl_methods` / `parse_impl_methods_strict` / `parse_impl_methods_with_bodies_tolerant`
  三份几乎相同的循环骨架 → 合并为一个参数化函数。
- `parse_db_read_block` 与 `parse_db_query_block` 几乎相同 → 共享
  `parse_db_query_body(allow_fields)`。
- `parse_test_effects` 与 `parse_test_effect_sequence` 的字段循环重复 → 抽共享 outcome 解析器。
- desc/asc 方向解析在 db index 与 db query 两处重复 → 抽 `parse_index_direction()`。
- `parse_field_path` 与 `parse_patch_field_path` 逻辑相同，仅错误文案不同。
- `StmtSourceSpans` 内联构造 20 处（parser.rs:148 及 2020/2034/2053/2066/2081/2100/2111/2122/
  2141/2150/2182/2215/2238/2263/2288/2331/2357/2374/2404）、`children.push(x.spans.clone())`
  14 处、`.spans.clone()` 24 处 → span 装配 helper 化。

### 2.5 结构性欠债

1. **span 副作用通道**：`self.source_spans` 使 Parser 状态可变、装配路径不统一；
   `SourceFile.source_spans` 字段是 `#[serde(skip)]`（`ast.rs:34–35`；`SourceSpanTable` 本身
   无 serde 派生），完全可以从 Parser 移出，由 `parse_source_file` 局部收集。
2. **lookahead 克隆整个 Parser**：`looks_like_generic_call_suffix` 通过
   `Parser { tokens: self.tokens.clone(), ... }` 做试探解析，克隆的是 `tokens` 与
   `provider_capability`；`source_spans` 为 `SourceSpanTable::default()`，并非克隆。
   `parse_primary_pattern` 已有 `snapshot/restore current` 的先例，游标快照可完全替代克隆，
   并顺带消除克隆中的死状态字段。
3. **死状态 / 不可达分支**：`provider_capability` 字段与局部变量永远为 `None`（全文件无
   `Some` 赋值）；`reject_export_modifier` 三个入口（`parse_source`/`parse_source_metadata`/
   `parse_source_with_bodies_tolerant`）全部传 `true`。真正死的只有状态收集本身：字段、局部变量、
   克隆和 `parser.rs:799` 的冗余条件（约十余行）。`check_provider_capability_start`
   （3990–4004）、`parse_source_file` 的 provider 分支（403–407）、`interface_operation_start`
   （787）、`parse_function_modifiers` 的 provider 检查（1597）是可达错误路径，且被
   `rejects_provider_body_in_tolerant_mode`（tests.rs:1069）锁定。删除时只删状态收集，
   保留关键字错误分支。
4. **TypeRef 字符串化中间表示**：`parse_type/parse_primary_type/parse_record_type_name/
   parse_function_type_name/quote_string_type` 把类型拼成 `TypeRef { name: String }`，下游
   compiler 又用 `type_expr::TypeExpr::parse` 把这段文本再解析一遍。方向不是改 AST 结构，而是在
   syntax 内建立 token → 结构化类型 IR → 规范文本的单向管线，保持 `name` 字节不变。
5. **错误恢复未分层**：tolerant 模式靠“整体重试 + 回退 + skip_balanced_block”实现，与严格解析
   路径交织，应抽象为“保存游标 → 尝试 → 失败回退并跳过”的可恢复原语。

### 2.6 语义注意点

表达式已经是 precedence climbing（`parse_binary(min_prec)` + `peek_binary_op` 表），当前所有
运算符都是左结合、`prec + 1` 收紧右操作数。但没有任何测试锁定结合性。重构表达式前必须先补
结合性/优先级测试，避免无意识改变语义。

## 3. 目标架构

### 3.1 分层原则

按 rustc parser 风格重组：**一个 `Parser` 结构体（游标 + 模式），各语法域用独立文件里的
`impl Parser` 块实现**，避免模块环（表达式调 value block、语句调表达式、db 调 block），同时让
每个文件只有一个职责。这不是“把大文件切成小文件”，而是先剥离横切关注点：

1. **游标层**：只懂 token 移动，不懂 AST。提供 `snapshot()/restore()`，消灭 Parser 克隆。
2. **语法层**：按域分文件，函数只做“消费 token 产出 AST + span 包装”。
3. **装配层**：`ParsedStmt/ParsedExpr/ParsedBlock` 统一包装 + span helper；
   `parse_source_file` 负责把各域结果组装成 `SourceFile` 与 `SourceSpanTable`，Parser 不再持有
   span 表。
4. **策略层**：`ParseMode → CallableParseOptions` 单一映射函数；tolerant 回退收敛为
   `Recoverable` 原语。
5. **校验层**：跨声明校验与解析分离，保持独立模块。

### 3.2 目标模块划分

```
syntax/src/parser/
  mod.rs      入口 3 函数、Parser 定义、parse_source_file 编排、parse policy 映射
  cursor.rs   token 游标 + expect/match/check + snapshot/restore + skip_balanced_block
  span.rs     ParsedBlock/ParsedStmt/ParsedExpr + 装配 helper
  type.rs     token 驱动的类型解析 → 结构化类型 IR → TypeRef.name（规范文本）
  expr.rs     表达式：precedence 表、unary/postfix/primary、value block、object literal、
              record 构造、patch、catch
  stmt.rs     block、statement 分派、控制流（if/for/while/match/timeout/concurrent/serial）
  pattern.rs  pattern 三件套
  decl.rs     顶层声明：import/type/actor/alias/interface/impl/const/function + 类型参数
  callable.rs callable body 策略枚举 + parse_callable_body 家族 + build_function_decl
  db.rs       db 声明（object/index/retention/lease/storage）+ 全部 db 表达式
  test.rs     test 声明、effects、sequence、defaultRun
  validate.rs validate_actor_declarations / discriminator 校验
```

预计拆分后各文件 80–650 行，全部远低于 4073 门禁；单函数经第 4 节拆分后也全部远低于 534。

### 3.3 关键架构决策

- **类型结构化**：在 `type.rs` 内直接由 token 构建结构化类型（可复用/对齐
  `type_expr::TypeExpr`，它有 `to_type_string()` 和现成 round-trip 测试），再渲染成
  `TypeRef.name`。删除字符串拼接代码和 `quote_string_type`，新增“token 解析结果 == 文本再解析
  结果”的恒等测试，保证 `name` 字节不变。
- **span 单通道**：所有声明级解析改为返回 `(ast, spans)` 或复用已有 `Parsed*` 包装；
  `SourceSpanTable` 由 `parse_source_file` 在结束时组装。`Parser` 只剩 `cursor + mode`。
- **策略单点**：`fn callable_options(mode, exported) -> CallableParseOptions` 只定义一次，
  `parse_source_file` 与 `parse_impl` 不再各自 match ParseMode。
- **错误恢复原语**：`recoverable(|p| ...)` 或 `parse_or_skip(policy)` 包装“保存 → 尝试 → 回退
  跳过”，让 tolerant 语义局部化。
- **跨文件可见性**：拆分后 `cursor.rs`、`stmt.rs`、`expr.rs`、`db.rs` 等子模块的 `impl Parser`
  方法需要 `pub(super)` 或 `pub(crate)` 才能被兄弟模块调用；对外仍只公开
  `parse_source`/`parse_source_metadata`/`parse_source_with_bodies_tolerant` 三个函数。
- **移除死状态**：删 `provider_capability`、`reject_export_modifier` 配置位和不可达 provider
  状态收集（错误文案不变，provider 关键字错误分支保留，现有 provider 测试继续锁定行为）。

## 4. 分阶段计划

### Phase 0：测试先行（不动 parser.rs）

为后续行为等价重构建立基线。注意任何新测试都不能加进 parser.rs（已到门禁上限），一律放
`parser/tests.rs`（还有约 1443 行余量）或新建测试文件。

需要补的测试：

1. 二进制表达式结合性与优先级：`a - b - c`、`a / b / c`、`a + b * c`、`a && b || c`、
   `a < b == c` 的 AST 形状。
2. 类型文本 golden corpus：把现有测试和 fixtures 里出现的类型（泛型、record、fn 类型、
   `any I`、`/` 依赖路径、string literal 类型、nullable、union）收集成“输入 → 期望
   `TypeRef.name` 文本”表，并明确纳入控制字符用例（`serde_json` 转义与 `quote_string_type`
   的差异点）。
3. span 敏感用例：functions/impl_methods/consts/db_index_wheres 的 `SourceSpanTable` 内容；
   另含 `test_default_run_span`（`ast.rs:32–33` 同为 `#[serde(skip)]`，目前经局部变量而非
   `self.source_spans` 收集，单通道化时不要遗漏）。
4. tolerant 模式回退路径的精确行为（现有 827/852/1069 已覆盖部分，可补失败后游标位置断言）。
5. parse 输出差量载体：仓库当前没有现成 fixture 差量工具，Phase 3 的“全部 `.skiff` fixtures
   parse 输出差量”需要先落成可复现脚本或 `#[test]`，作为 Phase 0 交付的一部分。

验证：`cargo test -p skiff-syntax`；`node scripts/check-rust-file-lines.mjs`。

### Phase 1：机械解耦（纯行为等价，净减行）

1. 抽取 `cursor.rs`：`peek/advance/previous/match_*/check_*/expect_*/is_at_end/
   skip_balanced_block` 及 `import_tail_is_terminated`；新增 `snapshot()/restore()`。
2. 用快照替换 `looks_like_generic_call_suffix` 的 Parser 克隆。
3. 新增 span 装配 helper，替换约 15 处内联 `StmtSourceSpans`。
4. 删除死状态：只删 `provider_capability`、`reject_export_modifier` 状态收集与 `parser.rs:799`
   冗余条件，保留 provider 关键字错误分支；`is_old_db_dotted_operation` 改为 const 表
   （19 行 → 约 10 行，净省 5–9 行）。Phase 1 的净减行主力是 cursor 抽取、span helper、
   impl methods 合并等真实重复项。
5. 抽 `match_statement_terminator()`、`parse_index_direction()`；删 `parse_patch_field_path`，
   统一走 `parse_field_path(msg)`。
6. 合并 `parse_impl_methods` 三胞胎为单循环 + options 参数。

每步独立提交，parser.rs 行数只减不增。

### Phase 2：按语法域拆模块（文件门禁正式解除）

在 Phase 1 的干净结构上，把 `impl Parser` 块按
`stmt.rs / expr.rs / pattern.rs / db.rs / test.rs / decl.rs / callable.rs / validate.rs` 移动。
`parser/mod.rs` 保留入口 + `parse_source_file` 编排。移动时同步做函数级拆分：

- `parse_primary`（187 行）：按关键字抽 `parse_throw_expr/parse_rethrow_expr/parse_construct_tail`
  等；`as`/call/generic/field 后缀抽成 `parse_postfix_tail` 子函数。
- `parse_db_operation_expr`（196 行）：抽 `parse_db_find_like_tail / insert_tail / update_tail /
  upsert_tail / replace_tail / delete_tail`，公共头部（op 识别、many、target）保留。
- `parse_test_effects` / `parse_test_effect_sequence` 抽共享 outcome 字段解析。
- `parse_source_file`（155 行）的 ParseMode 分支替换为 `callable_options(mode, ...)` 单点调用。

提交顺序建议：先 `stmt.rs + pattern.rs`，再 `expr.rs`（最大、测试最多），再 `db.rs`，再
`decl.rs + callable.rs + test.rs`，最后 `validate.rs`。每拆一个模块跑一次 syntax 测试 + 文件门禁。

### Phase 3：架构深化（行为保持等价，除非显式决策）

1. **类型 IR**：`type.rs` 用 token 直接构建结构化类型，输出 `TypeRef.name`；用 Phase 0 的
   golden corpus 断言与旧文本逐字节一致；删除字符串拼接代码（预计净省 150 行左右）。先实现、再
   对照 corpus 跑差量，最后删旧函数。
2. **precedence 表形式化**：把 `peek_binary_op` 改为 `const BINARY_OPS: &[(BinaryOp, u8, Assoc)]`
   表；结合 Phase 0 新增的结合性测试保持语义不变。若发现结合性确实与语言定义不符，单独决策，
   不混在重构里改。
3. **错误恢复分层**：`recoverable` 原语落地，tolerant 路径不再手写“回退 + skip_balanced_block”。
4. 可选后续方向（不强制）：两阶段“CST + builder”目前收益有限（AST 已带完整 span），不建议本轮做。

## 5. 风险与对策

| 风险 | 说明 | 对策 |
|---|---|---|
| `TypeRef.name` 字节变化 | 文本进 identity/别名解析/降级链路；`type_expr::to_type_string` 用 serde_json 转义，与现 `quote_string_type` 在极端控制字符上可能有差异 | Phase 0 golden corpus；重构后对仓库全部 `.skiff` fixtures 做 parse 输出差量；先建后删 |
| tolerant 回退行为漂移 | 回退涉及游标复位与 skip 语义 | 现有 tolerant 测试在 Phase 0 补精确断言；Phase 1 先只做快照替换 |
| span 装配遗漏 | source_spans 移出 Parser 后易漏收集 | span 包装统一走 `span.rs` helper；compiler/test-runner 的 span 消费测试兜底 |
| 门禁中间态失败 | parser.rs 已满 4073，先加代码再删会失败 | 严格“先抽后加”，每个提交 `check-rust-file-lines` 必须过 |
| 过度抽象 | 小重复不值得引入通用组合子框架 | 只抽明确重复 ≥2–3 处的 helper；不建宏、不建 trait 泛化 |
| 下游编译面 | 3 个公开函数是唯一契约 | 保持 `skiff_syntax::parser::parse_source*` 路径与行为；跨 crate 测试在 Phase 2 边界补跑 |

## 6. 验证方式汇总

每个阶段提交前执行（在 `/Users/geek/workspace/skiff`）：

```bash
cargo test -p skiff-syntax
node scripts/check-rust-file-lines.mjs
cargo fmt --check -p skiff-syntax
cargo clippy -p skiff-syntax --all-targets
```

跨 crate 边界（Phase 2 完成后）：

```bash
cargo test -p skiff-compiler -p skiff-test-runner
# 按 AGENTS.md，完整测试使用 tests selector；权威命令：
node scripts/verify.mjs --only tests --list
# 聚焦 syntax 影响面可用：
node scripts/verify.mjs --only implementation-tests --list
```

全量门禁（收尾时跑）：

```bash
node scripts/verify.mjs --only rust-quality
pnpm verify   # 完整非 live 验证，耗时较长；AGENTS.md 提示不要放在关键路径上
```

## 7. 结论

`parser.rs` 的问题不是“文件太大”，而是游标、语法、span 装配、模式策略、校验五类职责挤在同一个
结构体里，且存在死状态、克隆式 lookahead、字符串化类型 IR 三处结构性欠债。推荐的落地顺序是：
Phase 0 补测试基线 → Phase 1 机械解耦（快照、span helper、删死代码，行为零变化）→ Phase 2 按
语法域拆模块（顺带拆巨型函数）→ Phase 3 类型结构化与 precedence 表形式化。第一批最适合动的就是
Phase 1 的游标快照替换和死状态删除：风险最低、立即释放门禁空间，且为后续拆分提供干净地基。
