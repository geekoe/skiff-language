# Phase 03：Deployment And Assembly Plane

状态：outline-only；Phase 02 验收后细化

## 输入

- ServiceContract、PackageArtifact、BoundaryCallableProjection、ContractRequirement 与 ServiceRequirement。

## 完成态

- 仅凭 typed artifacts 生成并校验无源码 `ServiceDeployment`。
- 从 root deployments 解析唯一 provider、完整 package/service requirement 闭包并生成 `RuntimeAssembly`。
- replica 内 package code 只链接一次，生成 per-ActivationContext binding templates 和独立 AssemblyIdentity。
- runtime 能 load/link/admit assembly；零或多 provider、identity mismatch、remote-only closure fail closed。
- service source compile、code-owning ServiceUnit 与旧 serviceAssembly 的 closure/link 语义退出生产 owner。

## 预期波次

1. deployment/assembly schema、identity、reference 和 binding template checkpoint。
2. deployment projection、assembly resolver、runtime loader/linker 三域并行。
3. router/reload adapter、legacy semantic owner 删除、批次 gate 与独立验收。

## 阶段验收

- deployment projection 不读取 AST、source text 或 lowering helper。
- service requirement cycle 可以闭合；缺失、多 provider 或隐式 display-name linking 拒绝。
- 新 assembly load path 不需要 runtime 猜 raw JSON、source path 或 provider executable。
- 本阶段不要求执行 service boundary；Phase 04 负责 dispatcher 与 materialization。
