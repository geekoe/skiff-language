# P5-F440R Router WebSocket RPC profile / broker core result

状态：`PASS`。

R0a 已在 Router 内完成可独立单测的 `jsonrpc-2.0-text` profile 与 profile-neutral
`WebSocketRequestBroker`。实现未读取 active assembly、未接 `RuntimeDispatcher`、gateway、server 或
upgrade，也未修改 F440P strict wire。R0b 可以通过 captured generation method admission、
`InboundDispatchAction` / `InboundDispatchResult` 和 captured runtime response callback 接入当前 core，
不需要改变 T0 wire outcome 或 artifact selector。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 implementation baseline | `c2abd2e84d7d1ff9ac3f018c67c00518f890c3dd` | `3389741b6ee927cf97c4943130e0fdc29af5af82` |
| worktree 实际起点 | `b99bfd40df25bb538e73b178bfa3d5645661c322` | `1febd5d6c575d34f7682ba7694911bf6af565d06` |
| implementation | `3eeb7213bdd148eed9adea5d2becaf3c9404cc08` | `98e2d5034608b7297712c40c232b5c288471bf02` |

`c2abd2e8..b99bfd40` 只新增 F440Q/F440R 两个 task 文档，没有 production/test 差异。
Implementation 与本文 result 分离提交；result commit/tree 由最终交付消息记录。

## 2. Test-first red

当前 worktree 没有安装依赖。首次 `pnpm --dir router exec vitest ...` 因找不到 Vitest 退出，随后只在单个
shell 生命周期内临时链接 `/Users/geek/workspace/skiff/router/node_modules`，并在命令退出时删除链接。

production owner 创建前取得两个真实 compile-red：

| Selector | Red |
| --- | --- |
| `tests/json-rpc-20-text-profile.test.ts` | 1 failed suite、0 tests：无法解析不存在的 `src/protocol/jsonRpc20TextProfile.js` |
| `tests/websocket-request-broker.test.ts` | 1 failed suite、0 tests：无法解析不存在的 `src/router/webSocketRequestBroker.js` |

第二个 red 时测试源码已经包含 fake writer/dispatcher、同值双向 id、duplicate、cancel-vs-complete、
deadline/disconnect 等状态机用例；失败点是 broker owner 尚不存在，而不是零匹配 selector。之后才新增
production modules。

## 3. Profile implementation

`router/src/protocol/jsonRpc20TextProfile.ts` 与 direct helper
`router/src/protocol/losslessJson.ts` 提供：

- 只接受单个 UTF-8 text JSON value；binary 由 broker 按 `1003` 关闭；
- exact outer/error/cancel control member 集合，解码后 key duplicate 检测，batch/non-object/parse/
  malformed response 的冻结分类；
- request 的 non-empty string / 数学意义 safe integer id，`-0`/`1e0` canonical echo；
- 不经 JS `number` 往返的 params/result/data raw slice；业务 JSON 中
  `9007199254740993`、重复业务 member 等保持 opaque；
- exact cancel notification、ordinary ignored-notification action、response success/remote error；
- safe-integer remote code、non-empty bounded remote message、独立 data presence；
- depth/node/string/text 限制和 `1009` close；
- 在产生 typed-id action 前验证最小 fixed terminal envelope 可编码，避免安装 execution 后才发现 id
  无法回写；
- fixed platform code/message、outbound request/cancel/result encoder 与 runtime payload purpose
  validation。

控制 number 使用十进制 lexeme 做精确整数判定；`1e-324` 或会被 JavaScript 舍入成整数的 fraction
不能冒充 safe integer。Opaque 业务 number 不进入该转换。

## 4. Broker implementation

`router/src/router/webSocketRequestBroker.ts` 及其 types/state helpers 提供单一 connection-state owner：

- generation 捕获 exact connection/service/WebSocket entry/owner token、profile adapter、method
  admission、peer writer 与不回退的 outbound id generator；
- outbound peer index 与 runtime `(sender object, session token, correlation)` index；
- 独立 inbound peer index/execution token/AbortController；
- 两方向独立 TTL/FIFO/capacity tombstone store，lazy sweep，无 per-tombstone timer；
- global/per-generation 双向 active capacity，以及可观测的 peer/runtime/inbound/generation active、
  timer 和 terminal lease 计数。

