# Runtime Loop Risk Hardening Plan

日期：2026-07-10

本文是阶段性实现方案，不是长期架构契约。目标是把 2026-07-09
`skiff-runtime` CPU 拉满暴露出的同类风险收敛成可实现、可测试、可观测的工程约束。

## 背景

本次事故的直接原因已经在 `8004727 Fix outbound stream cancel watcher lifecycle`
修复：outbound server stream 正常消费到 `End` / `Error` 后，stream cancel watcher
会通过 `CompletionSignal` 退出，不再留下 1ms 轮询任务。

但这只是同类问题中的一个点。当前 runtime / router 仍存在几类共同风险：

- 异步资源没有单一 owner 和唯一 terminal path。
- cancellation 在跨层传递时从 notify-backed token 退化成裸 `Arc<AtomicBool>`。
- stream / pending request / spawned task 缺少“最终必须归零”的测试和指标。
- router 侧 WebSocket / HTTP stream 的取消、backpressure、client close 没有统一清理入口。

因此本方案不把“再补一个局部测试”当作根治标准，而是建立 lifecycle contract。

## Goals

- 生产 hot path 不再依赖 1ms cancellation polling。
- 每个 runtime outbound request、stream registry entry、router pending request 都有唯一 terminal cleanup。
- dropped / unconsumed stream、client disconnect、backpressure、protocol error、timeout、runtime disconnect 都能触发 cancel 或 cleanup。
- 新增 counters / health snapshot，使 smoke、stress 和 e2e 可以断言资源最终归零。
- 为后续 runtime / router 开发建立 checklist：每个 `tokio::spawn`、fire-and-forget promise、registry insert 都必须声明 owner、退出条件和测试。

## Non-Goals

- 不把所有业务长连接 stream 强行加固定 lifetime timeout。Skiff 必须允许合法的长时间 serverStream。
- 不引入全局 async runtime framework 或重写 router dispatch 架构。
- 不把 telemetry 做成业务正确性依赖。指标用于诊断和测试断言，不能改变业务返回语义。
- 不兼容旧 runtime internal API。Skiff 尚未发布，内部契约应收敛到当前正确模型。

## Current Evidence

### 1. Cancellation 仍可能退化为 polling

证据：

- `runtime/capability-context/src/cancellation.rs` 中
  `CancellationToken::wait_cancelled` 对 `notify: None` 的 token 使用
  `CANCEL_POLL_INTERVAL = 1ms`。
- 同文件 `CancellationSignals::wait_cancelled` 在 borrowed flag 或 flag-backed token
  场景也会周期 sleep。
- `runtime/host/src/capability_context/stream_runtime.rs` 的
  `wait_for_external_cancel` 也包含 1ms sleep fallback。
- 多处适配层仍传 `Arc<AtomicBool>`，例如
  `runtime/host/src/capability_context/outbound_service.rs`、
  `runtime/host/src/host/http_runtime/stream.rs`、
  `runtime/host/src/capability_context/stream_runtime.rs`。

风险：高并发请求取消或 stream cancel 时，等待者数量越多，timer wakeup 越多。之前事故就是同类生命周期等待泄漏被 cancel storm 放大。

### 2. Outbound serverStream 仍缺 dropped / unconsumed 路径

已修复路径：`OutboundServiceStreamSource` 消费到 `End` / `Error` 或协议错误时，会调用
`CompletionSignal::mark_completed`，watcher 退出。

仍有风险的路径：serverStream handle 被创建后未消费、被提前 drop、或者 request scope
提前退出时，当前语义没有显式的 stream lease 来保证发 cancel / remove registry / 唤醒 watcher。

证据：

- `runtime/eval/src/service_dispatch.rs` 创建 outbound serverStream source。
- `runtime/host/src/capability_context/outbound_service.rs` 的
  `spawn_stream_cancel_task` 只等待 cancellation 或 completed。
- `runtime/capability-context/src/outbound_response.rs` 的 `OutboundRequestRegistry`
  没有 TTL / sweeper，必须靠 terminal path remove。

### 3. StreamRuntime registry 只插入，缺少 terminal remove contract

证据：

- `runtime/host/src/capability_context/stream_runtime.rs` 中 `StreamRuntime.streams`
  是 `HashMap<String, Arc<StreamState>>`。
- `channel_stream` 和 `pull_stream_with_cancellation` insert entry。
- 文件内没有统一的 `streams.remove` terminal path。

当前大多数 `StreamRuntime` 是 request-scoped，风险被 request 生命周期缓冲。但一旦
stream source、producer task 或 watcher clone 延长生命周期，就会形成泄漏放大器。

### 4. Router HTTP stream 没有 backpressure cleanup

证据：

- `router/src/router/httpGateway.ts` 的 `writeHttpFrameResponseChunk` 调用
  `response.write(...)` 后忽略返回值。
- `router/src/router/runtimeDispatcher.ts` 的 `RuntimeBinaryStreamHandlers` 是同步
  `void` handler，dispatcher 无法等待 downstream drain。
