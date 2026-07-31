# Phase 6 Leaf Task: Builtin shape 替换（行为等价）

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md`（v3，含 identity 决策），
  §5.2 `BuiltinShape`、§6 Phase 6。设计 baseline commit：
  `81523e01e10d4811572c8b13af9cd1404514bf77`（本 worktree 基线，`git show`/`git grep` 锚定读取）。
- 直接父节点：Phase 1–5 已合入 main。`compiler_core/src/type_ref.rs` 已有
  `BuiltinShape::of_name`（16 短名 + `std.collection.Array`/`std.collection.Map`/
  `std.stream.Stream` 全名）与全部 shape 函数（`single_item`/`map_entry`/
  `exception_payload`/`catch_result_branches`/`is_null_type`/`normalize_union`）；
  `of_name` 测试在 `compiler/core/src/type_ref/tests.rs:1010`。
- 流程：`/Users/geek/workspace/multi-agent-development.md`（开发 Agent 角色、零 worktree 预检、
  叶子执行合同、自验收、交接给 `skiff_integration_phase1`）。

## 开工前 rg 快照（baseline 81523e01，生产文件，不含 `*/tests.rs`）

全量 16 名口径（`"Array"|"Stream"|"Map"|"Exception"|"CatchResult"|"DbUpsertResult"|
"Json"|"JsonObject"|"null"|"void"|"never"|"unknown"|"string"|"integer"|"number"|"bool"`）：

| 文件 | 行数 |
| --- | --- |
| `expression_type_model.rs` | 51 |
| `type_resolution_model.rs` | 15 |
| `expression_type_model/contract_call_typing.rs` | 2 |
| `expression_type_model/contract_call_typing/type_projection.rs` | 18 |
| `expression_type_model/expression_assignability.rs` | 10 |
| `type_resolution_model/catch_leaves.rs` | 2 |
| `type_resolution_model/shape_assignability.rs` | 6 |
| 合计 | 104 |

设计验收口径（`"Array"|"Stream"|"Map"|"Exception"|"Json"|"CatchResult"|"null"|"void"`）：
同集合生产文件 49 行。`std.*` 全名在目标文件内仅 2 行（etm 4763、4978），
且均为"短名+全名"联合匹配，`of_name` 可无损覆盖。

测试文件（`*/tests.rs`）不含在验收范围；其中 `expression_type_model/tests.rs` 已有
`map_entry_wrappers_lock_full_name_and_local_behavior`（1653 起）锁定
`map_entry_types` 必须继续拒绝 `std.collection.Map`，以及 `single_for_item_wrappers_...`（1517 起）
锁定 Local 包装行为。这些测试作为行为等价回归基线，不得改断言。

## 执行决策（机械闭合，不改变设计语义）

1. core `BuiltinShape` 补 `pub const fn name(self) -> &'static str`（16 个 canonical 短名），
   并配测试。用途：
   - 短名独占匹配点（如 `name == "Map"`、`name != "Exception"`、`canonical_name == "Map"`、
     `receiver_root == Some("Array")`）：`name == BuiltinShape::X.name()` 逐分支等价；
     不能用 `of_name`（会扩大接受 `std.collection.Map`/`std.collection.Array` 等全名）。
   - 构造点（`name: "Array".to_string()`、`resolve_builtin("bool")`、
     `builtin_type("string")` 等）：`BuiltinShape::X.name()` 逐字节等价。
2. 短名+全名联合匹配点仅 2 处（etm 4763 `stream_chunk_type`、4978 `array_item_type_ir`），
   改 `BuiltinShape::of_name` 集合匹配，与现状集合完全一致。
3. 显式序列化/显示点保留字面量并列入验收例外：etm 2750（null 字面量显示文本）、
   trm 4687（`artifact_type_text` 的 null 字面量序列化）。它们不是 builtin 类型名匹配/构造点。
