# P5-F439C Agine / Host JSON-RPC 协议差量审计结果

状态：`PASS / TASK_EXECUTABLE`。这是相对 F438B 的只读 consumer delta 审计，不是 implementation。

## 1. 输入、边界与结论

审计输入：

| Repo | Commit | Tree | 状态 |
| --- | --- | --- | --- |
| Skiff design | `aacee2129934a6aebc2975293b5b4ed4b209c42f` | `617021923ad3d7072d19deecb9f41460dd2163e4` | JSON-RPC / cancellation 权威检查点 |
| Internals | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` | production/test 只读且 clean |

读取范围严格限定为任务允许的 Skiff 父节点、F438B result及其权威设计，以及 Internals integration
worktree 中三项 Host 读取直接触及的 `agine/protocol/**`、`agine/host/**`、`agine/service/**`、
`agine/client/**` 和 `shared-client/**`。没有修改 Internals，也没有运行 build、browser、canonical
workflow、live 或 stable。

结论：

1. `host.files.list`、`host.files.search`、`host.current-directory` 的 typed HostService 已存在；
   前两项从 HostService 到文件系统/ripgrep 已完整接受 `AbortSignal`，current-directory 是立即同步读取。
2. 现有 Host WebSocket wrapper 不能直接复用为 JSON-RPC peer：它只按 `eventName` dispatch，非法 JSON
   只记录日志，而且 `send` 会把 response 排队到重连后的新 socket。所需修复仍可全部收敛在
   `agine/host/**` 的私有 transport adapter，不要求新增公共 Host framework 或业务语义。
3. Internals 当前没有 `requestJsonToConnection`、`WebSocketRequestError` 或 Host peer JSON-RPC
   protocol owner。Skiff shared checkpoint落地前，service production caller仍然 blocked。
4. F438B 的业务分类不变：三项读取是有限 PLATFORM-RPC；Host 主动业务上行仍迁 HTTP；Host file
   `HostFileBrowseRequest`/browser relay、current-directory polling和旧 raw receive均应删除；durable
   Host tool attempt identity不受影响。
5. 本任务不返回 `TASK_SCOPE_EXPANDED`，也没有需要用户裁决的新业务语义。

## 2. 相对 F438B 的协议差量

F438B 的 consumer/业务 owner图继续有效，但其旧 wire、错误和取消结论由下表替换：

| Concern | F438B 旧冻结 | F439 权威终态 |
| --- | --- | --- |
| request | `{type:"request", requestId, method, payload}` | 一条 text frame 一个 `{"jsonrpc":"2.0","id", "method","params"}` object |
| response | `{type:"response", requestId, ok, payload/error}` | JSON-RPC success `result` 或 error `{code: integer,message,data?}` |
| request identity | peer adapter理解 `requestId` | 平台生成非空 string `id`；Host只原样回显；业务代码不可见 |
| params | 自定义 `payload` | profile允许object/array；本任务三个canonical method均要求object |
| batch | 未冻结为标准profile规则 | 第一版拒绝全部JSON-RPC batch |
| cancel | `{type:"cancel", requestId}`，旧结果曾把未知cancel视作自定义协议失败 | `$/cancelRequest` notification，`params:{id}`；best-effort、无response |
| caller cancel | 旧可捕获cancel envelope | ancestor cancellation不可捕获；deadline仍抛`TimeoutError` |
| peer error | string `code/message/detail` | integer `code`、fixed `message`、受限 `data`；不发明`platform.*`字符串 |
| pending owner | service/Host自建correlation容易混入业务DTO | Router broker拥有caller pending；Host adapter仅拥有执行中的本地controller |

JSON-RPC `id`不是`toolCallId`、`attemptId`、`runId`或其它durable business identity。三项读取不得把
transport `id`写入HostService参数、Skiff record、HTTP payload、DB、日志payload或client state。

### 2.1 Delta owner矩阵

| Delta | 单一owner | 非owner约束 |
| --- | --- | --- |
| canonical method、TS params/result、integer error registry、golden fixture | `agine/protocol/hostPeer.ts` + protocol fixture | Host/client不复制method/error registry；Skiff private records由fixture校验 |
| JSON-RPC text parse、typed Host dispatch、response encode、local in-flight/controller | `agine/host/**` private peer adapter | HostService不接触id；`shared-client`不改 |
| Host内部异常 -> integer error、wire脱敏 | 同一Host peer adapter projector | Node Error/stack/path/string code不出进程 |
| caller pending、connection/generation/id pairing、deadline/cancel、broker tombstone | Skiff shared checkpoint | Host/service不另建caller pending或DB correlation |
| typed `requestJsonToConnection`、closed error projection、授权和exact connection | `agine/service/internal/host_peer_rpc.skiff` | HTTP/client不传connection id；业务handler不维护id map |
| browser list/search HTTP、AbortSignal、current-directory单次HTTP | `agine/client/**` | 不再调用Agine WS request；不改generic shared-client request |
| old DB/browser/Host relay和raw receive删除 | service leaf拥有`agine/service/**`，Host/client各删本域旧端 | 不拆出共同写`model.skiff/service.yml/api/agine.skiff`的第二leaf |

## 3. 当前三项读取的 owner、类型和测试

### 3.1 当前 production owner

| Canonical method | 当前 Host handler / typed callee | 当前 request / response type | 当前 service / browser caller | 当前聚焦测试 |
| --- | --- | --- | --- | --- |
| `host.files.list` | `HostRuntime.ts:attachHostFileHandlers` 的 `host/files/list-request` -> `HostService.listDirectory` | request仍是inline `any` (`path` + legacy `requestId`)；response是 `hostServiceTypes.ts:HostBrowseDirectoryResult` | `host_file_rpc.dispatchHostFileBrowseRequest`建立DB relay；`hostFileApi.listHostFiles`经Agine WS `request` | `HostRuntime.test.ts` list success/outside-root error；`HostService.test.ts` root/nested/limit；`host_file_browse.test.skiff` relay exactness/expiry |
| `host.files.search` | 同一 `attachHostFileHandlers` 的 `host/files/search-request` -> `HostService.searchBrowseFiles` | request仍是inline `any` (`path/query/requestId`)；response是 `HostBrowseSearchResult` | 同一DB relay；`hostFileApi.searchHostFiles`经Agine WS `request` | `HostService.test.ts` browse search；`RipgrepSearch.test.ts` parser/limit/path安全；当前没有HostRuntime search wire test |
| `host.current-directory` | `HostRuntime.ts:attachCurrentDirectory` -> `HostService.getCurrentDirectory` | request仍是inline `{requestId?,toolProviderId?}`；Host返回string，再包装旧 `host/current-directory` event | browser已HTTP调用`/toolproviders/current-directory`；service cache miss通过business-identity notification请求Host，client按`refreshRequested`轮询 | `HostRuntime.test.ts`旧event response；`host_toolprovider_current_directory.test.skiff` cache/poll owner；`toolproviderApi.test.ts` retry |

文件:符号级证据：

- Host typed facade：
  `agine/host/src/HostService.ts:HostService.getCurrentDirectory/listDirectory/searchBrowseFiles`。
- Host concrete result：
  `agine/host/src/hostServiceTypes.ts:HostFileEntry/HostBrowseBreadcrumb/HostBrowseDirectoryResult/
  HostBrowseSearchResult`。
- Host取消链：
  `BrowseWorkspace.listDirectory/searchFiles`把signal传给`FileWorkspace`与`RipgrepSearch`；
  `RipgrepSearch.consumeLines`在abort时终止child并reject。
- 旧Host wire：
  `agine/host/src/HostRuntime.ts:attachCurrentDirectory/attachHostFileHandlers`。
- 旧service relay：
  `agine/service/internal/host_file_rpc.skiff:dispatchHostFileBrowseRequest/
  receiveHostFileBrowseResult/cleanupExpiredHostFileBrowseRequests`。
- 旧DB correlation：
  `agine/service/internal/model.skiff:HostFileBrowseRequest`。
- current-directory polling：
  `host_toolprovider_connection.skiff:requestHostCurrentDirectoryRefresh/
  currentDirectoryForToolProvider`和`agine/client/src/lib/toolproviderApi.ts:
  getToolProviderCurrentDirectory`。
- browser Host file旧caller：
  `agine/client/src/lib/hostFileApi.ts:listHostFiles/searchHostFiles` ->
  `agine/client/src/lib/ws.ts:request` ->
  `shared-client/shared/lib/enhanced_websocket.ts:EnhancedWebSocket.request`。

### 3.2 当前类型缺口与目标 concrete type

当前`agine/protocol/**`没有Host peer模块；Host本地、client和Skiff各自重复或弱化shape：

- Host拥有完整response interface，但没有named list/search/current-directory request interface。
- Client在`hostFileApi.ts`重复directory/search result，`FileEntry`又在`client/src/lib/types.ts`重复。
- Skiff旧`HostFilesResultInput.result`只是`JsonObject?`，没有typed success decode。

后继protocol checkpoint应由`agine/protocol/hostPeer.ts`唯一拥有以下TypeScript method、params、result和
integer error常量；Host和client只import或alias，不再复制字段：

| Method | Params（全部是object） | Result |
| --- | --- | --- |
| `host.files.list` | `HostFilesListParams { path?: string }` | `HostBrowseDirectoryResult { root,cwd,parent,breadcrumbs,items,truncated }` |
| `host.files.search` | `HostFilesSearchParams { path?: string, query: string }` | `HostBrowseSearchResult { root,cwd,matches,truncated }` |
| `host.current-directory` | `HostCurrentDirectoryParams = Record<string, never>`，wire为`{}` | `HostCurrentDirectoryResult { currentDirectory: string }` |

共享nested types保持当前字段：

```text
HostBrowseBreadcrumb { name: string, path: string }
HostFileEntry {
  name: string,
  type: "directory" | "file",
  size?: number,
  path: string,
  relativePath?: string
}
```

Skiff caller在私有`agine/service/internal/host_peer_protocol.skiff`声明同shape的concrete records；它们不进入
ServiceContract，不加入`api.yml`，也不包含transport `id`。跨语言fixture是两份语言声明的一致性owner。

### 3.3 params 与 transport id 结论

- 三项canonical params都必须编码成JSON object；即使profile总体允许array，这三个method的typed validator
  也拒绝array、`null`和scalar。
- list只接受可选string `path`；search接受可选string `path`和必需string `query`；current-directory只接受
  空object。`limit`仍是Host私有常量（list 500、search 80），不进入peer payload。
- Host method handler签名只接收`(typedParams, AbortSignal)`并返回typed result；adapter单独保存string
  `id`。HostService、service业务函数和browser HTTP caller都不读取或生成它。

## 4. Host peer adapter的最小冻结合同

### 4.1 连接与I/O边界

adapter实例必须绑定一个捕获的物理WebSocket/generation，直接接收raw text并提供
`sendTextNowOnCapturedSocket`。它不能通过当前会跨重连排队的`EnhancedWebSocket.send/messageQueue`发送
RPC response，也不能把JSON-RPC对象交给`EventManager`按`eventName`dispatch。

每个generation拥有：

```text
inFlight: Map<string, Entry>
recentlySettled: bounded/expiring set<string>
closed: bool

Entry {
  controller: AbortController
  state: active | settled | cancelled
  captured generation/socket
}
```

`inFlight`和`tombstone`均有module-private上限；tombstone容量至少不小于in-flight上限，TTL至少覆盖Host
本地读取deadline。饱和时tombstone驱逐最旧项，不能拒绝新请求；in-flight饱和则以本结果§5的有限integer
error settle该request。

### 4.2 text frame解析与dispatch

处理顺序冻结为：

1. 只接受单个text frame；binary、其它application frame以`1003`关闭当前socket。
2. `JSON.parse`失败、顶层array(batch)、非object，或没有可安全回显的非空string `id`的非法request，
   以`1002`关闭且不发送伪造的`id:null` response。
3. ordinary request只允许顶层`jsonrpc/id/method/params`，且`jsonrpc === "2.0"`、`id`为非空string、
   `method`为非空string；旧`type/requestId/payload/eventName`不被识别。
4. outer shape无效但已有可信string `id`时，以`-32600` error原样回显该id；params不是该method的精确
   object shape时返回`-32602`，不得调用HostService。
5. unknown request method返回`-32601`；不触发任意event listener。
6. known method进入固定table：
   - list -> `host.listDirectory(params.path, {limit:500, signal})`
   - search -> `host.searchBrowseFiles({cwd:params.path, query:params.query, limit:80, signal})`
   - current-directory -> `{currentDirectory: host.getCurrentDirectory()}`
7. 在调用handler前原子登记entry。handler完成后只能经`trySettle(entry,id)`发response。

success精确为：

```json
{"jsonrpc":"2.0","id":"<original-string-id>","result":{}}
```

error精确为：

```json
{"jsonrpc":"2.0","id":"<original-string-id>",
 "error":{"code":-32603,"message":"Internal error","data":null}}
```

`result`按各method concrete type变化；所有error都显式使用`data:null`，Host不把Error、stack、真实路径、
credential或原始message放进wire。

### 4.3 notification与cancel

唯一接受的notification是：

```json
{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":"<opaque>"}}
```

- 它必须没有顶层`id`，params必须精确为只含非空string `id`的object；畸形cancel以`1002`关闭，不响应。
- active id：先从`inFlight`移除并写入tombstone、标记`cancelled`，再abort controller；不发送response。
- 已settled/tombstoned id：response已赢，忽略cancel。
- 未知id：best-effort no-op。它不能生成JSON-RPC error，也不能因cancel与settlement/tombstone eviction
  竞态关闭健康connection。
- 其它well-formed notification不属于第一版profile，以`1003`关闭且不响应。

这也是相对F438B旧自定义cancel的必要delta：unknown/late `$/cancelRequest`是幂等no-op，不是另一个需要
业务处理或response的request。

### 4.4 once-only、乱序、断线和重复

| 场景 | Host adapter精确处理 |
| --- | --- |
| 同connection并发/乱序 | 每个id独立entry；哪个handler先完成就先发response，不维持FIFO。 |
| success/error竞态 | `trySettle`先以entry identity和`active -> settled` CAS判定；先赢者remove + tombstone + send，另一分支drop。 |
| cancel先赢 | remove/tombstone发生在abort前；handler随后resolve/reject都无法通过`trySettle`，零response。 |
| response先赢、late cancel | settled tombstone命中，cancel no-op。 |
| duplicate active request id | 无法安全区分两个caller；以`1002`关闭generation并abort全部entry，不为同一id发第二个response。 |
| duplicate recently-settled request id | tombstone命中，按协议错误`1002`关闭；不得重新执行handler。 |
| disconnect/error/explicit close | adapter先标closed并detach captured socket，再清map和abort全部controller；不发送或排队response。 |
| reconnect | 新socket创建全新adapter/generation；旧promise持有旧entry，永远不能写到新socket。 |
| late local result/error | generation、map entry identity或active state任一不匹配即drop。 |
| raw send throw | entry已settled；不retry、不入message queue、不跨generation重发。 |

## 5. JSON-RPC integer error owner与脱敏

### 5.1 Host wire error registry

`agine/protocol/hostPeer.ts`拥有有限code常量与公开wire type；`agine/host/**` adapter拥有从Host内部异常到该
registry的唯一projector。HostService可以暂时保留`HostBrowseError.code` string作为进程内判别，但这些
string不能进入peer wire。

| Integer | Fixed message | 来源 |
| --- | --- | --- |
| `-32600` | `Invalid Request` | 有可信string id的非法outer request |
| `-32601` | `Method not found` | unknown method |
| `-32602` | `Invalid params` | canonical method params shape不匹配 |
| `-32603` | `Internal error` | 未分类handler throw、response encode失败 |
| `-32000` | `Host peer capacity exceeded` | Host本地in-flight上限 |
| `-32001` | `Invalid host file path` | 内部`HOST_FILES_INVALID_PATH` |
| `-32002` | `Host file path is outside workspace` | 内部`HOST_FILES_PATH_OUTSIDE_ROOT` |
| `-32003` | `Host file request timed out` | 内部明确local deadline / `HOST_FILES_TIMEOUT` |
| `-32004` | `Host file request failed` | 内部明确`HOST_FILES_FAILED` |

取消必须使用private cancel sentinel/entry state，而不是把任意`AbortError`等同timeout：platform cancel已经先
使entry inactive，因此不响应；Host自己的15秒local deadline使用单独reason并在entry仍active时投影
`-32003`。其它仍active的未知abort/throw投影fixed `-32603`，不能泄漏原始message。

所有error `data`固定为JSON `null`。后继若确需新增第四个domain error，必须先更新protocol checkpoint和
跨语言fixture；不能把任意Node `error.code`、路径或`platform.*`字符串透传。

### 5.2 Agine service error投影

service私有`host_peer_rpc.skiff`是唯一caller-side projector。它从
`requestJsonToConnection<TRequest,TResponse>`恢复typed result，并完整处理公开错误分支：

| Caller outcome | Public `ApiError` projection | HTTP status |
| --- | --- | --- |
| `connectionUnavailable` | `host_offline` / fixed message | `503` |
| `transportUnavailable` | `host_offline`；不声称peer未执行、不自动retry | `503` |
| `protocolError` | `host_files_protocol` | `502` |
| `resourceLimit` | `host_files_failed` / fixed unavailable message | `502` |
| remote `-32001` | `host_files_invalid_path` | `400` |
| remote `-32002` | `host_files_path_outside_root` | `400` |
| remote `-32003` | `host_files_timeout` | `504` |
| remote `-32000` / `-32004` / `-32603` | `host_files_failed` | `502` |
| remote `-32600/-32601/-32602` | `host_files_protocol` | `502` |
| 其它remote integer | `host_files_failed`；fixed message | `502` |
| `TimeoutError` | `host_files_timeout` | `504` |
| `std.json.DecodeError`（request encode/params shape/success result decode） | `host_files_protocol` | `502` |

remote `message/data`是不可信值：只用于有限integer branch判定，不能进入public HTTP message或普通日志。
`agine_transport.apiErrorHttpStatus`需显式覆盖上述Host codes，不能继续落到当前default `400`。

ancestor cancellation不出现在catch union中。HTTP/browser disconnect或上层supersession应直接终止当前
lane，交由平台发送best-effort cancel；service不得把它转换成`ApiError`、成功空值或detached retry。

## 6. Service typed caller、授权与旧graph删除

### 6.1 单一caller owner

`agine/service/internal/host_peer_rpc.skiff`应拥有三个窄函数；旧`host_file_rpc.skiff`不再拥有DB relay：

```text
listFiles(exactConnectionId, HostFilesListParams)
  -> requestJsonToConnection<HostFilesListParams, HostBrowseDirectoryResult>

searchFiles(exactConnectionId, HostFilesSearchParams)
  -> requestJsonToConnection<HostFilesSearchParams, HostBrowseSearchResult>

currentDirectory(exactConnectionId, {})
  -> requestJsonToConnection<HostCurrentDirectoryParams, HostCurrentDirectoryResult>
```

三者在当前execution的15秒operation deadline内调用，不启动detached work。调用前：

- list/search继续由`resolveThreadHostBinding`验证user owner、thread、mount、toolProvider、`canReadFiles`、
  online/presence、`host.files.v1` capability和query长度；
- current-directory由`ownedActiveHostToolProvider`验证owner/active，另验证online/presence、
  capability和非空`activeConnectionId`；它没有thread mount；
- connection只取当前DB `ToolProvider.activeConnectionId`，不得来自HTTP body或business-identity fan-out。

current-directory成功后可同步刷新ToolProvider metadata并直接返回
`{toolProviderId,currentDirectory}`；不再返回`refreshRequested`，client不再轮询。

### 6.2 F438B 删除分类继续生效

| 旧owner | 终态 |
| --- | --- |
| `model.skiff:HostFileBrowseRequest`及indexes | DELETE；不是durable business object |
| `host_file_rpc` TTL/insert/claim/delete/browser relay | DELETE/替换为上述direct typed caller |
| `host_toolprovider_runtime.dispatchHostFileBrowseRequest/receiveHostFileBrowseResult` | DELETE |
| `agine_ws_host_tool_files.dispatchUserFileRequest/dispatchHostFileResult` | DELETE |
| `api/agine.skiff:ThreadHostFiles*Input/HostFilesResultInput` | DELETE；新增HTTP payload与private peer records不含requestId |
| `requestHostCurrentDirectoryRefresh`、`refreshRequested` | DELETE；一个HTTP request内等待Host result |
| Host `host/*-request` event handlers与`host/*-result` send | DELETE；固定JSON-RPC adapter取代 |
| client Host file WS request + current-dir retry delay | DELETE；HTTP + AbortSignal |
| `agine/client/src/lib/ws.ts:request` | 最后一个Agine production consumer移除后DELETE export |
| `shared-client/shared/lib/enhanced_websocket.ts:request` | RETAIN；非Agine legacy consumers仍合法，明确不在写集 |
| Host tool `toolCallId/attemptId` ledger/reconciliation | RETAIN；不改成platform unary request |

## 7. 最小实现DAG、互斥写集与checkpoint

```text
P5-F439C result
       |
Internals protocol checkpoint (agine/protocol/**)
       |\
       | +--> Host private JSON-RPC adapter + unit fixtures (agine/host/**)
       |
       +----> Client HTTP/AbortSignal leaf may prepare against mocks (agine/client/**)

Skiff shared std/compiler/runtime/router checkpoint + combined Skiff probe
       |
       +--> Agine service typed caller + HTTP routes + relay/receive deletion
             (agine/service/**)

protocol + Host + service + client
       |
Internals cross-language focused combined probe (read-only gate)
```

互斥写集：

1. **Protocol checkpoint**：只写`agine/protocol/**`，新增`hostPeer.ts`、canonical fixture、HTTP
   list/search path/payload types及`package.json` export。
2. **Host leaf**：只写`agine/host/**`；新增private adapter/raw socket generation boundary、typed dispatch、
   integer projector和tests；不改protocol、service或shared-client。
3. **Service leaf**：只写`agine/service/**`；拥有全部Skiff records/caller/error projection、HTTP entries、
   DB relay/raw receive/API DTO删除，避免另一leaf共同修改`model.skiff`、`service.yml`或`api/agine.skiff`。
4. **Client leaf**：只写`agine/client/**`；Host file HTTP caller、AbortSignal、current-directory单次HTTP、
   hook supersession和Agine WS request export删除；不改shared-client。
5. **Combined owner**：无production写集；消费四个leaf的最终commit。

Skiff shared checkpoint前可实现并单测：

- `hostPeer.ts` method/params/result/error registry和fixture；
- Host raw text parser、fixed dispatch、integer projector、AbortController、once-only/generation state machine；
- fake socket上的success/error/batch/cancel/duplicate/out-of-order/disconnect tests；
- client HTTP caller和AbortSignal mock tests。

必须等待Skiff shared checkpoint：

- 任一production `std.websocket.requestJsonToConnection` caller；
- 对`WebSocketRequestError`封闭union、`TimeoutError`和不可捕获ancestor cancellation的真实Skiff处理；
- 删除service DB relay/Host result receive/最后raw receive并切换connect-only；
- 真实Router/runtime/Host cross-language request、cancel、typed decode和disconnect probe。

Host leaf的纯adapter/test可以提前提交，但旧event handler删除后的commit不能作为独立可运行Internals候选；
只有与service leaf合流后才进入combined验收。

## 8. Cross-language fixture与聚焦测试owner

### 8.1 Canonical fixture

唯一fixture owner是protocol checkpoint中的
`agine/protocol/fixtures/host-peer-jsonrpc-v1.json`（或等价、但只能有一个canonical文件）。它至少包含：

- 三个method的完整request和success response；
- list/search/current-directory的object params及nested result全部字段；
- `-32000`至`-32004`和`-32600`至`-32603` error response，`data:null`；
- `$/cancelRequest` notification；
- negative vectors：array batch、non-string/empty id、legacy `type/requestId/payload/eventName`、
  array/scalar params、unknown method、malformed cancel；
- 两个并发id按反序response的fixture。

fixture中的id只用于outer transport断言；params/result断言必须证明没有id/requestId。Host test直接消费
fixture；service Node/Skiff checker消费同一文件或通过combined fake peer对照，禁止复制第二份golden。

### 8.2 Leaf tests

| Leaf | 聚焦证据 |
| --- | --- |
| protocol | fixture schema/self-check；method、integer registry、HTTP path和package export静态一致性 |
| Host | 新`HostPeerAdapter.test.ts`覆盖三正例、object params、id回显、unknown/malformed/batch、全部error mapping/脱敏、local timeout、cancel before/after settle、unknown cancel、duplicate active/settled、乱序、disconnect/reconnect late result；保留`HostService.test.ts`、`RipgrepSearch.test.ts`、`FileWorkspace.test.ts` |
| service | 三个typed peer fake；owner/mount/capability/exact connection；五个`WebSocketRequestError` branch、remote integer、`TimeoutError`、`DecodeError`；ancestor cancel不转ApiError；HTTP list/search/current-directory；无DB relay/receive |
| client | `hostFileApi.test.ts`改HTTP + signal；hook新请求/close/unmount abort旧fetch；`toolproviderApi.test.ts`证明单次HTTP无poll；`ws.test.ts`和architecture test证明无Agine request export |
| combined | 同connection list/search反序；remote known/unknown error；browser abort/deadline/Host disconnect；cancel race与late result；reconnect new generation；typed fixture逐字段；无legacy event frame |

实现leaf可使用的聚焦入口包括：

```bash
npm --prefix agine/host exec -- tsx src/HostPeerAdapter.test.ts
npm --prefix agine/host exec -- tsx src/HostRuntime.test.ts
npm --prefix agine/host run test:architecture
npm --prefix agine/client exec -- vitest run \
  src/lib/hostFileApi.test.ts \
  src/lib/toolproviderApi.test.ts \
  src/lib/ws.test.ts \
  src/architecture.client-boundaries.test.ts
node agine/service/internal/host_runtime_architecture.test.mjs
node agine/service/internal/agine_service_architecture.test.mjs
```

service `.skiff` tests仍须通过linked-worktree isolated canonical test设施选择对应fixture；不得写stable artifact
root或注册stable watch。

### 8.3 终态反向搜索

```bash
rg -n 'host/(files/(list|search)-(request|result)|current-directory(/request)?)' \
  agine/host agine/service
rg -n 'HostFileBrowseRequest|dispatchHostFileBrowseRequest|receiveHostFileBrowseResult|refreshRequested' \
  agine/service agine/client
rg -n 'requestJsonToConnection|WebSocketRequestError|TimeoutError|std\\.json\\.DecodeError' \
  agine/service/internal
rg -n 'eventName.*requestId|requestId.*eventName|type.*requestId|requestId.*payload' \
  agine/host agine/service agine/client
rg -n 'EnhancedWebSocket\\.request|socket\\.request|export async function request' \
  agine/host agine/client
rg -n 'host\\.files\\.(list|search)|host\\.current-directory|\\$/cancelRequest' \
  agine/protocol agine/host agine/service
```

允许项：

- canonical fixture和Host adapter outer JSON-RPC `id`；
- negative tests断言legacy字段被拒绝；
- `shared-client`非Agine legacy request；
- durable `toolCallId`、`attemptId`、`runId`等业务identity。

不允许项：

- Host/business handler参数或result中的transport id；
- Host RPC response经跨重连message queue发送；
- service DB/browser/Host两层correlation；
- `platform.*` string error code；
- Agine browser或Host application `eventName + requestId` request/response。

## 9. Scope execution判定

不触发`TASK_SCOPE_EXPANDED`：

- typed dispatch已有明确HostService入口；
- list/search取消已经贯通到可终止的filesystem/ripgrep路径；
- current-directory同步返回，不需要新的取消型业务API；
- `ws` primitive、raw socket生命周期和close能力都已在`agine/host/**`；
- 缺口是把当前eventName parser/queued sender替换为generation-bound private adapter，不是建立新的公共
  Host framework。

若后继实现发现必须修改`shared-client` generic request、增加第四个Host peer method、让Host主动发
platform request、把cancel变成业务错误，或把transport id写进业务DTO，应停止并返回新的scope expansion；
这些都不由本审计授权。

## 10. 本次只读验证

| 验证 | 结果 |
| --- | --- |
| `git show -s --format='commit=%H tree=%T'`核对两个integration输入 | PASS，精确匹配任务commit/tree |
| `rg`追踪三项method、旧event、HostService、DB relay、HTTP/client caller | PASS，owner图如§3/§6 |
| `rg`检索Internals `requestJsonToConnection/WebSocketRequestError/$/cancelRequest` | PASS，当前均无production owner |
| test listing：Host runtime/service/architecture、service relay/current-directory、client caller | PASS，当前覆盖与缺口如§3/§8 |
| Internals `git status --short --branch` | clean，未写入 |
| build/browser/canonical workflow/live/stable | 按任务禁止，未运行 |
