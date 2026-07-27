# P5-F440F Agine Host private JSON-RPC adapter result

状态：`IMPLEMENTATION_PASS`。Host private JSON-RPC adapter 已落地；没有触发
`TASK_SCOPE_EXPANDED`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| F440D protocol/result 输入 | `6dbed271656cd647afe93f34c6801d23f183d183` | `dbcf6467363e0d8f128207be4b57df44e92f01b7` |
| Skiff dispatch 输入 | `75d5e2307c0ab55aedb95548f8b61960b4becf5e` | `2f98afed4a0509e94be23f57bf8b36d27f5234d2` |
| Internals 输入 | `605ebd209dacac7c95aa79dc3a508d428a352453` | `95cc84051c350f45e38a6092958d58734c5278db` |
| Internals implementation | `7d852f100a23599c48254049976e63ad0e9338ae` | `2f470dee6562a75b25f1d27091205b2d0e39d706` |

Internals implementation 的 15 个修改文件全部位于 `agine/host/**`。没有修改
`agine/protocol`、`agine/service`、`agine/client`、顶层 `shared-client` 或 Skiff production。
Skiff 侧只新增本文 result；result-only commit/tree 由交付消息记录。

## 2. 实现结果

### 2.1 Private adapter 与 strict frame demux

- `hostPeerFrame.ts` 只负责 private frame parse、raw business event demux、strict outer/params
  校验和固定 response 编码。
- `HostPeerAdapter.ts` 只负责 generation-local `inFlight`、settled tombstone、deadline、
  cancel、once-only settle 和 captured writer。
- canonical method、params/result 和有限 integer error registry 全部从
  `@agine/protocol/hostPeer` 消费，没有在 Host 复制。
- parse、invalid、batch 使用 `id:null`；可信 profile string id 之后的
  method/params/capacity/deadline/internal/cancel error 回显原 id。
- list/search success 包装为 `{kind:"ok",value}`；
  `HOST_FILES_INVALID_PATH` 和 `HOST_FILES_PATH_OUTSIDE_ROOT` 分别投影为
  `{kind:"invalidPath"}` / `{kind:"outsideWorkspace"}`。其它 active throw 或不可编码结果固定脱敏为
  `-32603 / Internal error`。
- strict request 只接受 `jsonrpc/id/method/params`，三个 method 都要求精确 object params；
  legacy transport field、错误 version 和不可置信 id 使用 `id:null`。

### 2.2 Generation、cancel 与 id lifecycle

每个物理 socket 由 `EnhancedWebSocket` 的 Host-local factory 创建一个新 adapter。生产常量精确为：

| 状态 | 值 |
| --- | ---: |
| active limit | `128` |
| tombstone limit | `256` |
| local deadline | `15_000ms` |
| tombstone TTL | `30_000ms` |

- active cancel 先原子 remove、写 tombstone、标记 cancelled 并保存 captured writer，再
  best-effort 写 `-32800`，最后 abort；late resolve/reject 不再写。
- settled/tombstoned/unknown cancel 都是 no-op；malformed cancel 以 `1002` 关闭当前 generation。
- active 或 tombstoned duplicate id 以 `1002` 关闭并 abort 当前 generation 的全部 active request；
  tombstone 过期或最老驱逐后可复用。
- deadline 只在 entry 仍 active 时写 `-32001`，随后 abort；handler 的 late completion 被 entry
  identity/state/map 检查丢弃。
- disconnect、error、explicit close 和 heartbeat close 都先 detach captured adapter，再清 map、
  cancel timer、abort controller；不发送 response。
- binary application frame 以 `1003` 关闭；Host 没有 outbound pending 时收到 JSON-RPC response
  object 以 `1002` 关闭。

### 2.3 Captured socket 与 raw business event 共存

- response 只调用 entry 捕获的 `writer.sendText`；不调用普通 event `send`，不接触
  `messageQueue`，send throw 不 retry。
- reconnect 创建全新 adapter；旧 promise 即使晚到也不能写旧 socket，更不能写新 generation。
- 合法、无 `jsonrpc` 的 raw `eventName` object 继续进入原 `EventManager`；fake physical-socket
  lifecycle 直接证明 `tool_call/request` 仍可命中 listener。
- 任意带 `jsonrpc` 的 frame 都不会落入 event fallback。合法 JSON-RPC notification 除 cancel 外
  一律 ignore/no response。
- `host/current-directory/request`、`host/files/list-request`、
  `host/files/search-request` 以及对应旧 result event 只存在于 private retire denylist/test，
  不再 event dispatch。

### 2.4 HostRuntime 与 Host-local type cleanup

