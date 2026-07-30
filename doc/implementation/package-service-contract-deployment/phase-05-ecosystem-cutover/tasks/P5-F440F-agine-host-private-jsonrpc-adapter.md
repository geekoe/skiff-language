# P5-F440F Agine Host private JSON-RPC adapter

状态：Ready。确定性Internals实现leaf。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F440D-agine-host-peer-protocol-checkpoint-result.md`
- `P5-F439C-agine-host-jsonrpc-delta-audit-result.md`

协议冲突以F440父节点和F440D checkpoint为准。精确输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Internals | `605ebd209dacac7c95aa79dc3a508d428a352453` | `95cc84051c350f45e38a6092958d58734c5278db` |
| Skiff result | `6dbed271656cd647afe93f34c6801d23f183d183` | `dbcf6467363e0d8f128207be4b57df44e92f01b7` |

## 目标与写集

在Host进程内建立generation-bound private JSON-RPC server adapter，执行
`host.files.list`、`host.files.search`、`host.current-directory`，并删除这三项旧
`eventName/requestId` handler。它不建立公共Host framework，也不修改Skiff service/browser。

唯一production/test写集：

- `/Users/geek/workspace/internals-p5-f440f-host-jsonrpc-adapter/agine/host/**`

Skiff侧只允许新增本leaf result。禁止修改`agine/protocol`、`agine/service`、`agine/client`、
`shared-client`、Skiff production或权威设计。不得派子agent。

## 与raw business notification共存

Host socket仍会收到Skiff通过raw outbound send下发的业务notification，例如`tool_call/request`。因此
adapter不能吞掉所有合法`eventName`消息：

- 合法、非本任务退役的`eventName` object继续交给现有Host `EventManager`；
- `host/current-directory/request`、`host/files/list-request`、
  `host/files/search-request`及其旧result/envelope是退役RPC，不能再event dispatch；
- JSON-RPC object、batch、非event object和JSON parse失败由Host peer adapter处理；
- binary frame以`1003`关闭当前socket；
- JSON-RPC frame绝不能落入任意event-name fallback。

这只是同一物理socket上的明确frame demux，不恢复Skiff侧raw receive。

## Adapter状态机

每个物理socket/generation拥有独立adapter：

```text
inFlight: Map<string, Entry>
settled: bounded expiring tombstone set
closed: bool

Entry {
  identity: private object/token
  controller: AbortController
  deadline timer
  state: active | settled | cancelled
  captured socket/generation
}
```

生产默认常量：

- active上限`128`；
- tombstone上限`256`；
- local deadline`15_000ms`；
- tombstone TTL`30_000ms`。

测试可以通过module-private dependency injection收紧clock/limit，不能形成用户配置或公开protocol字段。

## 精确处理

1. Request只接受`jsonrpc:"2.0"`、非空string `id`、三个canonical method和必需object `params`。
   List只允许可选string `path`；search另要求string `query`；current-directory只接受空object。
2. Parse、invalid、batch、unknown、params、capacity、deadline、internal严格按F440D fixture回写固定error。
   能识别为合法profile request前的错误用`id:null`；合法id之后的错误回显原id。
3. List/search success把HostService结果包成`{kind:"ok",value}`；现有invalid-path/outside-workspace内部
   错误分别投影为typed result union。其它仍active的throw脱敏为`-32603`。
4. 收到合法`$/cancelRequest`：
   - active：先原子remove + tombstone + cancelled，保存captured writer/id，best-effort写`-32800`，
     再abort controller；
   - settled/tombstoned/unknown：no-op；
   - handler晚到resolve/reject不能写第二次。
5. 其它合法JSON-RPC notification忽略，不dispatch、不response。Malformed platform cancel以`1002`关闭。
6. Active或tombstoned duplicate id以`1002`关闭该generation并abort全部active；tombstone过期/最老驱逐后
   可复用。
7. 同connection请求可乱序完成。Disconnect/error/explicit close先detach captured socket，再清map、
   cancel timers并abort全部controller；不发送response。
8. Response必须直接写captured socket，禁止使用会排队/跨重连flush的普通`send`。Send throw不retry、
   不入queue；旧promise永远不能写新socket。
9. JSON-RPC response object在Host没有outbound pending时是`1002`；ping/pong/close仍由WebSocket协议栈拥有。
10. Strict outer request只允许`jsonrpc/id/method/params`。合法profile id下出现额外field返回
    `-32600`并回显id；错误version、legacy field或无法信任id时使用`id:null`。

## 代码迁移

- 新增private adapter与直接消费canonical fixture的unit tests。
- 在Host-local WebSocket/GatewayClient lifecycle中为每个raw socket构造/销毁adapter，提供direct captured
  send/close hook；不得修改`shared-client`。
- `HostRuntime`删除三项旧handler、旧timeout/error projector和对应requestId result发送；其它Host
  hello/presence/tool-attempt/tool receipt逻辑暂留给后续HTTP cutover。
- Host本地重复的browse result types应import/alias `@agine/protocol/hostPeer`，不再复制字段。
- 保留HostService/BrowseWorkspace/Ripgrep已有AbortSignal链和filesystem安全检查。

## 验证与停止规则

至少覆盖：

- fixture全部正/负分类；
- 三method成功、两个typed failure union、internal脱敏；
- cancel before/after settle、unknown cancel、timeout、capacity；
- duplicate active/tombstoned、expiry reuse、乱序；
- disconnect/reconnect late result、captured send throw、不跨generation排队；
- 合法`tool_call/request`等eventName notification仍进入EventManager；
- 三个退役Host RPC eventName不再注册或发送。

运行Host adapter/HostRuntime/HostService/Ripgrep/FileWorkspace与architecture focused tests、Host typecheck、
syntax/fmt/diff。禁止完整Internals workflow、browser、stable/live。

若实现必须修改Host之外的shared-client/protocol/service，或无法在不吞业务notification的情况下取得
captured socket，立即返回`TASK_SCOPE_EXPANDED`并保留最小证据。

## 交付

- Internals worktree：`/Users/geek/workspace/internals-p5-f440f-host-jsonrpc-adapter`
- Internals branch：`codex/p5-f440f-host-jsonrpc-adapter`
- Skiff result worktree：`/Users/geek/workspace/skiff-p5-f440f-host-jsonrpc-adapter`
- Skiff branch：`codex/p5-f440f-host-jsonrpc-adapter`
- result：`P5-F440F-agine-host-private-jsonrpc-adapter-result.md`

实现与result分别提交；返回两个commit/tree、测试计数、fixture覆盖、reverse search和两个clean状态。
不merge/rebase/push。
