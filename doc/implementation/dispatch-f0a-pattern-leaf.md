# Leaf Task: F0a compiler record-pattern match 降级修复

## 引用链

- 用户面契约：`doc/reference/dispatch.md` §3（`TaskStatus` /
  `TaskCancelResult` 为 discriminated union，用户必须能按 `kind` 分支匹配）。
- 批次父节点：`doc/implementation/dispatch-e-batch.md`（F0a 为 dispatch
  阶段 F 首节点，批次行 baseline `integration@62d8ee99`；集成 Agent
  `/root/dispatch_e_integration`）。
- 发现来源：`doc/implementation/dispatch-e3a-e2e-leaf.md`（E3a 记录
  “compiler record-pattern match 降级缺陷”，fixture 用
  `std.json.encode` / `std.json.decode` 投影绕道）。
- 仓库规则：`/Users/geek/workspace/AGENTS.md`、
  `/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。
- baseline：`62d8ee996361cbf74ab3429e378c6cc3a6db309e`
  （`dispatch-e-integration` 记录 E3a 合并点，已 `git rev-parse` 验证）。
- worktree：`/Users/geek/workspace/skiff-f0a-pattern`，branch
  `pattern-match-fix`。
- 集成 Agent：`/root/dispatch_e_integration`；主 Agent：`/root`。本任务不
  merge、不 push、不写共享集成分支；共享主 worktree 只读。

## 任务合同摘要

修复 compiler/lowering 中 record pattern 一律降级为 wildcard 的缺陷：
`match` 对 discriminated union（如 `std.task.TaskStatus` /
`std.task.TaskCancelResult`）的 record pattern 必须按 `kind` 判别选择正确
分支，字段绑定正确；其它 pattern 类型（literal、wildcard、binding、array
等）语义不变。

交付范围：

1. 预检定位 record pattern 降级的确切位置与既有 pattern/IR 能力，判断最小
   正确修复是补 record pattern 匹配（kind 判别 + 字段绑定）还是已有机制被
   错误绕开。
2. 实现修复（PatternIr 扩展 + lowering + runtime 判别/绑定）。
3. 测试：parser/lowering/runtime 各层正例与负例；E3a durable-task E2E
   fixture 中把 std.json 投影绕道改回直接 match TaskStatus；既有
   compiler/lowering/runtime match 测试全绿。
4. 叶子文档记录设计决策与反事实。

禁止：

- 不改 dispatch 执行语义与 wire；
- 不 push；不动共享主 worktree；不改 `doc/reference/` 与
  `doc/architecture/` 既有文档；不改 `doc/implementation/**` 既有文件
  （本叶子文件除外）。

## 预检结论（只读，锚定 62d8ee99）

- 集成分支 `dispatch-e-integration` 在 62d8ee99 后新增一个纯文档提交
  `e75aa2f2`（批次索引登记 F0a/F0b 行）；F0a 批次行 baseline 正是
  `integration@62d8ee99`，故本任务按任务书从 62d8ee99 建分支。
- 缺陷确切位置：`compiler/lowering/src/function_lowering.rs` 的
  `lower_pattern_and_bind` 中
  `skiff_syntax::ast::Pattern::Record { fields }` 分支只调用
  `declare_pattern_fields`（为裸字段声明 slot）后返回
  `PatternIr::Wildcard`，导致 record pattern 恒命中，match 恒走第一分支。
- 既有能力：
  - parser 已正确解析 record pattern（`syntax/src/parser/pattern.rs`：
    `{ kind: "succeeded" }` → `Pattern::Record`，字段可带子 pattern）；
  - `PatternIr` 目前只有 `Wildcard | Literal | Type | Binding`（artifact：
    `artifact-model/src/executable.rs`；linked：
    `runtime/linked-program/src/linked.rs`），没有承载 record 字段的变体；
  - runtime match 执行：`runtime/eval/src/eval_context.rs`
    `exec_statement_match` 按 arm 顺序调用
    `runtime/eval/src/program_ir.rs` 的 `program_pattern_matches` /
    `bind_program_pattern`；`PatternIr::Binding` 已能“匹配任意值并绑定
    slot”，`PatternIr::Literal` 已能按 `runtime_values_equal` 判别；
  - 运行时 discriminated union 值（含 TaskStatus/TaskCancelResult）是 heap
    object，字段 `kind` 等按名可查（`RequestHeap::object_field_carrier`；
    `runtime/model/src/type_plan/builtins.rs` 的 union branch plan）。
- 结论：这是“缺能力”而非“机制被绕开”——record pattern 的判别与字段绑定
  需要 PatternIr 携带结构化字段并让 runtime 递归匹配/绑定；不能只靠既有
  `Binding`/`Literal` 表达。最小修复见下。

## 关键实现决策

### 1. PatternIr 增加 Record 变体（最小方案）

- artifact 与 linked `PatternIr` 增加
  `Record { fields: Vec<RecordPatternFieldIr> }`；
  `RecordPatternFieldIr { name: String, pattern: PatternIr }`（serde
  camelCase，tag=kind；新增变体是 wire 后向兼容的增量，旧 artifact 无此
  变体照常反序列化）。
- `Pattern::Record` lowering：裸字段
  `{ status }` → `PatternIr::Binding { slot }`（复用既有
  `declare_slot`），显式子 pattern `{ kind: "succeeded" }` → 递归 lowering
  为 `PatternIr::Literal`；嵌套 record/binding 递归成立。
- 不新增语言概念、不新增 dispatch/union 专用机制；runtime 只复用既有
  object field 查找、`runtime_values_equal` 与 slot 绑定。

### 2. runtime 判别与绑定

- `program_pattern_matches`：Record 要求值为 heap object，所有字段存在且
  子 pattern 递归匹配；缺失字段/非 object 视为不匹配（落到下一 arm）。
- `bind_program_pattern`：Record 递归把每个字段值绑定到其子 pattern 的
  slot；无绑定字段（纯 literal）不产生 slot。
- 其它 pattern 语义不变；`PatternIr::Type` 仍保持“erased runtime value 上
  不能匹配”的既有行为，不在本叶子扩展 nominal 运行时匹配。

### 3. PatternIr 消费方收敛

- `runtime/linker/src/linker/file_conversion.rs::linked_pattern` 递归转换
  Record 字段（含嵌套 Type ref 转 linked）；
- `compiler/lowering/src/publication_local_refs.rs::rewrite_pattern` 与
  `compiler/lowering/src/external_refs.rs::collect_pattern_external_refs`
  递归处理 Record 字段（嵌套 Type 的外部 ref 仍被收集/改写）。

### 4. E3a fixture 改回直接 match

- `test-runner/fixtures/durable-task-e2e-live/main.skiff` 的
  `statusKind` / `cancelKind` 去掉 `std.json.encode/decode` 投影，改为
  `match` TaskStatus / TaskCancelResult 的 record pattern 分支返回
  `kind`；probe 断言（`succeeded` / `alreadyStarted` 等）不变。

### 反事实检查

- 若只改 lowering 让 Record 携带 `Binding` 字段而不加 runtime 判别：字段
  绑定成立但 `{kind:"succeeded"}` 无法限制分支（literal 子 pattern 无法
  携带），第一分支仍恒命中——不满足 §3。
- 若只加 runtime Record 判别而 lowering 仍降级 wildcard：IR 没有字段
  信息，runtime 无可匹配——不满足。
- 若另建 union-kind 专用 IR（如 `UnionBranch { kind }`）：与通用 record
  判别重复，且丢失“其它字段同时匹配/绑定”的表达力——删除它后既有能力
  不可组合出该语义，故不采用。

## 写集

### 生产代码

- `artifact-model/src/executable.rs`：PatternIr 增加 Record + 字段结构。
- `runtime/linked-program/src/linked.rs`：同款 linked PatternIr 扩展。
- `runtime/linked-program/src/lib.rs`：re-export `RecordPatternFieldIr`。
- `runtime/linker/src/linker/file_conversion.rs`：`linked_pattern` 递归。
- `compiler/lowering/src/function_lowering.rs`：`Pattern::Record` lowering
  产出 `PatternIr::Record`；Nominal 路径保持 `PatternIr::Type` 不变。
- `compiler/lowering/src/publication_local_refs.rs`、
  `compiler/lowering/src/external_refs.rs`：Record 递归。
- `runtime/eval/src/program_ir.rs`：Record 判别 + 字段绑定。

### 测试

- `syntax/src/parser/tests.rs`：record pattern 解析正例。
- `compiler/lowering/src/source_file_lowering/tests.rs`：record pattern
  lowering（kind literal + 裸字段 binding slot）。
- `runtime/eval/src/program_ir.rs`：runtime Record 判别/绑定单元测试
  （命中分支、乱序分支、未知 kind 负例、字段绑定）。
- `runtime/linker/src/linker/file_conversion/tests.rs`：Record pattern
  artifact → linked 递归转换。
- `compiler/tests/dispatch_grammar.rs`：std.task TaskStatus/TaskCancelResult
  全管线 direct match + artifact 形状断言。
- `test-runner/fixtures/durable-task-e2e-live/main.skiff`：直接 match
  TaskStatus / TaskCancelResult。

### 叶子文档

- `doc/implementation/dispatch-f0a-pattern-leaf.md`（本文件）。

## 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试命令 |
| --- | --- | --- | --- |
| PatternIr 扩展 Record + 字段结构（artifact/linked） | `artifact-model/src/executable.rs`、`runtime/linked-program/src/linked.rs` 新增 `Record { fields }` + `RecordPatternFieldIr` | `rg -n "PatternIr::" compiler runtime`：全部消费方已收敛（eval/linker/publication_local_refs/external_refs/tests） | `cargo check -p skiff-artifact-model -p skiff-runtime-linked-program -p skiff-runtime-linker -p skiff-runtime-eval`（PASS） |
| lowering：record pattern 不再降级 wildcard，kind literal 与字段绑定保留 | `function_lowering.rs` `Pattern::Record` 分支产出 `PatternIr::Record`；裸字段 → `PatternIr::Binding { slot }` | `rg -n "Pattern::Record" compiler/lowering`：仅此一处 pattern lowering；Nominal 仍走 `PatternIr::Type` | `cargo test -p skiff-compiler-lowering record_pattern`（2 PASS：kind literal + 裸字段 slot、嵌套 record） |
| runtime：kind 判别选择正确分支、字段绑定正确 | `program_ir.rs` `program_pattern_matches` Record 分支（object 字段逐一递归匹配）、`bind_program_pattern` Record 分支（递归绑定 slot） | Record 不匹配/缺失字段落到下一 arm；`PatternIr::Wildcard/Literal/Binding/Type` 原语义未改 | `cargo test -p skiff-runtime-eval record_pattern`（6 PASS：命中、乱序、未知 kind、缺失字段、嵌套绑定、标量负例） |
| 乱序分支、未知 kind 负例 | 同 runtime 测试 `record_pattern_kind_discriminates_arms_in_any_order` / `record_pattern_unknown_kind_matches_no_literal_arm` | match arm 顺序由 lowering 保留（`lower_match_arm` 顺序 push） | 同上 |
| artifact → linked 递归转换 | `file_conversion.rs::linked_pattern` Record 分支 | 无第二套 linked pattern 转换 | `cargo test -p skiff-runtime-linker linked_pattern_converts_record_fields_recursively`（1 PASS） |
| parser 层 record pattern 正例 | `syntax/src/parser/tests.rs` `match_record_pattern_parses_kind_literal_and_bare_field` | parser 本身未改（`syntax/src/parser/pattern.rs` 零 diff） | `cargo test -p skiff-syntax match_record_pattern_parses_kind_literal_and_bare_field`（1 PASS） |
| dispatch 集成：E3a fixture 改回直接 match TaskStatus/TaskCancelResult | `test-runner/fixtures/durable-task-e2e-live/main.skiff` `statusKind`/`cancelKind` 用 record pattern match | fixture 中不再出现 `std.json.encode/decode` 投影；probe 断言 kind 字符串不变 | `cargo test -p skiff-compiler --test dispatch_grammar`（5 PASS，含 TaskStatus/TaskCancelResult 全管线 + artifact `PatternIr::Record` 形状断言） |
| 既有 compiler/lowering/runtime match 测试全绿 | 无既有 match 测试被改 | `git diff --name-only` 写集仅限上表文件 | `cargo test -p skiff-compiler --test runtime_slots`（40 PASS）；`cargo test -p skiff-runtime-eval --lib`（469 PASS）；`cargo test -p skiff-compiler-lowering --lib`（88 PASS）；`cargo test -p skiff-runtime-linker --lib`（89 PASS） |

### 基线既有失败（与本次改动无关，未触碰）

- `skiff-artifact-model --lib` `native_callable_semantics_registry_is_sparse_exact_and_safe`
  FAILED：62d8ee99 的 `native_signature.rs` 已含 `std.task.status/cancel`
  语义而测试期望表未同步（E1 遗留）。
- `skiff-syntax --lib` `fixture_parse_output_matches_phase0_baseline` FAILED：
  E1 新增 `std/task.skiff` 后 Phase 0 解析 golden 未重新生成（entry 76→78）。

## 停止条件

- 需要改变公共契约/架构语义、新增语言概念或集中式 owner：不原地猜测
  设计，停止上报。
- 预检/实现发现与既有 TaskStatus union 运行时表示不一致：停止上报。
