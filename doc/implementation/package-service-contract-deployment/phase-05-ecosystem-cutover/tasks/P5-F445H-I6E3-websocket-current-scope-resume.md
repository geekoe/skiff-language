# P5-F445H-I6E3 WebSocket request current-scope consumer resume

状态：Ready。消费 E1 carrier，把 `requestJsonToConnection` 的 pending registry迁移到调用点
current scope，并建立第一条 native projection到真实pending consumer的纵向闭环。

## 直接父节点

- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6C-websocket-request-current-scope-result.md`
- `P5-F445H-D2-websocket-peer-cancel-hard-cut-result.md`
- `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md`

## 固定输入

```text
E1 implementation commit  ba66719e03cbabde2e159b94761cc1a1c71b35d2
E1 implementation tree    0b1972158d710c4355274f7fb272be292dcc7927
integration base commit   e942efa99460ea2b9bf29f07d8dfe855c9715aff
integration base tree     46abc10c8fbdab6e70f2ea071539382dbf03a1be
```

## 行为要求

1. `requestJsonToConnection(connectionId, method, value)`公开三参数不变；普通四个send保持同步、
   non-suspending。
2. `RuntimeConnectionRequestParts`不再冻结request root token/deadline；registry install取得E1
   carrier在调用时导出的current scope全部signals、absolute deadline与clock。
3. pending winner为current ancestor/internal stop、有效deadline、response。internal stop只作为内部
   终止，不物化成用户错误。
4. 本地compare-and-set先settle、删除pending、释放timer/lease，再尝试可丢失internal stop hint。
5. hint失败或peer未收到不影响本地terminal；不得发送`$/cancelRequest`，不得新增`-32800`。
6. late/duplicate response返回`complete=false`；wrong runtime session、connection/generation fence
   保持不变。
7. response先提交时不得被同刻scope signal覆盖；所有terminal路径owner计数归零。

## 唯一写集

```text
runtime/capability-context/src/connection_request.rs
runtime/capability-context/src/connection_request_tests.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/eval_capability_adapter/assembly_execution_context.rs
runtime/host/src/eval_capability_adapter/carrier_delivery_tests.rs
runtime/host/src/eval_capability_adapter/mod.rs
runtime/host/src/capability_context/websocket.rs
runtime/host/src/host/router_session/tests.rs
```

只允许 `factory`、assembly caller、router session direct install做必需机械跟随。不得修改E1 Eval
wrapper、Router/wire、public std/native签名或业务identity。

## 纵向真实receipt

`carrier_delivery_tests.rs`必须在同一测试中经过：

```text
native projection
-> RuntimeNativeWebsocketCapabilityContext
-> E1 WebsocketRequestCapabilityApi
-> RuntimeWebsocketRequestCapabilityContext
-> concrete WebsocketCapabilityContext
-> ConnectionRequestRegistry::install
-> PendingConnectionRequest::wait
```

在registry真实pending后触发derived deadline或ancestor stop，断言lower wait被drop、late response
不提交、lease/timer/waiter归零。只检查adapter收到carrier不能替代此证据。

## 测试

```text
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture
cargo check -p skiff-runtime-capability-context -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

真实RED/GREEN必须覆盖deadline、ancestor stop、response竞争、late/duplicate、session/generation fence和
owner归零；两个selector listing均非零。

## 停止与禁止

若需要Router/wire、peer cancel、第四个业务参数、hint acknowledgement、公开cancel/error或E1共享接口
变更，提交 `TASK_SCOPE_EXPANDED` result并停止。禁止full gate、stable/live/network/Mongo、
merge/rebase/push。

## 完成

分开提交implementation/tests与
`P5-F445H-I6E3-websocket-current-scope-resume-result.md`。result必须给出纵向receipt、commit/tree、
非零计数、fence/owner矩阵、实际写集，并标明 `I6_WEBSOCKET_COMPLETE = YES/NO`。worktree保持clean。
