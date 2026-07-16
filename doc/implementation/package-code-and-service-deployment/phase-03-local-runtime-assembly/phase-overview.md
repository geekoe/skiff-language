# Phase 03：Runtime Assembly 与 InProcessBoundary

状态：`outline-only`。Phase 02 验收后再细化。

## 目标

把一组ServiceUnit及其完整PackageUnit/service requirement closure组装成一个原子Runtime
Assembly；所有service-to-service edge在同一runtime replica内通过逻辑Boundary ABI执行。

## 进入条件

- service已是config-only deployment，所有executable code来自PackageUnit graph。
- loader/linker有显式root package和清晰activation owner。
- Phase 01的LinkableValue/callback/effect契约已在真实artifact中稳定使用。

## 预计工作域

1. 定义RuntimeAssembly artifact/lock/identity：package code closure、service deployments、provider
   binding、entrypoint、resource/admission facts和deterministic ordering。
2. assembly builder闭合每个 `ServiceContractRequirement`；缺失、多个provider、version/protocol/
   operation不匹配全部fail closed。
3. 建立transport-neutral `BoundaryDispatcher` seam与本轮唯一实现 `InProcessBoundary`。
4. caller adapter按LinkableValuePlan materialize detached ordinary data，注册request-scope callback/
   native capability，建立stream/error/timeout/cancel上下文。
5. provider dispatcher进入独立 `ServiceRuntimeContext`/activation：provider config、DB/state owner、
   principal/effect/trace归属不能泄漏为caller。
6. 支持callback reentrancy、stream backpressure和request结束后的capability失效。
7. runtime一次加载完整assembly；本阶段保证dependency-closed的新assembly要么整体激活、要么
   保持旧assembly，不能逐service出现半新半旧。in-flight drain、失败回退/readiness运维策略由
   Phase 04加固。
8. production service call不再经过router或网络；缺本地provider直接失败，不remote fallback。

## Replica 模型

每个runtime replica包含完整assembly和独立heap/CPU调度/activation。扩容复制replica；MongoDB、
Redis等外部存储按deployment配置共享。router仍只负责ingress和runtime control/distribution，
不承载service-to-service data plane。

## 预计验收

- 三层service调用链完全在单一runtime内执行，但每条service edge都遵守detached/capability
  boundary semantics。
- mutable package helper本地调用保留引用语义；同函数若无boundary projection不能被service选择。
- `any I`/native callback往返、stream、throw/error、deadline/cancel和principal owner有聚焦测试。
- provider state/config/DB owner在in-process优化下仍正确隔离。
- 缺provider/protocol mismatch启动或reload失败，绝不交给router。
- production trace证明service-to-service没有网络hop。

## 细化前必须裁决

- assembly发布/指针/原子reload的最小artifact shape和identity owner；
- 同一service deployment在一个replica内的activation数量与并发模型；
- callback reentrancy/order、stream buffer和non-cooperative cancel的当前保证；
- 单replica的memory admission依据及无法满足时的错误边界；
- revision选择与同一 `(serviceId, version)` provider唯一性校验的具体assembly数据结构；冲突
  provider始终拒绝，不重新开放隐式选择。

不能从canonical execution contract唯一推导的事项询问用户，不由runtime Agent自行决定。
