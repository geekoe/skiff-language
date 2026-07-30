# P5-F423B Agine current HTTP authoring and zero service-call API result

状态：`PASS`。Agine 的 14 条 HTTP ingress 已精确迁移为 current named raw HTTP entries；
Service API receipt 收敛到 protocol v5，并保持 2 个 package helper、0 个 service operation。
WebSocket block 与 `timeout.default: 120000` 原样保留。

## 1. Exact start 与 commits

| 锚点 | commit | tree |
| --- | --- | --- |
| Internals exact start | `baf0c907ee26e48a5fb4c153825c233bde3a6234` | `13f2f6e604fedbad80e0390e5408507430e28f8c` |
| Internals implementation | `dca5ac7ce94b2384afa2cb36d2093123e359d05f` | `f6cd25e4a899c8a4a14025e9f19027f7ab8c6cba` |
| Skiff result base | `5486f7ed4a7c23ea2b874871b12ff23155c1108a` | `31895c173a781222ac7d9995a79918f20235b2d4` |
| Skiff task checkout | `a24309d11ffbb858d7547c9270dc7dab39123fe1` | `5206c2e61469cfb37e13ccafc33cc169b066f3f5` |

两个 exact start 均经 ancestor 检查确认。Internals 启动时精确位于记录起点且 clean；Skiff task
checkout 相对 result base 只新增 F423 batch 与两个 leaf task 文档，production diff 为 0。
result-only commit/tree 由交付消息记录。

## 2. HTTP authoring closure

`agine/service/service.yml` 现在精确包含任务冻结的 14 个 entry key 与 path。所有 entry 均为
`POST`，并具有同一个完整 current binding：

```yaml
kind: rawHttp
handler: internal.agine_service.handleAgineHttp
adapterArgs:
  - param: request
    source: { kind: http.request }
```

`service-api-receipt.test.mjs` 与 `agine_service_architecture.test.mjs` 精确断言：

- 14 个 named entry key 按冻结集合出现，无缺失或额外 entry；
- 14 个 path 各出现一次，typed route table 仍各有一个业务 owner；
- 14 个 handler、`rawHttp` kind 与 `http.request` adapter 完整存在；
- HTTP 子块内旧 `routes`、`operation`、`handlerArgs` 计数均为 0。

WebSocket 没有被删除、改写或绕过。以 `websocket:` 起始直到文件末尾的基线与 implementation
SHA-256 均为
`a8640c0e5be799cbf5f1c815c23ab30b371ca62b911423ab64f8842a68984a29`，因此其 `routes`、
`operation: websocket` 与 `timeout.default: 120000` byte-equivalent 保留。

## 3. Service API oracle closure

`service-api-receipt.mjs` 的 positive protocol identity 已从 v2 收敛为
`skiff-service-protocol-v5:sha256`。package API 闭集合精确为：

```text
handleAgineHttp
websocket
```

两项 status 均必须为 `available`，并且都不得携带 `serviceOperationId`。validator 与 synthetic
fixture 覆盖以下 fail-closed 条件：

- 任一 helper 被提升为 service operation；
- package API projection 缺失；
- package API projection 多出。

`service.yml` 仍没有 `serviceCalls`；HTTP/WebSocket external ingress 没有被写入
ServiceContract operation surface。

## 4. 聚焦验证

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs
node --check agine/service/service-api-receipt.mjs
```

实际结果：

| discovered | executed | pass | fail | skip |
| ---: | ---: | ---: | ---: | ---: |
| 11 | 11 | 11 | 0 | 0 |

| 静态检查 | 结果 |
| --- | --- |
| HTTP 子块 `routes` / `operation` / `handlerArgs` | `0 / 0 / 0` |
| `skiff-service-protocol-v2` | 0 命中 |
| `operation: handleAgineHttp` | 0 命中 |
| WebSocket/timeout tail hash | 基线与 implementation 相同 |
| `git diff --check` | PASS |
| Internals changed-file boundary | 精确 4 个授权文件 |

任务给出的广义 `routes:` 反搜只剩必须保留的 WebSocket production block，以及测试中的
WebSocket 正向断言和 HTTP 负向断言；没有旧 HTTP route sequence 残留。

## 5. 边界

Internals implementation 只修改：

```text
agine/service/service.yml
agine/service/service-api-receipt.test.mjs
agine/service/service-api-receipt.mjs
agine/service/internal/agine_service_architecture.test.mjs
```

没有修改 Agine `.skiff` source、`api.yml`、`package.yml`、config、其它 tests、AIHub 或 Skiff
production/test。没有执行 publish、stable/live、merge、rebase、push，也没有派子 Agent。Skiff
result 只新增本文档。