- 协议已有 `backpressure` cancel reason 语义，但 HTTP stream path 没有使用。

风险：慢客户端会让 Node response buffer 增长；runtime 继续产出 chunk，router 侧没有把 backpressure 转成 cancel。

### 5. Router pending lifecycle 分散

证据：

- `router/src/router/runtimeDispatcher.ts` 中 `completePending` 只做 map cleanup。
- `rejectPendingWithError` 在 protocol / callback error 场景只 reject，不保证向仍在工作的 runtime 发送 `request.cancel`。
- `response.start` 后清 timeout，这可以支持长 stream，但必须由 client disconnect、backpressure、runtime end/error/disconnect 等信号兜底。

风险：router 已经忘记 pending request，但 runtime 仍可能在执行或产出 stream。

### 6. WebSocket receive dispatch 缺少 in-flight 上限和 close abort

证据：

- `router/src/gateway/webSocketGateway.ts` 的 `ws.on('message')` 直接调用
  `handleClientMessage(...).catch(...)`，没有等待和 in-flight 上限。
- `pendingMessages` 只覆盖连接验证前；verified 后每个 message 都可以发起 runtime dispatch。
- `ws.on('close')` 只清索引和 connection state，没有 abort 运行中的 receive / connect dispatch。
- `dispatchReceive` 调 runtime dispatcher 时没有传 `AbortSignal`。

风险：一个浏览器 tab 或脚本可以制造 receive cancel storm；client close 后 runtime 请求可能等默认 timeout 才释放。

## Root Cause Model

这类问题不是“某个 loop 写错”这么简单，而是生命周期契约不完整：

```text
resource created
  -> watcher / stream / pending / spawned task starts
  -> normal completion OR cancel OR drop OR disconnect OR protocol error
  -> cleanup exactly once
  -> waiters wake without polling and observe terminal state
  -> counters return to zero
```

当前缺的是中间几条边。只要任意 terminal edge 漏掉，cancel storm、慢客户端或未消费 stream
就会把漏掉的等待者放大成 CPU / memory / pending request 问题。

## Camp Principle Check

本次方案直接触碰这些代码路径：

- runtime cancellation signal：`runtime/capability-context/src/cancellation.rs`。
- runtime stream runtime：`runtime/host/src/capability_context/stream_runtime.rs`。
- outbound service stream：`runtime/eval/src/service_dispatch.rs`、
  `runtime/host/src/capability_context/outbound_service.rs`。
- router dispatch pending lifecycle：`router/src/router/runtimeDispatcher.ts`。
- router HTTP gateway：`router/src/router/httpGateway.ts`。
- router WebSocket gateway：`router/src/gateway/webSocketGateway.ts`。

会被本功能继续放大的架构卫生问题：

- 裸 `Arc<AtomicBool>` 同时表达 state 和 waitable event。
- registry insert / remove 分散在多个 error branch。
- router pending cleanup 和 runtime cancel 分离。
- fire-and-forget async work 没有 owner。

这些必须作为本次实现的一部分处理，不能降级成 follow-up。允许作为 follow-up 的只有非 hot path
的历史 borrowed flag API 彻底删除，因为部分测试 helper 和旧 adapter 可以先保留兼容入口，但必须标注上限并通过 grep 验收。

## Target Contracts

### Contract A: Race-Free Waitable Cancellation

新增或收敛到 request-scoped waitable cancellation 类型，建议在
`runtime/capability-context/src/cancellation.rs` 内完成，不新建万能 crate：

```rust
pub struct CancellationSource {
    inner: Arc<CancellationState>,
}

pub struct CancellationToken {
    inner: Arc<CancellationState>,
}

struct CancellationState {
    cancelled: AtomicBool,
    notify: Arc<Notify>,
}

pub struct CancellationSignalSet {
    tokens: Vec<CancellationToken>,
}
```

规则：

- `CancellationSource` 是唯一取消 owner；`CancellationToken` 是 cloneable wait/check handle。
- Dropping `CancellationSource` does not imply cancellation. Request/stream owner terminal paths must
  call `cancel()` or another explicit terminal signal before dropping the source.
- `CancellationSource::cancel()` 必须幂等，只在 `false -> true` 状态迁移时调用
  `notify.notify_waiters()`，唤醒全部 waiter。
- 原子顺序使用 `AcqRel`/`Acquire` 或更强的 `SeqCst`；实现必须保证 `cancel()` 先发布
  terminal state，再通知 waiter。
- `CancellationToken::wait_cancelled()` 必须使用防丢通知模式：
  1. 先检查 `cancelled`，已取消则立即返回。
  2. 创建 `notify.notified()`，pin 后调用 `enable()`。
  3. 再次检查 `cancelled`，避免取消发生在注册前后导致丢通知。
  4. await notified；醒来后循环复查 flag。
- `CancellationSignalSet::wait_cancelled()` 对每个 token 都按同一模式 enable，再复查所有 flag，
  等任一 token 通知后返回或循环复查，不能用固定 sleep 作为组合等待机制。
