# P5-F429B Router current downlink-only WebSocket gateway result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 已把 Router WebSocket consumer 切换到 current RuntimeAssembly v2 /
ServiceDeployment v2 与 F426A `websocketConnect` wire。真实业务端口现在只执行 connect 与
runtime-to-client downlink；client data frame 不再进入任何 receive/message runtime 路径。

## 1. 精确输入与提交

| 项 | commit | tree |
| --- | --- | --- |
| 父节点冻结 checkpoint | `1f52b2f5053830134e59bfa6f5c67d787078efa2` | `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94` |
| leaf 启动 base（仅增加任务分发文档） | `3769eec5cc8599dbe1a54833eae9fcd00545c589` | `6f868363f97266fd3735c99bf8c4f1ce688f9645` |
| implementation | `b1dd9c9c2d58d3d8fac3665ed9cf99397452f165` | `68d7e1aed7dafe20293657e06caaf806e789534d` |

implementation 提交后 worktree clean；本 result 文档单独提交。

## 2. Current snapshot 与 connect lane

- RuntimeAssembly ingress 新增 closed `webSocket` selector：
  - canonical host；
  - `method: null`；
  - exact absolute path；
  - 与 HTTP selector 使用不同 index key。
- ServiceDeployment reader 接受唯一 compiler-owned `websocket` gateway entry，要求：
  - `protocol.kind: "websocketConnect"`；
  - fixed v1 request/result/policy shapes；
  - exact ordered external sources 与 `binary,text` downlink frames；
  - `pre` / `guard` 为 `null`；
  - optional handler；无 handler 时 adapter args 必须为空；
  - selector protocol 与 entry protocol、RuntimeAssembly selector/key/
    `GatewayEntryIdentity`、resolved contract identity 全部 exact join。
- Router 从 canonical preimage
  `{"gatewayEntryKey":"websocket","schema":"skiff-websocket-entry-identity-v1","serviceId":...}`
  推导 `WebSocketEntryId`。TypeScript 测试与 Rust language-neutral golden
  `skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616`
  一致。
- HTTP upgrade 在 Assembly HTTP server 上按 active snapshot 的 exact host/path 选择 binding，
  构造空 payload 的 F426A request；service/deployment/assembly/generation/entry facts 在连接
  生命周期内保持 pinned。

## 3. Admission、lifecycle 与 client uplink

### 3.1 Handler present

- Router 选择 current service replica，通过 `RuntimeDispatcher` 发送 exact connect request。
- runtime 必须在该 pending request 上 acquire exact generation tuple；response sender receipt、
  replica、assembly、generation 与 entry 均被固定。
- exact `accept` 才进入 admission；可选 business identity 与 connection policy 被应用。
- exact `reject` 在 admission 前失败；socket close、reject、transport error、client close 与
  shutdown 都只 release 一次 generation pin。

### 3.2 Handler absent

- Router synthesized accept，不调用 runtime selector/dispatcher，也不创建 generation expectation、
  acquire 或 release。
- 连接仍保存 exact assembly/deployment/service/entry ownership，用于 policy index 与 outbound
  authorization。

### 3.3 Downlink-only invariant

- client text 或 binary `message` 事件的第一项业务动作是 close `1003`，reason 为有界固定文本
  `client data frames are not supported`。
- gateway/lifecycle 不再包含 receive queue、active receive、pending receive counter、
  request construction 或 `scheduleReceive`。
- WebSocket protocol ping/pong 仍由 transport 正常处理；peer close 与 transport error 同步
  去索引。

## 4. `connection.send` trust 与 fan-out

- direct target 首先查 connection；已关闭 race 返回结构化 `connection-closed` miss，不因随后
  frame 中的 stale facts 隔离 runtime。
- 存活 direct target 要求 frame `serviceId`、`websocketEntryId` 与 connection 完全相同。
- handler-backed connection 进一步要求发送者是原 pinned assembly/generation/replica，且匹配
  dispatcher connection receipt。旧 generation 的已 pin 连接因此仍可由其原 replica 下行。
- 无 handler direct 与 business target 只接受当前 generation 且实际拥有该 service 的 replica。
- mismatch 返回 protocol violation；RuntimeEndpoint 记录 observation 并以 `1008` 隔离发送
  runtime，不向 client 发送。
- business key 精确为 `(serviceId, websocketEntryId, businessIdentity)`；不包含 version、
  build、deployment revision 或 generation。因此 current runtime 可向同 service/entry/identity
  的滚动 generation 连接共同 fan-out。

## 5. Legacy Router residue 收敛

- 删除 manifest WebSocket connect/receive/context DTO、identity 计算、loader 与 adapter source。
- artifact manifest projection 不再读取或生成 `gateway.websocket`；ServiceDeployment current
  snapshot 是唯一 WebSocket ingress 来源。
- 删除旧 WebSocket manifest fixture、identity tests 与 RouterHarness legacy WebSocket helper。
- F426A 发现的 29 个 legacy gateway tests 已重写为 current snapshot/connect/downlink/trust
  probes；1 个 receive storm test 已删除。
- `router/src/protocol/**`、shared corpus 与 protocol reader tests 未修改。

生产 reverse search 中，gateway/lifecycle/manifest/artifact current owners 不再含
`receiveEvent`、`contextCodec`、`contextPayloadPresent`、`websocket.message`、
`websocket.context`、`scheduleReceive` 或 receive queue。剩余相关 spelling 仅位于：

- F426A 禁止本 leaf 修改的 general protocol compatibility owner；
- 既有 control-plane/active-snapshot health DTO 的恒零/恒空展示 shape；
- 对已删除 top-level assembly spelling 的 strict rejection。

这些位置不构造、调度或消费 business uplink。

## 6. 自验收矩阵

| 完成标准 | 状态 | 证据 |
| --- | --- | --- |
| exact RuntimeAssembly/ServiceDeployment WebSocket join | PASS | handler present/absent positives；selector/key/frame order/args/hook/protocol alias negatives；contract identity join |
| current F426A request/response | PASS | exact request validator、空 payload dispatcher、exact accept/reject response validator |
| handler accept/reject 与 generation pin | PASS | network gateway tests；pending acquire sender tuple tests；close/reject/error exact-once release |
| no-handler synthesized accept | PASS | 0 dispatch、0 expect/acquire/release |
| client text/binary immediate `1003` | PASS | network text/binary probes；handler-backed data 不新增 dispatch |
| ping/pong 与 close deindex | PASS | live WebSocket ping/pong；peer close/error lifecycle tests |
| direct sender trust | PASS | service、entry、generation、replica、receipt mismatch；closed race miss |
| business fan-out key | PASS | 跨 assembly generation 两连接同时收到 binary downlink |
| legacy receive path 删除 | PASS | production reverse search；旧 29+1 tests/fixtures 收敛 |
| 未修改 frozen protocol/corpus | PASS | implementation commit changed paths 不含 protocol/corpus |

## 7. 验证证据

| 命令 | 结果 |
| --- | --- |
| `pnpm --dir router test` | PASS：50 files，642 tests |
| `pnpm --dir router exec tsc --noEmit` | PASS |
| `node scripts/verify.mjs --only router` | PASS：selected Router phase；50 files，642 tests |
| `git diff --check` | PASS |

没有 merge、rebase、push、stable/live、instance 或 combined probe 操作；没有自行承接 D4 或
F429A 合流后的 combined probe。
