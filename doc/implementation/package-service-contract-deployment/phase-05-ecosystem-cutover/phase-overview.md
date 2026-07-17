# Phase 05：Ecosystem Cutover

状态：outline-only；Phase 04 验收后细化

## 输入

- PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly 和完整 InProcessBoundary 生产路径。

## 完成态

- registry/release、CLI/watch/dev sync、router/runtime reload、test-runner、fixtures 与实际 services 全部切换。
- `skiff-packages` 与 `internals` consumer 使用新 artifact 和部署流程，跨仓库分别提交。
- 物理删除 Publication aggregate/pipeline、ServiceUnit、serviceAssembly tooling adapter、remote relay、legacy
  readers/writers 和旧 fixtures。
- 多个 runtime replica 加载同一完整 assembly，共享外部数据层，不承诺 service 级隔离或独立扩缩。

## 预期波次

1. Skiff tooling、`skiff-packages`、`internals` 三个写入域并行迁移。
2. legacy 反向搜索、完整非 live verify、必要 live/multi-replica/chat smoke、独立最终验收。

## 阶段验收

- production source tree 不存在四对象之外的共同 aggregate、dual path 或 runtime fallback。
- 平台支持 contract 先发布、packages 独立编译、deployment validation 和 assembly activation。
- 所有阶段 adapter 到期并物理删除；fixture disposition 有 replacement 或删除证明。
- 跨仓库 worktree/分支按各仓库规则合并和清理，未经用户要求不 push。
