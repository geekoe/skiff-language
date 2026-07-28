# P5-F445H-E4R7 stream deadline test semantics closure result

状态：`IMPLEMENTATION_COMPLETE / E4R7_GREEN`。

五条 F419 时代的 stream deadline tests 已迁到 E1/R4 current-scope 语义。request deadline
在 raw stream boundary 前保持 internal
`ScopeTerminalCarrier(InheritedDeadlineExceeded)`；进入 raw stream boundary 后只暴露内部
`StreamRuntimeError::Cancelled`。本节点只改 test，没有修改 production、current-scope
实现、error、shared fixture 或其它 consumer。

## 1. 输入、提交与写集

| 项 | 值 |
| --- | --- |
| 冻结 production/tests 候选 | `464a3319b153527d5d33093d52ea6af97b6f997b` |
| 本任务开始 HEAD | `0d18c4efa6f884cd36a8d0ffc056e8bfb0392674` |
| tests implementation | `19234714bbfddabb5dacb4248e3535ca81caefb0` |
| result | 本文独立 result-only commit；精确 hash 由最终交付消息记录，避免 commit 自引用 |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r7-fix` |
| branch | `codex/p5-f445h-e4r7-fix` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target` |

implementation 唯一源码写入为：

```text
runtime/eval/src/assembly_execution/async_stream_cancel.rs
  #[cfg(test)] mod tests
    五条目标 tests
    first_poll / carrier assertion局部 helper
    helper所需的局部 test imports
```

第二笔提交只新增本文。没有触发 `TASK_SCOPE_EXPANDED`。

## 2. 五条旧 actual 到新 expected

| 旧 test | preflight 冻结的旧 actual | 本节点冻结的新 expected |
| --- | --- | --- |
| `pending_provider_unary_wakes_from_deadline_and_cancels_provider_request` | `RuntimeError::ScopeTerminal(InheritedDeadlineExceeded(request deadline))`，旧断言却期待 generic `ExecutionBudgetExceeded` | internal request carrier；精确断言 deadline instant、`ExecutionDeadlineSource::Request`、nesting `0`、request deadline 非 lexical local owner；provider request 已 cancel |
| `stream_terminal_item_and_publication_deadlines_remain_typed` | provider terminal 首个分支实际为 request carrier；item/publication 的旧 generic 断言尚未执行 | provider terminal 与 terminal publication 在 raw boundary 前保留同一 request carrier；item publication 在 `StreamRuntimeResult` boundary 精确为 `Cancelled`；两类 provider request 均 cancel |
| `provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout` | raw consumer 实际为 `Cancelled` | provider wait 先保留 request carrier，raw consumer 只观察 `StreamRuntimeError::Cancelled`；provider request cancel，stream lifetime 精确释放一次 |
| `stream_item_deadline_remains_typed_through_provider_terminal` | raw consumer 实际为 `End`，来自 consumer attach 与 registry removal 竞态 | consumer 先经手动 first poll 进入真实 `Pending`，再触发 request deadline；item publication 与已 attach raw consumer 都观察 `Cancelled`，不再允许竞态性 `End` |
| `terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout` | 满 buffer、20ms deadline 和 50ms consumer sleep 后，raw terminal 唯一正确形态为 `Cancelled`，旧断言仍期待 generic timeout | already-expired request scope 立即选择 carrier；raw `Cancelled` publication 确认阻塞在满 buffer 后；先消费 buffered item，再观察 `Cancelled`；provider request cancel且 lifetime精确释放一次 |

五条分别重命名为统一非零 selector
`f445h_e4r7_stream_deadline`，仍是五个独立 `#[tokio::test]` 函数；没有删除、合并、
`ignore` 或用旧 exact 零匹配冒充成功。

## 3. 确定性 gate 与 raw boundary

局部 `first_poll` helper 使用 noop waker 对 pinned future 做一次同步 poll，只把
`Poll::Pending` 当作 attach/backpressure gate：

- provider-terminal raw consumer 在 terminal 形成前已取得 channel并进入真实 `Pending`；
- item-publication test 先把 raw consumer poll 到 `Pending`，然后才把 task切到 already-expired
  request scope并调用 item publication wait，因此 registry移除后再 attach所产生的 `End` 已被排除；
- blocked-terminal test 先把 capacity-one channel填满，再安装 already-expired request scope；
  第一次 poll `publish_provider_terminal(End)` 时，current-scope选择 request carrier，deadline
  publication投影为 raw `Cancelled`，并确定性阻塞在 buffered item之后；
- test随后消费 buffered item，完成 blocked publication，再从同一 stream读取 raw `Cancelled`。

上述定序不依赖 `tokio::spawn` 调度、`yield_now`、20/50ms fixed sleep或 timer winner。两条带
lifetime probe的 raw terminal test都只在 consumer实际接纳 terminal时断言 drop count从 `0`
收束为 `1`。

## 4. Production diff 为零

`19234714^..19234714` 的 `git diff --name-only` 只列出
`async_stream_cancel.rs`。该文件的 `#[cfg(test)] mod tests` 从第 `974` 行开始；所有
zero-context hunk的新侧起点均为第 `975` 行或之后。

对 implementation parent与 implementation tree分别计算该文件第 `1..972` 行的 SHA-256，结果完全
相同：

```text
6478d67ed6b5c620914e49c601e3bba419c09fe2b7dd9094951558bebd3db5ba
```

因此 non-test production区域以及 module外的既有 test-only `deadline_error` helper均未变化。
`async_stream_cancel/current_scope.rs`及其tests、ordinary shared test runtime、error/carrier、
stream runtime、capability-context、program stream/invocation、Cargo/manifest/lockfile均无 diff。

## 5. 实际验证

所有 Cargo命令都显式清除 `RUST_MIN_STACK`、`RUSTFLAGS`，设置
`CARGO_NET_OFFLINE=true`，并使用上述独立 target。

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --list` | PASS：`5 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --nocapture` | PASS：`5 passed / 0 failed / 0 ignored`；`390 filtered out` |
| `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --list` | PASS：`22 tests, 0 benchmarks` |
| `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --nocapture` | PASS：`22 passed / 0 failed / 0 ignored`；`373 filtered out` |
| `cargo check -p skiff-runtime-eval --tests --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 compiler-source/linker dead-code或unused-import warning，以及
`ordinary/tests.rs` unused import和 `service_error_channel.rs` unreachable-pattern warning；本节点
没有新增 warning owner。

反向搜索确认五个旧名称均已移除，统一 selector精确命中五个新函数；五条目标函数和局部 helper
不再包含 fixed sleep或 generic deadline expectation。

## 6. 未决项与边界

未决问题：无。五条均可在冻结的 test-only 写集内正确迁移，没有修改 production/current-scope/
error/shared fixture的需要。

没有运行完整 lib/eval、combined gate、stable instance、live selector或 MongoDB；没有访问
network，没有派子 Agent，也没有 merge、rebase或 push。result提交后 worktree应保持 clean，
等待 E4R8 合流后的唯一完整 gate owner。
