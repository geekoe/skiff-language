# P2-T05C8：Package Call Compiler Consumers

状态：consumer migration；依赖 T05C6，可与 T05C4/T05C7/T05C9 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package direct call”
“Compiler 与 Projection 流水线”“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- 迁移 compiler driver pipeline 与 core spawn-target reader到 T05C6 的
  `PackageCallableRef`/`CallTargetIr::PackageCallable`。
- 独占 `compiler/driver/pipeline/**` package-call direct consumers/tests与
  `compiler/core/src/spawn_targets.rs` direct consumer/tests。
- 禁止修改 artifact-model/identity、lowering、emission、source/input/projection、runtime/linker、checker
  与 compiler integration tests/test-support。

## 完成态

1. driver 的 requirement/used-std判断同时读取 type-only `external_refs.package_symbols.package` 与 callable
   `external_refs.package_callables.package_ref`，不读取callable id推导新的dependency closure，也不恢复
   operation symbol表。
2. core spawn-target traversal准确处理`PackageCallable` target，仅按现有语义判断外部package call；不查
   OperationAbiRef或dependency artifact target。
3. 本写域旧 target/ref/table零命中，直接tests使用PackageCallableId。

## 验证

- driver pipeline/core spawn-target聚焦tests、必要`cargo check -p skiff-compiler -p skiff-compiler-core`、
  反向搜索、targeted rustfmt、`git diff --check`。
- 不运行compiler integration tests、runtime或T07完整gate。

提交并保持worktree clean；回报consumer映射、测试与自验收矩阵。
