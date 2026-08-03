# runtime/linked-type-plan `type_plan.rs` 重构方案

日期：2026-08-03

状态：draft（review 后修订，2026-08-03）

本文是阶段性实现方案，不是长期架构契约。目标是消除 `runtime/linked-type-plan/src/type_plan.rs`
里的职责混杂、重复目录和三份同构实现，并把公共 API 面收敛到外部实际使用的入口。
文件行数不是目标；行数下降是去重与职责归位的副产品。

## 1. 背景

`runtime/linked-type-plan/src/type_plan.rs` 当前 2700 行，是 runtime 侧从 linked/artifact 类型
表示构建 `RuntimeTypePlan` 的主下沉点（另一个是 `runtime/boundary/src/db.rs` 的
`runtime_type_plan_from_artifact_type_ref`，见 2.2）：

- `runtime/eval` 的 actor、spawn、http gateway、invocation、websocket、recoverable 链路都调用
  `RuntimeTypePlan::from_linked` / `from_linked_nested_ref` 与 recoverable 入口；
- `runtime/driver/eval` 的集成测试直接消费 `ProgramTypeView` / `PlanContext`；
  `runtime/driver/value_codec` 有一个临时 adapter 整体再导出这几个类型（见 2.3）；
- trait 的 `from_artifact_type_ref` 外部调用点只有 1 处
  （`runtime/driver/eval/tests/program_execution.rs:2081`）；`runtime/service-db` 与
  `runtime/boundary` 用的是各自的 artifact 构建器，不是本 trait。

该文件的 git 历史（16 个 commit）显示它是连续叠加特性长大的：applied nominal、package nominal
codecs、encrypted index、config snapshot、actor wave a 等每次都往同一个主 match 和目录函数里加
分支，没有同步做职责分化。

仓库里已经有同类问题的先例与原则：`doc/implementation/compiler-type-ref-unification-plan.md`
明确“文件行数不是目标，抽象不足、代码重复、职责混杂才是要解决的问题”；`compiler/core/src/type_ref.rs`
已经建立了 `BuiltinShape` 目录抽象（`of_name` / `name`），runtime 侧目前没有对应物。

## 2. 当前证据

以下行号是 2026-08-03 的基线快照，随重构演进会失效。

### 2.1 结构与职责分布

| 职责 | 位置 | 规模 |
| --- | --- | --- |
| 视图与上下文（`ProgramTypeView`、`PlanContext`） | L40–247 | 约 210 行（含 `test_runtime_package` 56 行） |
| 公共 trait 契约 | L248–309 | 2 个 trait、16 个方法 |
| `impl RuntimeTypePlanLinkedExt for RuntimeTypePlan` | L311–911 | 600 行、14 个方法 |
| nominal 实例化（applied/close/instantiate/owner） | L911–1227 | 约 315 行 |
| recoverable 族 + identity helpers | L1227–1627 | 约 400 行 |
| 序列化排序（`sorted_json_string` / `sort_json_value`） | L1627–1653 | 26 行；只被 identity 三个 helper 使用，Phase 1 随它们进 recoverable.rs |
| label/kind/named_type_name 工具 | L1654–1743 | 约 90 行 |
| 地址解析（program_*_type_addr 等） | L1743–1899 | 约 155 行 |
| std builtin 目录 | L1899–2082 | 约 180 行 |
| db result 目录 | L2082–2250 | 约 170 行 |
| 测试 | L2250–2700 | 451 行（101 + 350；2 个内联 `#[cfg(test)] mod`） |

同一个文件同时承担：输入表示适配（artifact/linked）、递归 walk、地址解析、builtin 目录、
db result 目录、recoverable 投影、序列化排序、测试。任意新增 builtin 或 db result 形状需要
同步改多个入口；任意新特性（如 actor）需要往 `from_linked` 主 match 加分支。

### 2.2 具名重复（均已核对）