- `HostRuntime` 删除三项旧 `.on(...)` handler、`hostCurrentDirectoryResponseEvent`、旧
  `withTimeout`/string error projector、`hostFilesTimeoutMs` 和三个旧 result send。
- hello/ping/tool-attempt metadata、presence、activation、tool receipt/attempt 行为保留。
- `hostServiceTypes.ts` 的 breadcrumb/file/directory/search 类型改为
  `@agine/protocol/hostPeer` type alias/re-export；filesystem、ripgrep 和 AbortSignal 安全链不变。
- production adapter 拆为 bounded modules；architecture gate 要求
  `HostPeerAdapter.ts`、`hostPeerFrame.ts` 和 `hostPeerHost.ts` 均少于 500 行。

## 3. Canonical fixture 覆盖

`HostPeerAdapter.test.ts` 直接读取唯一
`agine/protocol/fixtures/host-peer-jsonrpc-v1.json`，覆盖全部 24 个 Host peer wire vector：

| Fixture section | 数量 | 结果 |
| --- | ---: | --- |
| request/response | 5 | PASS |
| platform error | 10 | PASS |
| notification | 2 | PASS |
| invalid request | 3 | PASS |
| concurrency | 1 | PASS |
| id lifecycle | 3 | PASS |
| **Host peer 合计** | **24 / 24** | **PASS** |

fixture 另有 3 个 browser HTTP contract；它们属于 protocol/service/client consumer，不是 Host socket
adapter 分类输入，本 leaf 没有越界执行。

fixture 之外还直接覆盖：cancel before/after settle、unknown/malformed cancel、cancel/deadline late
completion、全部 active abort、tombstone oldest eviction、response encode failure、captured send throw、
binary、unexpected response、disconnect/error/explicit close、跨 generation 同 id、raw
`tool_call/request` 和 JSON-RPC-with-eventName 不 fallback。

## 4. 聚焦验证

共 9 个聚焦 test entrypoint 全部通过；没有运行完整 Internals workflow。

| 命令（cwd=`agine/host`） | 结果 |
| --- | --- |
| `npm exec -- tsx src/HostPeerAdapter.test.ts` | PASS，`24 / 24` Host peer fixture vectors |
| `npm exec -- tsx src/CapturedSocketLifecycle.test.ts` | PASS |
| `npm exec -- tsx src/GatewayClient.test.ts` | PASS |
| `npm exec -- tsx src/HostRuntime.test.ts` | PASS |
| `npm exec -- tsx src/HostService.test.ts` | PASS |
| `npm exec -- tsx src/RipgrepSearch.test.ts` | PASS |
| `npm exec -- tsx src/FileWorkspace.test.ts` | PASS |
| `npm exec -- tsx src/cli.test.ts` | PASS |
| `npm run test:architecture` | PASS |
| `npm run type-check` | PASS |
| `node --experimental-strip-types --check`（13 个修改 TS 文件） | PASS |
| `prettier --check`（5 个新增 adapter/lifecycle module/test） | PASS |
| `git diff --check` / cached diff check | PASS |

linked worktree 的 test/typecheck 只读复用主 checkout 已安装的 Node 工具，通过 ignored
`agine/host/node_modules` symlink 解析当前 worktree 的 `@agine/protocol`；没有安装依赖、生成 lockfile
或写 stable artifact。

## 5. 反向搜索与边界

- production `.on("host/current-directory/request" | "host/files/list-request" |
  "host/files/search-request")`：0 命中。
- production `eventName: "host/current-directory" | "host/files/list-result" |
  "host/files/search-result"` send shape：0 命中。
- `hostCurrentDirectoryResponseEvent|hostFilesTimeoutMs|attachHostFileHandlers|HOST_FILES_TIMEOUT_MS`：
  0 命中。
- adapter 三个 production module 内 `messageQueue|\.send\(`：0 命中；唯一 response primitive 是
  captured `writer.sendText`。
- adapter 三个 production module 内 `platform\.|-32002|-32003|-32004`：0 命中。
- Host production 内四个 canonical browse type 的本地 interface/type definition：0 命中；
  `@agine/protocol/hostPeer` import/re-export 是唯一来源。
- `git diff-tree` 证明 implementation 的 15 / 15 文件都在 `agine/host/**`。

## 6. 隔离与后继

- 未修改或建立 public Host framework、第四个 method、Host-originated platform request 或业务可见
  transport id。
- 未运行 build/dev/start、package-boundary build、browser、stable/live、watch、reload 或完整
  Internals canonical workflow。
- 未 merge、rebase 或 push；未派子 agent。
- Internals implementation 提交后 clean；Skiff result 提交后的最终 clean 状态由交付消息记录。
