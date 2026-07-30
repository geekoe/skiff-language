# P5-F444B Host authenticated HTTP producer cutover result

状态：`IMPLEMENTATION_PASS`。五条 Host authenticated business upcall 已从 raw WebSocket
producer 硬切到 HTTP request/response；没有触发 `TASK_SCOPE_EXPANDED`。

## 1. 输入、提交与状态

| 项目 | Commit | Tree / 状态 |
| --- | --- | --- |
| Skiff production 输入 | `f94eb17ea85040d587cfd3e39378e4ae24c3aad6` | `cbe65ed0b90421925d000f8539bcf606ed8bdef1` |
| Skiff task/result worktree 输入 | `08faf208809e28b2c868916294a2e888a064fa5b` | `5a2ad05d2b0125c0d95e82bc871fe680f90e8d2b` / clean |
| Internals 输入 | `232094902785c6e725adafa6f4dc42137a1647b4` | `0178f3282eec1c07cdd031a365abd580fa0f204f` / clean |
| Internals implementation | `beb4ef25538da15a1982327308c5712c58d59f76` | `15c48e07cc3d51794269719c606c87169bd0ee72` / clean |

Internals 只有一个 implementation commit。Skiff 侧只新增本文；result-only commit/tree 和提交后
clean 状态由交付消息记录，避免 result commit 自引用。

## 2. Test-first RED

先在协议测试中要求 `AGINE_HOST_UPCALL_HTTP_POST_PATHS`、五个 canonical path、transport-free
payload/response 以及负类型断言，再运行：

```bash
npm test --workspace @agine/protocol
```

当前输入真实失败：`@agine/protocol/http` 不提供
`AGINE_HOST_UPCALL_HTTP_POST_PATHS` export。该 RED 发生在 production 实现之前；不是环境、
fixture 或预存失败。

## 3. 精确写集

implementation 共修改 22 个文件，全部落在任务允许的两个 package：

### 3.1 `agine/protocol/**`（4）

- `agine/protocol/http.ts`
- `agine/protocol/toolCall.ts`
- `agine/protocol/test/hostPeer.test.mjs`
- `agine/protocol/test/hostPeer.typecheck.ts`

### 3.2 `agine/host/**`（18）

- `agine/host/package.json`
- `agine/host/src/GatewayClient.ts`
- `agine/host/src/GatewayClient.test.ts`
- `agine/host/src/HostHttpClient.ts`
- `agine/host/src/HostHttpClient.test.ts`
- `agine/host/src/HostRuntime.ts`
- `agine/host/src/HostRuntime.test.ts`
- `agine/host/src/HostToolAttemptRuntime.ts`
- `agine/host/src/HostToolAttemptRuntime.test.ts`
- `agine/host/src/HostToolExecutor.ts`
- `agine/host/src/ToolAttemptLedger.ts`
- `agine/host/src/ToolAttemptLedger.test.ts`
- `agine/host/src/gatewayConnection.ts`
- `agine/host/src/hostHttpResponse.ts`
- `agine/host/src/protocol/toolCall.ts`
- `agine/host/src/protocol/toolCall.test.ts`
- `agine/host/src/cli.ts`
- `agine/host/src/cli.test.ts`

没有修改 `agine/service/**`、`agine/client/**`、`shared-client/**`、lockfile、`node_modules` 或
Skiff production/reference。package-boundary smoke 产生的临时 `dist` 已移出 worktree，最终没有
build output。

## 4. 实现结果

### 4.1 唯一协议 owner

`@agine/protocol/http` 现在唯一拥有以下五条 path 及其 payload/response contract：

| Method | Path |
| --- | --- |
| `POST` | `/host/hello` |
| `POST` | `/host/activation-ack` |
| `POST` | `/host/ping` |
| `POST` | `/host/tool-attempts` |
| `POST` | `/host/tool-call/result` |

`AGINE_HTTP_POST_PATHS` 精确扩展为 43 条且无重复。五项 HTTP body 从原业务字段机械提取，
保留 host/tool/attempt/run 等 durable business identity，删除 `eventName`、`requestId` 和
connection id；type-check negative fixture 同时禁止顶层和 nested execute request 恢复这些字段。

### 4.2 窄 HTTP client 与认证

- `gatewayConnection.ts` 是 WebSocket 与 HTTP 共用的 service/version selector 和 Host auth
  header builder；没有复制两套 gateway identity 规则。
