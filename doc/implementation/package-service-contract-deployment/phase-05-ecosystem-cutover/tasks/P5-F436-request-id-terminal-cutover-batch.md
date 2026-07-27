# P5-F436 Agine legacy requestId terminal cutover batch

状态：Ready。当前 WebSocket/HTTP 终态收敛批次父节点。

## 直接父节点与权威语义

- `P5-F427A-agine-http-correlation-owner-audit-result.md`
- `P5-F430B-agine-browser-http-direct-body-result.md`
- `P5-F429B-router-downlink-only-websocket-gateway-result.md`
- 唯一权威设计
  `doc/architecture/package-service-contract-deployment.md` 第 3、6.4 节

权威设计已经冻结：

- 第一版 WebSocket 只有 connect 与服务端下行，不存在 receive、业务消息 handler 或 WS RPC；
- 浏览器与 Host 的业务上行都走 HTTP；
- HTTP transport 自己关联 request 与 response，业务 wire 不携带旧 WS req/res
  `requestId` 或同义 correlation 字段；
- 真正的幂等键、异步任务句柄和 run identity 必须分别建模，不能借用 `requestId`。

因此 F427A 在旧 receive 尚存期间要求保留 legacy WS `requestId` 的过渡结论，不是当前终态；
本批次关闭该过渡面。Skiff Router↔Runtime/control/actor 内部协议的 correlation identity、
第三方上游协议字段，以及 `runId`、`toolCallId`、`attemptId`、`chatId` 等业务身份不在本批次
删除范围。

## 当前输入与已知事实

| repo | commit | tree |
| --- | --- | --- |
| Skiff integration | `6276ddbea46184ccc4251aa3173ab411f38ac28a` | dispatch 时记录 |
| Internals integration | `58950858a2e2cbf2bd95443d5e0704d0d29e7706` | `db88355a103e6e1939e9969756501c7f656c1344` |

已知 production residue 至少包括：

- Agine service `agine_ws_dispatch`、`agine_ws_*` 与 `agine_transport.WebSocketRequest`；
- Agine `service.yml` 的 legacy `websocket.routes[].operation` authoring；
- browser/shared-client 的 pending request map、legacy error matching 与 WS RPC helper；
- Host `GatewayClient`/`HostRuntime` 的双向业务 frame，以及 Host file/current-directory
  `requestId`；
- service 侧 Host file pending/two-hop workflow。

HTTP direct-body migration 已完成；不得让 legacy DTO 或 envelope 回流 HTTP。

## 执行 DAG

```text
F436B exact owner/path audit
  ├─> Agine service connect-only/downlink cleanup
  ├─> browser/shared-client WS-RPC residue cleanup
  └─> Host business HTTP uplink + async identity closure
          ↓
      current authoring/source/type/isolated combined
```

F436B 先冻结完整 owner、下行事件保留面、Host 两跳语义和最小任务拆分。若 Host HTTP 身份认证、
异步 job lifecycle 或 endpoint ownership 仍缺少会改变公共契约的设计决定，必须停止并上报；
不能在 leaf 内发明 nonce、credential、polling 或兼容 dual path。

## 候选与证据边界

当前是实现检查点。F436B 是只读路径审计，不产生 candidate PASS。后继 production owners 全部
合流且便宜 combined probe 通过前，不得冻结稳定候选或运行完整 gate。