| 重复 | 位置 | 性质 |
| --- | --- | --- |
| `db_result_node_from_parts` / `db_result_node_from_artifact_parts_in_program` / `db_result_node_from_linked_parts` | L2082 / L2137 / L2193 | 结构逐段相同（diff 仅在 `DbUpsertResult` 的 value 递归）；唯一实质差异是递归方式（artifact 无 ctx、artifact-in-program 有 ctx 无深度、linked 用 `deeper_by(2)`） |
| `std_runtime_builtin_node_from_artifact_parts` / `_in_program` / `_from_linked_parts` | L2025–2048 | 三个包装函数完全同构，都只是转发 `std_runtime_builtin_node(name, args.len())` |
| leaf builtin node 映射（Json/JsonObject/bytes/Date/string/bool/integer/number/null/void） | type_plan.rs 5 处（L816、L856、L898、L1613、L2065）；`runtime/boundary/src/db.rs` L606 第 6 处 | 同一目录 6 份复制；`runtime/boundary/src/type_descriptor.rs` 另有 2 个名称集合谓词（`is_builtin_named_type` L844，生产 API；`is_builtin_concrete_type_name` L1105），与目录重叠但不是 node 映射，Phase 5 可被 `of_name` 吸收 |
| std.http 记录目录（`std_http_header_plan`、`std_http_client_request_plan` 等 + `builtin_plan`/`leaf_builtin_plan`/`std_field`/`std_record_plan`/`std_nullable_plan`/`std_array_plan`/`std_stream_plan`） | type_plan.rs L1899–2007；`runtime/boundary/src/type_descriptor.rs` test-support 段 L889–1006 | 逐字复制的第二份；boundary 版入口是 Value 驱动的 `std_runtime_builtin_node_from_descriptor`（L1010），不是 name/args 驱动 |
| artifact → plan 结构分支（Record/Union/Nullable/Literal + Array/Map/Stream） | type_plan.rs `from_artifact_type_ref` L320；`runtime/boundary/src/db.rs` `runtime_type_node_from_artifact_type_ref` L545 | 同构实现；差异有三：db.rs 版处理 identity、不含 Db*Result、`AppliedNominal` 落 `Unknown`（type_plan 版报 `InvalidArtifact`） |
| `artifact_type_ref_label` / `artifact_type_ref_named_type_name` | type_plan.rs L1713/L1734；`runtime/boundary/src/db.rs` | 第二份 |

### 2.3 公共 API 与入口问题

`RuntimeTypePlanLinkedExt` 有 14 个方法，其中 5 个 `from_artifact_type_ref*` 入口、
4 个 `from_linked*` 入口。按外部调用点核对（排除 type_plan.rs 自身）：

| trait 方法 | 外部调用数 | 结论 |
| --- | --- | --- |
| `from_linked` | 20+（eval、同 crate native_call_plan/http_plan、driver 测试） | 保留 |
| `from_linked_nested_ref` | 12（eval） | 保留 |
| `from_artifact_type_ref` | 1（driver/eval 测试 program_execution.rs:2081） | 保留（artifact-native 构建入口） |
| `from_linked_ref` | 0（`RuntimeRecoverableExpectedTypePlan` 的 `from_linked_ref` 有 2 处，在 db_eval） | 可私有化 |
| `from_linked_substituted` / `resolve_addr_or_bridge` / `from_linked_declaration` / `from_linked_descriptor` | 0 | 可私有化 |
| `builtin_node` / `artifact_builtin_node` / `artifact_builtin_node_in_program` | 0 | 可私有化 |
| `from_artifact_type_ref_in_program` / `_in_type_view` / `_in_program_ref` | 0 | 仅内部/tests 使用，可降级 |

