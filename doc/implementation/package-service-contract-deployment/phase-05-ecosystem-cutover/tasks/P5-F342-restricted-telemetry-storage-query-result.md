# P5-F342 Restricted telemetry storage and query consumer result

状态：`PASS`（F342 写入边界内实现与验证完成；未修改 task 状态，未 push，未承接后续节点）。

## 候选与写入边界

- worktree：`/Users/geek/workspace/skiff-p5-f342-restricted-telemetry`
- branch：`codex/p5-f342-restricted-telemetry`
- task production 起点：`e3095ec642d49b59955f5f48a2950eafc9d92571`
- task production 起点 tree：`6b7fce6db07d7fde3b88609539150c53f5608e62`
- worktree 起始 HEAD：`5fa0389e151a27f9cbd2906a7394658190e42491`
- worktree 起始 tree：`b344fea27ed6da25ce899713d9b2397651edc93d`

`e3095ec6..5fa0389e`只新增 F340/F341/F342 三个 task 文档，telemetry 与 Router production tree
没有变化。本任务 production 只修改：

- `telemetry/src/{mongoStore.ts,queryApi.ts,redaction.ts}`
- `router/src/router/httpGateway.ts`

测试只修改：

- `telemetry/tests/{store.test.ts,server.test.ts,redaction.test.ts,queryApi.test.ts}`
- `router/tests/http-telemetry.test.ts`

另新增本 result。`telemetry/src/server.ts`继续复用 C0 strict admission与 store query；
`router/src/telemetry/producer.ts`本身已透明 queue/serialize整个 event，因此只补 forward probe而不制造字段
复制逻辑。没有修改 frozen `telemetry/src/protocol.ts`、Router protocol/runtime/gateway禁区、Rust、
shared corpus、package/lockfile、父 task/result或权威设计。

## Fail-closed ordinary query 与 top-level correlation

三个 store ordinary query都在其直接 Mongo filter与内存共享 filter中固定
`visibility: 'operational'`：

| surface | 最底层 filter |
| --- | --- |
| `queryLogs` | `visibility=operational`、`topic=log`，再叠加普通 selector |
| `queryTrace` | `visibility=operational`、exact `traceId`和可选 exact `errorId` |
| `queryTraces` | `visibility=operational`、`traceId exists`，再叠加普通 selector |

HTTP `/logs`、`/traces`、`/traces/:id`只调用这三个 store surface，没有 HTTP-only visibility
过滤或 restricted fallback。三条 route都由混存 probe证明不会返回 restricted；没有新增 restricted HTTP
route，`/restricted-diagnostics`仍为404。

`LogQuery`、`TraceQuery`与`queryTrace`增加可选 top-level `errorId`。query filter只读取 document
top-level `errorId`，不会读取`event.error.errorId`、attrs或 message；nested-only反例返回空。HTTP
`errorId`保持 exact值，空白值在`/logs`、`/traces`与`/traces/:id`进入 store前返回400，不会退化成
无 selector扫描。

## Store-only restricted reader

`TelemetryStore.queryRestrictedDiagnostics`在 Mongo与内存实现中共享同一 filter builder：

- 永远固定`visibility: 'restricted'`；
- 必须提供非空`traceId`或`errorId`，缺失、空值或纯空白值直接失败；
- 提供一个 selector时 exact匹配该 top-level correlation；同时提供两个时取交集；
- 固定最多返回1000条并按稳定时间顺序排序，不存在默认“全部 restricted”扫描；
- API只存在于未对 package export或 HTTP route暴露的 store module。

混存测试用同一 traceId/errorId写入 operational与restricted log/trace，分别按 traceId和errorId读取，
证明内部 reader只返回 restricted且普通三个 store query只返回 operational。

## Storage redaction 与 indexes

batch insert继续先对每个 event执行 non-mutating storage redaction，再展开 document metadata。
`visibility`、top-level `traceId/errorId`由 event spread原样保留，不推断、不重建，也不把 restricted
改写成 operational。

二次 redaction覆盖：

- snake/camel形式的 password、secret、token、authorization、cookie、key和 Mongo URL key；
- secret/bearer/basic/credential URL等敏感字符串值，包括 top-level free-form message；
- 默认深度12、字符串4096字符、数组50项、对象100 key的边界；
- attrs/error/dropped对象的递归 copy，不突变输入。

