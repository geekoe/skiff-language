# P5-F440E Cancellation runtime terminal checkpoint result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 已把 `runtime/capability-context` 自有的 execution、budget 和 stream cancellation 收敛为
编码无关的内部 terminal。取消仍保留 token wake、blocked operation wake、stream cleanup、
single-terminal 与 lifetime release；它不再实现普通 wire error contract，也不能在本 crate 产生
payload、catch identity 或 service serialization 事实。Deadline 与 instruction budget 仍精确投影为
`TimeoutError`。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| 精确上游 integration checkpoint | `adc846fcc102ab23ffdd461066e72459ed9f9cee` | `affa74598b94f41aa058e2be8d11ec0037fe5a83` |
| task worktree 起点 | `d1e8740529c40179f9e9eff1f32288c45ea04566` | `c972486d0ae57001f7e82fa3588ebae967932a3e` |
| implementation | `0d3b82a149184a31234df9c66d98b362fb5b1cc6` | `a53eb49ef16419c75ec14e14b86991dc20da28c3` |

task 起点相对精确上游只新增 F440E 任务文档，production tree 未漂移。

implementation 精确修改：

- `runtime/capability-context/src/execution_control.rs`
- `runtime/capability-context/src/stream.rs`
- `runtime/capability-context/src/file.rs`
- `runtime/capability-context/src/lib.rs`
- `runtime/capability-context/src/cancellation_terminal_tests.rs`

除此之外只新增本文 result。

## 2. 实现结果

### 2.1 Internal terminal hard cut

- `ExecutionControlError` 与 `StreamRuntimeError` 不再实现 `WirePayload`。
- `FileCapabilityError` 也不再实现 `WirePayload`，避免其 `Stream` / `Execution` wrapper 把内部取消重新
  公开化。
- `ExecutionBudgetReason`、`ExecutionControlError`、`StreamRuntimeError` 与
  `FileCapabilityError` 提供 `is_cancellation_terminal()` 布尔查询；没有新增 cancellation 名义类型、
  字符串 code 或 serializer。
- 两个 carrier 各有一条 `compile_fail` 契约，直接证明不能再调用 `WirePayload::payload(...)`。
- 非取消分支通过显式 `ordinary_payload()` / `ordinary_catch_projection()` 投影；取消分支在两者上均
  返回 `None`，不能形成普通 payload 或 catch identity。

### 2.2 Execution、budget 与 wrapper

- `ExecutionControlError::Cancelled` 是 internal terminal。
- `BudgetExceeded(ExecutionBudgetReason::Cancelled)` 经同一查询识别为 internal terminal。
- `DeadlineExceeded` 继续返回 code `TimeoutError`、message `execution deadline exceeded`，并保留
  `reason`、`instructionCount`、`limit`、`elapsedMs` details 与 Timeout catch identity。
- `InstructionLimitExceeded` 继续返回 `TimeoutError` 与 Timeout catch identity。
- `FileCapabilityError::{Execution,Stream}` 递归保留 terminal classification，但不返回 ordinary
  projection；普通 file/provider/resource/decode/producer 分支仍保持原投影。

### 2.3 Wake、cleanup 与 lifetime

- notify-backed token、already-cancelled token、注册 waiter 时的 cancel race、多 waiter 与 mixed
  signal set 均继续唤醒。
- flag-backed fallback waiter 完成后 active counter 回到零，保留 waiter removal 证据。
- 新增真实经过 `StreamSink::send_with_cancellation` 与
  `StreamRuntime::next_with_cancellation` wrapper 的阻塞探针：outer token 与 inner stream signal
  分别唤醒操作，结果均是 internal terminal，未触发的另一侧保持未取消。
- 既有 standalone/supervised stream cleanup 八个探针继续证明自然 End、producer error、partial
  success、drop、wrong-stream 与 outer-owner barrier 下的 exactly-once cancel/lifetime release。

## 3. 测试先行与验证

### 3.1 Red evidence

production 修改前先落最终测试：

