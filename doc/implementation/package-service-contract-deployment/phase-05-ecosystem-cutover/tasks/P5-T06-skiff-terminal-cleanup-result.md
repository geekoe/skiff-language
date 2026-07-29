# P5-T06：Skiff 旧 artifact surface 聚焦清理结果

## 范围

本结果只记录 T06 在 Skiff 仓库中的旧 artifact/public surface 清理，不代替 T13、A01 或最终生态验收。
协调分支已合入共享集成点 `28900481abf9bcc4edf79d047b46b2a75b9410f5`，并保留其中的
File IR v10、compiler-private relocation、runtime/boundary hard cut。

## 已完成

- 删除 `PackageUnit`、`ServiceUnit`、`PublicationAbiUnit`、`PackageTestAssembly`、
  `ServiceDependencyConstraint`、`ServiceDependencyOperationRef` 的 Rust 定义、公开导出、
  旧身份 owner、CLI/runtime fixture 引用和只验证旧形状的测试。
- 删除已经没有消费者的旧 identity/resolver/runtime-program、assembly/bundle/index/build record
  及 obsolete authoring 模块；保留 `PackageArtifact`、`ServiceContract`、`ServiceDeployment`、
  `RuntimeAssembly` 和当前 compiler/runtime 所需的中性叶类型。
- 扩展 artifact identity 单一来源检查器：生产代码和测试代码都禁止重新引用上述六个旧类型；
  注释、普通字符串、byte string、raw string 不会产生误报。self-test 覆盖生产引用、测试引用和
  字符串伪装。
- 反向 Rust 标识符扫描对上述六个类型为零命中。

`ServiceDependencySymbolRef`、remote-operation slot/table plan 等叶类型仍有 compiler/linker/runtime
生产消费者，本次按“只删已无生产消费者者”的约束保留；这不是对其长期设计的认可，若要继续 hard cut，
需要先迁移这些消费者。

## 聚焦验证

已通过：

- `cargo fmt --all -- --check`
- `node scripts/check-artifact-identity-single-source.mjs --self-test`
- `node scripts/check-artifact-identity-single-source.mjs`
- `node --test scripts/tests/check-artifact-identity-single-source.test.mjs`
- `node scripts/check-runtime-artifact-boundaries.mjs --self-test`
- `node scripts/check-runtime-artifact-boundaries.mjs`
- `node scripts/check-compiler-boundaries.mjs`
- `node scripts/check-command-execution-policy.mjs`
- `git diff --check`

按共享集成约束，本轮未运行 `cargo check --workspace` 或完整 Rust tests；这些是合入后的待补重型验证，
不能据此声称 T13、A01 或 Phase 5 最终通过。
