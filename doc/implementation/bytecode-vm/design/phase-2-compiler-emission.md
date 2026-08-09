# Phase 2 设计：compiler facts、typed lowering 与 bytecode emission

状态：draft（待主 agent + 用户确认关键决策后生效）；依赖 Phase 1 complete、Phase 0 ledger

本文是 Phase 2（`phases/phase-2-compiler-emission.md`）的详细设计。它把权威契约
`doc/architecture/bytecode-vm.md`、`doc/reference/static-semantics.md`、`doc/reference/syntax.md`、
`doc/reference/any-interface.md` 与 requirement ledger 中 Phase 2 部分（R-010、R-011、R-025、R-026(部分)、
R-038、R-070(部分)、R-084、R-087(部分)、R-097(部分)、R-123(部分)、R-134(部分)、R-158(部分)、R-166(部分)、
R-168、R-170、R-179、R-195、R-196、R-197(部分)、R-198(部分)、R-199(部分)、R-202、R-203、R-228(部分)、
R-230、R-243(部分)）落成可实现的 grammar、facts、MIR、emitter、const evaluator 与迁移方案。

本文只定义 Phase 2 的实现面，不重新定义语言语义；与权威文档冲突处以权威文档为准。

---

## 1. 现状盘点（已审计）

| 面 | 现状 | Phase 2 变更 |
| --- | --- | --- |
| 语法 | `let`=mutable、局部 `const`=immutable、顶层 `ConstDecl`；无 `var`/`inout` 关键字 | 翻转语义 + 新增关键字（R-202/R-203） |
| 局部绑定可写性 | lowering `Binding.mutable` 门禁赋值，无 writable-place 分析 | var 派生 path 才是 writable root（R-195） |
| Effects | `CallableMayEffects` 7 bool（含 may_suspend），SCC fixed point 已有 | mayPending + pending categories + inout path facts；aggregate flags 语义重定义（R-025/R-084） |
| Lowering | AST → File IR（blocks/labels、无显式 CFG 边、无 exception region、无 liveness） | 新增 typed MIR/CFG 层（R-011） |
| Emission | 只发布 File IR + PackageArtifact（`bytecode: None`） | MIR → BytecodeArtifact + identity + store（R-010/R-011） |
| Const | ConstIr 携带 executable initializer body，request-time 执行 | compile-time bounded evaluator → FrozenConstantGraph（R-087/R-196） |
| Callback | 无 FnExpr；interface carrier 三 carrier 走 `call_interface` | carrier provenance facts + box/call_interface 发射；make_callback 不发射 |
| 三仓源码 | internals ~951 let / 6852 局部 const；skiff-packages 28 let / 363 局部 const；skiff 仓 fixtures 若干 | 全部迁移到新 binding 语义 |

## 2. 公共契约（工兵写界与接口语义，本阶段冻结）

### 2.1 语法（WP1 产出）

```text
TopLevelConstDecl = "const" Identifier (":" Type)? "=" Expr        // 不变，仅顶层
LocalBindingDecl  = ("let" | "var") Identifier (":" Type)? "=" Expr // 局部 const 移除
Param             = ["inout"] name ":" Type                          // inout 是参数 mode
CallArg           = Expr | "inout" Place                             // Place = name/member/index path
```

- 关键字集合加入 `var`、`inout`（parser 关键字表）。
- AST 变更：`Stmt::Let { kind: LetKind (Let|Var), ... }`；`FunctionParam.mode: ParamMode (Value|InOut)`；
  `CallArg` 增加 `InOut { place: Expr }`。顶层 `ConstDecl` 不变。
- 解析错误：block 内 `const` 报"局部 const 不是语法"；顶层 `let`/`var` 报"let/var 只在 block 内"；
  顶层 `inout` 参数、interface requirement/Actor external/callback 上下文中的 `inout` 参数由 static
  semantics 拒绝（R-198/R-199）。

### 2.2 File IR 增量字段（WP4 产出，全部 serde default，一处 schema 变更即一次 identity churn）

