# P5-F440W1 WebSocket RPC Host dispatch / outcome result

状态：`Completed`。

F440T 的 strict `request.start.runtimeAssembly.websocketJsonRpc` 已接入真实 Host
执行路径。Host 只使用 captured Router session 与 F440W0 返回的同一 old-generation
`target + method_route`；没有回查 current assembly，也没有修改 generation resolver、
runtime request/eval、loader/target、transport/wire 或 Router production。

## 1. 基线与提交

| 项目 | Commit |
| --- | --- |
| 任务声明的 integration baseline | `05666969` |
| 实际实现父节点（只多 F440W1 恢复文档） | `ad1bf7bb` |
| implementation | `4160dc7f` |
| result | 本文提交 |

Worktree：
`/Users/geek/workspace/skiff-p5-f440w1-rpc-host-dispatch`

Branch：
`codex/p5-f440w1-rpc-host-dispatch`

## 2. Exact pinned dispatch

`assembly_wire.rs` 的 WebSocket JSON-RPC 分支不再返回 F440U 留下的
`Unsupported`，而是把 strict decoded header 精确投影为 F440W0 resolver 参数：

- captured `router_session_id`；
- `connection_id`；
- routing assembly identity / generation；
- physical `websocket_entry_id`；
- ingress host / path / method；
- method gateway entry identity；
- closed typed profile `jsonrpc-2.0-text`。

resolver 错误在 supervisor/eval 创建前 fail closed。resolver 返回后，Host 还验证
target 与 method route 的 assembly、selector、method identity、deployment owner、
implementation build、activation 指针及 execution image 指针一致。

pin A 后 active replacement B 的 direct Host 测试通过真实 binary request 证明仍执行
A handler 并返回精确 `"old"`。JSON-RPC admission source audit 证明该路径不调用
`lookup_active_assembly`、`active_runtime_assembly_route` 或
`assembly_admission`。

## 3. 同源 execution / capability context

新增 private `assembly_execution_context.rs`，把 HTTP、WebSocket connect 与
WebSocket JSON-RPC 共用的 assembly execution context 提取为一个构造器，而不是复制第三套
HTTP context：

- activation、execution image、config/package config；
- exact route DB source；
- exact route service protocol identity；
- route activation owned bindings 对应的 capability，以及 route policy clamp后的 deadline；
- file/http/outbound/actor/spawn/WebSocket capability；
- captured Router writer/session 与 shared connection request registry；
- request heap limit、telemetry、test-effect context。

Host 输入构造只从 F440W0 返回的 old `method_route`读取上述事实；不从 target 字段重建
DB context，也不使用 `unavailable()`降级 capability。共享 context 的 HTTP 聚焦回归
最终为 `6 passed / 302 filtered`。

opaque params 原样交给 F440V。业务 formal 的 connection id 来自 resolver 已验证的 strict
header；business identity 来自 captured Router-session header。identity direct test 证明
params 中伪造的 connection/business 字段只作为 params 字段可见，不能覆盖 formal。

## 4. Outcome 与 terminal owner

新增 `request_entry/websocket_jsonrpc.rs` 调用
`execute_runtime_websocket_jsonrpc`，并精确映射：

| F440V terminal | F440T response |
| --- | --- |
| `Success { payload }` | `websocketJsonRpc.success`，保留 payload |
| `InvalidParams` | `invalidParams`，空 payload |
| `InternalError` | `internalError`，空 payload |
| `DeadlineExceeded` | `deadlineExceeded`，空 payload |
| `Cancelled` | supervisor cancelled cleanup，零 response |

effective deadline 复用 assembly route policy clamp，并由 existing supervisor budget /
F440V kernel执行。success 的 JSON `null`保持为 present payload。

普通 terminal 先通过 supervisor claim/detach，再 encode/send；cancel 已抢占时
`complete_success`返回 false，late completion不能写第二次。response encode/send failure
只结束当前 execution，不重开。old method route 一直持有到 terminal settlement 之后。

direct Host 测试覆盖：

- record、array、void `null` success；
- business failure union仍为 success；
- invalid params、private throw、deadline精确 outcome/payload presence；
- cancel-vs-late-completion与 cancel-vs-deadline均零 ordinary response；
- peer-cancel reason、session disconnect及 writer send failure；
- terminal 后 supervisor active、connection request pending/lease/timer、outbound
  pending/lease均归零。

## 5. Test-first 记录

先在真实 acquired generation A pin、active generation B 的 Host binary入口重建 F440W RED：

```text
cargo test -p skiff-runtime-host \
  websocket_jsonrpc_host_dispatches_pinned_method_instead_of_unsupported \
  -- --nocapture
```

RED 实际执行 `1`：`1 failed / 300 filtered`。旧实现返回 ordinary
`response.error`，typed JSON-RPC decoder 因出现 `errorKind` 而拒绝；不是 compile failure、
零测试或 synthetic resolver probe。

完成实现后，同一 selector 返回 A handler 的 `"old"`并 GREEN。

## 6. 最终验证

所有 Cargo 命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 验证 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast` | `17 passed / 291 filtered` |
| `cargo test -p skiff-runtime-host websocket_generation --no-fail-fast` | `11 passed / 297 filtered` |
| `cargo test -p skiff-runtime-host connection_request --no-fail-fast` | `8 passed / 300 filtered` |
| `cargo test -p skiff-runtime-host host_http_gateway_ --no-fail-fast` | `6 passed / 302 filtered` |
| `cargo check -p skiff-runtime-host` | exit `0` |
| `cargo fmt --all -- --check` | exit `0` |
| `git diff --check` | exit `0` |

具名 selectors 均实际执行非零测试。

额外执行了非合同 mandatory 的 full
`cargo test -p skiff-runtime-host --lib --no-fail-fast`，结果为
`303 passed / 5 failed`。五个失败都在进入本实现前由 current strict identity parser
拒绝父节点已有的旧伪 identity fixture：

- 四个未改动的 assembly activation test fixture 使用非 canonical assembly identity；
- 一个未改动且不在本 leaf 写集内的 loader test 使用全零 gateway identity。

本 leaf 没有为 supplemental baseline fixture 扩写 Router/loader 范围。允许写集内的 HTTP
negative fixture存在同类不可解析伪 identity，已改为另一条真实 admitted route 的 canonical
identity；其 fail-closed语义不变，HTTP selector最终 `6/6` GREEN。

## 7. 范围与收尾

- production只修改任务允许的 Host request-entry与 eval capability adapter文件；
- tests只修改/新增 Host request-entry/router-session direct fixtures/tests；
- 未修改 generation resolver、runtime request/eval、loader/target、transport/wire、Router、
  artifact/compiler/native/std；
- 未运行 live/server、stable instance、watch或chat smoke；
- 未派子 Agent；
- implementation与result分开提交；
- 未 merge、rebase或push。