- 生产 hot path 传 `CancellationToken` 或 `CancellationSignalSet`，不传 `Arc<AtomicBool>`。
- `cancel_flag()` 只能作为只读快速检查或外部 legacy adapter 输入，不作为等待原语。
- `from_flag` / `from_flags` 保留时必须命名为 compatibility / polling fallback，并限制在测试、外部 borrowed adapter 或有明确上界的路径。
- `wait_cancelled()` 不得在 notify-backed token 上使用固定 1ms sleep。
- 需要组合多个取消源时使用 signal set，它等待任一 token 的 notify，而不是循环 sleep。

必须新增并发测试：

- pre-cancelled token：waiter 立即返回。
- cancel racing with waiter registration：取消发生在第一次检查和 `enable()` 前后都不会挂住。
- multiple waiters：一次 cancel 唤醒所有 waiter。
- signal set multi-token race：任一 token 取消都能唤醒 set waiter，且不依赖 timer。

剩余 polling fallback 必须进入 allowlist。每个 allowlist 项必须写明：

- 文件 / 函数。
- 为什么不能立即改成 notify-backed token。
- 最大并发上限或调用频率上限。
- 对应 counter，例如 `cancellation.flag_backed_waiters.active`。
- 退出测试或后续删除任务。

没有 allowlist 的 `from_flag` / `from_flags` / 1ms sleep 命中不允许合并。

### Contract B: OutboundRequestLease And Stream Lease

为每个 `OutboundRequestRegistry::insert` 引入 RAII terminal owner。serverStream 在通用
request lease 上再持有 stream-specific terminal signal，建议放在
`runtime/eval/src/service_dispatch.rs` 或 `runtime/capability-context` 中较窄的 outbound 模块。

Ownership graph：

```text
OutboundStartedRequest owns OutboundRequestLease
OutboundServiceStreamSource owns OutboundRequestLease for serverStream
stream cancel watcher owns OutboundStreamTerminalSignal only
router writer channel owns actual control-frame delivery
```

`OutboundRequestLease` 不可 clone。watcher、source 和 tests 如果需要观察 terminal，只能 clone
terminal signal，不能持有 lease，避免额外引用延迟 Drop。

职责：

- 持有 `request_id`、`OutboundServiceContext`、`CompletionSignal`、一次性 terminal state 和
  nonblocking cancel sender。
- `complete()`：用于 unary response terminal 或 stream `End` / `Error` 后同步标记 terminal、
  remove registry、notify completed，不发送 cancel。
- `cancel(reason)`：用于 protocol error、decode error、external cancel 等显式异步路径；
  它先同步 terminal，再通过已有 router writer channel 发送 cancel。
- `Drop`：只允许做同步幂等 terminal、registry remove、notify，以及通过 nonblocking sender
  enqueue cancel。`Drop` 不得 `await`，不得新建无 owner 的 `tokio::spawn`。
- cancel sender 必须是不会因数据 backpressure 返回 `Full` 的控制通道。当前可使用
  router writer 的 unbounded control channel；如果未来改为 bounded channel，必须在本方案内
  增加 `cancel_send_failed_full` counter、full-path 测试和明确的丢弃语义后才能合并。
- 如果 Drop 时 router sender 已关闭，lease 记录 `outbound_stream.cancel_send_failed_closed`
  counter；此时 runtime/router 已断连，不能用无 owner task 尝试补发。
- stream watcher 等待 lease 的 completed/cancelled terminal signal，而不是等待一个没有 owner 的裸 signal。

验收规则：

- 每个 `OutboundRequestRegistry::insert` 都能追到一个 lease。
- unary response end/error、stream `End`、stream `Error`、decode error、seq mismatch、
  request cancel、stream drop、router disconnect 都只能 terminal 一次。
- dropped / unconsumed serverStream 测试必须看到 outbound cancel 或
  `outbound_stream.cancel_send_failed_closed` counter，
  且 registry、lease、watcher 计数归零。
- unary outbound request 测试必须看到 normal response、provider error、caller cancel、
  timeout 和 router disconnect 后 registry / lease 归零。
- 重复 terminal race 测试必须覆盖 `complete()` 与 `Drop`、`cancel()` 与 `Drop` 同时发生。

### Contract C: StreamRuntime Registry Cleanup

`StreamRuntime` 应拥有显式 registry terminal API：

```rust
impl StreamRuntime {
    fn finish_stream(&self, id: &str, terminal: StreamTerminalReason);
    pub fn active_stream_count(&self) -> usize;
}
```

规则：

- `finish_stream` 必须幂等，只有第一次 terminal transition 执行 remove / notify / counter。
- `End` / `Error` / cancel / unknown source drop 都要调用 `finish_stream`。
- `StreamSink` 和 pull source 持有 stream id 或 lease；producer drop 时能唤醒 consumer。
- consumer terminal 后必须通知 producer；producer 后续 `send` / `end` / `fail` 看到 terminal
  后返回 cancelled，不得阻塞在 channel reserve/send。
- `next_with_cancellation` 在 terminal 后不只设置 `ended`，还要 remove registry entry。
- request-scoped `StreamRuntime` drop 仍是兜底，但测试不能只依赖 drop 整个 runtime。

