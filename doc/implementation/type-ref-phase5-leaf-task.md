# Phase 5 叶子任务：`ResolvedTypeRef` 瘦身（构造入口集中 → 读点 Display 化 → 删字段）

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md`（已合入 main，
  baseline `296d6133feff0bbeb44510361a2aaf09d3eb57a2`；Phase 5 条款见 §5.3 与 §6 Phase 5，
  验证条款见 §7）。
- 直接父节点：Phase 1-4 已合入 main（`296d6133` 即 “Merge Phase 4: canonical identity
  unification and type-ref conversion convergence”）；core 已有 `debug_text`（Phase 3 第 1 对）。
- 必读规则：`/Users/geek/workspace/AGENTS.md`、`/Users/geek/workspace/skiff/AGENTS.md`、
  `/Users/geek/workspace/multi-agent-development.md`。

## 任务范围

按设计 §5.3 三步推进，3 个 commit：

1. 集中构造入口：`impl ResolvedTypeRef { pub fn new(ir) / pub fn with_text(ir, text) }`
   （`new` 的 source_text = `debug_text(&ir)`，`with_text` 保留手拼文本），替换全部 65 个
   struct literal 构造点（57 production + 8 test，以开工前 rg 快照为准），行为逐字节不变。
2. 迁移读点：73 个 `.source_text` 读点（64 production + 9 test）改为 Display；
   `impl Display for ResolvedTypeRef = debug_text`。测试断言保持有效；任何输出变化必须
   先出现在 diff 并有测试覆盖。
3. 删字段：`pub struct ResolvedTypeRef(pub TypeRefIr)`（tuple struct，字段必须 pub 供
   lowering 跨 crate 访问 `.0`），删除 `source_text`；所有 `ResolvedTypeRef` 上的 `.ir`
   访问迁移为 `.0`，全部代码与测试编译通过。

范围与禁止：
- 仅限 `ResolvedTypeRef`；`CanonicalInterfaceSelectorResolution`（type_resolution_model.rs:193）
  的 `pub source_text` 字段及其消费点代码一律不动。
- 不改 `debug_text` 行为、不改 artifact-model、不动其他结构。
- 诊断断言是 golden 基线：任何输出变化必须先出现在 diff 并有测试覆盖，不允许静默变化。
- 禁止：改设计语义、push、写 main/合并、承接其他阶段。

## 只读预检结论（锚定 296d6133，零 worktree）

### 统计快照

- `ResolvedTypeRef` 定义：type_resolution_model.rs:58，
  `#[derive(Clone, Debug, PartialEq)] pub struct ResolvedTypeRef { pub ir, pub source_text }`。
- 构造点：struct literal 65 处（57 production + 8 test；已排除定义行与 11 处
  `-> ResolvedTypeRef {` 函数签名误匹配）。
  - lowering 5（function_lowering.rs:1177/1191/1990/2264、object_literal/fact_validation.rs:101）
  - source 52（expression_type_model.rs 24、type_resolution_model.rs 9、shape_assignability.rs 6、
    type_projection.rs 5、contract_call_typing.rs 1、db_projection.rs 1、expression_assignability.rs 6）
  - test 8（expression_type_model/tests.rs 2、type_resolution_model/tests.rs 4、
    compiler/tests/package_interface_identity.rs 2）
- `.source_text` 读点（ResolvedTypeRef 范围内）：73 处（64 production + 9 test）。
  已排除：`parsed.source_text()` 方法调用 2、`selector.source_text`（另一结构体）11、
  object_materialization/tests.rs:83 `self.source_text`（测试结构自身字段）1。
- `ResolvedTypeRef` 无跨 compiler/ 使用；无既有 `impl Display/Debug/PartialEq/Clone`；
  无 `impl ResolvedTypeRef`；无 `ResolvedTypeRef::` 调用；无 `let ResolvedTypeRef { .. }` /
  match 解构；无整值 `assert_eq!/assert_ne!`（source_text 不参与相等性语义；
  `same_canonical_type` 收 `&TypeRefIr`，不受影响）。tuple struct 化无 trait/解构冲突。

### 构造点分类（新 = source_text 直接是 `debug_text(<同一 IR>)`；其余一律 with_text）

`new`（18 处 production + 2 处 test）：etm 3764/3772/3941/3949/3992/4003/4775/4788/5074/5203；
expression_assignability 243/259/483/818 视同 debug_text 直连（818 的临时 Record 与 ir 同源，
  见代码证据）/925；trm 1362/1462/1523/1582/1669；shape_assignability 827/863/881/905/1188/1207；
  tests 1526/1642。

`with_text`（其余全部；关键手拼清单）：
- lowering：1177 `value_source_text.clone()`、1191 `selector.source_text.clone()`、
  1990/2264 `type_ref_ir_type_text(...)`（**与 core debug_text 输出不同**：LocalType
  `$localTypeN` vs `#N`、PublicationType、Record 全字段、Literal Number/Bool、Function 全参数）。
- etm：1222 `subset<...>`、1903/1914 `selector.source_text`、2056/3829 `"{}"`、
  2735 serde_json 字符串、2747 `"null"`、3337/5107 `Array<...>`、4851/4861
  `{name}<...>`/`CatchResult<...>`、5025 `source_text` 变量（trim 自 ty.source_text）、
  5081 `...?`、5094 手拼 record 文本、contract_call_typing 212 `Stream<...>`。
- type_projection 665/688/700/706/718：name/alias 拼写（688 的 `{alias}.{key}` 与
  debug_text 的 `symbol_path`=key 不一致）。