另：`from_linked_nested_ref` 的实现只是 `from_linked_ref` 的别名；`#[allow(dead_code)]` 出现在
`PlanContext`（L146/L156）、`from_linked`（L504）、`builtin_node`（L783）上，而 `from_linked`
实际被 runtime/eval 大量调用——这些标注已经失真，说明入口面没有明确归属。另注意
`runtime/driver/value_codec/type_descriptor.rs:12` 以 `#[allow(unused_imports)]` 整体再导出
`RuntimeTypePlanLinkedExt` / `PlanContext` / `ProgramTypeView`（临时 adapter），Phase 0 审计
必须列入；它只 import trait 本身，Phase 4 缩小方法不会破坏它，但后续删除方法时这里是盲区。

### 2.4 测试布局

仓库从 8336c2ad 起约定内联 `#[cfg(test)] mod tests` 迁到 `<module>/tests.rs`，本文件仍有两个内联
test mod（`recoverable_expected_plan_tests` 101 行、`applied_nominal_type_plan_tests` 350 行），
`test_runtime_package` 也是生产文件里的 `#[cfg(test)]` 函数。

## 3. 非目标

- 不改变 `RuntimeTypePlan` / `RuntimeTypeNode` / identity 的序列化格式与字节语义；不重写 legacy
  JSON `from_descriptor` 路径，它仍是行为基准。
- 不合并 `artifact-model` 与 `linked-program` 的类型 enum；归一化只做“输入视图”，不新建第三个
  递归类型。
- 不为了过行数门禁机械拆文件；行数下降是去重与职责归位的副产品。
- 不在重构中引入 fallback / adapter / dual path 维持中间态。
- 不一次性跨 crate 搬迁目录；Phase 5 单独评估依赖方向，未确认前不执行。

## 4. 目标形态

### 4.1 模块划分

```text
runtime/linked-type-plan/src/type_plan/
  mod.rs        // 公共 trait 契约 + 薄入口 impl；lib.rs 的重导出保持兼容
  context.rs    // ProgramTypeView + PlanContext（含 depth/substitution 语义注释）
  linked.rs     // from_linked 主 walk、resolve_addr_or_bridge、declaration/descriptor、TypeParam 闭合
  nominal.rs    // applied_nominal_plan、close_linked_type_ref、instantiate_linked_descriptor、
                // apply_nominal_owner_context
  recoverable.rs// RuntimeRecoverableExpectedTypePlanLinkedExt impl + recoverable_expected_* 族
                // + identity helpers（linked_interface_instantiation_runtime_id 等）
  address.rs    // program_*_type_addr、merge_type_addr、is_actor_declaration_symbol
  labels.rs     // kind/label/named_type_name/unknown_plan_* 工具
  builtins.rs   // RuntimeBuiltinShape 目录：leaf 形状 + Array/Map/Stream + std.http 记录
                // + Db*Result 形状 + Duration
  tests.rs      // 现有两个 test mod + test_runtime_package
```

依赖方向：`mod.rs → linked/recoverable → nominal ↔ linked` 存在双向依赖——`applied_nominal_plan`
与 `linked_named_union_branch_plan` 直接调用 `RuntimeTypePlan::from_linked_ref` /
`from_linked_descriptor`（L983、L1180–1192）。两个选择：nominal 的递归走输入视图/回调注入
（与 builtins 同机制，推荐），或明确承认 linked ↔ nominal 同层循环并在注释说明。
`builtins` / `labels` 不依赖 `PlanContext`（递归通过回调/输入视图注入）；`address` 依赖
context.rs 的 `ProgramTypeView`，不依赖 `PlanContext`。

### 4.2 单一 builtin 目录

目录分两层，避免一个枚举承担三种语义：

