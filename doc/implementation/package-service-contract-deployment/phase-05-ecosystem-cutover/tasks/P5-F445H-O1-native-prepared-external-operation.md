# P5-F445H-O1 Native prepared external operation

状态：Ready。actual-Pending correction DAG 的 native owner；与 O2–O5 并行。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

production prerequisite 为 Skiff integration `d39ad5b0`。父结果已冻结语义与方向，本任务不得
采用 detached Actor snapshot、unsafe heap alias、future restart、pre-suspend或静态
`maySuspend`调度。

## 生产目标

在 native dispatch owner内建立 prepared operation协议：

1. prepare在当前同步 segment中解码/校验 caller args并完成所有必要 heap读取；
2. 纯同步调用直接返回 Ready；
3. 外部调用返回一个不借 caller `RequestHeap`、`Env`或 `EvalContext` 的 owned wait；
4. wait完成后产生 owned outcome；finalize才重新接收 caller heap并物化返回值/错误；
5. wait/drop guard只启动一次副作用并拥有 cancel/cleanup，不能由 evaluator重建请求；
6. API形状必须让后继 E4R在 wait存活期间继续安全访问 caller heap并交给 E3
   `await_if_pending`。

至少覆盖：

- time sleep：decode/clamp在 prepare，owned timer wait，Ready/zero仍由真实首次 poll决定；
- file普通操作与 `createFromStream`；后者保留 source/partial result exactly-once cleanup；
- HTTP request/stream/SSE与 response stream emit；
- WebSocket四个 send为同步 Ready；`requestJsonToConnection`为 owned request wait；
- Actor registry get/replace/find/remove；
- bytes/json/crypto/resource/telemetry等同步 route不伪装成外部 wait。

不得让后继根据 binding name选择 suspend。`NativeCallableSemantics.may_suspend`可以继续用于静态
effect/detachment分析，但不能进入 prepared runtime调度。

现有 `dispatch_resolved_native_call(..., heap).await` 在 E4R切换前仍有 caller；可保留为只组合
prepare→wait→finalize的薄 wrapper以维持编译，不能复制 route状态机或新增兼容分支。

## Test-first 与验收

先写 RED，证明现 API无法在 external wait存活时再次独立借用/mutate caller heap；随后覆盖：

- pending time/HTTP/file/WebSocket request的 wait不借 caller heap；
-同步/zero/四个 WebSocket send返回 Ready；
- owned wait第一次 poll Ready与Pending都只启动一次副作用；
- error/cancel/drop和 `createFromStream` cleanup exactly once；
- finalize前不写 caller heap，finalize失败保持既有错误和heap失败原子性；
- route/required-context校验、现有返回类型与错误不回归。

使用独立 target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target \
  cargo test -p skiff-runtime-native dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target \
  cargo test -p skiff-runtime-native --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target \
  cargo check -p skiff-runtime-native --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o1-native-prepared/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数；零测试不算证据。

## 写集与停止规则

只允许：

- `runtime/native/src/dispatch/adapter.rs`
- `runtime/native/src/dispatch/core.rs`
- `runtime/native/src/dispatch/{time,file,http,websocket,actor}.rs`
- `runtime/native/src/dispatch/**` 中对应窄 child/tests
- 必要时 `runtime/native/src/capability.rs`
- 本 result

不得修改 eval、host、service-db、artifact/native semantics、Cargo manifest或 lockfile。
若任何 ExternalWait仍需借 caller heap/env、必须改变 public Skiff API、或资源清理无法在本 owner
内 exactly once，立即 `TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o1-native-prepared
branch   codex/p5-f445h-o1-native-prepared
```

先提交 implementation，再提交
`P5-F445H-O1-native-prepared-external-operation-result.md`；最终 clean，不
merge/rebase/push，不运行 stable/live/network，不派子 Agent。
