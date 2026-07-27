# P5-F440W WebSocket RPC Host dispatch / outcome

状态：Ready。E0b2；把F440T request、F440U pinned target和F440V typed kernel接入Host。

## 直接父节点

- `P5-F440V-websocket-rpc-typed-evaluation-result.md`
- `P5-F440U-websocket-rpc-pinned-method-target-result.md`
- `P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`
- `P5-F440Q-websocket-rpc-invocation-linkage-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md` §§8–9

四个checkpoint分别冻结typed execution API、generation-pinned target、inbound wire和Host
session/cancellation capability。F440U在`assembly_wire`保留的显式`Unsupported`是本leaf必须替换的唯一
production入口。

实现基线为`b3ca0f0e`对应的current integration tree。

## 目标

Host收到F440T `RuntimeAssemblyRequest::WebSocketJsonRpc`后：

1. 从current captured Router session与request metadata取得exact acquired connection pin；
2. 调F440U resolver得到old-generation pinned method target；
3. 以target的execution image/activation构造F440V要求的current eval/capability context；
4. 传入opaque params、trusted connection id与optional business identity执行typed kernel；
5. 映射为F440T response end；
6. cancel/disconnect只清理执行，不发普通response。

本leaf不修改typed kernel、target、wire outcome、Router broker/gateway或artifact schema。完成后E0闭合并解除
R0b。

## 唯一写集

- `runtime/host/src/host/request_entry.rs`
- `runtime/host/src/host/request_entry/{assembly.rs,assembly_wire.rs}`
- 新建`runtime/host/src/host/request_entry/websocket_jsonrpc.rs`
- `runtime/host/src/eval_capability_adapter/{mod.rs,assembly_request_adapter.rs}`
- 可在同目录新建由HTTP/JSON-RPC共同消费的private
  `assembly_execution_context.rs`，用于避免复制第三套execution/capability context构造
- Host request-entry/router-session/generation范围内的直接fixtures/tests
- 本leaf result

禁止修改runtime/request/eval production、loader/generation target语义、transport/wire、Router、
artifact/compiler/native/std、fixture/tooling、其它task/result。不得派子Agent，不得运行live/server。

## Request validation与source trust

Host必须在执行前复验：

- request routing assembly identity/generation与captured session pin一致；
- connection id、physical `WebSocketEntryId`、host/path/method、method gateway identity、profile均由F440U
  resolver exact join；
- mode unary、payload present、request id/correlation使用现有request supervisor；
- business identity只来自validated Router metadata/connection receipt，不从peer params读取；
- params payload保持opaque交给F440V，不在Host做第二套业务decode；
- target/execution context使用pinned old activation，不调用current assembly。

malformed/mismatched target在handler前通过现有request failure/control路径fail closed，不得把另一个service、
connection或generation的target暴露给eval。

如果generation registry拥有connect时冻结的business identity，wire metadata必须与其一致；如果current
canonical trust model只让captured Router session拥有该字段，则result必须用代码路径证明peer payload无法
写入且伪造runtime session不能调用本entry，不得默默选择较弱来源。

## Execution context

复用current Host assembly execution/capability construction：

- pinned target的execution image与activation owner；
- request-scoped cancellation、effective deadline、budget/heap limits；
- current DB/file/http/websocket/native capability adapter；
- F440Q已冻结的captured Router writer/session与connection request registry；
- test-effect finalization保持F440V seam。

`assembly_request_adapter.rs`已很长；不得复制HTTP adapter的完整context builder形成第三套近似实现。若确有
共同构造逻辑，在允许的新private module提取，HTTP行为与测试保持不变。

## Outcome与唯一terminal

精确映射：

| F440V terminal | F440T response |
| --- | --- |
| `Outcome::Success { payload }` | `response.end.websocketJsonRpc.success`，payload present |
| `InvalidParams` | `response.end...invalidParams`，payload absent |
| `InternalError` | `response.end...internalError`，payload absent |
| `DeadlineExceeded` | `response.end...deadlineExceeded`，payload absent |
| internal `Cancelled` | 不编码response，调用existing cancelled terminal cleanup |

cancel/peer disconnect/runtime session loss与handler completion竞争时，existing request supervisor的首个terminal
唯一生效：

- cancellation分支biased优先；
- pending/timer/lease/active execution先归零，再做任何best-effort transport action；
- late success/outcome丢弃；
- 不发送`response.error`、`response.end`或JSON-RPC cancelled platform error；
- deadline仍可产生唯一`deadlineExceeded` response；
- response encode/send失败只走现有transport failure cleanup，不重开execution。

## Test-first与验证

先把F440U的显式Unsupported改为direct test期望真正dispatch，使旧实现真实失败，再实现。至少覆盖：

- record/array params success与exact payload；
- void success含exact `null`；
- invalid params/internal throw/deadline四类wire mapping/presence；
- business failure union仍success；
- pinned A后active B，真实Host dispatch返回A的`"old"`；
- wrong session/connection/generation/physical/method identity在eval前拒绝；
- cancel与peer disconnect不发任何ordinary response；
- cancel-vs-complete、cancel-vs-deadline、late completion最多一个terminal；
- response send failure与session disconnect后pending/timer/lease归零；
- shared execution context提取后HTTP聚焦测试不回归。

必跑：

```bash
cargo test -p skiff-runtime-host websocket_jsonrpc --no-fail-fast
cargo test -p skiff-runtime-host websocket_generation --no-fail-fast
cargo test -p skiff-runtime-host connection_request --no-fail-fast
cargo check -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

先列出/确认具名Host selectors实际非零；result记录每条执行count。Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若完成真实dispatch必须修改Router、wire、target或typed kernel，提交仍有效的Host context/entry检查点并返回
`TASK_SCOPE_EXPANDED`；不得吞入R0b。若business identity trust owner无法从current pin/session唯一证明，
停止并报告精确候选，不自行降级。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440w-rpc-host-dispatch`
- branch：`codex/p5-f440w-rpc-host-dispatch`
- result：`P5-F440W-websocket-rpc-host-dispatch-result.md`

Implementation与result分开提交；不merge/rebase/push。
