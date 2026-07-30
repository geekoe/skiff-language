# P5-F440W1 WebSocket RPC Host dispatch / outcome

状态：Ready。恢复F440W；F440W0已提供同源pinned execution route context。

## 直接父节点

- `P5-F440W0-pinned-rpc-execution-route-context-result.md`
- `P5-F440W-websocket-rpc-host-dispatch-result.md`
- `P5-F440V-websocket-rpc-typed-evaluation-result.md`
- `P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`
- `P5-F440Q-websocket-rpc-invocation-linkage-result.md`

F440W已证明真实Host入口RED与business identity trust owner；F440W0新增
`websocket_jsonrpc_execution_route`，同时返回F440V target和同一old pin的method route/DB/protocol
context。不得再查询current assembly。

实现基线为`05666969`对应的current integration tree。

## 目标

替换F440U保留的WebSocket JSON-RPC `Unsupported`：

1. F440T request header投影为F440W0 resolver exact tuple；
2. 使用returned old target + method route构造current Host execution/capability context；
3. 调F440V `execute_runtime_websocket_jsonrpc`；
4. 映射F440T response end；
5. cancellation/disconnect只清理，不发ordinary response。

完成后E0 runtime/Host execution闭合并解除R0b。

## 唯一写集

- `runtime/host/src/host/request_entry.rs`
- `runtime/host/src/host/request_entry/{assembly.rs,assembly_wire.rs}`
- 新建`runtime/host/src/host/request_entry/websocket_jsonrpc.rs`
- `runtime/host/src/eval_capability_adapter/{mod.rs,assembly_request_adapter.rs}`
- 可新建同目录private `assembly_execution_context.rs`，仅抽取HTTP/JSON-RPC共同的context builder
- Host request-entry/router-session范围的direct fixtures/tests
- 本leaf result

禁止修改generation resolver、runtime/request/eval、loader/target、transport/wire、Router、
artifact/compiler/native/std、fixture/tooling、其它task/result。不得派子Agent，不得运行live/server。

## Exact dispatch

Host从captured Router session接收strict decoded request，必须：

- 使用request routing identity/generation、connection id、physical entry id、host/path/method、method
  gateway identity与profile调用F440W0 resolver；
- resolver错误在eval前fail closed；
- target与method route的activation/image/deployment owner必须一致；
- capability context只使用returned method route的DB source、service protocol identity、policy/bindings；
- params保持opaque交给F440V；
- connection id使用validated metadata；
- business identity使用captured Router-session strict header；peer params无法覆盖，伪造runtime session无法
  选择另一个connection pin；
- request id/correlation、supervisor、budget、heap、effective deadline复用current assembly request路径。

不得按selector查current active assembly、从target字段重建DB context、使用unavailable capability，或把
peer/Router id注入业务formal。

`assembly_request_adapter.rs`不得复制第三套HTTP execution-context构造。若HTTP和JSON-RPC需要同一
activation/capability join，在允许的private module提取；HTTP observable behavior/test不变。

## Outcome与terminal

| F440V terminal | F440T response |
| --- | --- |
| `Success { payload }` | `websocketJsonRpc.success` + payload |
| `InvalidParams` | `invalidParams`，无payload |
| `InternalError` | `internalError`，无payload |
| `DeadlineExceeded` | `deadlineExceeded`，无payload |
| internal `Cancelled` | existing cancelled cleanup；零response |

必须保持：

- success的JSON `null`算payload present；
-cancel/disconnect不发送`response.error`、`response.end`或cancelled outcome；
-deadline唯一发送`deadlineExceeded`；
- cancel-vs-complete/deadline biased cancellation；
- terminal先detach/清timer/lease，再做send；
- late completion无第二次write；
- response encode/send失败不重开execution。

## Test-first与验证

重建F440W记录的真实 acquired-pin Host RED，再实现。至少覆盖：

- record/array、void `null` success；
- invalid params/private throw/deadline精确outcome/payload presence；
- business failure union仍success；
- pin A后active B，真实Host request返回A handler `"old"`；
- wrong session/connection/generation/physical/method identity/profile在eval前拒绝；
- business identity来自header而非params；
- cancel、peer/runtime disconnect零ordinary response；
- cancel-vs-complete/deadline、late completion最多一次terminal；
- send failure、session close后pending/timer/lease归零；
- shared context extraction后HTTP request-entry聚焦测试不回归。

必跑：

```bash
cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast
cargo test -p skiff-runtime-host websocket_generation --no-fail-fast
cargo test -p skiff-runtime-host connection_request --no-fail-fast
cargo check -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

确认具名selector非零并记录count。Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若F440W0 route仍缺Host capability fact，停止并报告精确字段；不得current lookup。若完成需要Router/wire/eval
修改，返回`TASK_SCOPE_EXPANDED`，不得吞入R0b。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440w1-rpc-host-dispatch`
- branch：`codex/p5-f440w1-rpc-host-dispatch`
- result：`P5-F440W1-websocket-rpc-host-dispatch-result.md`

Implementation与result分开提交；不merge/rebase/push。
