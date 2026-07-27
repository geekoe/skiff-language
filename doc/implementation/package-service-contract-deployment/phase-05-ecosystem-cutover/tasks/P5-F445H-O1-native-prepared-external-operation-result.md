# P5-F445H-O1 Native prepared external operation result

状态：`IMPLEMENTATION_COMPLETE / NATIVE_GREEN / EVAL_COMPATIBLE`。

O1 已在 native dispatch owner 内建立 `prepare -> owned wait -> finalize`
协议。外部 wait 不借 caller `RequestHeap`、`Env` 或 `EvalContext`；纯同步
route 直接返回 `Ready`，外部 route 的 Ready/Pending 只能由后继 evaluator
真实首次 poll 判断。既有 async dispatch 入口只组合新协议，没有保留第二套
route 状态机、静态 pre-suspend 或 `may_suspend` 调度。

## 1. 输入与提交

| 项 | 值 |
| --- | --- |
| production prerequisite | `d39ad5b0` |
| task document checkpoint | `87e85911` |
| implementation | `70598c80e26c3afd942f08ba0590bac4f92a5b01` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared` |
| branch | `codex/p5-f445h-o1-native-prepared` |

直接父结果：

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

production/test 写集保持在任务允许的 `runtime/native/src/dispatch/**`；
没有修改 eval、host、service-db、artifact/native semantics、manifest 或
lockfile。

## 2. Test-first 证据

先加入 ownership 测试，要求 sleep prepared wait 存活时仍可独立
mutate caller heap，再执行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target \
  cargo test -p skiff-runtime-native dispatch -- --nocapture
```

得到预期 RED，exit `101`：

- `PreparedNativeCall` 尚不存在；
- `TimeNativeDispatch::prepare` 尚不存在。

这证明旧 API 只能把 `&mut RequestHeap` 留在整个 async dispatch future
中，不能交给 E3 actual-Pending seam。随后才实现 prepared protocol 和完整
route 测试矩阵。

## 3. Prepared protocol

`dispatch/prepared.rs` 提供：

```text
PreparedNativeCall =
  Ready(RuntimeValue)
  | ExternalWait(PreparedExternalNativeOperation)

PreparedExternalNativeOperation::into_parts()
  -> (NativeExternalWait, NativeExternalFinalize)
```

合同如下：

1. prepare 在当前同步 segment 内完成 capability route、linked plan、
   argument 和 required-context 校验，并读取/编码 caller heap 中的参数；
2. `NativeExternalWait` 只拥有 capability context、owned 参数、请求状态和
   cleanup guard，类型上不接收 caller heap；
3. wait outcome 是 opaque、single-owner 的 owned value，只能交回配对的
   `NativeExternalFinalize`；
4. finalize 重新接收 caller heap并物化结果；它在物化前建立 heap checkpoint，
   失败时 truncate 本次新增节点并恢复 stats；
5. async `dispatch_resolved_native_call` 仅调用 prepare、等待同一个 future、
   再 finalize，供 E4R 接线前保持现有调用编译；
6. 协议中没有名为 `Pending` 的预判状态；只有后继 E3 首次 poll 可以观察真实
   `Poll::Pending`。

wait 和 finalize 都是 `Send`，因此既有 eval 的 `async_recursion` Send
future 继续成立。`cargo check -p skiff-runtime-eval --locked` 已验证薄 wrapper
兼容；没有修改 eval production。

## 4. Route 收束

| route | prepare / wait / finalize 结果 |
| --- | --- |
| `std.time.sleep` | duration decode、safe-integer 校验和 clamp 在 prepare；owned timer wait；zero 仍是 external wait，但真实首次 poll 为 Ready |
| 普通 `std.file.*` | string/bytes/options/file ref 全部先转为 owned 参数；file capability future 不借 caller heap；wire result只在 finalize 解码 |
| `std.file.createFromStream` | source、item plan、limits、options和 consumer cleanup 由 wait拥有；自然 End disarm，error/drop取消 exactly once |
| HTTP request/stream/SSE | request和 item plan 在 prepare 固化；owned HTTP future；stream handle只在 finalize按 internal-handle规则物化 |
| HTTP response stream emit | event先编码；owned send wait；成功后 finalize `null` |
| HTTP request/response/header/stream-event helpers | 同步执行并返回 `Ready` |
| WebSocket四个 send | 同步 capability调用并返回 `Ready`，不形成 external wait |
| `requestJsonToConnection` | error owner、connection/method/payload先固化；owned request wait；terminal和 JSON decode语义保持 |
| Actor get/replace/find/remove | actor id key、activation fence、bootstrap先固化；owned registry request；ActorRef/bool只在 finalize返回 |
| bytes/json/registry/telemetry/resource等现有同步 route | core直接包装为 `Ready`；不按 binding name伪造 wait |

`NativeCallableSemantics.may_suspend` 未改，仍可供静态 effect/detachment
分析；prepared runtime 调度不读取它。

## 5. Lifecycle 与失败原子性

新增测试直接证明：

- pending time、HTTP、file和 WebSocket request 的 prepared wait 存活时，
  caller heap仍可独立分配；
- HTTP pending future第一次 poll才启动 capability，第二次 poll继续同一个
  future，调用次数保持一次；
- zero sleep真实第一次 poll为 Ready，而不是由 binding name预判；
- 四个 WebSocket send全部是 `PreparedNativeCall::Ready`；
- file wait第一次 Pending后 drop只 drop一次已启动 future；
- `createFromStream` 未 poll即 drop时，source registry只清理一次、零 item poll、
  零 partial chunk；
- `createFromStream` item decode、file write、commit-after-end和自然 End的
  既有 cleanup矩阵全部保持；
- HTTP wait完成、finalize之前 caller heap checkpoint完全不变；
- nested bytes result在一节点 heap limit下发生中途 materialization失败时，
  finalize恢复原 checkpoint和 stats；
- WebSocket request linked error owner在任何 capability副作用前校验，既有五类
  ordinary error、deadline和 ancestor cancellation不回归；
- Actor四个 registry route都返回 owned external wait并保持既有返回类型。

## 6. 验证

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-native dispatch -- --nocapture` | PASS：实际执行 37/37 matching tests |
| `cargo test -p skiff-runtime-native --locked --no-fail-fast` | PASS：112/112 unit tests，1/1 doc test |
| `cargo check -p skiff-runtime-native --locked` | PASS |
| `cargo check -p skiff-runtime-eval --locked` | PASS；只出现既有 linker dead-code 与 service-error unreachable-pattern warnings |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

反向搜索：

```text
rg 'may_suspend|maySuspend|native_call_suspends|suspend_actor_segment|yield_now' \
  runtime/native/src/dispatch runtime/native/src/capability.rs
```

结果为空。`unsafe` 唯一命中是既有测试文字
`unsafe integer payload`，不是 Rust unsafe 代码。

## 7. E4R 消费方式

E4R 只需：

1. 调用 `NativeDispatch::prepare_resolved_native_call(...)`；
2. `Ready(value)` 留在当前 Actor segment；
3. `ExternalWait(operation)` 拆出 wait/finalize；
4. 把同一个 wait交给 E3 `await_if_pending`；
5. E3返回且 Actor segment恢复后，调用
   `finalize.finalize(outcome, caller_heap)`。

E4R 不需要识别 native binding、读取 `may_suspend`、重建 future或复制
file/HTTP/WebSocket/Actor cleanup状态机。当前没有命中
`TASK_SCOPE_EXPANDED`，也没有需要用户决定的新语义。
