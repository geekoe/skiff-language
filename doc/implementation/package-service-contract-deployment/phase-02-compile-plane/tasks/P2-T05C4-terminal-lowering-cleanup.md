# P2-T05C4：Terminal Lowering Cleanup

状态：consumer migration；依赖 T05C6，可与 T05C7 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“不变量”
“Package 与 PackageArtifact”“Compiler 与 Projection 流水线”“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- 清理 `compiler/lowering/**` 中空的 `PackageOperationIndex`、`ServiceDependencyOperationIndex` 及沿
  executable/source/function lowering 传播的旧参数链。
- 按 expression key消费 `ResolvedCallTarget::DependencyPackageFunction`，生成 T05C6 冻结的
  `CallTargetIr::PackageCallable { package_ref, package_callable_id }`；service call继续使用既有 typed facts。
- 独占 lowering production 与直接 tests；禁止修改 source/input/core/driver/projection/emission、artifact
  wire/identity、Cargo/checker、runtime 与 compiler integration tests。

## 完成态

1. 两个空 legacy index 类型、构造点与参数链物理归零；`compile_publication_source_file_ir_unit` 等共同
   publication 命名改为准确 package/source lowering 名称。
2. package direct call使用dependency alias与PackageCallableId；expectedLocalAbi不进入call site。service
   boundary call继续生成既有`LoweredServiceCalls`/`ServiceCallRef` handoff。
3. 不二次分析 source、不读取 provider、不恢复 used-symbol closure、runtime witness 或 compatibility adapter。

## 验证

- lowering 聚焦测试与 `cargo check -p skiff-compiler-lowering`。
- 两个 index/旧函数名/空 carrier 反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests或 T07 完整 gate。

提交并保持 worktree clean；回报删除参数链、保留 call handoff、测试与自验收矩阵。
