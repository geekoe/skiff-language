# Skiff 剩余问题

日期：2026-04-26

本文只记录当前 canonical 尚未规范化的问题。已解决的旧审阅项不再放在这里；历史交叉审阅记录只用于追溯设计背景。

这些问题不是当前 syntax / static semantics / runtime reference 的阻塞项。

## 当前仍待规范化的问题

1. **用户级异步任务和后台流程**

   Actor 文档已定义 `spawn` 的最小后台调用语义，queue 文档已定义 durable queue 和 timer 的平台边界，但用户可见的完整 async task、cron、startup / managed worker 和 `TaskHandle` surface 仍未规范化。后续版本仍需定义任务声明、结果查询、重试、取消、观测、启动恢复、后台 worker 和业务错误记录方式。

2. **宿主互操作 / FFI**


3. **观测生产化扩展**

   Observability capability 已定义日志、trace、指标、health、topic、归属、查询和 best-effort 交付承诺。后续仍需细化 OpenTelemetry 兼容映射、错误聚合算法、告警规则、长期采样策略和具体存储后端的生产 schema。

4. **stream / client session 外部协议细节**

   当前规范已经明确内部 transport 支持 unary / server-stream、`requestId` 配对、chunk / end / error / cancel、最小 backpressure 保证，以及 `std.client` 的最小 API。仍需细化的是具体 wire encoding、chunk size、WebSocket ack / error outbound frame、SSE event 映射、client capability discovery、离线重放、reconnect resume 和 actor binding 过期 / 刷新策略。

5. **nominal union branch 的 discriminator pattern**

   当前规范定义了 anonymous discriminator record branch 的结构 narrowing，也定义了 nominal pattern `TypeName {}`，但没有明确“带 literal discriminator 字段的 nominal record branch”是否可以被结构 pattern 选中。为避免歧义，示例服务使用 `ApiError { ... }` 这类 nominal pattern；后续若希望支持 `{ tag: "error" }` 匹配 nominal branch，需要补充优先级、歧义和不可达分支规则。

6. **状态层和存储边界**


7. **数据 migration**

   Publication reference 已定义 protocol identity、ingress entry identity、dependency lock 和 revision retire 的边界，但当前不定义持久化数据 schema migration。后续需要单独设计 Mongo / Redis / future storage 的 schema evolution、backfill、dual-write、read repair 和回滚规则。

8. **snapshot / read view**


## 建议处理顺序

1. 先细化宿主互操作 / FFI，避免所有第三方 SDK 都必须进入核心平台；普通用户插件开放前必须先完成 ABI、沙箱、权限和崩溃隔离设计。
2. 然后设计用户级 async task、cron、startup 和 managed worker surface。
3. 再补观测生产化扩展，支撑长期运行、告警和聚合。
4. 最后细化状态层、数据 migration 和未来 read view，避免过早引入语言级共享状态。
