# P2-T04：PackageArtifact 与 Boundary Projection

## 目标

用唯一PackageArtifact projection/materializer取代PackageUnit canonical compiler output，并为每个package
API callable生成显式boundary availability。任务消费typed effects，不实现effect分析。

## 依赖与 worktree

- 依赖 T01 checkpoint。
- 建议 branch：`codex/package-service-p2-t04-package-artifact`。
- 可与 T02、T03 并行；测试使用构造的Analyzed/Unknown facts，不等待T02 producer。

## 完成态

1. 新PackageArtifact projection复用Phase 01唯一File IR/export/implementation-link leaf，不复制PackageUnit
   builder。
2. PackageLocalAbi与PackageBuild identity只调用T01 canonical API；PackageArtifact不嵌入PublicationAbiUnit。
3. 所有package API callable都有Local ABI及BoundaryCallableProjection；Unknown、caller mutation/alias/
   escape、same-heap、unknown target、unsupported value/callback/native plan形成稳定Unavailable reasons。
4. Available保存contract-agnostic BoundaryOperationContract和BoundaryImplementationRequirements；不得
   填入或伪造ContractOperationId/stable key。implementation facts不污染contract/protocol identity。
5. PackageRequirement、ContractRequirement、ServiceRequirement、ServiceCallRef、config/resource/capability、
   effect/provenance完整进入artifact与build identity projection。
6. production 与 compiler test-support 共享同一个 PackageArtifact projection/materializer API；
   runtime package-test 留待终态 consumer 阶段，不允许第二 builder。
7. projection/emission按model、boundary policy、materialization、tests拆分；不得继续扩大现有monolith。

## 写入范围

- `compiler/projection` 新PackageArtifact与boundary目录及直接复用leaf。
- `compiler/emission` 新PackageArtifact materializer与直接tests。
- 必要的 projection-input typed mapping；不修改 source/lowering/driver/artifact 公共 wire。

## 验证

```bash
cargo test -p skiff-compiler-projection
cargo test -p skiff-compiler-emission
git diff --check
```

测试覆盖Available/每类Unavailable、Unknown fail closed、direct local mutation仍存在、identity inclusion/
exclusion、production/package-test parity和provider字段禁止。

## 回报

提交commit、自验收矩阵、每个Unavailable reason对应代码证据、旧PackageUnit builder反向搜索和新的唯一
materializer symbol。
