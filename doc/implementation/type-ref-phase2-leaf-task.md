# Phase 2 叶子任务文件：union 规范化语义对齐

状态：完成，已自验收，待集成（随实现 commit 一并提交）

## 引用链

- 权威设计：`doc/implementation/compiler-type-ref-unification-plan.md`
  - baseline commit：`77c0cef3557b4b48d6d74856dcadd8894d5e1fda`
  - 条款：§4.2 `union_type_ir` ×2 语义分叉、§5.2 `normalize_union`、§6 Phase 2、§7 验证、§8 风险
- 直接父节点：Phase 1 已合入 `main`（merge `e5848547`；实现 `f9a745b0`，
  core `normalize_union` 位于 `compiler/core/src/type_ref.rs:307`，配套测试
  `compiler/core/src/type_ref/tests.rs`；Phase 1 叶子任务
  `doc/implementation/type-ref-phase1-leaf-task.md`）
- 本节点解除：Phase 3 逐对替换（`record_field_type_from_ir`、`single_for_item` 等）的
  union 语义前提

## DAG 位置与基线

- 阶段：Phase 2（union 规范化语义对齐；本方案唯一允许的 union 语义行为变化步骤）
- baseline：`77c0cef3`（当前 `main` HEAD）
- 分支：`impl/type-ref-phase2`；worktree：`/Users/geek/workspace/skiff-phase2`
- 集成 Agent：`/root/skiff_integration_phase1`（集成后合并 `main`、清理 worktree/branch）

## 预检结论（只读，零 worktree 阶段完成）

### 写集（预期 owner）

1. `compiler/source/src/type_resolution_model.rs`
2. `compiler/source/src/expression_type_model.rs`
3. `compiler/source/src/type_resolution_model/shape_assignability.rs`
   （经 `use super::*` 消费 trm 私有 `union_type_ir` 的机械调用点）
4. `compiler/core/src/type_ref/tests.rs`（canonical 矩阵测试常驻）
5. `compiler/source/src/expression_type_model/tests.rs`、
   `compiler/source/src/type_resolution_model/tests.rs`
   （仅临时差分测试，提交前删除；若现有测试期望因语义对齐变化，在此更新并说明）
6. 本叶子任务文件

### 真实调用点（baseline `77c0cef3`，`git grep` 确认）

trm `union_type_ir`（5915）调用点：
- `type_resolution_model.rs:931`（`expand_alias_type_ref_inner` Union 分支）
- `type_resolution_model.rs:5903`（`record_field_type_from_ir` Union 分支）
- `shape_assignability.rs:1718`（`transparent_alias_ir` Union 分支）

trm `normalize_source_type_ref`（5919）调用点（并入 core `normalize_union`）：
- `type_resolution_model.rs:746`（`expand_alias_type_ref` 收尾 `.map(...)`）
- `type_resolution_model.rs:2070/2073`（`canonicalize_type_ref` Nullable/Union）
- `type_resolution_model.rs:2145/2148`（`canonicalize_type_ref_for_module` Nullable/Union）

etm `union_type_ir`（5202）调用点：
- `expression_type_model.rs:4933/4937`（`record_field_type_from_ir`）
- `expression_type_model.rs:5133`（`non_nullable_type`）
- `expression_type_model.rs:5151`（`narrow_type_by_tag`）

删除的私有函数：两处 `union_type_ir`、trm `normalize_source_type_ref` /
`normalize_source_union` / `collect_source_union_member`（无其他 crate 消费；
`git grep` 全仓库确认 trm/etm 私有函数无外部引用）。

### 发现（超范围，需上报主 Agent 决策归属）

`compiler/lowering/src/type_lowering.rs:269` 存在第三份 `union_type_ir`
（`pub(super)`）：排序键为 `type_ref_ir_type_text`、空 union 折叠为 `never`、
顶层 sort/dedup，语义与 trm/etm 都不同；消费点为
`lowering/src/function_lowering.rs:2245`。设计 §4.2 只列了 trm/etm 两份。
本阶段不写入 lowering；建议主 Agent 决定追加到 Phase 3（等价替换）或单独节点。
记录为 non-blocking follow-up，不阻塞 Phase 2。

## 差分矩阵与预期差异（代码阅读推导，临时差分测试运行后确认）

输入行（均为 `Vec<TypeRefIr>`，即两个旧 `union_type_ir` 的入参；canonical 侧
包装为 `TypeRefIr::Union { items }` 后调 core `normalize_union`）：

