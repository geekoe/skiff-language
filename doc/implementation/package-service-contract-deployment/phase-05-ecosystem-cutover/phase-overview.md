# Phase 05：Ecosystem Cutover

状态：outline-only；Phase 04 验收后细化

## 输入

- PackageArtifact、ServiceContract、ServiceDeployment、RuntimeAssembly 和完整 InProcessBoundary 生产路径。

## 完成态

- registry/release、CLI/watch/dev sync、router/runtime reload、test-runner、fixtures 与实际 services 全部切换。
- `skiff-packages` 与 `internals` consumer 使用新 artifact 和部署流程，跨仓库分别提交。
- consumer 直接读写 PackageArtifact/ServiceContract/ServiceDeployment/RuntimeAssembly；不新增
  旧 artifact reader/writer、转换器或 fallback。
- 多个 runtime replica 加载同一完整 assembly，共享外部数据层，不承诺 service 级隔离或独立扩缩。

## 预期波次

1. 先冻结不改变四对象 owner的 authoring/storage/control checkpoint；再并行迁移 Skiff本仓 registry/release、
   CLI/watch/dev sync、router/runtime reload、test-runner与 fixtures。
2. 本仓 checkpoint稳定后，从 exact integration commit并行迁移 `skiff-packages`、`internals` consumer与实际
   services；最后做旧对象/旧路径反向搜索、完整 non-live verify、必要 live/multi-replica/chat smoke和独立验收。

## 阶段验收

- production source tree 不存在四对象之外的共同 aggregate、dual path 或 runtime fallback。
- 平台支持 contract 先发布、packages 独立编译、deployment validation 和 assembly activation。
- 旧 DTO、reader/writer、转换器和 fallback 的 production 命中归零；fixture disposition 有 replacement
  或删除证明。
- 跨仓库 worktree/分支按各仓库规则合并和清理，未经用户要求不 push。