4. 窄形状函数收编结论：
   - 重写为 `BuiltinShape` 匹配（保持私有）：`direct_stream_item_type`（etm 522）、
     `stream_chunk_type`（etm 4759）、`map_entry_types`（etm 4775）、
     `array_item_type_ir`（etm 4972）、`type_ir_is_void_or_null`（etm 5273）、
     `type_ir_is_never`（etm 5278）。
   - 保持 core 函数包装（已收编，无名字字面量）：`single_for_item_type`、
     `single_for_item_projection`、`map_entry_projections`、`map_key_type_ir`、
     `map_value_type_ir`。
   - 保持私有签名：`projection_record_type(name: &str, ...)`（调用点传 shape name）、
     `catch_result_type`（内部构造改 shape name）。
5. 禁止：动 `compiler/lowering` 的 `union_type_ir`、artifact-model、identity/wire、
   `runtime_type_projection.rs`、`source_rules/`（不在目标文件范围）。

## 写集（production）

- `compiler/core/src/type_ref.rs`（+`name()`）
- `compiler/core/src/type_ref/tests.rs`（+`name()` 测试）
- `compiler/source/src/expression_type_model.rs`
- `compiler/source/src/type_resolution_model.rs`
- `compiler/source/src/expression_type_model/contract_call_typing.rs`
- `compiler/source/src/expression_type_model/contract_call_typing/type_projection.rs`
- `compiler/source/src/expression_type_model/expression_assignability.rs`
- `compiler/source/src/type_resolution_model/catch_leaves.rs`
- `compiler/source/src/type_resolution_model/shape_assignability.rs`
- 本文件（叶子执行合同）

## 验证矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| §6 目标文件字面量归零（除定义/构造/序列化点） | 各替换点 diff | `git grep -E '"(Array|Stream|Map|Exception|Json|CatchResult|null|void)"' <commit> -- 目标生产文件` 仅剩 etm 2750 / trm 4687 两个显式序列化点 | core+source 测试 |
| core `of_name` 覆盖短名+全名 | `type_ref.rs:565` | — | `type_ref/tests.rs` of_name 测试 |
| `name()` 新增 API | `type_ref.rs BuiltinShape::name` | 反向：目标文件短名独占点不再出现字面量 | `type_ref/tests.rs` name round-trip |
| 行为等价（含 map_entry_types 拒绝全名、Local 包装 None） | `map_entry_types` 保留 `name != BuiltinShape::Map.name()` | `rg 'std.collection.Map'` 目标文件 0 命中 | etm `map_entry_wrappers_lock_...`、`single_for_item_wrappers_...` |
| 禁止面未动 | diff 不含 lowering/artifact-model/identity | — | — |
| 编译/质量 | — | — | `cargo check`、`node scripts/verify.mjs --only compiler,rust-quality` |

## 自验收结果（2026-07-31，提交前）

- `node scripts/verify.mjs --only compiler,rust-quality`：4/4 passed
  （compiler-boundaries、全部 compiler Rust 测试、rustfmt、file-lines 门禁 1361 files ≤ 6533）。
- 聚焦测试：`skiff-compiler-core type_ref` 28 passed（含新增
  `builtin_shape_name_round_trips_all_short_names`）；`skiff-compiler-source` 行为锁定测试
  `single_for_item_wrappers_lock_container_and_local_behavior`、
  `map_entry_wrappers_lock_full_name_and_local_behavior` 均通过。
- 反向搜索：设计口径字面量在目标生产文件仅剩 2 处，均为显式序列化点——
  etm 2754（null 字面量显示文本）、trm 4687（`artifact_type_text` null 序列化）；
  全量 16 名口径同 2 处。`std.*` 字面量 0 处（etm 4819 仅注释，非字面量）。
- 禁止面：diff 不含 `compiler/lowering`、artifact-model、identity/wire。
- 基线对照：`cargo test -p skiff-compiler-source --lib type_resolution_model` 直接运行时
  8 个 prelude-registry 未初始化失败在 baseline main 同样存在（须经 verify harness 提供
  platform sources），与本次改动无关；verify 全量测试中无失败。
