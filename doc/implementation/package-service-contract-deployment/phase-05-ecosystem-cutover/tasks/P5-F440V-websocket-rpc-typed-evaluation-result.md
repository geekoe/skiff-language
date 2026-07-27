# P5-F440V WebSocket RPC typed evaluation kernel result

状态：`PASS / E0B1_TYPED_KERNEL_VALID`。

本节点在 `runtime/request` / `runtime/eval` 冻结了后继 Host 可消费的单一 typed execution
入口。入口只接收 generation-pinned method target、opaque params、connection/business identity
与当前 execution/cancellation/deadline context；它不读取 current assembly，不重新打开 artifact，
也不处理 Host、Router 或 wire。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务声明的 integration baseline | `9cc5d012f46258aa3ae96a5708c8facaeeac2fb5` | `d86b53ceac6078934b30bcbac2a85ab3d0b7b214` |
| worktree 实际起点 | `6c3acb23649b4bb28c0da47a2fe9793584f12318` | `c2d84d79b8bcfbda06c7b46714c636f603dd9d7b` |
| implementation | `07f7f292e52c732adb5efe0f86b0ccfea04256b3` | `3cc22c40fc8e60ffc509cfd11db5b2e4f83c8ddd` |

`6c3acb23` 相对 `9cc5d012` 只增加本任务声明，没有 production/test 变化。Implementation
与本文 result 分离提交；result commit/tree 由最终交付消息记录。

Worktree：
`/Users/geek/workspace/skiff-p5-f440v-rpc-typed-eval`

Branch：
`codex/p5-f440v-rpc-typed-eval`

## 2. Test-first RED