### Contract D: Router PendingLifecycle

在 `runtimeDispatcher.ts` 中收敛 pending terminal 操作，并明确 stream pending state：

```ts
type StreamPendingState = 'waitingStart' | 'streaming' | 'terminal';

type PendingTerminalSource =
  | 'runtime_response_end'
  | 'runtime_response_error'
  | 'runtime_request_cancel'
  | 'timeout'
  | 'caller_abort'
  | 'client_disconnect'
  | 'backpressure'
  | 'protocol_error'
  | 'callback_error'
  | 'runtime_disconnect'
  | 'router_shutdown';

type PendingTerminal =
  | { source: PendingTerminalSource; kind: 'completed' }
  | { source: PendingTerminalSource; kind: 'failed'; error: unknown }
  | { source: PendingTerminalSource; kind: 'cancelled'; reason?: RequestCancelReason };

finishPending(requestId, pending, terminal)
```

规则：

- `response.start` 不是 terminal。它只把 stream pending 从 `waitingStart` 推进到
  `streaming`，pending 必须保留到 `response.end` / `response.error` / client disconnect /
  backpressure / timeout / runtime disconnect。
- 每个 stream pending 拥有一个 `StreamWriteOwner`，负责 downstream frame 顺序和 writer
  abort。`finishPending` 必须先 terminal pending，再调用 writer owner 的 silent close API。
- timeout、caller abort、client disconnect、runtime disconnect、protocol error、callback error、backpressure、normal end 都走同一个 terminal helper。
- helper 根据 pending kind、stream state、terminal source 和 runtime socket状态派生是否发送
  `request.cancel`；调用方不得传 `cancelRuntime` 布尔值。
- `completePending` 退化为内部 primitive，不允许业务分支直接调用它绕过 cancel decision。
- `pending` counters by kind 在 helper 内维护。
- 对 stream pending，normal `response.end` 是唯一 completed terminal source；HTTP stream
  completed terminal 必须等 end frame 经 `StreamWriteOwner` 成功 flush 后才调用
  `finishPending`。`response.start` 后的 HTTP gateway / WebSocket gateway 错误必须通过
  `finishPending` 关闭 runtime 侧工作。

Terminal decision matrix：

| Terminal source | Send `request.cancel` to runtime? | Wire reason | Close writer owner? |
| --- | --- | --- | --- |
| `runtime_response_end` | No, runtime already ended | none | Yes, silent completed close |
| `runtime_response_error` | No, runtime already errored | none | Yes, silent failed close |
| `runtime_request_cancel` | No, runtime initiated cancel | none | Yes, silent cancelled close |
| `timeout` | Yes if runtime socket is open | `timeout` | Yes, silent cancelled close |
| `caller_abort` | Yes if runtime socket is open | `caller_cancel` | Yes, silent cancelled close |
| `client_disconnect` | Yes if runtime socket is open | `client_disconnect` | Yes, silent cancelled close |
| `backpressure` | Yes if runtime socket is open | `backpressure` | Yes, silent cancelled close |
| `protocol_error` | Yes if runtime socket is open | `protocol_error` or mapped fallback | Yes, silent failed close |
| `callback_error` | Yes if runtime socket is open | `protocol_error` or mapped fallback | Yes, silent failed close |
| `runtime_disconnect` | No, socket is gone | none | Yes, silent cancelled close |
| `router_shutdown` | Yes if runtime socket is open | `router_shutdown` | Yes, silent cancelled close |

For forwarded runtime-originated requests, the same matrix applies to the target runtime socket; the
caller runtime receives a response error/cancel only after the forwarded pending is terminal.

### Contract E: Backpressure-Aware HTTP Stream

把 HTTP stream writer 改成 per-request sequential writer owner：

```ts
interface StreamWriteOwner {
  enqueueStart(frame): void;
  enqueueChunk(frame): void;
  enqueueEnd(frame): void;
  markEndReceived(): void;
  requestTerminal(source: PendingTerminalSource, error?: unknown): void;
  closeFromPendingTerminal(terminal: PendingTerminal): void;
}
```

规则：

- Dispatcher handlers remain synchronous and may only enqueue frames into `StreamWriteOwner`.
  They must not create unowned promises.
- `StreamWriteOwner` owns the async write/drain promise chain, catches errors, and calls
  `requestTerminal(...)` when drain timeout、client close、write failure or queue error happens.
- `finishPending` may call only `closeFromPendingTerminal(...)`; that method must never call
  `finishPending` / `requestTerminal` again. This prevents `finishPending -> owner close ->
  finishPending` recursion.
- Runtime `response.end` arrival is not itself a completed terminal for HTTP stream output. The
  dispatcher enqueues the end frame, calls `markEndReceived()`, and stops accepting further runtime
  frames. Only after the writer queue flushes all previous chunks and the end frame successfully
  flushes does `StreamWriteOwner` request terminal with source `runtime_response_end`.
- If client disconnect、backpressure timeout、write failure or protocol error happens while queued
  chunks/end are pending, that error wins the terminal race and queued frames are dropped by the
  writer owner after terminal.
