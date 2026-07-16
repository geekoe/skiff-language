# Phase 04：删除 Remote Production Path 与 Runtime 加固

状态：`outline-only`。Phase 03 验收后再细化；范围过大时可拆成两个可验收阶段。

## 目标

物理删除已不再被production选择的service-to-service router relay/remote fallback，并把完整
Runtime Assembly模型补到可长期运行的ready/reload/drain/admission/telemetry状态。保留的是逻辑
Boundary contract与dispatcher扩展缝，不是未验证的remote实现。

## 进入条件

- production所有service edge已由 `InProcessBoundary` 执行。
- router只需保留业务ingress、runtime control和artifact distribution职责。
- 至少有一组真实多service fixture验证本地assembly。

## 预计删除

- runtime outbound service call经router/WebSocket relay的选择与协议分支；
- router service-to-service routing/data-plane handler；
- provider缺失时的remote fallback、配置开关和兼容测试；
- 只服务旧service source/service_files模型的message/artifact adapter；
- 已无调用者的remote stub/protocol DTO。

删除后仍保留transport-neutral `BoundaryDispatcher` trait/contract test。未来实现真正remote模型时，
应新增一个完整dispatcher和placement/transport阶段，而不是复活隐式fallback。

## 预计加固

- assembly完整性、memory/CPU/resource admission与readiness gate；
- atomic reload、旧assembly drain、in-flight request/callback/stream生命周期；
- reload失败回退、版本可观测性和健康状态；
- provider trap、non-cooperative cancel、callback reentrancy/resource exhaustion的错误归类；
- per-service与per-assembly telemetry、trace owner和内存估算；
- 多runtime replica独立注册、健康摘除与共享外部存储语义。

本轮不提供process/trust/native-fault isolation。deployment若声明当前runtime无法满足的isolation
requirement，应在assembly admission fail closed；不能因为历史remote代码还在就偷偷远程执行。

## 预计验收

- 生产二进制和router协议中无service relay/fallback可达路径。
- router故障不影响同runtime内已接收请求的service-to-service调用；ingress/control故障语义另测。
- assembly reload要么整体切换，要么保持旧版本；无混合provider closure。
- drain期间callback/stream/cancel结果可解释，资源最终释放。
- 超出admission上限或要求不支持的isolation时，在接流量前失败。
- telemetry能按replica/assembly/service/operation定位内存、错误和reload状态。

## 细化前必须裁决

- remote协议相关通用boundary codec哪些是future seam，哪些应直接删除；
- atomic reload与drain的时间/强制终止保证；
- memory admission硬上限和overcommit策略；
- unsupported isolation requirement的manifest表达和错误taxonomy。

评审只阻塞真实生产残留、无法fail closed或reload不原子的问题；更多运维指标不默认阻塞。