| # | 输入 | trm 递归版（= core canonical） | etm 顶层版 | 差异 |
| --- | --- | --- | --- | --- |
| 1 | `[Union[string,number], bool]` | `Union[bool, number, string]` | `Union[bool, Union[string,number]]` | 递归 flatten |
| 2 | `[string, null]` | `Nullable[string]` | `Union[null, string]` | null 折叠 |
| 3 | `[string, Nullable[number]]` | `Nullable[Union[number, string]]` | `Union[Nullable[number], string]` | Nullable 折叠 |
| 4 | `[Nullable[Union[string,number]]]` | `Nullable[Union[number, string]]` | `Nullable[Union[string,number]]` | 递归 + 排序 |
| 5 | `[]` | `Union[]` | `Union[]` | 相同 |
| 6 | `[string, string]` | `string` | `string` | 相同 |
| 7 | `[Union[string,string], string]` | `string` | `Union[string, Union[string,string]]` | 递归 dedup |
| 8 | `[Record{f: Union[string,null]}]` | `Record{f: Nullable[string]}` | `Record{f: Union[string,null]}` | 字段递归 |
| 9 | `[Function{params:[x: Union[string,null]], -> number}]` | 参数折叠为 `Nullable[string]` | 原样 | 参数递归 |
| 10 | `[AnyInterface(args=[Union[string,null]])]` | args 折叠为 `Nullable[string]` | 原样 | 参数递归 |
| 11 | `[Array<Union[string,null]>]`（`PackageTypeRef::Local` 解包后的容器 IR） | `Array<Nullable[string]>` | `Array<Union[string,null]>` | 容器参数递归 |

第 11 行说明：`PackageTypeRef::Local` 内嵌完整 `TypeRefIr`，转换本身属 Phase 4；
本阶段锁定的是"Local 解包后容器成员参与 union 归一化"的 IR 层行为。

临时差分测试：
- `type_resolution_model/tests.rs`：`trm union_type_ir == core normalize_union`（逐行断言相等）
- `expression_type_model/tests.rs`：`etm union_type_ir` 与 canonical 逐行断言（含差异行 `assert_ne`）

运行结果（2026-07-31，`cargo test -p skiff-compiler-source phase2_ -- --nocapture`，
2 passed；trm 版与 core canonical 全部 11 行相等；etm 版差异行与上表一致）：

- `nested`：legacy `Union[bool, Union[string, number]]` vs canonical
  `Union[bool, number, string]`（递归 flatten + 排序）
- `null_member`：legacy `Union[null, string]` vs canonical `Nullable[string]`（null 折叠）
- `nullable_member`：legacy `Union[Nullable[number], string]` vs canonical
  `Nullable[Union[number, string]]`（Nullable 折叠）
- `nullable_wrapping_union`：legacy `Nullable[Union[string, number]]`（内层未排序）vs
  canonical `Nullable[Union[number, string]]`（递归 + 排序）
- `empty` / `duplicate`：legacy == canonical（`Union[]` / `string`）
- `nested_duplicate`：legacy `Union[string, Union[string, string]]` vs canonical
  `string`（递归 dedup + 单成员折叠）
- `record_member`：legacy `Record{f: Union[string, null]}` vs canonical
  `Record{f: Nullable[string]}`
- `function_member`：legacy 参数原样 `Union[string, null]` vs canonical
  `Nullable[string]`
- `any_interface_member`：legacy args 原样 vs canonical `Nullable[string]`
- `local_wrapped_container`：legacy `Array<Union[string, null]>` vs canonical
  `Array<Nullable[string]>`

临时差分测试是证据，不进入最终提交；canonical 矩阵作为常驻测试落到
`compiler/core/src/type_ref/tests.rs`。

## 实现步骤

1. 临时差分测试（两个测试模块内，访问父模块私有函数）→ 运行确认差异清单 → 回填本文件。
2. 生产替换：
   - trm 调用点：`union_type_ir(x)` → `normalize_union(TypeRefIr::Union { items: x })`；
     `normalize_source_type_ref(ty)` → `normalize_union(ty)`；删除 5 个私有函数。
   - etm 调用点：`union_type_ir(x)` → `normalize_union(TypeRefIr::Union { items: x })`；
     删除私有 `union_type_ir`。
   - 两个文件顶部 import `skiff_compiler_core::type_ref::normalize_union`；
     `shape_assignability.rs` 同样 import。