```rust
// artifact-model/src/executable.rs
pub struct SlotIr { pub index: u32, pub name: String, pub kind: SlotKind,
                    #[serde(default, skip_serializing_if = "Option::is_none")] pub ty: Option<TypeRefIr> }
pub struct ExecutableIr { ...,
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub expression_types: Vec<TypeRefIr>, // 与 body.expressions 对齐
    #[serde(default, skip_serializing_if = "Vec::is_empty")] pub statement_spans: Vec<Option<SourceSpanRef>>, // 与 statement 流对齐
}
```

- lowering 在发射每个表达式/语句时写入类型与 span（`FunctionLowerer` 已有 `expression_types: Option<&ExpressionTypeModel>` 与语法 AST span）。
- `CallIr` 增加 `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub inout_args: Vec<InOutArgIr>`，
  `InOutArgIr { root_slot: u32, path: Vec<InOutPathSegmentIr> }`（`InOutPathSegmentIr = Field(String) | Index`）。
  Phase 2 中 legacy runtime 不执行 inout 调用（三仓迁移不引入 inout 用法），该字段只为 emitter 表示。

### 2.3 Effects 线格式（WP2 产出，R-025/R-084）

```rust
// artifact-model/src/effects.rs（决策 D1：彻底删除 3 个 aggregate flags）
pub struct CallableMayEffects {
    pub escapes_caller_value: bool,      // identity-bearing 值 escape（资源/能力），aggregate 不置位
    pub requires_same_heap_identity: bool,
    pub invokes_unknown_target: bool,
    pub may_pending: bool,               // 取代 may_suspend
    pub pending_effect_categories: Vec<PendingEffectCategory>,
    pub inout_path_effects: Vec<InOutPathEffect>,              // { parameter_index, read: Vec<SelectorPath>, write: Vec<SelectorPath> }
}
pub enum PendingEffectCategory { ServiceCall, ActorCall, InterfaceCall, NativeCall, Stream, HostEffect, Unknown }
```

- `writes_caller_reachable`/`returns_caller_alias`/`throws_caller_alias` 从 wire 彻底删除；
  `BoundaryUnavailableReason::{WritesCallerReachable, ReturnsCallerAlias, ThrowsCallerAlias}` 变体退休
  （R-084：普通 aggregate 参数/返回/throw payload 是 logical snapshot，只有显式 InOut 路径写 caller
  place；InOut 使 service projection `Unavailable(InOutNotAllowedAtServiceBoundary)`，R-123/R-170）。
- `may_pending = !pending_effect_categories.is_empty()`；interface/service/callback/未知动态 target 保守
  `Unknown`；local call join callee categories；SCC fixed point 沿用。
- `FileIr ExecutableIr.may_suspend` 通道（runtime legacy 执行元数据）保留，取值改由 `may_pending` 派生
  （`contract_type_resolution/executables.rs` 的桥接改读 `may_pending`）。
- 删除旧的 `may_suspend` 字段（wire 上拒绝旧字段，`deny_unknown_fields` 语义保持）。
- 边界可用性（R-134）：普通 aggregate mutation 不再使 callable `Unavailable`；剩余 gates =
  escapes_caller_value（provenance 交叉校验）/requires_same_heap_identity/invokes_unknown_target/
  provenance（return/throw origins、escape lanes）。
- `compiler/lowering/src/suspend_analysis.rs` 本阶段不动（R-026 删除归 Phase 8）。

### 2.4 MIR（WP4 产出，emitter 唯一语义输入）

`compiler/lowering/src/mir/` 新模块，`skiff_compiler_lowering::lower` 的产物，随 `LoweredPackage` 携带：

