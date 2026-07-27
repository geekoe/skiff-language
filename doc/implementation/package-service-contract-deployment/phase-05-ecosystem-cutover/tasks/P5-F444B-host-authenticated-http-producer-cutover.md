# P5-F444B Host authenticated HTTP producer cutover

状态：Ready。Internals H1 implementation leaf。

## 直接父节点

- `P5-F444A-agine-service-terminal-owner-preflight-result.md`

只从该 result 沿引用读取完成本 leaf 必需的 F438B、F440D、F440F 事实。任务文件不重新定义协议；
若发现必须增加第六条 Host upcall、修改 `agine/service/**`、修改 shared-client、引入业务 correlation
或改变 activation/attempt lifecycle，按工作流停止并返回 `TASK_SCOPE_EXPANDED`。

## 输入

| Repo | Root | Expected commit |
| --- | --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` | `f94eb17e` |
| Internals integration | `/Users/geek/workspace/internals-phase-05-integration` | `2320949` |

两个输入必须 clean。

## 完成目标

只在 `agine/protocol/**` 与 `agine/host/**` 完成以下硬切：

1. `@agine/protocol` 成为五条 Host authenticated HTTP upcall 的唯一 path、payload、response owner：
   - `POST /host/hello`
   - `POST /host/activation-ack`
   - `POST /host/ping`
   - `POST /host/tool-attempts`
   - `POST /host/tool-call/result`
2. HTTP payload/response从当前业务字段机械提取；删除 transport-only `eventName`、`requestId`、
   connection id。attempt/tool/run identity是 durable business identity，必须保留。
3. Host 新增窄 HTTP client，从同一个 gateway URL派生 `http:` / `https:` URL，保留 service/version
   selector，并复用当前严格的两种 Host header：
   - activation阶段：`X-Agine-Host-Activation`
   - 已激活阶段：`Authorization: AgineHost <hostId>`
   credential不得进入query或body；两种header不能同时发送。
4. `GatewayClient` 只管理 WebSocket connect/reconnect、物理状态和 F440F Host peer responder；
   不再通过 socket发送 hello、ack、ping、tool-attempt sync或Host tool result。
5. `HostRuntime` 保留现有 lifecycle/timer/ledger语义，但把：
   - connect/reconnect hello；
   - activation ack；
   - presence heartbeat；
   - attempt snapshot sync及actions response；
   - Host tool result及receipt
   改成一次HTTP request/response。HTTP promise是唯一in-flight owner；不得再用
   `toolAttemptSyncOutstanding`或event listener做transport correlation。
6. F440F 的 JSON-RPC responder、captured writer、cancel/deadline/generation状态机保持不变；
   server -> Host 的单向业务 notification仍可走当前 raw event observer。

## 写入边界

允许：

- `agine/protocol/http.ts`
- `agine/protocol/toolCall.ts`
- `agine/protocol/package.json`
- 上述协议的 package-local tests/type-check fixtures
- `agine/host/src/**`
- `agine/host/package.json`（仅测试入口确有需要时）

禁止：

- `agine/service/**`
- `agine/client/**`
- `shared-client/**`
- Skiff production/reference
- lockfile、node_modules、build output

文件已经过长或重复时按 repo 规则提取窄模块，不继续扩大 `GatewayClient.ts` /
`HostRuntime.ts`。

## Test-first 与验收

先修改/新增聚焦测试，使当前输入真实 RED，至少覆盖：

- 五个 canonical path与无transport字段的payload/response；
- ws/wss到http/https转换、service/version保留、Host auth header；
- activation hello取得Host id、持久化后ack；
- reconnect hello、presence、attempt sync single-flight、actions；
- Host result receipt、HTTP非2xx、malformed success、network failure；
- WebSocket production不再发送五种旧event；
- F440F peer adapter和server notification共存不回归。

聚焦命令至少包括：

```bash
npm test --workspace @agine/protocol
npm run type-check --workspace @agine/protocol

cd agine/host
npm exec -- tsx src/GatewayClient.test.ts
npm exec -- tsx src/HostRuntime.test.ts
npm exec -- tsx src/HostToolAttemptRuntime.test.ts
npm exec -- tsx src/protocol/toolCall.test.ts
npm exec -- tsx src/HostPeerAdapter.test.ts
npm exec -- tsx src/CapturedSocketLifecycle.test.ts
npm run test:architecture
npm run type-check
```

若聚焦验证全部通过，再运行 `npm test --workspace @agine/host`。不得运行 Internals service build、
canonical service graph、browser、stable、live或network。

反向搜索必须证明 production 中不再构造/发送：

```text
eventName: "host/hello"
eventName: "host/activation-ack"
eventName: "host/ping"
eventName: "host/tool-attempts"
eventName: "tool_call/result"
```

允许 protocol/test 中的 retired negative，允许 server -> Host notification类型；
不允许为了搜索归零改名保留旧wire。

## 提交与结果

Internals implementation worktree：

`/Users/geek/workspace/internals-p5-f444b-host-http-upcall`

branch：

`codex/p5-f444b-host-http-upcall`

提交一个聚焦 implementation commit，最终 clean。

Skiff result worktree：

`/Users/geek/workspace/skiff-p5-f444b-host-http-upcall-result`

branch：

`codex/p5-f444b-host-http-upcall-result`

只新增并提交：

`P5-F444B-host-authenticated-http-producer-cutover-result.md`

result记录RED、精确写集、验证计数、反向搜索、两个commit/tree/status。不得 merge、rebase、push；
不得派子 Agent，除非遇到一个阻止正确实现、可在10分钟内回答的具体未知量。该子 Agent只能只读探查，
且不得再派 Agent。
