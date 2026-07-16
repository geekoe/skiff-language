# P1-T06：拆分 Package Artifact Projection

状态：`ready`
类型：营地前置任务，行为等价
依赖：无
执行者：Package Projection Agent，一份提交

## 背景

`compiler/projection/src/package_unit_artifacts.rs` 已同时处理 file/resource refs、exports、public
instances、dependency constraints、config/effect metadata、Publication ABI 和 identity，超过千行。
T09 需要加入 boundary contract，不能继续扩张该聚合文件。

## 目标

把 package projection 拆成有单一职责的模块，保持当前 PackageUnit、File IR、resource、ABI、
identity 和错误结果行为等价。

## 建议模块边界

```text
compiler/projection/src/package_unit/
  mod.rs             orchestration only
  files.rs
  resources.rs
  exports.rs
  public_instances.rs
  dependencies.rs
  config.rs
  assembly.rs
```

保留一个明确的 `project_package_ir_artifacts` facade；T09 后续可以增加独立 `boundary.rs`，而不是
把逻辑塞回 facade。

## 范围

- 移动 `package_unit_artifacts.rs` 的实现与测试。
- 更新内部 import/module path。
- 可删除空壳旧文件，或保留极小兼容 module re-export；阶段合并时不得有两个实现 owner。

## 非目标

- 不增加 code/effect/boundary contract。
- 不改变 PackageUnit schema/identity。
- 不重写 package export、public instance 或 dependency 算法。
- 不清理 service projection。

## 实现约束

- orchestration 只组合 typed projections，不内联 domain 算法。
- config 与 effect 不因历史 struct 同名而继续放在一个职责模块；本任务可以保持输出 shape，
  但模块边界要允许 T08/T09 替换 empty effect。
- helper 只放到真正的 shared owner；不复制到多个新模块。
- 测试按行为域移动，不能按行号切成无语义文件。

## 验收标准

- 原 1250 行聚合文件消失或成为小于约 150 行的 facade。
- 同一输入的 typed artifacts/JSON/identity 完全不变。
- package projection tests、artifact conformance 和 compiler boundary gate 通过。
- T09 有清晰的 boundary extension point。

## 聚焦验证

```bash
cargo test --no-fail-fast -p skiff-compiler-projection package
cargo test -p skiff-compiler artifact_model_conformance
node scripts/check-compiler-boundaries.mjs
git diff --check
```

## 停止条件

- 模块拆分需要改变 PackageUnit 或 identity；
- 同一个 helper 在当前文件中已经表达两种不同语义；
- tests 依赖 private 函数布局而无法用 behavior assertion 替代。

## 提交

提交信息建议：`refactor(compiler-projection): split package artifact projection`