- `writeHttpFrameResponseChunk` can be async internally, but its promise is owned by
  `StreamWriteOwner` queue.
- 如果 `response.write` 返回 `false`，等待 `drain`。
- 等待 drain 时绑定 client disconnect signal 和 configurable `backpressureDrainTimeoutMs`。
- 所有 `response.start` / `response.chunk` / `response.end` frame 必须进入同一个
  `StreamWriteOwner` 队列，保证顺序，禁止后续 chunk 绕过正在等待 drain 的 chunk。
- writer owner 被 terminal 后拒绝后续 frame，并保证只请求一次 terminal。
- drain timeout 或 buffer 超阈值时，writer owner 调 router terminal helper，使用 reason
  `backpressure` 取消 runtime request。
- client close 时 writer owner 调 router terminal helper，使用 reason `client_disconnect`
  取消 runtime request。
- `backpressureDrainTimeoutMs` 默认 10s；测试可配置为 50ms。buffer threshold 初版使用
  Node `response.write` 的 `false` 作为 backpressure 信号，不另设字节阈值；若后续增加字节阈值，
  必须加入 Contract H reason mapping 和 router tests。

### Contract F: WebSocket Receive Dispatch Owner

WebSocket gateway 增加 connection-level dispatch owner：

- 每个 verified connection 有 `inFlightReceives` 和 `receiveQueue`。
- 默认同一 connection 同时只允许一个 receive dispatch；需要并发时必须显式配置上限。
- queue 复用 `MAX_PENDING_CONNECTION_MESSAGES` 或拆出 verified queue 上限。
- 每个 receive / connect dispatch 创建 `AbortController`，传给 runtime dispatcher。
- socket close 时 abort 所有 running dispatch，并清空队列。
- 已 dispatch 到 runtime 的 receive/connect 在 abort 后必须进入 `finishPending`，source 为
  `client_disconnect`，并按 matrix 向 runtime 发送 `request.cancel`。未 dispatch 的 queued
  message 只做本地 reject/drop，不发送 runtime cancel。
- queue overflow 关闭连接或返回 429 等价错误，不能无限并发。WebSocket 场景优先发送
  service error frame；如果协议阶段无法发送错误 frame，则用 policy violation close code
  关闭 socket。

### Contract G: Health Counters

第一阶段不要求完整 telemetry 产品化，但 runtime / router 必须提供测试可读的 health snapshot：

Runtime counters：

- `outbound_requests.pending`
- `outbound_stream_leases.active`
- `stream_runtime.streams.active`
- `cancellation.flag_backed_waiters.active`
- `spawned_tasks.active`（只覆盖 request-scoped / 本次触碰的 spawned task，不包含合法长生命周期 runtime loops）

Router counters：

- `dispatcher.pending.unary|stream|forward`
- `http_stream.backpressure_waiters`
- `http_stream.backpressure_cancels`
- `websocket_receive.in_flight`
- `websocket_receive.queued`
- `websocket_receive.abort_on_close`

规则：

- T2/T3 focused tests 可以先使用模块内部 active count；T7 负责把这些 count 收敛成统一
  in-memory health snapshot / test hook，随后再接入 observability。
- stress 测试必须断言 counters 最终归零。
- 日志只能辅助排查，不能作为唯一验收。

Stable instance 可读接口：

- 扩展 router control endpoint：
  `GET /__router/health?detail=loop-risk`。
- 返回 JSON 追加 `loopRisk` 字段；普通 `/__router/health` 可保持当前轻量摘要。

最小 schema：

```json
{
  "loopRisk": {
    "observedAt": "2026-07-10T00:00:00.000Z",
    "router": {
      "dispatcher": { "pendingUnary": 0, "pendingStream": 0, "pendingForward": 0 },
      "httpStream": { "backpressureWaiters": 0, "backpressureCancels": 0 },
      "websocketReceive": { "inFlight": 0, "queued": 0, "abortOnClose": 0 }
    },
    "runtimes": [
      {
        "runtimeId": "runtime-...",
        "connected": true,
        "fresh": true,
        "counters": {
          "outboundRequestsPending": 0,
          "outboundStreamLeasesActive": 0,
          "streamRuntimeStreamsActive": 0,
          "flagBackedCancelWaitersActive": 0,
          "spawnedTasksActive": 0
        }
      }
    ]
  }
}
```

Runtime counters reach router through a new runtime-to-router control frame:

```ts
type RuntimeHealthFrame = {
  type: 'runtime.health';
  runtimeId: string;
  observedAt: string;
  counters: {
    outboundRequestsPending: number;
    outboundStreamLeasesActive: number;
    streamRuntimeStreamsActive: number;
    flagBackedCancelWaitersActive: number;
    spawnedTasksActive: number;
  };
};
```

Runtime sends `runtime.health`:

- immediately after `runtime.registered`;
- every 1s while the runtime session is connected, even when all counters are zero;
- once more when all counters transition to zero;
- before orderly runtime session close when possible.