```rust
pub struct MirUnit { pub module_path: String, pub functions: Vec<MirFunction> }
pub struct MirFunction {
    pub symbol: String, pub kind: MirExecutableKind,          // Function | ImplMethod
    pub type_params: Vec<String>,
    pub params: Vec<MirParam>,                                 // { name, slot, ty, mode: Value|InOut }
    pub return_type: TypeRefIr, pub self_type: Option<TypeRefIr>,
    pub slots: Vec<MirSlot>,                                   // { slot, name, kind, ty }
    pub blocks: Vec<MirBlock>,                                 // 显式 CFG
    pub regions: Vec<MirRegion>,                               // 表达式/语句级 exception region 描述
    pub statements: Vec<MirStatementEntry>,                    // statement index -> Option<SourceSpanRef>
    pub may_pending: bool,
    pub effect_summary_ref: String,                            // PackageCallableId / operation ABI id
    pub source_span: Option<SourceSpanRef>,
}
pub struct MirBlock { pub id: u32, pub label: String, pub statements: Vec<MirStmt>, pub successors: Vec<u32> }
// MirStmt = File IR StmtIr 的 CFG 化形态：If/ForIn/While/Match/Timeout/Concurrent 拆为
//   分支语句 + 后继边；Catch 保留为表达式内 region 描述（pc 由 emitter 在 linearize 后定）
pub struct MirRegion { pub id: u32, pub catch_expr: u32, pub catch_slot: u32, pub catch_type: TypeRefIr,
                       pub cleanup_depth: u32 }               // 嵌套由 emitter 按包含关系决定
pub struct MirLiveness { /* per-block live-in/out: BTreeMap<block, Vec<slot>> */ }
```

- MIR 构建 = FileIrUnit 后处理 + source facts（effects 按 callable 取，`may_pending`/`effect_summary_ref`
  经 compile pipeline 传入 `lower`）。
- liveness：标准 dataflow（may 语义，slot 粒度），产出 `MirLiveness`（每个 `MirFunction` 一份）。
- **停止条件**：MIR/emitter 不得从 File IR 恢复类型/liveness/effect 事实（这些由 lowering/source 写入）。

### 2.5 Const graph（WP5 产出）

- `compiler/lowering/src/const_evaluator.rs`：对每个顶层 const 的 lowered 表达式 DAG 做 bounded 求值，
  产出 `FrozenConstantGraph`（artifact-model::bytecode::dto 类型，`nodes: Vec<FrozenConstantNode>`，
  `child index < parent index` 编码）。
- 支持节点：Literal / Array / Record（含 impl instance 构造 → Behavior 节点）/ TypeRef /
  RepresentationWrap / Field / Index / Unary / Binary（number/string 运算子集）。**不支持调用**
  （local call / native / service）——Phase 2 求值器对 const initializer 内任何 call 报编译错误
  （"const initializer call not supported in Phase 2 evaluator"）；真实 closure 的 const 全部为
  literal 与 impl 构造（已核实），不受影响。
- 确定性：求值顺序固定（表达式 DAG 拓扑序），无 HashMap 迭代序。
- Bounds：step 上限（可配置，默认 100_000）、深度上限（64）、结果节点数上限（100_000）、字节上限
  （64 MiB）；cycle（表达式 DAG 无环，由 lowering 保证，仍做防御性检查）与超限均为编译错误。

### 2.6 Emitter（WP6 产出）

- `compiler/emission/src/bytecode/` 新模块（emission 新增对 lowering 的依赖，无环），入口：
  `pub fn emit_bytecode_artifact(units: &[MirUnit], const_graphs: &BTreeMap<String, FrozenConstantGraph>,
  opcode_fingerprint: &str) -> Result<BytecodeArtifact, BytecodeEmissionError>`。
- 每 unit → 一个 `BytecodeArtifact`（D11：每 package 一个 image，本阶段按 unit 发射、driver 合并为一个 image
  或每 unit 一个 artifact 由 driver 决定——**契约：driver 把 unit 函数 map 进同一 `BytecodeImage.functions`
  （function_key = `"{module_path}::{symbol}"`），pools 按 image 去重**）。
- Wordcode：block 线性化（CFG 拓扑序，deterministic：按 block id 序），表达式 DAG → 栈机指令
  （multi-ref 表达式先求值进 Temp slot 再 LoadSlot；单 ref 内联），branch delta 以 instruction header
  为基准，所有算术 checked。
