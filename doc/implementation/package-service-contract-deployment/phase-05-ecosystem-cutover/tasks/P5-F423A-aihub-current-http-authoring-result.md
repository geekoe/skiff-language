# P5-F423A AIHub current HTTP authoring result

状态：`PASS`。AIHub 的七条 HTTP ingress 已迁移到 current named gateway entry authoring，
non-production receipt oracle 已同步到 service protocol v5；WebSocket block 原样保留，因此
fresh publish 仍由后继 WebSocket owner 处理。

## 1. Exact start 与 commits

| 锚点 | commit | tree |
| --- | --- | --- |
| Skiff result base | `5486f7ed4a7c23ea2b874871b12ff23155c1108a` | `31895c173a781222ac7d9995a79918f20235b2d4` |
| Skiff task checkout | `a24309d11ffbb858d7547c9270dc7dab39123fe1` | `5206c2e61469cfb37e13ccafc33cc169b066f3f5` |
| Internals exact start | `baf0c907ee26e48a5fb4c153825c233bde3a6234` | `13f2f6e604fedbad80e0390e5408507430e28f8c` |
| Internals implementation | `4ce6b3c3125bab33d6d42cfc91ad55359092d210` | `9fb7fc5acd474f0df4174b89e6ec13b13cc29e60` |

Internals 启动时 HEAD/tree 精确匹配任务起点。Skiff task checkout 以记录的 result base 为 ancestor，
相对起点只新增 F423 batch 与两个 leaf task 文档，production diff 为 0。result-only commit/tree
由交付消息记录。

## 2. HTTP authoring 与 receipt oracle

`aihub/service/service.yml` 的旧 HTTP `routes` sequence 已精确替换为七个 named entries：

| key | selector |
| --- | --- |
| `healthGet` | `GET /health` |
| `v1ProvidersGet` | `GET /v1/providers` |
| `v1ModelsGet` | `GET /v1/models` |
| `v1ChatEventsPost` | `POST /v1/chat/events` |
| `chatEventsPost` | `POST /chat/events` |
| `v1ChatCompletionsPost` | `POST /v1/chat/completions` |
| `chatCompletionsPost` | `POST /chat/completions` |

每条 entry 都精确使用 `kind: rawHttp`、
`handler: internal.aihub_service.handleAihubHttp`，以及唯一的
`request <- http.request` adapter argument。source test 对完整 HTTP section 做 exact equality，
并断言 entry 数为 7、HTTP section 内 `routes` / `operation` 为 0。

receipt validator 的 positive protocol prefix 与 synthetic fixture 从 v4 同步到 v5。五个
service-call operation、三个 package-only helper、`skiff-contract-operation-v1` 断言及其
promotion/missing/interface negative cases 均未改变。

## 3. WebSocket 不变量

相对 Internals exact start 对 `service.yml` 从 `websocket:` 到 EOF 做逐字 diff，结果为空；该后缀
SHA-256 前后均为：

```text
a8640c0e5be799cbf5f1c815c23ab30b371ca62b911423ab64f8842a68984a29
```

因此 `websocket.routes[/ws].operation: websocket` 与 `timeout.default: 120000` byte-equivalent
保留。没有删除、改写或绕过 WebSocket block，也没有执行 fresh publish；current compiler 对该
WebSocket authoring 的后继阻断不属于本 leaf 失败。

## 4. 聚焦验证

| 检查 | discovered | pass | fail | skip |
| --- | ---: | ---: | ---: | ---: |
| `node --test aihub/service/service-api-receipt.test.mjs` | 7 | 6 | 0 | 1 |

唯一 skip 是未提供 `SKIFF_SERVICE_API_RECEIPT` 时既有的 expected generated-receipt skip。

| 其它命令 | 结果 |
| --- | --- |
| `node --check aihub/service/service-api-receipt.mjs` | PASS |
| HTTP section `routes:` / `operation:` 反向搜索 | 0 命中 |
| `operation: handleAihubHttp` / `skiff-service-protocol-v4` 反向搜索 | 0 命中 |
| `git diff --check` | PASS |

任务给出的组合 `rg` 仍精确命中两个获准的 WebSocket `routes:` 文本：manifest 中保留的 block，
以及 source test 对该 block 的不变量断言；两处都不是旧 HTTP authoring。旧 HTTP operation 与
protocol v4 命中均为 0。

## 5. 写入边界

Internals implementation 只修改：

```text
aihub/service/service.yml
aihub/service/service-api-receipt.test.mjs
aihub/service/service-api-receipt.mjs
```

没有修改 AIHub `.skiff` source、`api.yml`、`package.yml`、config、其它 tests、Agine 或 Skiff
production/test。Skiff result 只新增本文档。没有访问 stable/live，没有启动 instance/watch，
没有派子 Agent，也没有执行 merge、rebase 或 push。
