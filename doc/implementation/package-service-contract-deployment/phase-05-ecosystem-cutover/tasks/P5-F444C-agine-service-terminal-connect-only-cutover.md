# P5-F444C Agine service terminal connect-only cutover

状态：Ready。Internals S1 implementation leaf；Phase 05 最后一个已知 production owner。

## 直接父节点

- `P5-F444A-agine-service-terminal-owner-preflight-result.md`
- `P5-F444B-host-authenticated-http-producer-cutover-result.md`

只从这两个 result 沿引用读取必要的当前设计与旧 owner证据。不得恢复 F439C 已被 F440D 覆盖的
错误码，也不得把 outbound Host method写进 `websocket.yml.jsonRpc`。

## 输入

| Repo | Root | Expected commit |
| --- | --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` | `534e75f1` |
| Internals integration | `/Users/geek/workspace/internals-phase-05-integration` | `19d4100` |
| skiff-packages integration | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `19cfab5d` |

三棵输入必须 clean。

## 终态

### Authoring

- `service.yml`精确只含：

  ```yaml
  id: agine.ai/api
  ```

- 新 `http.yml` 精确43条 direct `rawHttp` mapping：
  - 当前原36条；
  - `/thread/host-files/list`、`/thread/host-files/search`；
  - `/host/hello`、`/host/activation-ack`、`/host/ping`、
    `/host/tool-attempts`、`/host/tool-call/result`。
- 新 `websocket.yml` 精确为 `/ws` + `internal.agine_connect.acceptConnection`，
  adapter只绑定 `websocket.connectRequest`；没有 `jsonRpc`、routes、operation、receive或message。
- `config.dev.yml`继续唯一拥有120秒 timeout。
- `api.yml`精确为空 mapping；external handler/connect和private Host peer类型都不是Package API。

### Connect、HTTP 与 Host RPC

1. `agine_connect.acceptConnection`使用当前非泛型
   `std.websocket.WebSocketConnectRequest/Result`；accept result没有旧 `context`字段，继续完成
   browser/Host admission、business identity、max-one policy和 exact active connection持久化。
2. Browser Host files list/search：
   - cookie session + thread/mount/provider/capability授权；
   - 从当前 `ToolProvider.activeConnectionId`取得唯一connection；
   - 在当前HTTP execution内以15秒deadline调用
     `std.websocket.requestJsonToConnection` 的 `host.files.list/search`；
   - typed business union直接投影HTTP成功或固定错误，不建DB relay、不使用业务 request id。
3. Current directory：
   - 验证owner、active Host provider、presence、`host.files.v1`和 exact connection；
   - 同步调用 `host.current-directory`并返回 `{toolProviderId,currentDirectory}`；
   - 不返回 `refreshRequested`，不轮询、不 detached refresh；metadata可同步刷新但不能短路请求。
4. 五条 Host HTTP upcall使用从connect抽出的唯一严格Host header/auth owner：
   - browser session authority与Host header冲突时 fail closed；
   - body不能提交owner、business identity或connection id；
   - 保留现有Host activation、presence、attempt reconciliation、tool settlement业务语义；
   - HTTP response直接承接 hello/ack/ping、attempt actions和tool receipt。
5. private peer params/result/union放在 `internal/host_peer_protocol.skiff`；严格对照唯一
   `agine/protocol/fixtures/host-peer-jsonrpc-v1.json`，不得出现 transport `id` / `requestId`。
6. caller投影完整处理当前
   `WebSocketRequestError`、`TimeoutError`、`std.json.DecodeError`、remote integer code；
   remote message/data不进入public response或日志。ancestor cancellation不可捕获、不可包装为成功或
   `ApiError`。

### 删除旧图

按 F444A §5 删除/替换：

- 所有 `internal/agine_ws_*.skiff` raw receive dispatcher；
- `internal/host_file_rpc.skiff`和 `model.HostFileBrowseRequest`及indexes；
- `model.ChatStreamConnection`、connection cache/scan；
- `internal.agine_service.websocket`统一 ingress facade；
- `agine_transport`中只服务旧request envelope的decoder/send helper；
- current-directory refresh/polling；
- `api/agine.skiff` 中只由receive消费的 event DTO、`ClientMessage`、`ServerEnvelope`。

保留 server -> browser/Host 单向 notification、exact
`ToolProvider.activeConnectionId`、HostToolAttempt与所有durable tool/run/message identity。
不得为了搜索归零删除真实下行通知。

## 写入边界

只允许 `agine/service/**`。同一 Agent独占所有 service manifest/source/model/API/test/receipt文件。

禁止修改：

- `agine/protocol/**`
- `agine/host/**`
- `agine/client/**`
- `shared-client/**`
- Skiff 或 skiff-packages production/reference
- lockfile、node_modules、stable artifact/watch/config

若必须修改禁止范围、增加第四个Host peer method、恢复raw receive或引入业务 correlation，
按工作流停止并返回 `TASK_SCOPE_EXPANDED`。

## Test-first 与聚焦验证

先改 Node receipt/architecture assertions，使当前输入真实 RED，至少同时命中：

- inline manifest / 36而非43；
- list/search或五条Host route缺失；
- `requestJsonToConnection`零调用；
- raw receive、DB relay、`refreshRequested`仍存在；
- `api.yml`仍导出legacy websocket。

最早聚焦：

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs \
  agine/service/internal/host_runtime_architecture.test.mjs
```

新增或重写 service `.test.skiff`，覆盖：

- 三项peer success、list/search两个business union；
- owner/mount/capability/presence/exact connection；
- current-directory不使用cache短路；
- Host header两种认证、冲突/缺失/错误credential；
- 五条Host HTTP happy path与业务拒绝；
- `WebSocketRequestError`各branch、remote platform integer、timeout、decode、不可捕获cancel；
- connect browser/Host两类admission及current non-generic result。

使用 linked-worktree canonical isolated workflow；不得借 stable artifact root。聚焦通过后运行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  npm run type-check
```

cwd=`agine/service`。若 package 提供 canonical `npm test`且不会访问live/stable，再运行它；否则在result中
列出实际受支持入口与完整发现计数，不发明script。

终态反向搜索至少执行 F444A §6 的四组命令，并证明：

- `WebSocketIngressEvent|agine_ws_dispatch.receive|HostFileBrowseRequest|ChatStreamConnection|
  refreshRequested|requestHostCurrentDirectoryRefresh` production为零；
- 旧Host file/current-directory event wire为零；
- private peer protocol无 `id|requestId`；
- `requestJsonToConnection`与完整error projector正向命中。

## 提交与结果

Internals implementation worktree：

`/Users/geek/workspace/internals-p5-f444c-agine-service-terminal`

branch：

`codex/p5-f444c-agine-service-terminal`

提交一个聚焦 implementation commit，最终 clean。

Skiff result worktree：

`/Users/geek/workspace/skiff-p5-f444c-agine-service-terminal-result`

branch：

`codex/p5-f444c-agine-service-terminal-result`

只新增并提交：

`P5-F444C-agine-service-terminal-connect-only-cutover-result.md`

result记录RED、精确删除/新增、验证计数、反向搜索、三个候选HEAD/tree/status。不得
merge/rebase/push、stable/live/network。仅当一个阻止实现的具体未知量可在10分钟内回答时才可派一个
只读子 Agent；该子 Agent不得再派 Agent。探查证明范围扩张或仍不明确时必须停止，不得擅自扩大。