```rust
// 层 1：名称 → 形状 + leaf 映射（仿 compiler/core/src/type_ref.rs 的 BuiltinShape）
pub enum RuntimeBuiltinShape {
    Array, Stream, Map, Json, JsonObject, Date, String, Integer, Number,
    Bool, Bytes, Null, Void,
    DbInsertManyResult, DbUpdateManyResult, DbDeleteManyResult, DbUpsertResult,
}
impl RuntimeBuiltinShape {
    pub fn of_name(name: &str) -> Option<Self>;  // 统一处理 bare/full 名（含 std.collection.* 等别名）
    pub fn leaf_node(self) -> Option<RuntimeTypeNode>;  // 仅 leaf 变体返回节点
}

// 层 2：记录目录（std.http.*、Duration）——枚举之外的 builtin_plan 记录项
fn std_http_header_plan() -> RuntimeTypePlan;
fn std_http_client_request_plan() -> RuntimeTypePlan;
// ...
```

- 6 份 leaf node 映射收敛到 `leaf_node` 的 1 份；`std.http.*` 与 `Duration` 进记录目录。
- 三种调用方的 fallback 语义必须保留在各自入口：`builtin_node` / `artifact_builtin_node*` 的
  `Unknown`、`native_builtin_plan` 的 `Err(InvalidArtifact(...))`、
  `recoverable_expected_builtin_node` 的 `Unresolved { diagnostic_label }`——目录只提供
  `Option`，不吞 fallback。
- `builtin_node` / `artifact_builtin_node` / `artifact_builtin_node_in_program` 里
  Array/Map/Stream 的结构分支收敛为目录上的一个泛型方法，递归通过输入视图完成。
- 与 compiler 侧 `BuiltinShape` 不共享实现：compiler 版只做 name↔shape，不含 Date/bytes/
  DbInsert/Update/Delete/Duration，也没有 node 映射；runtime 版需新增 leaf 映射与
  arg-count/递归语义。

### 4.3 归一化输入视图（消三份 db result）

在 crate 内定义输入视图，不新建递归类型：

```rust
enum PlanInput<'a> {
    Artifact(&'a TypeRefIr),          // 无 ctx：递归不加深
    ArtifactInProgram(&'a TypeRefIr), // 有 ctx：递归不加深
    Linked(&'a LinkedTypeRef),        // 有 ctx：递归按 from_linked_ref 的分支规则加深
}
trait PlanInputView<'a> {
    fn bare_name(&self) -> &str;
    fn recurse_plan(&self, ctx: &PlanContext<'a>) -> Result<RuntimeTypePlan>;
}
```

`db_result_node(input)` 只写一份；`DbUpsertResult` 的 value 递归走 `recurse_plan`。
`builtin_node` 的 Array/Map/Stream 分支同样收敛。

注意：三种输入的 depth 语义目前不一致（见 2.2 表），归一化必须把“各自 depth 规则”作为输入视图
的一部分保留，不能拍平：

- artifact（无 ctx）：递归不增加 depth；
- artifact-in-program：有 ctx，但 `DbUpsertResult` value 不 `deeper_by`；
- linked：`DbUpsertResult` value 用 `deeper_by(2)`。

`Linked` 变体的 `recurse_plan` 本质是现有 `from_linked_ref` 的委托——linked 的深度计账是
分支级的（Record/Union/Array/Map/Stream 用 `deeper_by(2)`、Nullable/representation/alias 用
`deeper_by(1)`、入口 `over_depth_cap` 截断），不能用一个统一 ctx 表达；视图 impl 保留这些
规则，调用方只传原始 ctx。`DbUpsertResult` 调用方自带的 `deeper_by(2)`（L2193 处）要移入
`Linked` 变体实现，避免双倍加深或深度语义漂移。

### 4.4 公共 trait 收敛

按 2.3 的调用面，把 trait 瘦身为外部契约：

- `RuntimeTypePlanLinkedExt` 公开保留：`from_artifact_type_ref`、`from_linked`、
  `from_linked_nested_ref`（其余方法移入子模块私有 impl）；