- db_projection 106 `String::new()`。
- trm 717/730（resolve_named_type / resolve_type_text 的 source spelling，alias 展开后
  ir 与拼写不同）、1037/2273 `any {selector.source_text}`。
- tests：trm tests 1300/1799/2528（手拼 interface/record 文本）、2864（debug_text(ty) 与
  Union 包装 ir 非同一表达式）、package_interface_identity 206/520。

### Display = debug_text 的收敛风险（预检已识别，实施时按规则处理）

预检确认存在“读点可见的手拼/spelling 文本 ≠ debug_text(ir)”的值，例如
`resolve_type_text`/`resolve_named_type`（trm 717/730，ir 经 alias 展开而 source_text 保留
拼写）、type_projection 688（`{alias}.{key}` vs `key`）、db_projection 106（`String::new()`）。
这些读点在 Display=debug_text 下输出会收敛为 canonical 文本——这是设计终态
（§5.3 第二步注释 `impl Display = debug_text`），不是本任务可以绕开的行为。
处理规则：
1. 测试失败先归类：仅诊断文本变化 → 更新 golden 到新输出，在 commit message 与 diff 中
   逐项记录（“任何输出变化必须先出现在 diff 并有测试覆盖”）。
2. 若变化影响非诊断语义（如 `resolve_type_text(&x.to_string())` 再解析、identity/ABI、
   wire/artifact 内容、selector 消费逻辑）→ 停止并返回 `TASK_NOT_EXECUTABLE`，附精确
   diff/失败证据，不猜测实现。
3. 生产路径变化若无测试覆盖，补聚焦测试锁定新输出（属于本任务机械闭合）。

已知派生值传递：trm 1066 把 `resolved.source_text` 拷入
`CanonicalInterfaceSelectorResolution.source_text`（该结构体及其消费点代码不动，但值会随
Display 收敛；按规则 1/2 验证测试覆盖）。

### Step 2/3 修订：主 Agent 决策 B2（2026-07-31）

设计文档已更新（main `31345d78`，doc-only）：§5.3 目标改为
`pub struct ResolvedTypeRef { ir: TypeRefIr, source_text: Option<String> }`；
`new(ir)` → None（Display 回退 debug_text）、`with_text(ir, text)` → Some(text)；
Display 渲染 override 或 debug_text；不删除字段、不删 spelling。

实施形态：
- Step 1（`d4474d3d`，已提交）保留，仅 `new` 语义在 Step 3 由 `Some(debug_text)` 改为 None。
- Step 2：73 读点迁移到 Display；Display 先渲染存储文本（字段仍为 `String`，逐字节不变，
  6 个 golden 测试恢复通过）。
- Step 3：字段 Option 化 + `new`→None + Display 渲染 override-or-debug_text；所有读点输出
  仍逐字节不变（with_text 保留 spelling，new 的 None 回退 debug_text 与旧存储值相同）。

Step 2 失败证据（修订前）已随设计 commit 31345d78 的修订原因记录，此处不再重复。

### 写集

- `compiler/source/src/type_resolution_model.rs`（结构体、构造入口、读点、`.0` 迁移）
- `compiler/source/src/expression_type_model.rs` 及
  `expression_type_model/{contract_call_typing.rs, contract_call_typing/type_projection.rs,
  db_projection.rs, expression_assignability.rs}`
- `compiler/source/src/type_resolution_model/{shape_assignability.rs, catch_leaves.rs}`
- `compiler/lowering/src/{function_lowering.rs, type_inference.rs, suspend_analysis.rs}` 及
  `function_lowering/object_literal.rs`, `function_lowering/object_literal/fact_validation.rs`
- 测试：`expression_type_model/tests.rs`、`expression_type_model/contract_call_typing/tests.rs`、
  `expression_type_model/object_materialization/tests.rs`、`type_resolution_model/tests.rs`、
  `compiler/tests/package_interface_identity.rs`
- 本叶子文件 `doc/implementation/type-ref-phase5-leaf-task.md`

## 验证矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| 集中构造入口（行为不变） | `new`/`with_text` 定义 + 65 构造点替换 | `rg "ResolvedTypeRef \{"` 0 命中（除定义） | cargo check + 聚焦测试 |
| 读点 Display 化（输出变化先 diff 后有测试） | Display = debug_text；73 读点迁移 | `rg "\.source_text"` 仅剩 selector/parsed/self（排除项） | 全部断言过；变化项有 golden 更新 |
| 删字段（tuple struct） | `pub struct ResolvedTypeRef(pub TypeRefIr)` | `rg "source_text"` 在 ResolvedTypeRef 相关代码 0 命中 | cargo check + 聚焦测试 |
| CanonicalInterfaceSelectorResolution 不动 | 结构体/消费点代码无 diff | `git diff` 该文件仅 1066 上游读点行变化 | — |
| 阶段全绿 | — | — | `node scripts/verify.mjs --only compiler,rust-quality`；worktree 内 `--only skiff-tests` 自验收 |

每步 `cargo check`（受影响 crate）+ 聚焦测试 + `cargo fmt --check`；最终自验收
`node scripts/verify.mjs --only compiler,rust-quality`（`--only skiff-tests` 由验收 Agent
合并后跑，设计 §7；worktree 内先跑一遍作为自验收）。

## 交接

branch `impl/type-ref-phase5`、worktree `/Users/geek/workspace/skiff-phase5`，完成 3 个
commit（message 标注 “Phase 5 第 1/2/3 步”）后直接交接集成 Agent `skiff_integration_phase1`，
并通知主 Agent（/root）。