先增加 request module 与 direct test，让测试引用尚不存在的 eval outcome：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-request websocket_jsonrpc_execution -- --nocapture
```

结果为 exit `101`，production API 缺失导致真实 compile RED：

```text
unresolved import `skiff_runtime_eval::RuntimeWebSocketJsonRpcExecutionOutcome`
```

此时 selector 已命中新增测试源码；失败不是零测试、依赖下载或既有 baseline failure。

## 3. Host-callable execution seam

`runtime/request` 公开：

```text
execute_runtime_websocket_jsonrpc(
  RuntimeWebSocketJsonRpcExecutionInput {
    pinned target,
    opaque params bytes,
    connection id,
    optional business identity,
    cancellation token,
    execution budget,
    request heap limits,
    eval capability adapter
  }
) -> RuntimeWebSocketJsonRpcExecutionTerminal
```

`RuntimeWebSocketJsonRpcEvalAdapter` 只构造当前 capability / execution context。它没有
transport header、Router correlation、peer request id 或 trace id 输入，因此这些值无法进入业务
formal。request 层把 F440U target 投影成 eval-only exact target view，且不修改 target、Host
capability construction 或 request-entry lifecycle。

response-producing closed outcome 精确为：

```text
Success { payload }
InvalidParams
InternalError
DeadlineExceeded
```

`Cancelled` 位于独立的 internal terminal enum，不属于 response outcome，也没有 payload 或字符串
reason。这样后继 Host 无法把 ancestor/peer cancellation 误编码成普通 JSON-RPC response。

## 4. Exact typed adapter 与 handler

eval kernel 在 handler 前依次防御性复验：

- `ProgramExecutionContext` 与 target 的 execution image、activation `Arc`、request generation
  精确一致；
- protocol surface 为 unary `WebSocketJsonRpc`，adapter kind精确一致；
- linked executable address、function kind、non-receiver、non-generic、suspension summary 与 frozen
  signature一致；
- formal 与 adapter args 等长、名称唯一且保持 compiler-owned signature order；
- source集合与 surface `externalSources` 精确一致；
- exactly one `websocket.jsonRpcParams`，connection/business source各至多一个；duplicate、
  missing、reordered、unknown 或 HTTP/connect source均在执行前 fail closed。

params 独立执行 non-empty / 1 MiB、UTF-8、JSON parse 与 top-level object/array gate。随后按每个
formal 的 linked `RuntimeTypePlan` decode：

- `websocket.jsonRpcParams` 使用完整 params value；
- `websocket.connectionId` 只使用 runtime 提供的 trusted string；
- `websocket.businessIdentity` 使用 trusted optional string，缺失经 `null` decode 为 canonical none。

peer object中同名字段只属于 params record，不能覆盖后二者。malformed UTF-8/JSON、scalar、null、
oversize 或 params type mismatch统一为 `InvalidParams`，且不会进入 handler。

执行只调用 target 持有的 exact `ExecutableAddr`，通过 canonical
`execute_runtime_assembly_addr` 保留真实 suspension/capability/effect 语义；没有按名字查询
current image。return 按 linked return plan与现有 boundary codec编码为 UTF-8 JSON：

- string、record、array、nominal union保持 typed boundary shape；
- expected business-failure union仍为 `Success`；
- `void` 强制为精确四字节 `null`；
- codec failure、oversized output或 uncaught private throw统一为无字段 `InternalError`。

private error type、message、source与 stack没有进入任一公开 outcome。

## 5. Cancellation / deadline 唯一终态

kernel 外层使用 current biased `tokio::select!`：

1. cancellation branch；
2. effective deadline branch；
3. handler completion branch。

handler完成分支返回前仍再次按 cancel-then-deadline 顺序检查。deadline 赢时取消底层 execution，
但保留 response-producing `DeadlineExceeded`；cancel 赢时只返回 internal `Cancelled`。execution
future随终态丢弃，晚到 handler value不能覆盖已选 terminal。

request 层在 test-effect finalization 前后继续执行同一终态收敛：

- 已选 `Cancelled` / `DeadlineExceeded` 不会被 private finalization error改写；
- 尚未 settled 时 cancellation优先于同时到期的 deadline；
- finalization failure只降级成无详情 `InternalError`；
- execution budget记录对应 cancellation/deadline reason。

## 6. Old A / replacement B

direct eval test先从 active fixture A取得 generation 1 target，再把 active fixture替换为 B并取得
generation 2 target；A fixture owner随后被释放。最后才执行两个 target：

```text
A target -> "old"
B target -> "new"
```

A仍只依赖自己持有的 old execution image / activation / address。kernel没有 current-active
lookup；F440U 已在 Host generation registry层另行证明 connection pin在真实 active replacement后
仍解析 old sibling target，两层证据闭合 A/B owner链。

## 7. 规定 GREEN

所有 Cargo命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-request websocket_jsonrpc_execution` | 2 | PASS：2 passed / 35 filtered |
| `cargo test -p skiff-runtime-eval runtime_websocket_jsonrpc` | 10 | PASS：10 passed / 209 filtered；两个 integration binaries 均 0 executed（分别 4 / 6 filtered） |
| exact A/B selector | 1 | PASS：1 passed / 218 filtered；两个 integration binaries 均 0 executed |
| `cargo check -p skiff-runtime-eval` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

focused 12 项覆盖：

- record / array params；
- malformed、scalar、null、invalid UTF-8、type mismatch与oversize params；
- trusted connection/business identity与absent canonical none；
- string、record、array、union、expected-failure union与 exact void result；
- private throw、return codec mismatch与实际 public-kernel oversized result；
- deadline、disconnect/cancel、simultaneous cancel-vs-deadline；
- formal/source missing、duplicate、reordered、unknown；
- old A / replacement B exact execution。

最终命令只显示 repository既有 unused/dead-code/unreachable-pattern warnings，没有本节点 test/check
失败。

## 8. 范围与反向审计

Implementation 精确修改/新增四个任务允许文件：

- `runtime/request/src/lib.rs`
- `runtime/request/src/websocket_jsonrpc_execution.rs`
- `runtime/eval/src/lib.rs`
- `runtime/eval/src/runtime_websocket_jsonrpc.rs`

反向搜索与 staged diff确认没有新增：

- Host request entry、Host eval adapter或 capability construction；
- transport/wire DTO、response mapper或 request correlation；
- Router、gateway/server、broker；
- loader/generation、artifact/compiler/native/std；
- current assembly lookup、artifact I/O、第二套 id或字符串 outcome。

没有运行 live/server/stable instance/watch/chat smoke；未派子 Agent；未 merge、rebase或 push。
