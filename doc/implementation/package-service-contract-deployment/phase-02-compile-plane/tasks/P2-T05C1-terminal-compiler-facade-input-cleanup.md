# P2-T05C1：Terminal Compiler Facade / Input Cleanup

状态：checkpoint repair split；R06 合流后可与 R04/R13 并行，T05C 的独立前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“核心结论”“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 目标与 ownership

- 收敛 `skiff-compiler` facade/Cargo 到 PackageArtifact 与 ServiceContract 两个 terminal producer。
- 物理删除 compiler input、projection-input、直接 production error surface 与 facade 侧已断链的
  publication/service orchestration owner。
- 独占 `compiler/Cargo.toml`、必要 workspace Cargo、`compiler/input/**`、
  `compiler/projection-input/**` 及直接 projection/error cleanup；不修改 `compiler/publication-abi/**`，
  其 source edge 与 orphan crate disposition 由 T05C2 独占。
- 禁止修改 `compiler/core/**`、source、lowering、driver pipeline、emission、checker 与 integration tests；
  core 残留由 R13 后的 T05C 处理。

## 完成态

1. facade 显式声明实际使用的 artifact-model/artifact-identity 依赖，并删除
   `skiff-compiler -> skiff-compiler-publication-abi` normal edge。
2. compiler input 不再公开或构造 PublicationInputKind、RawServicePublicationJob、service dependency
   assembly reader；facade 不再依赖或导出 publication-ABI，crate 最终 disposition 交给 T05C2。
3. projection-input 和直接 production error surface 不保留已断链的 service dependency/ingress DTO、
   service-publication/conformance adapter 分支。
4. 不通过 feature、phase、exception、legacy/compatibility adapter 或 provider inference 隐藏旧边。
5. 不修改 `compiler/tests/**`、test-support 或 Cargo integration test target；断链 fixture 由 R10 迁移。

## 验证

- `cargo check -p skiff-compiler` 及直接受影响 production crates；若只被尚未合流的 R13/T05C core
  cleanup 阻断，记录精确诊断。
- `node scripts/check-compiler-crate-dag.mjs` 必须 PASS。
- boundary checker 只允许命中明确留给 T05C/T05C2 的 core、input-model、source、lowering 写域，
  并列出完整命中。
- production 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests或 T07 完整 gate。

提交并保持 worktree clean；回报删除 owner、Cargo/DAG 变化、剩余 core-only boundary 命中和聚焦证据。