- `RuntimeRecoverableExpectedTypePlanLinkedExt` 公开保留：`from_linked`、`from_linked_ref`；
- 若某方法在 Phase 0 审计时发现新的外部调用点，按实际使用面调整，不以本表为准；
- 删除或修正 `#[allow(dead_code)]` 标注；`from_linked_nested_ref` 若继续保留别名身份，
  在文档注释里写明与 `from_linked_ref` 的语义差异，并接受双公开入口的代价；
  `from_artifact_type_ref` 虽只有 1 个外部调用点，但它是 artifact-native 构建入口
  （service-db/boundary 未来可接入），保留。

### 4.5 测试布局

- 两个内联 test mod 与 `test_runtime_package` 迁到 `type_plan/tests.rs`
  （`test_runtime_package` 仍被 `native_call_plan/tests.rs`、`http_plan/tests.rs` 引用，
  需保持 `pub(crate)` 并更新 `use` 路径）；
- 为 builtin/db result 目录新增表驱动测试（名字 → 期望 node），锁定“新增 builtin 只改一处”；
- 为三入口补差分测试：语义允许的等价形状（artifact vs linked）产出相同 node/label；
  depth 语义差异用现有测试锁定，不在归一化中变更；
- legacy JSON 基准 `RuntimeTypePlanDescriptorExt::from_descriptor` 只在
  `#[cfg(any(test, feature = "test-support"))]` 下实现（boundary/type_descriptor.rs L34–38），
  差分测试只能在 cfg(test)/test-support 下跑，不是生产路径基准。

## 5. 阶段计划

### Phase 0：基线锁定（不改行为）

- 运行 runtime 组的 verify（`skiff-runtime-linked-type-plan` 包 + eval/driver 依赖方），记录基线；
- 审计 trait 方法外部调用面（输出 2.3 表的最终版本）；
- 补差分测试：from_linked 与 legacy JSON 路径在 label/named_type_name/depth-32 截断上的一致性。

验收：全部测试绿；调用面清单入库（写在 4.4 对应位置）。

### Phase 1：纯搬迁（低风险）

- `labels.rs` / `address.rs` / `unknown_plan_*` 等无依赖自由函数搬入子模块；
  `sorted_json`（L1627–1653）随 identity helpers 进 `recoverable.rs`；
- 两个 test mod 与 `test_runtime_package` 迁到 `tests.rs`；
- `mod.rs` 重导出保证 `lib.rs` 的导出不变。

验收：编译 + 现有测试全绿；文件行数下降；无任何行为 diff。

### Phase 2：leaf/std 目录归一（crate 内）

- 建 `RuntimeBuiltinShape` + std.http/Duration 目录；
- 替换 5 处 leaf match、3 个 `std_runtime_builtin_node_from_*_parts` 包装。

验收：crate 内 `"JsonObject" =>` 只剩目录 1 处；新增 leaf builtin 只改一处；三种 fallback
（Unknown/Err/Unresolved）仍由调用方保留；表驱动测试新增。

### Phase 3：db result 三份合一 + 结构分支收敛

- 实现 `PlanInputView`；`db_result_node` 只写一份；
- Array/Map/Stream 结构分支在三个入口收敛；
- 保持各输入的 depth 规则，不改行为。

验收：三个 `db_result_node_from_*` 函数消失；对现有 fixture 做两两差分并输出“预期差异清单”
——linked 可解析 Address/LocalType/ServiceSymbol/PackageSymbol/DbObjectSymbol，
artifact-in-program 只桥接 PackageSymbol，artifact 对 AppliedNominal 直接报错——不能默认
三入口等价。

### Phase 4：trait/入口收敛

- 按 Phase 0 清单把无外部调用点的方法私有化或移入子模块；
- 移除 `#[allow(dead_code)]`；`from_linked_nested_ref` 决定去留后更新调用点。

验收：无 `#[allow(dead_code)]`；公开 trait 方法均有外部调用点或文档说明。

### Phase 5：跨 crate 目录共享（可选，需先决策）