restricted probe保留 source object、local stack frame与 remote-boundary frame结构，同时证明
`provider-private-secret`、credential key/value不可见，过长字符串、数组、对象与深层值有界，
top-level correlation与`visibility: restricted`不变。

保留全部既有 index并新增稳定命名的：

- `visibility_topic_ts_desc`：普通 log默认路径；
- `visibility_trace_ts_asc`：ordinary/restricted trace相关路径；
- `visibility_error_ts_asc`：ordinary/restricted error相关路径。

既有`batch_dedupe`与`ttl_receivedAt`及 service/request/target/level/provider索引均未移除。测试直接断言
Mongo filter结构、完整 index名字、compound keys与 TTL秒数；内存实现消费相同 filter builder，避免两套
visibility/errorId语义漂移。

## Router telemetry

Router HTTP telemetry literal现在显式写`visibility: 'operational'`，200、404和client-disconnect三个既有
producer probe均断言该字段。Router producer无需 production修改：它把同一个`TelemetryEvent`对象加入
queue并直接放进 batch。新增本地 WebSocket probe完成 register/queue/flush/JSON forward全链，断言
restricted event的`visibility`与 top-level `errorId`连同整个 event原样到达 batch。

## Selector 与验证

先枚举且确认非零：

```text
pnpm --filter @skiff/telemetry exec vitest list \
  tests/store.test.ts tests/server.test.ts tests/redaction.test.ts tests/queryApi.test.ts
  10 selectors

pnpm --filter @skiff/router exec vitest list tests/http-telemetry.test.ts
  4 selectors
```

本任务核心 selector：

```text
in-memory telemetry store >
  isolates ordinary queries and exposes restricted diagnostics only by correlation
in-memory telemetry store >
  forces visibility in Mongo filters and declares correlation indexes
telemetry server >
  keeps every public query route operational and filters only by top-level errorId
telemetry redaction >
  preserves restricted stack structure while bounding and redacting diagnostic data
router HTTP telemetry >
  forwards telemetry visibility and top-level errorId without rewriting the event
```

最终验证：

```text
pnpm --filter @skiff/telemetry test
  6 files passed
  18 tests passed

pnpm --filter @skiff/telemetry run type-check
  PASS

pnpm --filter @skiff/router exec vitest run tests/http-telemetry.test.ts
  1 file passed
  4 tests passed

git diff --check
  PASS
```

worktree没有自有依赖；selector枚举与验证期间临时链接主 Skiff worktree现成的
`telemetry/node_modules`和`router/node_modules`，验证后已删除，未进入 diff。按任务约束没有运行完整
workspace/root、Router完整 type-check、stable或 live。

## 自验收与反搜

| 条款 | 代码/测试证据 | 结论 |
| --- | --- | --- |
| 三个 ordinary store query最底层 operational | 三个 `buildOperational*Filter`同时被 Mongo与内存实现消费；混存 store probe | PASS |
| 三个 public route排除 restricted | server真实 HTTP混存 probe逐条检查`/logs`、`/traces`、`/traces/:id` | PASS |
| top-level errorId且空值 fail closed | common/path query parser、Mongo filter断言、nested-only负例、400负例 | PASS |
| restricted reader store-only且强制 selector | 双 store实现、restricted filter、缺失/空白 selector负例、404 route负例 | PASS |
| Mongo/index与内存 parity | 共享 filter builder、compound index结构断言、内存行为 probe | PASS |
| storage redaction保留结构且不泄密/突变 | restricted source/stack/remote-boundary、sentinel、size与原输入断言 | PASS |
| Router forward与HTTP默认 visibility | producer真实 WebSocket batch probe、三个 HTTP telemetry probe | PASS |
| 写入边界 | `git diff --name-only`无 frozen protocol、Rust、Router禁区、package/lockfile命中 | PASS |

额外反搜：

- telemetry/Router production中`error.errorId`、`attrs.errorId`或 message→errorId推断命中为零；
- production没有`restricted-diagnostics` route；
- `queryRestrictedDiagnostics`只命中 store interface及 Mongo/内存实现；
- task状态、父 task/result与权威设计无 diff。

## Blocker 与剩余边界

Blocking issues：无。

本任务没有运行 Mongo live实例；Mongo证据按任务要求由实际 production filter/index结构与共享内存行为
parity提供。Router完整 type-check、H/R/T合流后的跨层 S1 probe与阶段级昂贵 gate仍分别属于 R consumer、
C1及阶段 gate owner，不是 F342 blocker。