Router stores the latest frame per runtime session and includes `fresh: true` when
`Date.now() - Date.parse(observedAt) <= 5000`. Disconnected runtimes remain in the detail endpoint
for at least 30s with `connected: false` and their last counters; after that they may be omitted
only if their last counters were all zero. Stress verification records the touched runtime ids at
the start of the run, waits up to 5s after input stops for each touched runtime to report all active
counters as zero and fresh, and fails immediately if a touched runtime disconnects unexpectedly or
disappears before reporting fresh zero.

### Contract H: Cancel Reason Mapping

本次实现必须把 terminal reason 映射收敛到单点，避免不同模块发散命名：

| Situation | Router/runtime reason |
| --- | --- |
| User/request caller abort | `caller_cancel` |
| Client socket/HTTP connection closed | `client_disconnect` |
| Router request timeout | `timeout` |
| Runtime/service deadline exceeded | `deadline_exceeded` |
| HTTP downstream backpressure timeout/overflow | `backpressure` |
| Runtime/router stream protocol violation | `protocol_error` |
| Outbound stream handle dropped before consumption | `stream_dropped` |
| Runtime WebSocket disconnected | `runtime_disconnect` |
| Router shutdown | `router_shutdown` |

如果协议枚举尚未包含某个 reason，实现要么扩展枚举，要么在一个显式 mapper 中降级到现有 wire reason，同时保留内部 counter 的原始 reason。

## DAG Task Plan

### T0. Shared cancel reason mapper

依赖：无。

改动：

- 使用独立前置 worktree / branch 落地 shared cancel reason contract，覆盖 runtime、router 和
  transport/protocol 需要的 reason。
- 扩展或集中映射 `RequestCancelReason`，覆盖 Contract H。
- 如果 wire protocol 暂不扩展 `protocol_error` / `stream_dropped` / `deadline_exceeded`，
  必须在一个 mapper 中明确降级到现有 wire reason，并保留 internal reason counter。

验证：

- Router protocol type-check。
- Rust transport/runtime mapper 单测。
- Reason mapper 单测：Contract H 表格中的每个 situation 都有稳定输出。

### T1. Runtime cancellation token cleanup

依赖：T0 已合并。

改动：

- 把 request supervisor / execution control / outbound / stream runtime 的 hot path 改为传
  `CancellationToken`。
- 引入 `CancellationSource` 或等价 owner，定义 cancel 幂等状态迁移、`notify_waiters`
  全量唤醒和防丢等待算法。
- 标记 `from_flag` / `from_flags` 为 compatibility fallback。
- 删除 hot path 中 `wait_cancelled` 的 1ms sleep。

验证：

- `cargo test -p skiff-runtime-capability-context cancellation -- --nocapture`
- 新增 pre-cancel、registration race、multiple waiters、signal set race 测试。
- grep `CANCEL_POLL_INTERVAL`、`from_flag`、`from_flags`，确认生产 hot path 不再新增 polling。
- 新增 polling fallback allowlist 文档或测试 fixture；无 allowlist 的 production polling 命中即失败。

### T2. OutboundRequestLease

依赖：T0 已合并；T1 可并行开始，但最终应使用 waitable token。

改动：

- 为所有 outbound request registry insert 引入 `OutboundRequestLease`。
- 为 outbound serverStream source 引入 stream-specific terminal signal。
- `End` / `Error` / protocol error / decode error / drop 均走 lease terminal；Drop 只做同步
  terminal 和 nonblocking cancel enqueue。
- watcher 从等待裸 `CompletionSignal` 改为等待 lease terminal。
- watcher 只持有 terminal signal，不持有 lease。

验证：

- `cargo test -p skiff-runtime-host outbound_service -- --nocapture`
- 新增 dropped / unconsumed outbound serverStream 测试。
- 新增 unary outbound normal/error/cancel/timeout/disconnect cleanup 测试。
- 新增 repeated terminal race、sender closed drop fallback 测试，断言
  `outbound_stream.cancel_send_failed_closed`。
- 若 cancel sender 被改为 bounded channel，必须新增 full fallback 测试；否则测试断言 sender 不会因 full 失败。
- 新增 registry count / lease count / watcher count 最终归零断言。

### T3. StreamRuntime terminal cleanup

依赖：无，可与 T2 并行。

改动：

- `StreamRuntime` 增加 `finish_stream` 和 active count。
- channel / pull stream 在 `End` / `Error` / cancel / drop 后 remove entry。
- producer / consumer 双方通过 notify 唤醒，避免挂住。
- `finish_stream` 幂等，重复 terminal 不重复 remove / notify / counter。

验证：

- `cargo test -p skiff-runtime-host stream_runtime -- --nocapture`
- 新增 `stream_runtime_removes_entry_on_end_error_cancel_drop`。
- 新增 consumer terminal wakes producer / repeated terminal idempotent tests。

### T4. Router pending lifecycle helper

依赖：T0 已合并。T4 和 T5 必须在同一 router branch 内一起验收合并；不能只合并 T4 的半套 pending contract。

改动：

