# P5-F333 Wire and observability delta audit

状态：Ready（只读审计）。

## 直接父节点

- original W2-W owner/wire audit：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- current runtime fixed-channel convergence：
  `P5-F331-service-error-channel-convergence-probe-result.md`
- current pre-W2-W owner map：
  `P5-F319-service-error-channel-delta-audit-result.md`

引用链已连接唯一权威设计。本任务把F280旧代码事实更新为R0–R4合流后的当前request/host/transport/router/
telemetry真实跳点和可并行实现DAG；不做A5 verdict，不实现代码。

## 候选与只读范围

- 审计HEAD：worktree创建时integration HEAD，result记录commit/tree。
- 只读：
  - `runtime/request-contract/**`
  - `runtime/request/**`
  - `runtime/transport/**`
  - `runtime/host/**`
  - `router/**`
  - `telemetry/**`
  - 仅为接缝读取`runtime/eval` fixed carrier与`runtime/capability-context` typed response
- 唯一写入`P5-F333-wire-observability-delta-audit-result.md`并提交。
- 不运行cargo/pnpm/完整测试/stable/live，不push、不承接实现。

## 必须回答

1. Eval/Host如何把`FixedServiceFailure(OpaqueServiceError)`转换成runtime response；目前在哪一步仍压平成
   `RuntimeErrorPayload`、`ResponseError`、`UnhandledServiceError`或generic JSON。
2. request-contract、transport Rust frame、router TS frame/validator与WebSocket/HTTP gateway当前
   `response.error`真实形状、版本owner、encode/decode入口和unknown/extra/payload presence校验。
3. strict response.error v2的最小canonical Rust owner与TS parity边界；encoded payload放置布局如何复用
   既定`ServiceErrorEnvelope`而不引入第二分类器。
4. pre-ingress/control error与service response error如何在type/frame上明确区分；哪些generic DTO必须保留，
   哪些legacy路径必须删除。
5. host request supervisor、request runner、transport session与router各自需要的最小consumer；列出上游失败
   遮挡关系和central ownership，禁止host/router按message/code重分类。
6. restricted local diagnostic event如何与external response分离：current trace/request/span owner、需要新增的
   top-level errorId、完整local stack owner及redaction边界。
7. telemetry Rust DTO、router TS protocol、validator、storage/forwarding需要哪些一致字段；哪些事件是受限内部
   诊断，哪些可以到外部。
8. HTTP/WebSocket external response当前脱敏策略和fixed service error映射；wire不得含callee path/function/
   stack/private payload。
9. 给出最少串行checkpoint与最大安全并行consumer，精确production/test写入范围、blocked-by、最小正负探针、
   证据失效边界；不得把同一frame/telemetry DTO让多个Agent并行修改。
10. 是否有真正需要用户决定的公共布局/协议问题。F280已说明fixed envelope物理放置属于实现布局；若当前代码
    证明存在额外选择，给最小选项，不自行决定。

## 搜索与result格式

至少反搜并归类：

```bash
rg -n 'RuntimeErrorPayload|ResponseError|UnhandledServiceError|response\\.error|FixedServiceFailure' runtime router telemetry
rg -n 'TelemetryEvent|errorId|traceId|sourceFrames|diagnostic|stack' runtime/host runtime/request runtime/transport router telemetry
rg -n 'ProviderUnavailable|ProtocolError|InternalError' runtime/host runtime/request runtime/transport router
```

result必须包含：

- production跳点表；
- canonical/duplicate/legacy owner清单；
- strict wire和restricted telemetry目标接线；
- 建议DAG及互斥写入范围；
- W1正负最小探针和外部泄露反搜；
- 设计缺口（没有则明确“无新增用户决策”）。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f333-wire-audit`
- branch：`codex/p5-f333-wire-audit`
- 新的一次性只读Agent，5分钟内开始写result；
- 提交并返回commit、审计HEAD/tree、关键跳点、建议DAG和设计缺口；
- 不push、不承接实现。

