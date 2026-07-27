# P5-F445E-R1 Timeout source semantics responsibility split result

状态：`COMPLETED`。F445E 的 source plan、diagnostic、test 与 public API 行为保持不变；
原 1951 行 `execution_semantics.rs` 已按职责拆为可审阅模块，没有修复既有 source 基线或接管
I3 lowering owner。

## 1. 输入、写集与提交

| 项 | commit |
| --- | --- |
| 直接父节点 implementation | `d6b4b78a945bd6ff70d195acb44225cc5214727c` |
| 直接父节点 result | `68ebd348bfd4cda5bd766b58c0e047fc390a3aed` |
| 本 task 初始 HEAD | `2308376f1c18f4b179efd3795e6e33ad32ca4504` |
| 纯重构 implementation | `f2b4203d05894269f3b07450986cc2b6f23ddb41` |

implementation 写集只有：

- 删除 `compiler/source/src/execution_semantics.rs`；
- 新增 `compiler/source/src/execution_semantics/*.rs`。

`compiler/source/src/lib.rs` 不需要修改：Rust 的目录模块继续由原
`mod execution_semantics;` 解析，原 public re-export 名称与可见性不变。implementation
提交后只新增本 result。没有派子 Agent，也没有 merge、rebase、push、stable、live、
network 或 instance 操作。

## 2. 职责边界

| 模块 | 行数 | 单一职责 |
| --- | ---: | --- |
| `mod.rs` | 109 | pass orchestration、owner 遍历调度、最终稳定诊断聚合 |
| `model.rs` | 124 | public plan model、只读 accessor 与 plan 完整性校验 |
| `effects.rs` | 290 | callable external-effect fixed point、DB effect profile、lane effect/cancel-safety 冲突 |
| `owner.rs` | 458 | owner lexical traversal、普通 statement/expression/value/timeout 分派、source site 与诊断归属 |
| `concurrent.rs` | 280 | lane DAG、sibling scope/reference、concurrent surface 与 serial lane 验证 |
| `mutation.rs` | 283 | binding root provenance、fresh/opaque taint、direct/builtin/local-helper mutation 验证 |
| `collectors.rs` | 536 | callable/expression 索引、static execution-scope detector、name/reference/AST helpers |

最大 production 模块为 536 行，全部低于任务给出的约 600 行审阅阈值。原
`OwnerAnalyzer` 的职责不再集中：

- 通用 owner/表达式遍历留在 `owner.rs`；
- concurrent lane/scope/reference 进入 `concurrent.rs`；
- root provenance、mutation 与 payload taint 进入 `mutation.rs`；
- external effect fixed point 与 lane conflict 进入 `effects.rs`；
- AST visitor/collector 与地址、pattern、statement 辅助进入 `collectors.rs`。

跨模块协作只使用 `pub(super)`，没有扩大 crate 或 public API 面。

## 3. 行为不变证据

拆分前在 task 初始 HEAD 运行聚焦模块：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445e-timeout-source/build/cargo-target \
  cargo test -p skiff-compiler-source timeout_source_semantics -- --nocapture
```

结果为 12/12 PASS。拆分并格式化后重复同一命令，仍为 12/12 PASS。以下合同没有改变：

- timeout checked milliseconds、value/non-value plan 与 source site；
- concurrent lane source order、kind、dependency 与 tail shape；
- lexical/sibling/forward-reference/serial scope；
- illegal concurrent surface 与 value-boundary control flow 的 fail-closed 诊断；
- outer/fresh/opaque root provenance、mutation 与 payload taint；
- callable fixed point、external conflict key 与 cancel-safety；
- static const/DB index execution-scope rejection；
- public plan accessor 与 `validate_complete()`。

没有修改 `timeout_source_semantics_tests.rs` 或任何测试预期。

## 4. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler-source timeout_source_semantics -- --nocapture` | PASS：12/12 |
| `cargo test -p skiff-compiler-source --no-fail-fast` | 预期 exit `101`：331 PASS、同 4 个 inherited failure；doc-tests 0 PASS |
| `cargo check -p skiff-compiler` | 预期 exit `101`：source crate check 完成，只停在同 11 个 I3-owned lowering exhaustive site |
| `cargo fmt --check` | PASS |
| `git diff --check` 与 staged diff check | PASS |

完整 source suite 仍只有 F445E result 已记录的四个 inherited failure：

1. `package_rules::reserved_validation::tests::collects_local_and_pattern_reserved_root_bindings`
   的 tolerant-parser fixture 索引越界；
2. `prelude_registry::tests::platform_source_context_pins_current_prelude_identity`
   的 schema identity snapshot 不一致；
3. `prelude_registry::tests::p5_f18a::p5_f18a_prelude_loader_snapshot`
   的 prelude identity snapshot 不一致；
4. `type_resolution_model::tests::prelude_registry_is_the_only_source_builtin_spelling_owner`
   的 compiler-owned `std.date.Date` ownership 预期不一致。

本节点没有修改这些 owner。

## 5. I3 handoff 保持不变

`cargo check -p skiff-compiler` 仍精确留下 11 个原 I3 exhaustive site：

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

没有新增 source error，也没有在本重构中添加 wildcard、临时 lowering、IR 或 runtime
推断来掩盖这些 I3-owned site。
