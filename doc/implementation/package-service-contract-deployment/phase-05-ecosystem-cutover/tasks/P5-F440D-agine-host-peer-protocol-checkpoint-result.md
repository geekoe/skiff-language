# P5-F440D Agine Host peer protocol checkpoint result

状态：`IMPLEMENTATION_PASS`。Internals protocol checkpoint 已建立唯一 Host peer TypeScript owner 和唯一
canonical fixture；没有触发 `TASK_SCOPE_EXPANDED`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| Skiff task 输入 | `a5fdcbd712dbcd30f6a421ee48b6b2876f970e36` | `33911300aa666a610f6ed82087682efe1153fe97` |
| Internals dispatch 输入 | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` |
| Internals implementation | `ff7df57c47b07913fe693550a13056dd41696a55` | `95cc84051c350f45e38a6092958d58734c5278db` |

implementation 只修改：

- `agine/protocol/hostPeer.ts`
- `agine/protocol/http.ts`
- `agine/protocol/fixtures/host-peer-jsonrpc-v1.json`
- `agine/protocol/test/hostPeer.test.mjs`
- `agine/protocol/test/hostPeer.typecheck.ts`
- `agine/protocol/package.json`

Skiff 侧只新增本文 result；result-only commit/tree 由交付消息记录。没有修改 `agine/host`、
`agine/service`、`agine/client`、`shared-client`、Skiff production 或权威设计。

## 2. Protocol owner

`@agine/protocol/hostPeer` 现在单一拥有：

- 三项 method 常量和 method -> params/result type map；
- list/search/current-directory params，以及 breadcrumb、file entry、directory/search/current-directory
  result；
- JSON-RPC request、notification、success/error response、cancel notification 和 JSON-only remote
  error `data` 类型；
- 有限 platform error code/message registry。

`@agine/protocol/http` 同时成为三条 browser HTTP contract 的唯一 owner：

- `/thread/host-files/list`：
  `{ chatId, mountId, toolProviderId, path? } -> HostBrowseDirectoryResult`；
- `/thread/host-files/search`：
  `{ chatId, mountId, toolProviderId, path?, query } -> HostBrowseSearchResult`；
- `/toolproviders/current-directory`：
  `{ toolProviderId } -> { toolProviderId, currentDirectory }`。

三条路径都属于 `AGINE_HTTP_POST_PATHS` 和 `AGINE_ORDINARY_USER_HTTP_POST_PATHS`，并由
`AGINE_HOST_HTTP_POST_PATHS` 与 path -> payload/response type map 统一关联。HTTP payload/response
不含 `eventName`、`requestId` 或 `connectionId`；current-directory success不再有
`refreshRequested`。

list/search 的业务结果严格为：

```text
{ kind: "ok", value: HostBrowseDirectoryResult | HostBrowseSearchResult }
| { kind: "invalidPath" }
| { kind: "outsideWorkspace" }
```

current-directory 仍为 `{ currentDirectory: string }`。canonical Host method 的 params 在类型层必须存在且
为各自 object shape；普通 JSON-RPC notification 仍允许省略可选 params。平台 request id 只出现在
outer wire/control 类型，三项业务 params、result 和 nested type 均不含 `id` 或 `requestId`。

有限 registry 精确为：

| Code | Message | 用途 |
| --- | --- | --- |
| `-32700` | `Parse error` | parse |
| `-32600` | `Invalid Request` | invalid request / batch |
| `-32601` | `Method not found` | unknown method |
| `-32602` | `Invalid params` | canonical params mismatch |
| `-32603` | `Internal error` | 脱敏 internal |
| `-32000` | `Server busy` | Host local capacity |
| `-32001` | `Request timed out` | Host local deadline |
| `-32800` | `Request cancelled` | cancel 赢得 active request |

平台生成的 error 默认省略 `data`；公开 outer error type只允许可选递归 JSON value。没有恢复 F439C
过时的 Host 私有 path/failure error table，也没有 `platform.*` string code。

package 新增 `./hostPeer` export，并把唯一 fixture 作为
`./fixtures/host-peer-jsonrpc-v1.json` export/pack file。dry-run package 中存在两者，test/typecheck
文件不进入 package；self-test 也验证现有 `./http` package export。

## 3. Canonical fixture

唯一 fixture 是 `agine/protocol/fixtures/host-peer-jsonrpc-v1.json`：

| 分组 | 数量 | 覆盖 |
| --- | ---: | --- |
| browser HTTP contract | 3 | list/search/current-directory path、payload 和 success response |
| request/response | 5 | 三项 success、`invalidPath`、`outsideWorkspace`，含全部 nested 字段 |
| platform error | 10 | parse、invalid、batch、unknown、scalar/array params、internal、capacity、timeout、cancel |
| notification | 2 | active cancel；合法业务 notification ignore/no response |
| invalid request | 3 | legacy fields、empty id、non-string id |
| concurrency | 1 | 两个并发 string id，response 按反序完成 |
| id lifecycle | 3 | active duplicate、tombstoned duplicate、tombstone 到期/驱逐后复用 |

batch 的预期 `dispatchCount` 为 0；parse、invalid request 和 batch response 都使用 `id:null`。
unknown/params/internal/capacity/timeout/cancel 原样回显可信 string id。active/tombstoned duplicate
分类为 WebSocket `1002`，到期/驱逐后复用分类为重新 dispatch。fixture 只记录输入和预期分类，没有在
本 leaf 实现 socket 状态机。

HTTP list/search success response 与同一 fixture 内 Host peer `ok.value` 逐字段相等；current-directory
response 精确为 `toolProviderId/currentDirectory`。self-test 同时证明三条 payload/response 没有
transport 字段或 `refreshRequested`。

## 4. 聚焦验证

| 命令 | 结果 |
| --- | --- |
| `npm test --workspace @agine/protocol` | PASS，`11 passed / 0 failed`；所有测试直接读取唯一 fixture |
| `npm run type-check --workspace @agine/protocol` | PASS，strict TypeScript；含 peer/HTTP transport-field 负例 |
| `node --experimental-strip-types --check protocol/hostPeer.ts`，以及同命令检查 `protocol/http.ts` | PASS |
| `jq -e . protocol/fixtures/host-peer-jsonrpc-v1.json` | PASS |
| `prettier --check`（6 个 protocol 新增/修改文件） | PASS |
| `npm pack --workspace @agine/protocol --dry-run --json` | PASS，9 个 package entries，包含 module 与 fixture |
| `git diff --check` | PASS |

type-check 在 linked worktree 中只读复用主 Internals checkout 已安装的 TypeScript CLI；没有安装依赖、
生成 lockfile 或写 stable artifact。

## 5. 反向搜索与边界

- 对 `hostPeer.ts` 的三项 business params/result/nested 声明区反搜 `id|requestId`：0 命中。
- 对 `http.ts` 的三项 Host HTTP payload/response 声明区反搜
  `eventName|requestId|connectionId|refreshRequested`：0 命中。
- 对 `agine/protocol` 反搜 `platform\.`：0 命中。
- 对 `agine/protocol` 反搜 `-32002/-32003/-32004` 和 F439C Host 私有 path/failure message：0 命中。
  唯一 `-32001` 是父节点覆盖后的 `Request timed out` platform error。
- fixture/test 中的 `requestId` 只存在于 legacy negative vector和禁止业务 correlation 的
  `@ts-expect-error` 负例。
- 提交前 write-boundary audit 证明所有 Internals 修改都位于 `agine/protocol/**`。

## 6. 隔离与后继

- 未实现 Host socket adapter、Skiff caller、service/client迁移或任何第四个method；HTTP范围只落
  protocol path/type/fixture owner。
- 未运行 Host/client/full canonical workflow、build/dev/start、browser、stable/live、watch、reload
  或固定端口 workload。
- 未 merge、rebase 或 push；未派子 agent。
- Internals implementation 提交后 clean；Skiff result 提交后的最终 clean 状态由交付消息记录。
