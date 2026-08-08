# Skiff 剩余问题

日期：2026-07-26

本文只记录当前 canonical 尚未规范化的问题。已解决的旧审阅项不再放在这里；历史交叉审阅记录只用于追溯设计背景。

Package / Service 的HTTP gateway identity、external schema owner、`api.yml`存在性、service-call显式选择、
nominal public path和service dependency cycle已经在
[`package-service-contract-deployment.md`](package-service-contract-deployment.md)收敛，不再作为待决项。

## 当前仍待规范化的问题

1. **用户级异步任务和后台流程**

   `dispatch`已定义唯一detached-call surface、`TaskRef`、status与before-start cancel；内部
   TaskStore/lease/timer契约归`durable-task-dispatch.md`。用户可见的结果型async task、cron、
   startup / managed worker 仍未规范化。后续版本还需定义这些surface的任务声明、结果保留、
   重试、观测、启动恢复、后台worker和业务错误记录方式。

2. **宿主互操作 / FFI**

   当前native/platform surface由内建实现提供，还没有用户可扩展的FFI ABI、类型映射、权限声明、沙箱、
   版本兼容、崩溃隔离或部署模型。在这些边界明确前，不应把任意Rust/JavaScript动态加载或SDK handle暴露为
   普通Package能力。

3. **观测生产化扩展**

   Observability capability 已定义日志、trace、指标、health、topic、归属、查询和 best-effort 交付承诺。后续仍需细化 OpenTelemetry 兼容映射、错误聚合算法、告警规则、长期采样策略和具体存储后端的生产 schema。

4. **stream / client session 外部协议细节**

   当前规范已经明确内部 transport 支持 unary / server-stream、`requestId` 配对、chunk / end / error / cancel、最小 backpressure 保证，以及 `std.client` 的最小 API。仍需细化的是具体 wire encoding、chunk size、WebSocket ack / error outbound frame、SSE event 映射、client capability discovery、离线重放、reconnect resume 和 actor binding 过期 / 刷新策略。

5. **nominal union branch 的 discriminator pattern**

   当前规范定义了 anonymous discriminator record branch 的结构 narrowing，也定义了 nominal pattern `TypeName {}`，但没有明确“带 literal discriminator 字段的 nominal record branch”是否可以被结构 pattern 选中。为避免歧义，示例服务使用 `ApiError { ... }` 这类 nominal pattern；后续若希望支持 `{ tag: "error" }` 匹配 nominal branch，需要补充优先级、歧义和不可达分支规则。

6. **状态层和存储边界**

   DB object、queue、actor和外部resource已经分别有owner；service DB固定由operator选择的受信Mongo
   endpoint/storage domain、profile与serviceId共同定界，不引入platformId，也不再存在
   developer-authored state namespace。仍待设计的是跨对象一致性、跨service transaction、缓存一致性和
   事件/outbox组合。Redis、queue或第三方存储若需要新配置，必须定义独立capability，不能恢复通用
   `state` binding。

7. **数据 migration**

   Package/Service contract architecture 已定义 protocol identity、gateway entry identity、dependency
   binding 和 revision retire 的边界，但当前不定义持久化数据 schema migration。尚未实现的设计输入见
   `durable-schema-evolution.md`；进入实现前仍需收敛 Mongo / Redis / future storage 的 schema evolution、
   backfill、dual-write、read repair 和回滚规则。

8. **snapshot / read view**

   当前没有定义跨多个DB object、外部resource或长stream的一致read snapshot。若未来引入read view，需要
   明确isolation、lifetime、分页/stream语义、重试、跨request可恢复性和与写transaction的关系，不能把当前
   request heap或driver session handle直接暴露为持久值。

9. **WebSocket业务notification与binary RPC**

   第一版已经支持双向JSON-RPC unary request：Skiff使用
   `std.websocket.requestJsonToConnection`向peer发起；peer只能调用`websocket.yml.jsonRpc`显式声明的
   method。Raw frame `receive`、任意event-name dispatcher与业务可见transport id仍不存在。
   第一版不声明任何notification handler，也不支持peer request cancellation或binary RPC。未来若出现
   真实需求，必须分别定义notification的admission/错误/背压语义、取消请求的业务与副作用边界，或binary
   profile的版本、framing、codec与协商；不能把普通text/binary frame自动变成业务入口。

## 建议处理顺序

1. 先完成已冻结的编码无关broker、`jsonrpc-2.0-text`双向request与typed method dispatch，同时保持raw
   receive、业务notification和binary RPC关闭。
2. 再细化宿主互操作 / FFI，避免所有第三方 SDK 都必须进入核心平台；普通用户插件开放前必须先完成 ABI、沙箱、权限和崩溃隔离设计。
3. 然后设计用户级 async task、cron、startup 和 managed worker surface。
4. 再补观测生产化扩展，支撑长期运行、告警和聚合。
5. 最后细化状态层、数据 migration 和未来 read view，避免过早引入语言级共享状态。