- `max_operand_depth`：线性化时按指令累加计算。
- 指令覆盖（closure 驱动，缺失即报 `BytecodeEmissionError::UnsupportedConstruct`）：
  const/copy_slot/move_slot/store_slot/drop/dup、jump/jump_if_true/jump_if_false/switch_tag、
  call_local/tail_call_local/call_service/call_actor/call_interface/return、
  interface_box_local/interface_box_remote、new_record/get_dense_field/set_writable_path/representation_wrap、
  new_array_builder/array_builder_push/freeze_array/array_get/array_push_owned、
  new_map_builder/map_builder_put/freeze_map/map_get/map_put_owned、stream_next/emit_stream、
  throw/rethrow/enter_region/leave_region、invoke_host。
- tail call（R-158 部分）：`return <local-call>`（同 image 的 LocalExecutable/PublicationExecutable 精确
  target、参数求值顺序完整、无未闭合 cleanup region）发射 `tail_call_local`（`LocalExecutableRef`）；
  其余不动。interface const receiver/service/Actor/callback/native 不发射 tail_call_local。
- inout 调用（合法）：emission 报 `BytecodeEmissionError::InOutEmissionPending`（writable-region 编码归
  3B/4）；非法 inout 在 source 边界已拒绝（WP3）。
- 常量：函数级 `const` 指令引用 image 级 FrozenConstantGraph 的节点（`FrozenConstantRef`），
  ConstIr 的 request-time body 不进 bytecode image。
- frame metadata：`FrameLayout`（slot_count/parameter_slots/result_count/result_plans/slot_plans）；
  plan 声明规则：普通参数/结果/let slot = SnapshotShare；inout 参数 slot = MoveOnly；var slot 默认
  SnapshotShare（drop/transfer 语义证明归 6B）。
- Statement/source：`StatementEntry { pc, statement_id }`（`"s:{module}:{stmt_index}"`）、
  `SourceMapEntry`（word 区间 → SourceSpanRef 位置）、`DebugTable`（绑定名 → slot）。
- 错误模型：`BytecodeEmissionError`（structured enum），任一错误使该 package 的 bytecode 产出失败
  （fail closed，不写部分记录）。

### 2.7 迁移 lane（WP6/CLI）

- `PackageCompileInput.emit_bytecode: bool`（默认 false）；CLI `skiff package build|publish --emit-bytecode`；
  新子命令 `skiff-compiler bytecode-verify <artifact-root> [--manifest <path>]`：walk store 中全部
  bytecode records，逐条 `ValidatedBytecodeArtifact::admit`（C1–C9），失败即退出码非 0，输出 manifest
  （identity/ISA/function/word/relocation 计数）。
- emit_bytecode=true 时：pipeline 在 publish 前发射 → `assign_bytecode_identity` →
  `write_package_bytecode`（bytecode record 先于 package record）→ `PackageArtifact.bytecode = Some(ref)`。
- 默认 false 保证 legacy lane（stable release pointer / dev watch）不发布新 schema（阶段页 §3 迁移约束）。

---

## 3. Binding 语义（WP3）

### 3.1 static semantics 新增/变更

- **Writable-place 模型**（`compiler/source/src/writable_places.rs` 新模块）：
  writable root = 局部 var binding、当前有效 inout loan 参数、Actor method 中 `self.field`。
  `WritablePlace { root, path: Vec<Selector> }`；派生精确 path 可写；let/普通 parameter/顶层 const/
  loop/pattern/with binding 派生 path 不可写。赋值目标、mutating receiver op（`builtin_receiver_ops` 中的
  mutator 集合 + 容器方法）、inout 实参都走该检查。
- **let 不可写**：现有 "cannot assign to immutable binding"（lowering `function_lowering.rs:976-1010`）
  扩展为包括 member/index path 写与 mutator receiver（let/普通参数 receiver 的 push/put/set/remove 等拒绝）。
- **Narrowing 清除**（R-196）：对稳定 path 或其前缀赋值、以该 path 作 inout 实参后清除该 path 及子路径
  narrowing（扩展现有 `expression_type_model` narrowing 逻辑）。
