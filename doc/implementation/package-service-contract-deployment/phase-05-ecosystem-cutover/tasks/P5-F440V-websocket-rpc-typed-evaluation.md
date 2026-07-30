# P5-F440V WebSocket RPC typed evaluation kernel

状态：Ready。E0b1；只实现pinned target上的typed params/handler/result kernel，不接Host wire。

## 直接父节点

- `P5-F440U-websocket-rpc-pinned-method-target-result.md`
- `P5-F440T-inbound-runtime-assembly-websocket-rpc-wire-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md` §8.3
- `P5-F440Q-websocket-rpc-invocation-linkage-result.md`

F440U target已持有pinned eval target、exact linked entry/signature/address、adapter source plan与
formal/return type owner。F440T冻结wire outcome。本leaf只消费这些既定接口，不重查artifact/current
assembly。

实现基线为`9cc5d012`对应的current integration tree。

## 目标

在`runtime/request`/`runtime/eval`提供可由后继Host调用的单一执行API：

```text
(pinned RuntimeAssemblyWebSocketJsonRpcTarget,
 opaque params bytes,
 connection id,
 optional business identity,
 current eval/cancellation/deadline context)
  -> success opaque result bytes
   | invalidParams
   | internalError
   | deadlineExceeded
   | internal cancelled terminal
```

执行必须：

1. 防御性验证params是UTF-8 JSON object/array；
2. 按target冻结的adapter source与formal `RuntimeTypePlan`构造每个参数；
3. 调exact pinned handler，遵守其真实suspension/capability/effect语义；
4. 按linked return plan编码result，`void`精确为JSON `null`；
5. 不泄漏private error type/message/stack；
6. cancellation为不可响应的内部terminal，不加入wire outcome。

本leaf不修改Host request entry、Router、wire、loader/pin或public std。完成后冻结E0b2 Host所消费的
execution/outcome API。

## 唯一写集

- `runtime/request/src/lib.rs`
- 新建`runtime/request/src/websocket_jsonrpc_execution.rs`
- `runtime/eval/src/lib.rs`
- 新建`runtime/eval/src/runtime_websocket_jsonrpc.rs`
- 可在`runtime/eval/src`内提取一个由HTTP/JSON-RPC共同消费的private linked-handler execution helper，
  仅当能避免复制current invocation/context逻辑且不改变HTTP行为
- 上述范围的colocated tests
- 本leaf result

禁止修改Host、transport/wire、Router、artifact/compiler、loader/generation、native/std、fixture/tooling、
其它task/result。不得派子Agent，不得运行live/server。

## Adapter sources

对target已验证的每个formal参数按exact adapter plan处理：

- `websocket.jsonRpcParams`：使用完整opaque params payload，按该formal linked type plan decode；
- `websocket.connectionId`：构造exact string runtime value；
- `websocket.businessIdentity`：构造`string?`，缺失为canonical none；
- peer payload不能覆盖connection/business identity；
- formal name集合、source name集合与target冻结结果必须再次防御性一致；缺失/重复/未知source在handler前
  fail closed。

params只接受object/array；scalar、null、malformed UTF-8/JSON、超限或type decode失败均为
`invalidParams`，不调用handler。Router profile已验证shape，但runtime必须独立防御。

不得把Router request id、peer id、transport session或trace id构造成业务参数。

## Handler/result与outcome

- 调用exact target linked callable/executable address，不从current image按name重查；
- 普通return按return plan编码为UTF-8 JSON；
- `void`编码字节精确为`null`；
- nominal record/array/union保持current boundary codec；
- expected business failure由return union表达，仍是`success`；
- uncaught custom throw、invalid return encode、non-UTF8/oversized result统一为小型`internalError`；
- handler或codec的private message/type/stack不得进入outcome；
- effective deadline赢时为`deadlineExceeded`；
- ancestor/peer cancellation赢时返回内部cancelled terminal，后继Host不得编码任何response；
- cancellation与deadline同时ready时遵守current biased cancellation规则；
- late handler completion不能把value注入已cancelled execution。

不要增加`cancelled`、`remote`或任意字符串outcome。

## Test-first与验证

先增加typed execution direct test，使current缺少module/API而compile red，随后实现。至少覆盖具名矩阵：

- record params、array params；
- malformed/scalar/null/type mismatch -> invalidParams且handler未调用；
- connectionId与businessIdentity来源不可被payload覆盖；
- normal string/record/array/union result；
- void -> exact `null`；
- expected return union仍success；
- uncaught throw、return encode失败、oversized result -> internalError且无message/stack；
- deadline -> deadlineExceeded；
- cancel/disconnect -> internal cancelled，零response payload；
- cancel-vs-deadline biased cancel；
- pinned old A target在active replacement B后执行A handler返回`"old"`；B target返回`"new"`。

必跑：

```bash
cargo test -p skiff-runtime-request websocket_jsonrpc_execution
cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc
cargo check -p skiff-runtime-eval
cargo fmt --all -- --check
git diff --check
```

若selector名称不同，先确认非零执行并在result记录count。Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若正确执行必须修改Host capability construction或request-entry lifecycle，冻结纯eval API与通过的direct
tests并返回`TASK_SCOPE_EXPANDED`；不得吞入E0b2。若F440U target缺少必要linked fact，返回最小target字段
缺口，不从artifact/current assembly重查。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440v-rpc-typed-eval`
- branch：`codex/p5-f440v-rpc-typed-eval`
- result：`P5-F440V-websocket-rpc-typed-evaluation-result.md`

Implementation与result分开提交；不merge/rebase/push。
