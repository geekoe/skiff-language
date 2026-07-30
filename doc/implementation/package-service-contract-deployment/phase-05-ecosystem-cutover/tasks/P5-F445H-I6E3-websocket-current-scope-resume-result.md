# P5-F445H-I6E3 WebSocket request current-scope consumer resume result

状态：

```text
IMPLEMENTATION_PASS
TASK_SCOPE_EXPANDED = NO
I6_WEBSOCKET_COMPLETE = YES
```

`requestJsonToConnection(connectionId, method, value)` 的公开三参数与四个同步 send 均未改变。
E1 的 invocation carrier 现在在 Host request adapter 的真实调用点导出 current
`ExecutionScope`；`RuntimeConnectionRequestParts` 只保存 registry/session，不再冻结 request-root
token 或 deadline。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| integration base | `e942efa99460ea2b9bf29f07d8dfe855c9715aff` / `46abc10c8fbdab6e70f2ea071539382dbf03a1be` |
| task publication HEAD | `8628b37cd0056c550ef62ab40a5aa3e54b06baab` / `754b9ac86ead0a6012e06720024a4ee9ced5ece0` |
| implementation commit | `dbab9c4bc90ff167d4a266cf9209f80f1544b334` |
| implementation tree | `8ec34f0dd61455695d48ecba54e63065f6208912` |

固定 E1 implementation `ba66719e03cbabde2e159b94761cc1a1c71b35d2` /
`0b1972158d710c4355274f7fb272be292dcc7927` 已包含在 integration base 中。

## 2. 实现与纵向 receipt

`ConnectionRequestRegistry::install` 现在按值接收完整 `ExecutionScope` 并立即取得唯一
`ExecutionScopeLease`。pending 的本地 CAS winner 是 response、current ancestor/internal stop 或
effective absolute deadline。CAS 成功后统一按以下顺序收束：

```text
settled CAS
-> remove registry pending
-> release registry timer/lease counters
-> complete/drop ExecutionScope lease (timer/waiter/lease归零)
-> deliver local terminal
-> stop winner才尝试可丢失 internal hint
```

response 的 CAS 先提交时，即使 scope deadline 同刻 ready，response 仍是唯一 terminal。hint 返回
`Err` 不影响本地 terminal；late/duplicate response 返回 `false`。没有新增 peer acknowledgement、
`$/cancelRequest`、`-32800` 或公开 cancellation error。

`f445h_i6_websocket_scope_native_projection_reaches_real_pending_and_ancestor_closes_it` 在同一测试中
经过：

```text
project_runtime_native_capability_context(Websocket)
-> RuntimeNativeWebsocketCapabilityContext
-> E1 WebsocketRequestCapabilityApi
-> RuntimeWebsocketRequestCapabilityContext
-> concrete WebsocketCapabilityContext
-> ConnectionRequestRegistry::install(current ExecutionScope)
-> PendingConnectionRequest::wait
```

测试在 derived current scope 的真实 registry pending 建立后触发 ancestor stop，观察 native lower
wait 返回内部 `AncestorCancelled`，随后断言 registry pending/timer/lease 与 scope
timer/waiter/lease 全部为零、late response `complete=false`，并只收到既有 best-effort internal
hint。

## 3. fence / winner / owner 矩阵

| 路径 | winner / fence | 可观察结果 | owner 归零证据 |
| --- | --- | --- | --- |
| derived deadline | scope lease effective absolute deadline | `DeadlineExceeded`；hint reason `deadline_exceeded` | registry `0/0/0`，scope lifecycle `0/0/0` |
| ancestor/internal stop | full current scope ancestor signals | 内部 `AncestorCancelled`；不物化普通用户错误 | registry `0/0/0`，scope lifecycle `0/0/0` |
| response first at ready deadline | registry CAS | 原 response 保留，duplicate `false`，不发 hint | registry与scope均归零 |
| hint sender failure | local CAS已完成 | 本地 terminal 不变 | registry与scope均归零 |
| late/duplicate response | pending entry已删除/settled | `complete=false` | 无 owner 重建 |
| wrong router session | exact `ConnectionRequestSession` equality | forged completion `false`；原 session disconnect收束 | registry `0/0/0` |
| connection/generation | 既有 exact generation pin/tuple owner不变 | old/current route只解析到各自 pin，错 generation fail closed | 聚焦 generation fence test PASS |
| pending/registry drop | Drop 走同一 local settle | waiter获得内部 terminal或 transport terminal | registry与scope均归零 |

`RuntimeConnectionRequestParts` 的 root `CancellationToken` / `deadline` 字段已删除；assembly caller 与
rebinder 只做必要 constructor 跟随。wire deadline 仅在 operation start 从 current scope 的
effective absolute deadline 导出，保留既有 wire shape。

## 4. RED / GREEN 与验证

恢复前的固定代码事实由父节点 `P5-F445H-I6C-websocket-request-current-scope-result.md` 记录：
Host adapter 丢弃 E1 carrier，并从 `RuntimeConnectionRequestParts` 读取 request-root token/deadline；
因此 native call-point derived scope 无法成为 pending winner。E1 checkpoint 解除了缺失内部 seam，
本提交没有建立 root snapshot 或 task-local 影子路径。

最终候选证据：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --list` | `7 tests` |
| `cargo test -p skiff-runtime-capability-context f445h_i6_connection_request_scope -- --nocapture` | `7 passed` |
| `cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --list` | `6 tests` |
| `cargo test -p skiff-runtime-host f445h_i6_websocket_scope -- --nocapture` | `6 passed` |
| `cargo test -p skiff-runtime-host websocket_jsonrpc_target_matches_websocket_jsonrpc_execution_route_for_old_context -- --nocapture` | `1 passed`；generation fence |
| `cargo check -p skiff-runtime-capability-context -p skiff-runtime-host --locked` | PASS；仅既有 warnings |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

两个合同 selector listing 均非零且 listing/execution 数量一致。测试覆盖 deadline、ancestor stop、
response 竞争、late/duplicate、hint failure、session fence、generation fence 与全部 owner 归零。

## 5. 实际写集与边界

implementation commit 恰好修改合同允许的 9 个文件：

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

没有修改 E1 Eval wrapper、Router/wire、public std/native signature、业务 identity、Cargo manifest 或
lockfile。没有运行 full gate，没有访问 stable/live/network/MongoDB，没有 merge、rebase 或 push。

```text
I6_WEBSOCKET_COMPLETE = YES
```
