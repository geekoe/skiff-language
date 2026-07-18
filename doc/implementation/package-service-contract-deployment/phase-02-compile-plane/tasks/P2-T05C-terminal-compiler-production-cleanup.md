# P2-T05C：Terminal Compiler Production Cleanup

状态：checkpoint repair；由 `9b9f88b` 独立验收升级，必须在 R10 fixture 迁移前完成。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“核心结论”“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 背景

T05/T05A/T05B 合流后的 structure checker 已正确 fail closed，但 production tree 仍保留旧
publication/service orchestration owner；facade 还缺直接依赖，并继续声明 publication-ABI crate edge。
旧 compiler integration tests 的迁移属于 R10，不由本任务处理。

## 目标与 ownership

- 让 `skiff-compiler` facade 在 terminal PackageArtifact/ServiceContract 依赖图上可编译。
- 物理删除 compiler production 中残留的 PublicationInput/Kind、RawServicePublicationJob、
  PublicationAbiUnit、PackageUnit/ServiceUnit/serviceAssembly consumer/producer 及 dead public DTO。
- 独占本次 production cleanup 所需的 compiler facade/Cargo、input、core、publication-abi、
  projection-input 与直接 production error surface；不修改 source/lowering canonical 语义。
- 在 R06、R13 合流后执行，避免争抢 driver/Cargo 与 core/lowering 邻接写域。

## 完成态

1. facade 显式声明实际使用的 artifact-model/artifact-identity 依赖，并删除
   `skiff-compiler -> skiff-compiler-publication-abi` normal edge；不通过 feature、phase 或 exception 隐藏旧边。
2. `compiler/**` production Rust 中 structure boundary checker 零 deny；旧 crate/module 不再由 workspace、
   facade 或 production crate graph 引用。
3. projection-input 与 production error surface 不保留已断链的 service dependency/ingress DTO 或
   service-publication/conformance adapter 分支。
4. 不建立 legacy/compatibility adapter，不恢复 provider inference，不改 artifact wire/identity。
5. `compiler/tests/**`、test-support、Cargo integration test target 与旧 fixture 不在本任务处理；由 R10
   按逐项 disposition 迁移，不能为让测试暂时编译而恢复 production owner。

## 验证

- `cargo check -p skiff-compiler` 及直接受影响 production crates。
- `node scripts/check-compiler-boundaries.mjs`。
- `node scripts/check-compiler-crate-dag.mjs`。
- production 旧 symbol/owner 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests、T07 完整 gate或 runtime/router/test-runner 测试。

提交并保持 worktree clean；回报删除的 production owner、Cargo/DAG 变化、checker 结果、聚焦编译证据
与留给 R10 的测试断链清单。
