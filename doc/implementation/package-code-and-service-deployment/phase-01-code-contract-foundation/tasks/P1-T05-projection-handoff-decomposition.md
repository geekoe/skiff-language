# P1-T05：拆分 Compiler Projection Typed Handoff

状态：`ready`
类型：营地前置任务，行为等价
依赖：无
执行者：Compiler Handoff Agent，一份提交

## 背景

`compiler/compiled/src/projection_input.rs` 与 `compiler/projection-input/src/lib.rs` 都超过千行，
集中承载 source、lowering、package ABI、config、service ingress/dependency 等多类 facts。T08/T10
需要新增 effect/link/service requirement handoff；继续追加会形成新的隐式总线。

## 目标

按 fact owner 拆分 producer 与 DTO crate，在不改变生成结果的前提下形成明确扩展点；每个事实
只在 `projection-input` 定义一次，由 `compiled` 负责 typed conversion。

## 建议模块边界

```text
compiler/projection-input/src/
  input.rs
  source.rs
  exports.rs
  package.rs
  entrypoints.rs
  config.rs
  service.rs
  types.rs

compiler/compiled/src/projection_input/
  mod.rs
  source.rs
  exports.rs
  package.rs
  entrypoints.rs
  config.rs
  service.rs
  types.rs
```

可以调整名称/粒度，但不得把同一 DTO 同时定义在 producer 与 consumer。

## 范围

- `compiler/compiled/src/projection_input.rs` 拆成目录模块。
- `compiler/projection-input/src/lib.rs` 拆成职责模块并从 `lib.rs` re-export。
- 对应 unit tests 按 domain 移动。
- 必要的 import path 修复。

## 非目标

- 不新增 effect/link/service requirement DTO；由后续任务完成。
- 不改变 source/lowering/projection 行为、排序、diagnostic 或 artifact。
- 不修改 crate DAG；若拆分暴露真实 cycle，应上报。
- 不重构 `compiler/lowering/src/function_lowering.rs`。

## 实现约束

- public API 可以通过 re-export 保持，内部调用逐步指向具体模块。
- DTO 字段保持 private + accessor 或现有可见性策略，不因拆分全部改成 `pub`。
- source model 到 projection DTO 的 conversion 只有一个 owner。
- 不引入 raw JSON/string bridge。

## 验收标准

- 两个原超长入口成为小型 module index/facade。
- compiled/projection-input 的现有测试和 compiler boundaries/DAG gate 通过。
- 生成相同 fixture 的 projection facts 深度相等；不更新 semantic golden。
- 后续 T08 可在独立 `analysis` domain 增加 typed facts，不必再扩张总入口文件。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-compiled -p skiff-compiler-projection-input
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

## 停止条件

- 必须改变 DTO shape 或 artifact 才能拆分；
- producer/consumer 对同一字段已有不一致语义；
- 发现 cycle 只能通过移动 ownership 跨 crate 解决。

这些属于独立架构修复，不能在机械拆分里静默处理。

## 提交

提交信息建议：`refactor(compiler): split projection handoff domains`