- `runtimeDispatcher.ts` 引入 `finishPending(..., terminal)`。
- 所有 pending terminal 分支迁移到 helper。
- helper 根据 terminal source matrix 派生 runtime cancel、writer close 和 counter 更新，调用方不得传 `cancelRuntime`。
- protocol / callback error 按 matrix 发送 cancel。

验证：

- `pnpm --filter @skiff/router type-check`
- `pnpm --filter @skiff/router test`
- 新增 timeout、callback error、protocol error、runtime disconnect 都 cleanup + cancel 的单测。
- 新增 `response.start` 后 pending 仍保留，直到 `response.end` / cancel 才 terminal 的单测。
- 新增 matrix 单测，覆盖每个 terminal source 的 cancel/no-cancel 和 writer close 行为。
- 新增 forwarded runtime-originated request 测试，覆盖 forward pending 的 target cancel、
  target runtime disconnect、caller response/error 行为。

### T5. HTTP stream backpressure

依赖：T4，同一 branch 验收。

改动：

- HTTP stream chunk writer async 化，并引入 per-request `StreamWriteOwner` 队列。
- 等待 `drain`，绑定 client disconnect 和 drain timeout。
- 超限时通过 T4 helper cancel runtime，reason 为 `backpressure`。
- Dispatcher handler 只同步 enqueue，async Promise 归 `StreamWriteOwner` 所有。

验证：

- `pnpm --filter @skiff/router type-check`
- 新增慢 client 测试：制造 `response.write` false，断言不会无限 buffer，runtime 收到 cancel。
- 新增 frame ordering 测试：等待 drain 的 chunk 不能被后续 chunk / end 绕过。
- 新增 end flush 测试：chunk 等 drain 时收到 runtime end，必须先 flush chunk，再 flush end，
  最后才 completed terminal。
- 新增 pending terminal abort writer 测试：client close / backpressure 后 writer queue 关闭且只 cancel 一次。
- 新增 recursion guard 测试：`finishPending` silent close writer 不会再次调用 terminal helper。
- `pnpm --filter @skiff/router test`

### T6. WebSocket receive owner

依赖：T4。

改动：

- verified WebSocket connection 增加 receive queue / in-flight limit。
- receive / connect dispatch 使用 `AbortController`。
- socket close abort running dispatch 并清空 queue；已 dispatch 请求必须通过 T4 matrix 发送
  `client_disconnect` cancel。

验证：

- `pnpm --filter @skiff/router type-check`
- 新增 WebSocket close during receive / queue overflow / serial receive 测试。
- 新增 close abort sends runtime cancel 测试；queued-but-not-dispatched messages 不发送 runtime cancel。
- 本地 cancel storm fixture：同一 connection 快速 send + close 1000 次，router / runtime counters 归零。

### T7. Health snapshot and stress tests

依赖：T1、T2、T3、T4、T5、T6。

改动：

- runtime / router test hooks 暴露 counters。
- 扩展 `GET /__router/health?detail=loop-risk` schema。
- 增加 `runtime.health` control frame，把 runtime counters 汇总到 router。
- 加一个跨 runtime-router 的 focused stress 测试。
- 更新本地验证脚本，使 chat smoke 后能检查 counters。

验证：

- `cargo test --manifest-path runtime/Cargo.toml --no-fail-fast`
- `pnpm --filter @skiff/router test`
- stable instance rebuild/restart 后运行 Agine chat smoke。
- `curl 'http://127.0.0.1:4001/__router/health?detail=loop-risk'` 可以读取 schema。
- cancel storm 后 runtime CPU 回落，且 touched runtime counters 在 5s 归零窗口内归零；
  touched runtime 非预期断连或消失视为失败。

## Worktree And Agent Plan

实现阶段建议拆成四个 worktree，减少冲突并支持独立验收：

1. `skiff-loop-risk-reason-mapper`
   - 负责 T0。
   - 主要文件：shared runtime protocol / transport mapper、router protocol mapper、runtime reason mapper。
   - 必须先合并回 main；后续 runtime/router worktree 都基于这个提交创建。

2. `skiff-runtime-cancellation-lifecycle`
   - 负责 T1、T2、T3。
   - 主要文件：`runtime/capability-context`、`runtime/host/src/capability_context`、
     `runtime/eval/src/service_dispatch.rs`。

3. `skiff-router-pending-lifecycle`
   - 负责 T4、T5、T6。
   - 主要文件：`router/src/router/runtimeDispatcher.ts`、
     `router/src/router/httpGateway.ts`、`router/src/gateway/webSocketGateway.ts`。
   - T4/T5 必须同分支同批验收，不允许只合并 pending helper 而没有 writer owner。

4. `skiff-loop-risk-observability-tests`
   - 负责 T7 和跨模块 stress / smoke 验收。
   - 在 runtime/router lifecycle 分支合并后启动，避免 test hook 目标漂移。

每个 worktree 都应在本地提交，验收通过后 ff merge 回 `skiff/main`，再删除 worktree 和已合并分支。push 仍需显式用户确认。

## Test Matrix

Rust focused tests：

