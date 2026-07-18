# P2-T05C7：Canonical Package Call Coverage

状态：consumer migration；依赖 T05C6，可与 T05C4 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package direct call”
“Compiler 与 Projection 流水线”“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- 让 PackageArtifact emission/materialization 对
  `ExternalRefTable.package_callables`执行 canonical PackageRequirement coordinate coverage校验。
- 独占 `compiler/emission/**` 的 package-call coverage 与直接 tests；禁止修改 artifact-model、lowering、
  source/compiled/driver/projection、runtime/linker、checker 与 integration tests。

## 完成态

1. Dependency alias与PackageId引用必须由PackageRequirement覆盖；unknown alias/id与external self ref
   fail closed，额外transitive requirement不影响合法direct ref。
2. coverage只验证package coordinate，不从symbol path/kind重建closure，也不重复验证 expectedLocalAbi；
   callable id存在性由已验证dependency input与未来linker重复验证。
3. `package_operation_symbols`/`PackageOperationSymbolRef` 在emission production与直接tests零命中。

## 验证

- emission package requirement coverage聚焦tests、必要cargo check、反向搜索、targeted rustfmt、
  `git diff --check`；不运行compiler integration tests或T07 gate。

提交并保持worktree clean；回报coverage矩阵、测试与consumer handoff。
