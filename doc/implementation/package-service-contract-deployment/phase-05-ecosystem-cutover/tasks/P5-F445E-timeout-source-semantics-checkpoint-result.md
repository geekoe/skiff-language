# P5-F445E Timeout source semantics checkpoint result

状态：`COMPLETED`。F445B-I2 的 source-semantics 边界已闭合；artifact/lowering/link/runtime
仍明确交给 I3 及后继节点，本节点没有越界实现临时 IR 或 runtime 猜测。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| 任务指定 integration input | `128129ff40ddc3302360a9cfadf500f4b0dc7194` |
| 本 task 初始 HEAD | `356f66f8e93acdce7f59888340ff6b6ee2d44a55` |
| implementation | `d6b4b78a945bd6ff70d195acb44225cc5214727c` |

implementation 写集只有：

- `compiler/source/**`
- `compiler/driver/pipeline/mod.rs`

implementation 提交后只新增本 result。没有 merge、rebase、push、stable、live、network 或
instance 操作。

## 2. Test-first 证据

先新增独立 `timeout_source_semantics_tests` 并注册测试模块，再运行任务要求的 source suite：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo test -p skiff-compiler-source --no-fail-fast
```

production 尚未修改时命令真实 RED，exit `101`；编译器报告新
`Stmt::{Timeout,Concurrent,Serial}` / `Expr::{ValueBlock,ConcurrentValue,Timeout}` 在 21 个
source production match 中未穷举，同时测试要求的稳定 execution-plan API 尚不存在。

实现过程中又分别保留了以下 RED→GREEN 收据，避免只覆盖 happy path：

- local helper 对 lane 外层参数的 transitive caller-reachable mutation；
- top-level const 中 execution scope 被静默丢弃；
- serial 内 shadow initializer 对前序 sibling const 的依赖；
- fresh wrapper 中携带 caller root，以及后续 store caller root 后的 projection mutation；
- block-local 名称退出 scope 后不应遮蔽同名 top-level callable；
- 同名直属 `const` shadow 应选择最近的前序 lane，而不是误报 self-forward。

最终聚焦模块 12 tests 全部 PASS。

## 3. Source semantics

### 3.1 Value、timeout、type 与 flow

- `value` body 使用独立词法 scope，body 的直属顺序 binding 对 tail 可见，退出后不泄漏；
- `concurrent value` 只向后续 sibling/tail 暴露直属前序 `const`；
- tail 决定 wrapper 类型，并把外部 expected type 传到最终 object-literal
  materialization/assignability target；
- `Expr::Timeout` 类型、contract projection、call/effect、throw、root provenance 与
  `maySuspend` 对被包装 value 透明；
- statement timeout 不产值；
- value/concurrent-value 边界内的 `return|break|continue` fail closed；
- duration 只调用 F445D `checked_milliseconds()`，source plan 只保存已 checked 的 `u64`
  milliseconds；
- top-level const 与静态 DB index predicate 中出现 execution scope 会显式拒绝，不会形成
  缺失 plan 的静默路径。

### 3.2 Concurrent surface、scope 与 mutation

每个 `concurrent` 的直属 statement 是一个 lane，直属 `serial` 是单一 `Serial` lane，
concurrent-value tail 是最终 `Tail` lane。

实现的 source rule 包括：

- 只有直属前序 `const` sibling-visible；forward、`let`、nested/serial binding 泄漏拒绝；
- shadow initializer 读取最近的前序同名 `const`，依赖不会被 lane-local binding inventory
  擦除；
- tail 隐式依赖全部前序 lane normal exit；
- `if|match|for|with|timeout|value|return|break|continue|throw|rethrow|catch|emit|spawn`、
  nested serial/concurrent 等 reference 禁止项全部 fail closed；
- direct mutation、mutating receiver builtin 和 local helper 的 caller-reachable mutation 都
  进入 outer-root 检查；
- literal/record/object/patch 形成的 lane-local fresh root 可 mutation；
- fresh container 若携带 caller/sibling/opaque payload，或后续 mutation 写入这类 payload，
  会保守降级为 opaque，不会借 fresh 外壳绕过 outer-root rule；
- 未知 root provenance fail closed。

### 3.3 External effect 与 cancel safety

source pass 为本地 callable 建立固定点 external-effect profile：

- DB read/read sibling lanes 可并行；
- external read/write、write/write 与 `exclusive` transaction/lease 冲突；
- DB target/conflict key 进入稳定诊断；
- compiler-owned DB operation 使用明确的 response-discard/transaction terminal
  cancel-safety；
- config 与 receiver builtin 是 local effect；
- local function/method 递归合并 profile；
- native/package/service/interface/actor/unknown target 目前没有完整
  target/conflict-key/cancel-safety tuple，因此一律 fail closed，不从名称或
  `maySuspend` 猜安全性。

## 4. I3 可消费的稳定 source plan

`PackageSourceModel::execution_semantics()` 公开：

- `TimeoutSourcePlan { duration_milliseconds, produces_value, source_site }`
- `ConcurrentSourcePlan { produces_value, lanes, source_site }`
- `ConcurrentLanePlan { source_order, kind, dependencies, source_site }`
- `ConcurrentLaneKind::{Statement,Serial,Tail}`

plan 完整性检查冻结：

- lane `source_order` 连续且稳定；
- dependency 严格递增、唯一且只能指向前序 lane；
- value plan 恰有一个最终 tail，statement plan 没有 tail；
- tail dependency 精确覆盖全部前序 lane；
- plan/lane source site 的 module/owner 一致且 span 有效；
- timeout duration 非零。

`compiler/driver/pipeline/mod.rs` 注册了 plan 完整性 pass。source analysis 已完成全部
source 推导；I3 必须消费 plan，runtime 不得重新解析 AST、按名称推断依赖或重算 lane kind。

## 5. F445D production consumer closure

| consumer | closure |
| --- | --- |
| expression source/type/assignability | body/tail 精确 preorder key；lexical/expected type/transparent target 全部显式处理 |
| name resolution | block 顺序 scope、value scope、concurrent prior-const scope、loop/match/lease scope |
| root refs | syntax visitor 完整遍历；value body-to-tail 的 `package` shadow depth 显式保存/恢复 |
| resolved call targets | 与 expression preorder 对齐的 scope-aware collector；body/tail call 不丢失 |
| callable effects/provenance | timeout 透明；value body/tail、concurrent sibling/tail 与 scoped outer-root update 显式 transfer |
| config | timeout/serial/concurrent body 与 value tail 完整遍历，const-string scope 保留 |
| stream | production type/key walker、call walker和 test helper 均显式覆盖 body/tail |
| package/function type | statement body 与 value tail 的 type-ref/generic validation 完整递归 |
| DB field path | value body/tail walker完整；DB change path 通过 DB projection resolver 校验 |
| alias/contract/root projection/provider | `AstVisitor`/`AstVisitorMut`、`expr_contains` 已由 F445D 完整 visitor 递归；consumer 自身只处理命中节点 |
| prelude/semantic interface/type resolution | 只消费 type syntax/registry，不直接遍历 statement/expression |
| callable analysis/call transfer | module const 未知形态已有 `_ => Unsupported`；call target 继续委托完整 expression transfer |

没有 wildcard 把新 AST 当作空 effect、空 type 或安全 concurrent path。

## 6. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-source timeout_source_semantics -- --nocapture` | PASS：12/12 |
| `cargo test -p skiff-compiler-source --no-fail-fast` | exit `101`：331 PASS，4 个 inherited failure；新测试全部 PASS；doc-tests 0 PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |
| `cargo check -p skiff-compiler` | 预期 exit `101`：source crate check 完成，停在 11 个 I3-owned lowering exhaustiveness error |