- `HostHttpClient.ts` 只负责 `ws:` -> `http:`、`wss:` -> `https:`、selector 保留、严格 header、
  JSON POST 和 response parse。
- activation 阶段只发送 `X-Agine-Host-Activation`；已激活阶段只发送
  `Authorization: AgineHost <hostId>`。两种 credential 不共存，也不进入 query 或 body。
- HTTP non-2xx、network rejection 和 malformed success 都作为失败返回；success parser 对五类
  response 做精确 shape 校验，attempt actions 和 result receipt 不依赖 transport correlation。

### 4.3 Runtime、ledger 与 receipt

- connect/reconnect hello、activation ack、presence heartbeat、attempt snapshot/actions 和 Host tool
  result/receipt 全部改为一次 HTTP promise。
- activation hello 返回 Host id 后先持久化，再切换 authorization，最后发送 ack。
- attempt sync 的 promise 是唯一 in-flight owner；并发触发 single-flight，共享 response actions，
  不再存在 `toolAttemptSyncOutstanding`、response listener 或业务 correlation id。
- `HostToolAttemptRuntime` 在 receipt 成功后按 durable identity prune ledger；network/non-2xx
  失败保留 result payload，下一轮可重发。ledger schema 只保存 transport-free request/result。
- `GatewayClient` 不再拥有五项业务 send/correlation API，只保留 connect/reconnect、physical
  state、Host peer adapter 和 raw server notification observer。

### 4.4 F440F 不回归

Host peer JSON-RPC responder、captured writer、generation、cancel、deadline 和 tombstone 状态机没有
改变。raw server -> Host `tool_call/request` notification 仍与 peer responder 共存；socket
open/reconnect 测试证明不会重新发送五个 retired Host event。

## 5. GREEN 验证

### 5.1 Protocol

| 命令 | 结果 |
| --- | --- |
| `npm test --workspace @agine/protocol` | PASS，`12 / 12` |
| `npm run type-check --workspace @agine/protocol` | PASS |

### 5.2 Host 聚焦验收

任务要求的 6 个 Host test entrypoint、新增 HTTP client entrypoint、architecture 和 type-check 全部
通过：

| 命令（cwd=`agine/host`） | 结果 |
| --- | --- |
| `npm exec -- tsx src/GatewayClient.test.ts` | PASS |
| `npm exec -- tsx src/HostHttpClient.test.ts` | PASS |
| `npm exec -- tsx src/HostRuntime.test.ts` | PASS |
| `npm exec -- tsx src/HostToolAttemptRuntime.test.ts` | PASS |
| `npm exec -- tsx src/protocol/toolCall.test.ts` | PASS |
| `npm exec -- tsx src/HostPeerAdapter.test.ts` | PASS，`24 / 24` Host peer fixture vectors |
| `npm exec -- tsx src/CapturedSocketLifecycle.test.ts` | PASS |
| `npm run test:architecture` | PASS |
| `npm run type-check` | PASS |

### 5.3 完整 Host package gate

```bash
npm test --workspace @agine/host
```

PASS：17 个 TypeScript test entrypoint、`host-architecture.test.mjs` 和
`package-boundary-smoke.mjs` 全部通过。最终 package gate 使用只读复用的已安装工具和
`npm_config_offline=true` 临时 cache；没有下载依赖、修改 lockfile 或写入 repo-local
`node_modules`。

`git diff --check`、cached diff check 和提交后 clean check 均通过。

## 6. 反向搜索与边界

- production `eventName: "host/hello"`：0 命中。
- production `eventName: "host/activation-ack"`：0 命中。
- production `eventName: "host/ping"`：0 命中。
- production `eventName: "host/tool-attempts"`：0 命中。
- production `eventName: "tool_call/result"`：0 命中。
- production `toolAttemptSyncOutstanding|completeToolAttemptSync`：0 命中。
- `GatewayClient.ts` 内
  `sendHello|sendActivationAck|sendPresence|sendToolAttemptState|sendToolCallResult`：0 命中。
- old response `.on(...)` transport listener：0 命中。
- `git diff-tree` 证明 implementation 的 `22 / 22` 文件均在允许写集。

协议/test 中的 retired negative 和 server -> Host notification type 保留；没有通过改名保留旧
wire。没有运行 service build、canonical service graph、browser、stable、live 或 network；
没有 merge、rebase、push，也没有派子 agent。