所有 terminal 都先 exact-token 检查、detach 全部 index、清 timer、写本方向 tombstone，再调用 peer
writer、dispatcher abort 或 captured runtime response。Generation teardown 会先批量 detach，再执行外部
terminal，并同步清理该 generation 的 tombstone queue/index。

### 4.1 Outbound

- ownership/profile/method/payload/capacity 校验后才分配 peer id、安装 deadline 与写 request；
- response 按 exact generation/outbound key 乱序完成原 captured runtime source；
- remote platform-looking code 仍保持 `remote` outcome，不能冒充 fixed platform terminal；
- runtime cancel 无普通 response，deadline 为 `deadlineExceeded`，两者 best-effort peer cancel；
- writer failure、runtime disconnect、peer disconnect、binary/oversize/protocol close 分别按冻结 outcome
  清理；
- duplicate/late tombstoned response 静默丢弃，tombstone 驱逐后的 unknown response 关闭 `1002`；
- replacement generation 不接管旧 pending，wrong-generation response 也不能完成旧 request。

### 4.2 Inbound

- request action 包含 profile、connection/generation、transport peer id、opaque params、method、
  unique execution token 与 AbortSignal；peer id 不进入 params；
- ordinary notification 只产生无 terminal 的 observation action，不进入 dispatcher；
- captured method admission 在 active install 前返回 `methodNotFound`；无 method table 的 generation
  默认 fail closed；
- success/invalidParams/internalError/deadlineExceeded/runtimeUnavailable 精确映射 result 或 fixed error；
- peer cancel、deadline、disconnect、duplicate、capacity、late completion 均按 exact execution token
  最多 terminal 一次；
- 同值 id 在 outbound/inbound 两张表中互不影响；
- tombstone FIFO/TTL 驱逐后允许 peer 重用 id，旧 Promise completion 仍因旧 entry/token 不匹配而无 write。

Broker core 只消费 profile action/terminal/opaque payload；反向搜索确认它不访问 `jsonrpc`、JSON
`error.code` 或业务字段。

## 5. Focused validation

任务给出的 pnpm wrapper 会把进程 cwd 改为 `router`：list 退出 0 但没有 listing 输出，run 将
`--root router` 解析为 `router/router` 并以 “No test files found” 退出 1。因此按 F440P 已记录的 fallback
直接调用现有 Vitest binary。

实际 listing：

```text
router/node_modules/.bin/vitest list --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
```

得到 `60` 个非零测试：

- profile：`34`
- broker：`26`

实际 execution：

```text
router/node_modules/.bin/vitest run --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts
```

结果：`2 files passed`，`60 passed / 60 total`。

`pnpm --dir router type-check` 使用两个临时链接：

- `router/node_modules -> /Users/geek/workspace/skiff/router/node_modules`
- 根 `node_modules -> /Users/geek/workspace/skiff/telemetry/node_modules`，只用于提供现有 `mongodb`

结果 PASS；两个链接均已删除，没有安装依赖。

| Check | Result |
| --- | --- |
| direct Vitest listing | PASS，60 non-zero |
| direct Vitest execution | PASS，60/60 |
| `pnpm --dir router type-check` | PASS |
| `git diff --cached --check`（implementation） | PASS |

未展开完整 Router suite。

## 6. Scope audit

Implementation 只修改/新增：

- `router/src/index.ts` 的两个 mechanical exports；
- `router/src/protocol/{jsonRpc20TextProfile,losslessJson}.ts`；
- `router/src/router/webSocketRequestBroker{,State,Types}.ts`；
- 两个任务指定 direct test 文件。

没有修改 `runtimeEndpoint.ts`，也没有修改 F440P wire、`RuntimeDispatcher`、assembly/gateway/server/
upgrade、`assemblyControlPlane.ts`、README、Rust、fixture、scripts 或其它 task/result。

未启动 server、stable、live、instance 或网络；未派子 Agent；未 merge、rebase 或 push。
