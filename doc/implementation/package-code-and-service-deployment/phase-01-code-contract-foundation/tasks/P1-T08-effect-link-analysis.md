# P1-T08：实现 Sound Effect / Link Analysis

状态：`ready`
类型：Compiler semantic analysis
依赖：P1-T03、P1-T05
执行者：Effect Analysis Agent，一份提交

## 目标

从 typed source/lowered File IR 计算 callable-level sound may-effect 与 link requirements，并通过
T05 的 typed handoff 交给 projection。分析不要求完备：可以保守拒绝 boundary projection，但
不能把有 caller-visible heap/alias 行为的函数误标为可跨 boundary。

## 分析位置

首选在 `compiler/compiled/src/code_analysis/` 建立独立模块：

```text
code_analysis/
  mod.rs
  call_graph.rs
  provenance.rs
  direct_effects.rs
  fixed_point.rs
  diagnostics.rs
```

它消费 `SourceCompileModel` 与 `FileIrUnit`，产出 artifact-model 定义的 typed facts，再由
projection-input 的独立 analysis domain 携带。linker/projection 不得重读 AST。

## 最小 effect 域

每个 callable 至少可表达：

- 对 caller-reachable parameter graph 的 read/write；
- 返回值或 throw payload 是否 alias caller-reachable graph；
- caller-reachable value 是否 escape 到 capture、callback、stream、spawn、DB/native/external；
- 是否依赖 same-heap identity/alias observation；
- callback/stream capability requirements；
- package/local executable、service operation、native adapter、未知 external target 等 link
  requirements；
- `Unknown` 的结构化来源。

内部 DB/native 操作本身不自动使 service operation 不可部署。只有它影响 caller-reachable
语义、需要不存在的 adapter/capability，或 effect 无法界定时，才成为 boundary blocker；同时
仍应记录 execution/link requirement 供 assembly 使用。

## 算法要求

- 构建 deterministic call graph；local/package known calls 传播 effect。
- 递归和互递归用 monotone fixed point，直到稳定；不得依赖遍历顺序。
- slot/expression provenance 至少区分 fresh、parameter-root、derived-from-parameter、callback/
  native carrier、unknown。
- field/index mutation 沿 provenance 归因；`input.name = ...` 必须标为 caller-reachable write。
- return/throw/escape 传播 alias provenance。
- 未知 native/external/builtin contract fail closed；已知 builtin/native contract 由单一 registry
  提供，不在分析器里散落字符串特判。

## Local 与 Boundary 的关系

本任务只产生事实，不直接决定最终 `Available/Unavailable`。但必须留下足够信息使 T09 得到：

- mutable helper 的 Local Code ABI 仍存在；
- caller-reachable write、return alias、same-heap identity 依赖成为稳定 boundary blocker；
- ordinary internal mutation/fresh allocation不被误报为 caller mutation；
- callback capability 可以有明确 requirement，而不是被统称为不可序列化。

## 范围

- `compiler/compiled/src/code_analysis/` 新模块
- `compiler/compiled/src/lib.rs` 最小 orchestration
- T05 拆出的 `compiler/projection-input` analysis DTO domain
- 必要的 artifact-model effect/link types使用
- 聚焦 unit/integration tests

## 非目标

- 不生成 boundary value schema；T09 负责。
- 不修改 runtime/linker。
- 不追求 path-sensitive、flow-perfect 或全语言定理证明。
- 不把 effect 分析塞进 `compiler/lowering/src/function_lowering.rs`。

## 必须测试

- pure/fresh allocation、parameter field/index write、local slot write。
- direct/indirect return alias、throw alias、callback/stream/spawn escape。
- known local/package call 传播、unknown external/native 保守传播。
- 递归、互递归、不同声明/遍历顺序结果一致。
- service dependency call 形成 structured link requirement，而非 raw symbol string。
- diagnostic detail 不进入 identity fact。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-compiled code_analysis
cargo test --no-fail-fast -p skiff-compiler-projection-input analysis
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

## 验收标准

- `function mutate(input: User)` 的字段写入被准确识别；local slot/fresh object 写入不误判。
- 所有 public callable 都得到完整 effect/link summary；无“缺字段即 empty”。
- unknown callee 保守且可解释。
- fixed point deterministic，未复制 native/builtin contract registry。

## 停止条件

- File IR 丢失参数/slot/provenance 关系，且只能实质扩张 2300+ 行的
  `function_lowering.rs` 才能补足；
- known native/builtin effect 没有 canonical registry，必须在多处复制；
- package call graph 无稳定 callable identity；
- soundness 与保留 mutable local helper 发生无法解释的冲突。

前三项应升级为独立前置任务；最后一项带最小反例询问用户。

## 提交

提交信息建议：`feat(compiler): analyze callable effects and link requirements`