| 命令 | Red 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context --doc` | FAIL：2 个 `compile_fail` probe 都“compiled successfully”，直接证明两个 cancellation carrier 仍实现 `WirePayload` |
| `cargo test -p skiff-runtime-capability-context cancellation_terminal -- --nocapture` | FAIL：42 个 `E0599`，最终测试要求的 terminal / ordinary 分离 API 尚不存在 |

第一条失败直接命中待删除的普通 wire contract，不是 skip 或零测试 selector。

### 3.2 Focused green matrix

| 命令 / selector | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context tests::execution_control_cancellation_is_terminal_and_timeouts_remain_ordinary -- --exact` | PASS：1 passed |
| `cargo test -p skiff-runtime-capability-context tests::file_capability_wrappers_separate_internal_terminal_from_ordinary_projection -- --exact` | PASS：1 passed |
| `cargo test -p skiff-runtime-capability-context tests::stream_runtime_cancellation_is_terminal_and_ordinary_errors_still_project -- --exact` | PASS：1 passed |
| `cargo test -p skiff-runtime-capability-context cancellation::tests::` | PASS：10 passed |
| `cargo test -p skiff-runtime-capability-context cancellation_terminal_tests::` | PASS：3 passed |
| `cargo test -p skiff-runtime-capability-context stream_cleanup::tests::` | PASS：8 passed |
| `cargo test -p skiff-runtime-capability-context --doc` | PASS：2 passed |

focused matrix 合计 26 passed、0 failed。selector 覆盖 direct execution cancel、cancelled budget、
deadline/instruction timeout、stream cancel、already-cancelled token、pending waiter、blocked
send/next、outer/inner cancellation、single-terminal 与 cleanup/lifetime。

### 3.3 Crate 与静态检查

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context --no-fail-fast` | PASS：38 unit + 2 doctest |
| `cargo check -p skiff-runtime-capability-context` | PASS |
| `cargo check -p skiff-runtime-native` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

## 4. Reverse search

对整个 `runtime/capability-context/**` 搜索
`CancelError|PlatformBuiltinErrorIdentity::Cancel`：`ZERO_MATCHES`。

对同一根搜索
`impl WirePayload for (ExecutionControlError|StreamRuntimeError|FileCapabilityError)`：
`ZERO_MATCHES`。

保留的 `Cancelled|CancellationToken` 只出现在六个文件，逐文件分类如下：

| 文件 | 分类 |
| --- | --- |
| `actor_invocation.rs` | actor invocation 的内部取消 outcome/reason；不是错误 payload |
| `cancellation.rs` | canonical token/source/signal、notify/poll fallback 与 waiter tests |
| `execution_control.rs` | budget reason、execution terminal/query、token plumbing 与 negative compile contract |
| `stream.rs` | stream terminal/query及 cancellation-aware send/next/pull API |
| `lib.rs` | canonical exports 与 direct terminal/timeout/wrapper tests |
| `cancellation_terminal_tests.rs` | blocked send/next、outer/inner 与 single-terminal test harness |

没有保留 public error code、platform cancel identity、catch projection或普通 serializer。

## 5. Consumer compile checkpoint

只读下游检查：

- `cargo check -p skiff-runtime-native` 通过；native 已结构化匹配 capability carrier。
- `cargo check -p skiff-runtime-eval` 按预期失败于
  `runtime/eval/src/error.rs:992`：旧代码仍将 `ExecutionControlError::Cancelled` 装箱为
  `Box<dyn WirePayload>`，Rust 报 `E0277`。这是 R1 的精确迁移点。
- 静态核对 `runtime/host/src/error.rs:393-407` 还有 execution、stream、file 三个旧 opaque
  boxing 点，归后续 R2；当前编译在 eval blocker 处先停止。

本 leaf 未修改 eval/native/driver/request/host/transport/model production，因此没有越界修复这些预期
consumer break，也不需要先扩张公共 model。

## 6. Scope 与禁令

- 没有修改 `runtime/capability-context/**` 和本文 result 之外的文件。
- 没有运行完整 verify、Router、live、instance、stable 或昂贵 combined gate。
- 没有 merge、rebase、push 或注册 stable watch。
- implementation 与 result 分开提交；result commit/tree 由交付消息记录。
