# Phase 05：RuntimeAssembly Resolution And Linking

状态：outline-only；Phase 04 验收后再细化

## 输入

- 一组 root ServiceDeployments。
- 全部引用的 ServiceContracts 与 PackageArtifacts。

## 目标

- 解析 deployment/package/service requirement 完整闭包和唯一 provider。
- 每个 replica 内 package code只链接一次；生成 per-ActivationContext service binding vector、config/state
  bindings与activation templates。
- 以 `(callerPackageBuildId, serviceRequirementSlot)` 解析 service call，不全局 patch call site。
- 生成独立 AssemblyIdentity并实现原子 load/reload/admission readiness。

## 验收边界

- 循环 service requirement graph可闭合；零/多 provider、identity mismatch或需要 remote provider均 fail
  closed。
- 同一 PackageArtifact 被多个 deployments复用时只共享只读 code/type/link image，不共享 activation-owned
  mutable state。
- 每个 runtime replica加载同一完整 assembly；本阶段可以构建和加载，service boundary执行留给 Phase 06。
- 旧`serviceAssembly`退出runtime/linker semantic owner；Phase 07前若保留，只能是无closure/link语义且有
  结构gate的tooling input adapter。

## 细化前复查

复查 runtime loader/linker、artifact graph、linked program image、router reload和当前 serviceAssembly。
任何 `service_files`、隐式 root slot或按 display name链接必须成为删除任务。