- **InOut 规则**（R-198，`compiler/source/src/inout.rs` 新模块）：
  - actual 必须是 `inout place`，place 必须 var 派生精确 path；let/普通参数/顶层 const/self.field 不是合法 actual；
  - callee 必须精确解析到同 Package local target 或经 Package Local ABI 的 package-direct concrete
    callable；interface/dynamic dispatch 目标拒绝；
  - callee `may_pending == false`（loan 不得跨 Pending）；
  - loan exclusivity：同一调用内两个重叠 path 实参拒绝；loan 不能被 callback capture、保存、被
    concurrent sibling 观察；sibling lane 不得发起 inout 调用、不得写外层捕获 var 与 inout 派生 path；
  - inout 不得出现在 interface requirement/method table/callback/ServiceContract/gateway/Actor
    external/host effect/recoverable payload（边界 projection 拒绝）。
- **顶层 const purity**（R-196）：initializer 表达式必须 pure/`may_pending==false`/无 request-Actor-
  resource-callback-capability 值；source checker 拒绝 effectful/nondeterministic 的 const initializer
  （native/service/Actor/DB/stream 调用与 request 派生值），错误信息给出可迁移提示。
- **effect transfer 更新**（WP2 与 WP3 交界，契约）：3 个 aggregate flags 已删除；transfer 记录 inout
  path effects（本阶段未启用，空集 + 防御性校验）；返回/throw 的 identity-bearing provenance 仍经
  provenance 通道；`execution_semantics/mutation.rs:106-116` 的 concurrent 门禁（旧读
  writes_caller_reachable）由 WP2 暂时降级、WP3 以 writable-place 分析恢复（sibling lane 不得写外层
  捕获 var 与 inout 派生 path，R-198）。

### 3.2 三仓迁移（WP7，分三个工兵按 repo 写界）

- 规则 1（机械脚本）：block 内 `const` → `let`（正则 `^\s*const\s+` 且不在顶层，脚本先验匹配规则并抽查 diff）。
- 规则 2（编译器驱动）：编译报 "cannot assign to immutable binding"/"cannot mutate through immutable
  binding"/"cannot write ... let binding" 的 binding 名改为 `var`。逐包迭代到零错误。
