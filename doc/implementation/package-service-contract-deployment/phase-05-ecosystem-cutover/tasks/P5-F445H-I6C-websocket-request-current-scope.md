# P5-F445H-I6C WebSocket request current-scope consumer

状态：Ready。消费I6-A carrier，把`requestJsonToConnection` pending registry迁移到调用点current
execution scope；普通四个send保持同步、non-suspending。

## 直接父节点

- `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
- `P5-F445H-D2-websocket-peer-cancel-hard-cut-result.md`
- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`

## 固定输入

```text
base commit  8db08c539acaf0b3fc41733365f06e9883bdbdd8
base tree    71123064dd0948d5946ad8c6312df909670794e0
```

## 实现要求

1. `requestJsonToConnection(connectionId, method, value)`三参数Skiff API保持不变。
2. `RuntimeConnectionRequestParts`与registry install接收I6-A invocation carrier导出的current child
   scope全部signals和absolute deadline，不再只保存request root token/deadline。
3. Pending winner顺序为ancestor/internal stop、有效deadline、response；同刻不得把internal stop物化成
   用户error。
4. 本地CAS先settle、删除pending并释放timer/lease，再尝试可丢失的internal stop hint。
5. Hint失败或peer未收到不影响本地terminal；不发送`$/cancelRequest`，没有`-32800`。
6. Late/duplicate response返回`complete=false`；wrong runtime session、connection/generation继续
   fail closed，不能恢复其它pending。
7. 普通四个send继续`PreparedNativeCall::Ready`，不增加lease、timer、yield或等待。

## 允许写集

Production：

```text
runtime/capability-context/src/connection_request.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
```

Tests：

```text
runtime/capability-context/src/connection_request_tests.rs
runtime/host/src/eval_capability_adapter/websocket.rs
runtime/host/src/eval_capability_adapter/factory.rs
runtime/host/src/capability_context/websocket.rs
```

## 禁止写集

- Router/profile/request broker、wire schema；
- peer cancellation、transport ID业务投影；
- std/native公开签名、business identity fan-out request；
- E4 actual-Pending、I6-A/B/D、Cargo/lockfile。

## 任务内并行

父Agent可派最多两个有界子Agent且子Agent不得继续委派：

1. capability-context registry owner：只读核对或在独立worktree修改
   `connection_request.rs`及其tests；
2. 父Agent冻结registry内部签名后，Host adapter/test-only consumer可作为互不重叠分片。

父Agent统一集成并负责session/generation、settlement priority及完整验证。若需要Router/wire/public API，
立即返回`TASK_SCOPE_EXPANDED`。

## Test-first与验证

RED：

- root active而derived child deadline/ancestor stop时pending不醒；
- wire/registry仍使用root deadline；
- internal hint失败会否阻止本地terminal；
- late response重开pending的既有负例必须保留。

GREEN：

- current signals/deadline唤醒；
- CAS先settle，pending/timer/lease归零；
- late/duplicate/wrong session/generation不命中；
-三参数与四个同步send语义不变；
- no peer cancel。

命令：

```bash
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture
cargo check -p skiff-runtime-capability-context -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

所有listing非零且与execution一致；不得运行完整gate、server/network/stable/live/MongoDB。

反向搜索：

```bash
rg -n '\\$/cancelRequest|-32800|Request cancelled' runtime router
rg -n "requestJsonToConnection" std doc/reference/std-surface.md
rg -n "RuntimeConnectionRequestParts|registry\\.install" runtime/host/src runtime/capability-context/src
```

Production/profile保持无peer cancel；所有install分类为current scope或明确无request的test fixture。

## 交付

Implementation提交后新增
`P5-F445H-I6C-websocket-request-current-scope-result.md`并提交。Result记录精确tree、RED/GREEN、
实际写集、pending生命周期、反向搜索及I6-J WebSocket case是否解除。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i6c-websocket
branch   codex/p5-f445h-i6c-websocket
```

最终clean；不得merge/rebase/push。五分钟内开始production修改，范围扩张时停止。
