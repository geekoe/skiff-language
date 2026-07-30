# P5-F440D Agine Host peer protocol checkpoint

状态：Ready。确定性Internals实现leaf。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F439C-agine-host-jsonrpc-delta-audit-result.md`

F439C提供当前代码owner和三项业务shape；父节点“明确覆盖”一节拥有冲突协议规则。

## 目标与写集

在Internals建立三项Host peer method的唯一TypeScript协议owner和canonical fixture，不实现Host socket
adapter、Skiff caller或client迁移。

唯一production/test写集：

- `/Users/geek/workspace/internals-p5-f440d-host-peer-protocol/agine/protocol/**`

Skiff侧只允许新增本leaf result。禁止修改`agine/host`、`agine/service`、`agine/client`、
`shared-client`、Skiff production或权威设计。不得派子agent。

## Canonical method与业务类型

```text
host.files.list
  params  { path?: string }
  result  { kind:"ok", value:HostBrowseDirectoryResult }
        | { kind:"invalidPath" }
        | { kind:"outsideWorkspace" }

host.files.search
  params  { path?: string, query:string }
  result  { kind:"ok", value:HostBrowseSearchResult }
        | { kind:"invalidPath" }
        | { kind:"outsideWorkspace" }

host.current-directory
  params  {}
  result  { currentDirectory:string }
```

Nested breadcrumb/file/result字段以F439C inventory为准。Transport `id`只存在outer JSON-RPC wire type，
不能进入任一params/result/nested business type。

同一protocol checkpoint还拥有browser到Agine service的三条普通HTTP contract：

```text
POST /thread/host-files/list
  payload  { chatId, mountId, toolProviderId, path? }
  success  HostBrowseDirectoryResult

POST /thread/host-files/search
  payload  { chatId, mountId, toolProviderId, path?, query }
  success  HostBrowseSearchResult

POST /toolproviders/current-directory
  payload  { toolProviderId }
  success  { toolProviderId, currentDirectory }
```

前两条必须加入canonical HTTP path/ordinary-user path registry；第三条保留现有path。HTTP payload/response
不含`eventName`、`requestId`、transport `id`、connection id或旧`refreshRequested`。

## Wire checkpoint

- 一条WebSocket text frame一个JSON-RPC 2.0 object；Host作为Skiff outbound request的peer，只接受平台生成
  的非空string request id并原样回显。
- Parse error：`-32700 / "Parse error" / id:null`。
- Invalid request或batch：`-32600 / "Invalid Request" / id:null`，batch成员不执行。
- Unknown method、invalid params、internal分别为`-32601/-32602/-32603`并回显可信string id。
- Host本地capacity和local deadline使用通用`-32000 Server busy`、`-32001 Request timed out`。
- Cancel赢得active request后，原子settle并best-effort发送原id的`-32800 Request cancelled`；Skiff侧
  tombstone可丢弃该晚到response。
- Invalid path/outside workspace是上述typed result union，不占用JSON-RPC error code。未分类Host异常只
  能成为脱敏`-32603`。
- 除`$/cancelRequest`外的合法notification忽略且无response；旧`type/requestId/payload/eventName`不是
  合法wire。
- Active或仍在settled tombstone中的duplicate id是`1002`；tombstone到期/驱逐后可复用。Fixture只记录
  预期分类，不在本leaf实现socket状态机。
- Platform error默认省略`data`；protocol type允许remote peer的可选受限`data`，但业务type不读取它。

## 实现与证据

1. 在`agine/protocol`新增唯一exported module，拥有method常量、params/result/nested types、有限platform
   error code/message registry、outer JSON-RPC types，以及上述三条HTTP path/payload/success types。
2. 新增一个canonical JSON fixture，至少覆盖：
   - 三个request与success/result-union response；
   - parse/invalid/batch/unknown/params/internal/capacity/timeout/cancel；
   - cancel notification、business notification ignore；
   - legacy字段、empty/non-string id、scalar/array params；
   - 反序完成的两个并发string id；
   - active/tombstoned duplicate与tombstone后复用的期望分类。
3. 测试必须直接读取该唯一fixture，验证method/type/error registry、HTTP path/type registry和package
   export，不复制第二份golden。
4. 反向搜索证明params/result无`id`/`requestId`，没有`platform.*`或Host私有`-32001..-32004`错误表。
5. 运行protocol package的focused tests、typecheck/syntax、diff check；不运行Host/client/full workflow。

若现有`agine/protocol`包无法在本写集内导出module/fixture，或业务shape需要第四个method，立即返回
`TASK_SCOPE_EXPANDED`，不得修改其它package。

## 交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f440d-host-peer-protocol`
- Internals branch：`codex/p5-f440d-host-peer-protocol`
- Skiff result worktree：`/Users/geek/workspace/skiff-p5-f440d-host-peer-protocol`
- Skiff branch：`codex/p5-f440d-host-peer-protocol`
- result：`P5-F440D-agine-host-peer-protocol-checkpoint-result.md`

实现与result分别提交；返回两个commit/tree、测试计数、fixture摘要和两个clean状态。不merge/rebase/push。