- 编译顺序（依赖序，隔离 artifact root）：packages（llm-api → llm-providers → agent →
  skiff-packages/*）→ services（codex-relay → aihub → agine → registry）。
- 覆盖范围：internals（含 agent-tests/agine service-tests/aihub/codex-relay/skiff-platform）、
  skiff-packages、skiff 仓内 .skiff（test-runner fixtures、runtime live-tests、compiler/tests fixtures、
  std/prelude 已核实无 binding）。
- 不引入 inout 用法（语法合法但三仓不新增 inout 调用）。

---

## 4. 测试与证明（WP8）

| 证明 | 方式 |
| --- | --- |
| 确定性 | 同一 fixture 两次编译（bytecode on）byte 级一致；BTreeMap 遍历序不敏感；identity 相同 |
| exact targets | fixture 含 direct/mutual/generic/self call，断言 relocation kind + function_key/type args canonical |
| tail_call_local | `return f(x)` 在 tail 位置发射 0x21 + LocalExecutableRef；非 tail 不发射；args 求值序与 site 完整 |
| 负例（source 边界） | 非法 inout（非 var place/interface target/mayPending callee/重叠实参）、let 写、use-after-move
  （loan 期内使用）、callback capture of loan、局部 const、effectful const initializer、const call —— 各 ≥2 变体 |
| 负例（emission 边界） | inout 调用报 InOutEmissionPending；不支持构造报 UnsupportedConstruct |
| const evaluator | 相同输入相同 graph+identity；cycle/超 step/超深度/超 size 拒绝；literal/impl-instance 求值 golden |
| 结构性 | 全部产出过 `structurally_validate`（C1–C8）+ `ValidatedBytecodeArtifact::admit`（C1–C9） |
| 真实 closure | 隔离 artifact root 上 `skiff package publish --emit-bytecode` 编译完整 Agine closure，
  `bytecode-verify --manifest` 输出 manifest（三仓 commit、compiler SHA、每 package identity/ISA/function/
  word/relocation 计数），全部 structural-valid |
| golden 非 oracle | var/let/const/InOut 行为用 reference-derived golden（fixture 直接断言），不引用旧 evaluator 输出 |

Focused gate（同一候选）：`verify --only compiler`、`--only skiff-tests`、`--only foundation`、
`--only checks`、`git diff --check`。Live：`verify --only router-live:agine`（legacy lane）+ 隔离
bytecode 生成证明。

---

## 5. 决策清单（需主 agent/用户确认）

| # | 决策点 | 建议值 |
| --- | --- | --- |
| D1 | effects wire：may_suspend 删除、may_pending + categories + inout_path_effects 新增；3 个 aggregate flags（writesCallerReachable/returnsCallerAlias/throwsCallerAlias）**彻底删除**，边界门禁改由 provenance + InOut gate | 已确认（用户） |
| D2 | inout 本阶段只做 grammar + static semantics + projection；emission 对合法 inout 调用报 InOutEmissionPending | 已确认（用户） |
| D3 | File IR 增量字段（SlotIr.ty / expression_types / statement_spans / CallIr.inout_args），serde default；identity churn 一次接受 | 已确认（用户） |
| D4 | MIR 在 compiler/lowering/src/mir/，post-pass over FileIrUnit + source facts；emitter 只消费 MIR | 已确认（用户） |
| D5 | --emit-bytecode 默认关闭；bytecode-verify 子命令做 manifest 证明 | 已确认（用户） |
| D6 | const evaluator 表达式级、不支持 call；closure 全部 const（literal + impl 构造）可求值 | 采纳 |
| D7 | callback 面：interface carrier 三 carrier（box_local/remote + call_interface）发射；make_callback 不发射；负例覆盖 loan capture | 采纳 |
| D8 | 迁移：脚本 const→let + 编译器驱动 let→var；不引入 inout 用法 | 采纳 |
| D9 | 不做 worktree/分支，直接在 main 串行合流；工兵按写界提交 | 采纳（用户已定） |

## 6. 工兵任务包（DAG 与写界）

```text
Wave 1: WP1 语法(语法仓 syntax/**, 与 WP2 并行) ‖ WP2 effects(artifact-model effects/executable.rs 之外的
        effects.rs + compiler/source/callable_effects/** + contract_type_resolution + projection boundary
        eligibility/requirements + artifact-model boundary validation + 相关测试)
Wave 2: WP3 binding 语义(compiler/source: writable_places/inout/narrowing/const purity + lowering
        function_lowering 检查 + projection inout gate + CallIr.inout_args)  [依赖 WP1]
Wave 3: WP4 MIR + File IR 增量(compiler/lowering/src/mir/** + executable.rs/function_lowering 记录 types)  [依赖 WP3]
Wave 4: WP5 const evaluator(compiler/lowering/src/const_evaluator.rs) ‖ WP6 emitter + pipeline
        (compiler/emission/src/bytecode/** + driver pipeline + CLI + emission Cargo.toml + 依赖)  [WP6 依赖 WP4/WP5 接口]
Wave 5: WP7 三仓迁移（三个工兵：internals / skiff-packages / skiff 仓内 fixtures）  [依赖 WP1/WP3 合流后的 compiler]
Wave 6: WP8 证明与测试（确定性/golden/负例/closure manifest bytecode-verify）
Wave 7: WP9 rebuild + router-live:agine + results 文档
```

- 写界纪律：同一文件只允许一个工兵写；cargo 串行（共享 target 锁，排队执行）；工兵完成任务即提交
  （里程碑点），不合并他人未提交改动；涉及设计/契约发现一律上报，不自行扩大。
- 返回格式统一：`{完成了什么, 意外点, 尝试过什么, 需要什么}`。

## 7. 风险与残余（Phase 3 承接）

- 语义级 wordcode 正确性（stack effects 一致性、region 语义、move/share 追踪）未在本阶段证明——
  由 Phase 3B semantic verifier 拒绝/证明；本阶段 manifest 明确不声称"可执行语义"。
- make_callback/synthetic callback closure 发射、InOut writable-region 编码、const 内 local call 求值
  属 Phase 3B/6A/6B。
- effects wire 变更 + File IR 增量字段 → 全部 package identity 重算（单次 evidence epoch）。
- 迁移期间 dev watch 编译失败属预期（迁移完成后恢复）；Live 只在迁移完成后的候选上运行。