3. 删除临时差分测试；core `type_ref/tests.rs` 常驻矩阵测试（含 #4/#8/#9/#10/#11
   与 Phase 1 未覆盖的行）。
4. 跑聚焦测试，逐个处理因语义对齐变化的现有测试期望（本文件记录每处变化）。

## 现有测试期望变化：无

`cargo test -p skiff-compiler-source` 338/338 全绿，`cargo test -p skiff-compiler-core`
62/62 全绿（含新增矩阵测试）。etm 顶层版与 canonical 递归版的差异（见差分清单）
只存在于私有函数级；当前 338 个 source 测试没有任何断言经过差异输入
（嵌套 union / null 折叠 / Nullable 成员），因此零测试期望变化。语义对齐本身由
差分测试证据 + core 常驻矩阵测试锁定；`.skiff` 级行为由后续 `--only skiff-tests`
gate 验证（不属于本开发 Agent 的聚焦范围）。

## 自验收矩阵

| 设计/任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| §6 Phase 2：两文件统一调用 core `normalize_union` | trm `type_resolution_model.rs` 5 处调用点（746/931/2070/2073/2145/2148/5903）+ shape_assignability.rs 1718；etm `expression_type_model.rs` 4 处（4935/4941/5136/5156） | `rg union_type_ir|normalize_source_type_ref|normalize_source_union|collect_source_union_member` 在 compiler/source、compiler/core 0 命中（仅 doc 注释与 leaf 文件提及） | `cargo test -p skiff-compiler-source` 338 passed |
| §4.2/§5.2：canonical = trm 递归版 = core `normalize_union` | core `type_ref.rs:307`；trm 旧实现逐分支一致（差分测试全 11 行相等） | trm/etm 私有实现已删除 | `cargo test -p skiff-compiler-core` 62 passed（含新增 `normalize_union_phase2_matrix_function_any_interface_and_containers`） |
| 任务条款 1：差分测试 + 差异清单 | 差异清单记录于本文件"运行结果"节 | 临时差分测试已从两个 tests.rs 删除（`rg phase2_` 0 命中） | 运行证据：`cargo test -p skiff-compiler-source phase2_ -- --nocapture` 2 passed（临时，已删） |
| 任务条款 4：测试期望变化逐处说明 | 见"现有测试期望变化：无" | — | 338/338 无失败 |
| 非目标：不动 Phase 3/4 表面 | 未触碰 `type_ref_debug_text`/`record_field_type_from_ir` 其他逻辑/转换/identity | `git diff` 仅 4 个代码文件 + leaf | — |
| 非目标：不新增依赖方向 | import 仅 `skiff_compiler_core::type_ref::normalize_union`（source 已依赖 core） | `git diff compiler/*/Cargo.toml` 无改动 | `cargo fmt --check` passed |

## 验证命令结果（2026-07-31）

```text
cargo test -p skiff-compiler-core          62 passed; 0 failed
cargo test -p skiff-compiler-source        338 passed; 0 failed
cargo fmt --check                          passed
```

未运行 `verify.mjs` / `--only skiff-tests`（验收 gate，留给后续 gate owner）。

## 禁止（非目标）

- 不动 `type_ref_debug_text`、`record_field_type_from_ir` 其他逻辑、
  `single_for_item*`/`map_entry*`、`function/operation_callable_resolution`、
  `package_type_resolution`/`package_interface_fact`（Phase 3）。
- 不动转换/identity（`package_type_ref_ir`、`interface_abi_id` 等，Phase 4）。
- 不动 artifact-model；不动 `compiler/lowering`、`compiler/projection`。
- 不新增依赖方向；不改设计语义；不 push；不直接写 `main`。

## 验证命令（本阶段唯一 owner：本开发 Agent）

```bash
cargo test -p skiff-compiler-core
cargo test -p skiff-compiler-source
cargo fmt --check
```

不运行 `verify.mjs` / `--only skiff-tests`（验收 gate，由后续 gate owner 执行）。
测试期望变化必须在 diff 中显式出现并在此文件说明，不允许静默变化。

## 交接

完成后向 `/root/skiff_integration_phase1` 移交 branch/worktree/commit/写集/
自验收矩阵/测试期望变化说明，并通知主 Agent `/root`；含 lowering 第三份
`union_type_ir` 的归属建议。
