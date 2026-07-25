# P5-F342 Restricted telemetry storage and query consumer

状态：Ready。

## 直接父节点

- 当前 telemetry 跳点、T owner与 S1 probes：
  `P5-F333-wire-observability-delta-audit-result.md`
- 已冻结的 typed restricted diagnostic handoff：
  `P5-F335-restricted-service-diagnostic-acceptance-result.md`
- 已冻结并通过复验的 shared telemetry DTO/protocol checkpoint：
  `P5-F339-response-error-schema-reacceptance-result.md`

父节点已沿引用链连接唯一权威设计。本任务只实现 T：strict admission之后的存储、二次脱敏与查询隔离；
不改 shared telemetry protocol、Rust host、Router runtime/gateway error路径。

## 起点与目标

- 起点 commit：`e3095ec642d49b59955f5f48a2950eafc9d92571`
- 起点 tree：`6b7fce6db07d7fde3b88609539150c53f5608e62`
- `visibility`与 top-level `errorId`按 C0原样存储；不得从 nested error/message推断或重建。
- 所有普通查询必须 fail closed只返回`visibility=operational`：
  - store `queryLogs`；
  - store `queryTrace`；
  - store `queryTraces`；
  - HTTP `/logs`、`/traces`、`/traces/:id`。
- 普通 query支持可选 top-level `errorId`过滤，使 operational事件可用 traceId/errorId关联。
- restricted诊断不增加公开 HTTP route。新增一个明确的 store-only内部读取 API，例如
  `queryRestrictedDiagnostics`；调用必须提供非空 traceId或errorId，且该 API只返回
  `visibility=restricted`。
- Mongo与内存实现语义相同；Mongo索引必须覆盖 visibility、traceId与errorId的查询路径。
- storage redaction是 defense in depth：敏感 key/值、深度、字符串、数组和对象大小受限，同时保留
  restricted source/local-stack/remote-boundary结构和 top-level correlation。不得把 restricted event
  转换为 operational。
- Router telemetry queue/forward必须透明保留 visibility/errorId；现有 Router HTTP telemetry producer
  明确写`visibility:'operational'`。

## Production 写入边界

允许修改：

- `telemetry/src/{server.ts,mongoStore.ts,redaction.ts,queryApi.ts}`；
- `router/src/telemetry/producer.ts`仅在确认 queue/forward需显式处理新字段时；
- `router/src/router/httpGateway.ts`仅可给既有 telemetry event literal补
  `visibility:'operational'`及直接相关 top-level errorId透传；不得修改路由、HTTP错误或gateway行为。

明确禁止修改：

- `telemetry/src/protocol.ts`；
- `router/src/protocol/**`；
- `router/src/router/{runtimeEndpoint.ts,runtimeDispatcher.ts,errors.ts,assemblyHttpGateway.ts}`；
- `router/src/gateway/assemblyWebSocketGateway.ts`；
- Rust、shared corpus、权威设计、父 task/result、package/lockfile。

## 必须实现并证明

1. Mongo filter和内存 filter都在最底层强制 operational；即使 query API调用方漏过滤也不会返回
   restricted。
2. store-only restricted reader在 Mongo/内存中都强制 restricted且要求 correlation selector；没有公开
   route或默认“全部 restricted”扫描。
3. query `errorId`参数只读 top-level字段，空值/非法值按既有 query输入规则 fail closed。
4. indexes有稳定名字并覆盖 batch dedupe、TTL以及 visibility+trace/error correlation；不移除既有必要
   查询索引。
5. batch insert保留 visibility/errorId，redaction不突变输入；restricted stack数组与 source对象仍可读，
   secret sentinel不可见，过大值被有界截断。
6. Router producer forward测试证明同一 event的 visibility/errorId不丢失；HTTP producer默认
   operational。

## 测试与验证

允许测试写入：

- `telemetry/tests/{server.test.ts,store.test.ts,redaction.test.ts,queryApi.test.ts}`；
- telemetry其它现有 test文件仅用于补必填 visibility fixture；
- `router/tests/http-telemetry.test.ts`。

至少覆盖：

- operational/restricted同 traceId/errorId混存；
-三个普通 store query和三个 public HTTP route都排除 restricted；
- ordinary errorId filter；
-内部 restricted reader按 traceId与errorId读取且不返回 operational；
- Mongo filter/index结构与内存行为 parity；
- storage二次 redaction保留 stack结构但遮罩 secret；
- Router forward保留 visibility/errorId。

先列出非零 selector，再运行：

```bash
pnpm --filter @skiff/telemetry test
pnpm --filter @skiff/telemetry run type-check
pnpm --filter @skiff/router exec vitest run tests/http-telemetry.test.ts
git diff --check
```

Router完整 type-check由 R consumer另行收敛，本任务不越界修 R。不得运行完整 workspace/root、
stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f342-restricted-telemetry`
- branch：`codex/p5-f342-restricted-telemetry`
- 新的一次性开发 Agent；
- 新增`P5-F342-restricted-telemetry-storage-query-result.md`，列出 fail-closed filter、内部读取 API、
  redaction/index、selector/数量与剩余 blocker；
- 提交并返回 implementation commit，不修改 task 状态，不承接后续验收。