四个 source-suite inherited failure 与本节点写集无关，且在起始 revision 的单测复现中已存在：

1. `package_rules::reserved_validation::tests::collects_local_and_pattern_reserved_root_bindings`
   的 tolerant-parser fixture 没有 function，测试直接索引 `[0]`；
2. `prelude_registry::tests::platform_source_context_pins_current_prelude_identity`
   的 schema identity snapshot 不一致；
3. `prelude_registry::tests::p5_f18a::p5_f18a_prelude_loader_snapshot`
   的 prelude identity snapshot 不一致；
4. `type_resolution_model::tests::prelude_registry_is_the_only_source_builtin_spelling_owner`
   对当前 compiler-owned `std.date.Date` ownership 的预期不一致。

本节点没有修改这些测试或 prelude/reserved-validation owner。

## 7. 明确 I3 handoff

`cargo check -p skiff-compiler` 当前精确留下 11 个 I3 exhaustive site：

- `compiler/lowering/src/executable_declaration_lowering.rs:819`
- `compiler/lowering/src/function_lowering.rs:307`
- `compiler/lowering/src/function_lowering.rs:1255`
- `compiler/lowering/src/function_lowering.rs:2473`
- `compiler/lowering/src/function_lowering.rs:2532`
- `compiler/lowering/src/function_lowering.rs:2624`
- `compiler/lowering/src/source_unit_lowering.rs:66`
- `compiler/lowering/src/suspend_analysis.rs:384`
- `compiler/lowering/src/suspend_analysis.rs:500`
- `compiler/lowering/src/suspend_analysis.rs:714`
- `compiler/lowering/src/type_inference.rs:196`

I3 仍拥有 `artifact-model/**`、`compiler/compiled/**`、`compiler/lowering/**`、
`compiler/emission/**`、`compiler/projection/**`、`runtime/linked-program/**` 与
`runtime/linker/**`：新增显式 timeout stmt/expr、可复用 user value block、
`ConcurrentPlanIr`，补齐 IR walker/link validation，并原子更新 File IR schema/format/opcode
version、canonical identity 与 golden。I3 应从本节点 source plan 投影；不得重新猜 source
dependency，也不得用临时 metadata 或 Agine 特例跨过 artifact 边界。