- 前置 1：`bare_type_name` / `type_name_root` 目前定义在 boundary/type_descriptor.rs，
  linked-type-plan 从 boundary 再导出（type_plan.rs L23–24）；依赖方向是
  boundary → runtime-model、linked-type-plan → boundary，因此目录落 `runtime-model` 前必须先
  下沉这两个名称解析函数（或最小子集）；
- 前置 2：boundary 的目录副本入口是 Value 驱动的 `std_runtime_builtin_node_from_descriptor`
  （type_descriptor.rs L1010），共享时需要 `Value → (root, args)` 适配层；
- 范围含 `is_builtin_named_type`（生产 API，被 boundary/map_key.rs 使用）、
  `is_builtin_concrete_type_name` 两个名称集合谓词，以及 db.rs 的
  `artifact_type_ref_label` / `artifact_type_ref_named_type_name` 第二份；
- 让 `runtime/boundary/src/db.rs` 与 linked-type-plan 共用目录，删除 test-support 副本。

验收：全仓库 production 代码 leaf match 只剩目录 1 处；`boundary/type_descriptor.rs`
test-support 段不再复制 std.http 目录。

每个 Phase 独立 commit；任何中间 commit 不越过 `MAX_FILE_LINES = 3151` 文件门禁
（这是唯一硬约束；clippy `too_many_lines` 阈值 534 是函数级 lint，本文件最长函数
`from_linked_ref` 仅 138 行，搬迁不触发，除非合并函数）。

## 6. 风险与决策点

- **行为等价**：`from_linked` 与 legacy JSON 路径必须字节一致（label/named_type_name/
  depth-32 截断）；目录归一与输入视图不能改变 error 文本/顺序。现有测试锁了一部分行为，
  Phase 0 必须补差分用例。
- **depth 语义差异**：DbUpsertResult value 在三种输入下 depth 不同（见 4.3），归一化时
  不能拍平，否则会改变 depth-32 截断点。
- **公共 trait 是跨 crate API**：eval、driver 测试、driver/value_codec adapter 都在用；
  改名/删除前先按 Phase 0 清单核对，任何入口变化必须同步更新调用点并单独 commit。
- **依赖方向**：Phase 5 的目录落点取决于 catalog 是否依赖 `RuntimeTypeNode`（runtime-model）
  与 artifact/linked 类型；未确认前不跨 crate 搬迁。`runtime-model` 若不能反向依赖
  artifact-model，则落点在 boundary production 段或新的共享 crate。
- **identity 稳定性**：`linked_type_ref_runtime_key` / `recoverable_interface_projection_identity`
  是 recoverable 链路的持久化键（`runtime/eval/src/recoverable_behavior.rs`、
  `spawn_ops.rs` 使用），本次只搬家、不改实现与输出。
- **行数门禁**：重构中间态文件数会增加；任何 commit 都不能触顶 3151 文件门（534 为函数级
  lint，见 §5），搬迁顺序按 Phase 1 → 2 → 3 推进，先降峰值再收敛 API。

## 7. 验收标准

- 重复消除：`db_result_node_from_*` 三份消失；leaf builtin node 映射在 crate 内 1 处、
  全仓库 production ≤1 处（Phase 5 后）；std.http 目录无第二份；
  `is_builtin_named_type` / `is_builtin_concrete_type_name` 两个名称集合谓词被目录吸收
  （Phase 5 后）。
- 职责归位：`type_plan` 模块内每个子模块只包含表 2.1 中对应职责；`builtins`/`labels`
  不依赖 `PlanContext`；`address` 只依赖 `ProgramTypeView`。
- API 收敛：`lib.rs` 重导出不变或按 Phase 4 明确迁移；无 `#[allow(dead_code)]`。
- 测试：`pnpm verify` 的 runtime 组全绿；新增目录表驱动测试与三入口差分测试。
- 行为：label/named_type_name/identity/depth 语义零变化。