- `cargo test -p skiff-runtime-capability-context cancellation -- --nocapture`
- `cargo test -p skiff-runtime-host outbound_service -- --nocapture`
- `cargo test -p skiff-runtime-host stream_runtime -- --nocapture`
- `cargo test -p skiff-runtime-eval --lib -- --nocapture`

Router focused tests：

- `pnpm --filter @skiff/router type-check`
- `pnpm --filter @skiff/router test`

Integration / smoke：

- Rebuild dev runtime when runtime code changes:
  `node scripts/build-dev-runtime.mjs`
- Restart stable runtime:
  `node scripts/skiff.mjs instance restart .skiff-instance/config.yml runtime`
- Read loop-risk health:
  `curl 'http://127.0.0.1:4001/__router/health?detail=loop-risk'`
- Run Agine smoke:
  `npm run e2e:chat-smoke` in `/Users/geek/workspace/internals/agine`
- After chat smoke, check loop-risk health from this repository:
  `node scripts/check-loop-risk-health.mjs --url http://127.0.0.1:4001/__router/health?detail=loop-risk --timeout-ms 5000`

Stress acceptance：

- 1000 cancelled WebSocket receive attempts do not leave router pending entries.
- Runtime `outbound_requests.pending`、`outbound_stream_leases.active`、
  `stream_runtime.streams.active` return to zero.
- Touched runtime ids remain connected through the stress run; unexpected disconnect is failure.
- No repeating `runtime.request_error` cancel storm after clients stop.
- Runtime CPU returns to idle range after the stress input stops: sample once per second for 30s;
  median must be below 5% and no sample may remain above 25% after the first 10s grace window on
  the local stable instance.

## Grep Acceptance

Before final merge of the implementation series:

- `rg "tokio::spawn|spawn\\(" runtime/host/src runtime/eval/src`
  - every touched spawn has owner, stop signal or join/abort handle.
- `rg "CancellationToken::from_flag|CancellationSignals::from_flags|CANCEL_POLL_INTERVAL|Duration::from_millis\\(1\\)" runtime`
  - every remaining production hit appears in the polling fallback allowlist with owner、bound、counter and test.
- `rg "register_outbound_response|outbound_requests\\.insert|OutboundRequestRegistry::insert" runtime`
  - every outbound request registration reaches an `OutboundRequestLease` terminal path.
- `rg "finishPending\\(|completePending\\(|rejectPendingWithError\\(|sendCancel\\(" router/src/router/runtimeDispatcher.ts`
  - direct terminal branches go through the lifecycle helper and no caller passes a `cancelRuntime` boolean.
- `rg "response.write\\(" router/src/router/httpGateway.ts`
  - stream writes are backpressure-aware.
- `rg "ws.on\\('message'|handleClientMessage|AbortController" router/src/gateway/webSocketGateway.ts`
  - verified receive dispatch has in-flight limit and close abort.
- `rg "stream_dropped|client_disconnect|backpressure|protocol_error|runtime_disconnect" runtime router`
  - terminal reasons are emitted through the documented mapper, not ad hoc strings.
- `rg "StreamWriteOwner|requestTerminal|closeFromPendingTerminal" router/src`
  - async stream writes are owned by writer queue, and pending silent close cannot recurse into terminal helper.

## Rollout

1. Land T0 shared reason mapper first.
2. Land runtime lifecycle changes. They remove the original CPU-leak class at the source and add counters.
3. Land router lifecycle changes. They prevent browser/client storms from amplifying runtime work.
4. Land stress / health assertions last. They turn the regression class into a CI-visible failure.
5. Rebuild and restart local stable runtime, then run Agine chat smoke and WebSocket cancel stress.

## Residual Risks

- Long-lived serverStream remains allowed. Correctness relies on downstream disconnect/backpressure/cancel and counters, not a mandatory lifetime timeout.
- Some borrowed `AtomicBool` compatibility paths may remain temporarily. They must be visibly named as polling fallback and kept out of high-cardinality hot paths.
- Backpressure behavior can change router response timing. Tests must cover chunk ordering and normal streaming latency, not only cancellation.
- Drop-based cleanup must be idempotent because normal completion and drop can race. Lease terminal state must use a single atomic transition.
- If runtime health frame delivery is delayed, stable-instance health can briefly show stale counters; verification uses a 5s zero window and observed timestamp.

## Definition Of Done

This hardening is complete when:

- The implementation tasks T0-T7 are merged.
- Focused Rust and router tests pass.
- T0 reason mapper tests pass, and terminal reason grep emits only documented mapper paths.
- Agine chat smoke passes after rebuilding and restarting stable runtime.
- Cancel storm stress leaves runtime/router counters at zero.
- The grep acceptance list has no unexplained hot-path polling or bypassed pending cleanup.
- The implementation documents each touched spawned task / fire-and-forget promise with owner, terminal condition and test coverage.
- Cancellation race tests prove notify-backed waiters cannot miss cancellation and do not rely on timer polling.
- `GET /__router/health?detail=loop-risk` exposes the documented schema and all active counters return to zero after stress.
