# P5-F346 Service error wire / observability independent acceptance

状态：Completed（W2-W / C1 / S1 高风险边界独立验收 PASS）。

## 直接父节点

- wire、gateway、telemetry 缺口及 owner：
  `P5-F333-wire-observability-delta-audit-result.md`
- restricted diagnostic handoff 验收：
  `P5-F335-restricted-service-diagnostic-acceptance-result.md`
- C0 shared wire/schema 独立复验：
  `P5-F339-response-error-schema-reacceptance-result.md`
- 三个 production consumer：
  - `P5-F340-service-error-host-consumer-result.md`
  - `P5-F341-service-error-router-consumer-result.md`
  - `P5-F342-restricted-telemetry-storage-query-result.md`
- test fixture closure：
  - `P5-F343-host-error-model-test-fixture-closure-result.md`
  - `P5-F344-router-bootstrap-test-determinism-result.md`
- C1 合流证据：
  `P5-F345-service-error-cross-layer-convergence-result.md`

以上父节点沿引用链连接唯一权威设计。本任务不重新解释错误语义。

## 候选与角色

- 候选 branch：`codex/package-service-phase-05`
- 候选必须包含 F345 merge commit；验收开始时记录 exact commit/tree。
- 角色：新的独立验收 Agent；不得复用 F333–F345 的开发或验收会话。
- 风险：高。该验收冻结 service error 的 wire / gateway / observability 边界，但不代表整个
  Phase 05 完成。

文档状态提交、验收 task/result 等不触及下述生产表面的改动可以发生；若候选的
`runtime/{model,boundary,eval,capability-context,request,request-contract,transport,host}`、
`router/src`、`telemetry/src`或共享 corpus/schema 发生任何改动，当前 verdict 失效。

## 只读验收范围

### 1. 固定错误语义

- 任意用户名义类型仍可在 request 内抛出；operation signature、Package ABI、ServiceContract
  不包含 throws set。
- 公开、可命名、`SchemaClosed`且编码成功的错误保留其实际 Package owner/type/payload；
  dependency package 声明的类型不得改写为 service owner。
- 私有、不可命名、非 closed 或编码失败的错误只在第一次跨 service 时转换为可序列化、可捕获的
  `std.service.InternalError`；不得泄露原 identity、字段、显示字符串或完整 callee stack。
- A→B→C 未捕获传播保持同一个固定错误 payload 和同一 `traceId/errorId`；每个 caller 创建新的
  request-local exception stack，并只附加脱敏 remote-boundary frame。

### 2. wire / Host / Router

- service `response.error` 只有严格 v2 fixed/control union；不存在 v1 reader、writer、fallback或按
  code/message 推断 fixed。
- fixed payload byte-for-byte 转发；Host、Router不得 stringify/re-encode，不得进入 generic
  `response_error_to_telemetry_map`、`RuntimeResponseError`或`runtimeErrorStatus`路径。
- matching generic control 仍保持 generic，payload presence、schema oneOf及 malformed 输入 fail closed。
- HTTP 与 WebSocket 实际 gateway 只返回稳定脱敏信息和 correlation；不得包含 provider payload、
  source/path/function/frames/stack。

### 3. observability

- operational事件只含有限 kind/cause、预算和 top-level correlation。
- 每 hop 完整本地 stack只进入 restricted telemetry，并按同一 correlation 关联；sink失败不得改变
  fixed response。
- `queryLogs/queryTrace/queryTraces`及公开 `/logs`、`/traces`、`/traces/:id`只能读取
  operational；restricted没有公开 route。
- store-only restricted reader必须带 traceId或errorId，返回经存储层二次脱敏的结构化诊断；
  Mongo filter/index与 in-memory语义一致。

### 4. C1 证据诚实性

- 新测试只消费 production schema/codec/carrier/dispatcher/gateway/store，不在测试中重新实现一个平行
  classifier、wire codec或查询策略。
- C0 corpus仍是唯一 service error wire bytes owner；C1 fixture只拥有场景事实。
- 手工构造 typed diagnostic只能证明 Host projection；必须与真实 eval multi-hop及顶层
  ContractOperation selectors组合，不能冒充单进程 Rust→Node live链。

## 独立抽查

先枚举 selector，确认非零。可选择最小但必须覆盖下列真实入口：

```bash
cargo test -p skiff-runtime-eval \
  restricted_service_diagnostic_ordinary_three_hop_preserves_bytes_and_local_stacks
cargo test -p skiff-runtime-host --test p5_f345_service_error_convergence
pnpm --filter @skiff/router exec vitest run \
  tests/service-error-cross-layer-convergence.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/assembly-websocket-gateway.test.ts
pnpm --filter @skiff/telemetry test
```

还必须做结构反搜并人工确认合法/非法命中，不得只报告命中数量。验收不运行 workspace/root/stable/live、
Mongo live，不安装或更新依赖，不修改 production/test/corpus/lockfile。

## 写入与交付

只允许新增：

- `P5-F346-service-error-wire-observability-acceptance-result.md`

result必须记录 exact candidate commit/tree、抽查命令和非零 selector、每项结论、合法残余命中、证据边界及
`PASS`或`FAIL`。发现 blocker时只报告，不修复。提交 result，返回 commit；不得修改本 task 状态，不 push。
