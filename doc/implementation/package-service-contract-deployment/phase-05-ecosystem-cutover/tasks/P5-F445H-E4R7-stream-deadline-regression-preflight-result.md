# P5-F445H-E4R7 stream deadline regression preflight result

状态：`READY_FOR_E4R7_FIX`。

五条失败均来自同一个 test-only regression owner：F419 时代的
`async_stream_cancel` deadline tests 仍把 caller request deadline 断言为可穿过 ordinary/stream
wrapper 的 `ExecutionBudgetExceeded(DeadlineExceeded)`。E1/R4 后这些 fixtures 已经提供真实
current `ExecutionScope`；request deadline 必须先保持 internal
`ScopeTerminalCarrier(InheritedDeadlineExceeded)`，不能恢复 generic budget error。carrier 到 raw
stream boundary 时按既有 E1 合同投影为内部取消；真实 evaluator/invocation consumer 则由自己的
current-scope wait 在接纳 raw `Cancelled`/`End` 前返回同一 deadline carrier。

没有 production current-scope 传播缺陷，没有缺失 current scope 的 fixture，也没有需要拆分的第二
production owner。唯一后继是 test-only E4R7 closure；无需用户决定。

## 1. 候选身份、tree 与边界

| 项 | 值 |
| --- | --- |
| preflight 开始 HEAD | `5c86c87325d74b34bb5c1a828ab3bf5effa7604f` |
| preflight 开始 tree | `3d6c0a80f8cea5166f4e0f7a3bfecb76192969d3` |
| 冻结 production/tests commit | `464a3319b153527d5d33093d52ea6af97b6f997b` |
| 冻结 production/tests tree | `17ae8ebe6bb05202d9b3992b812cc3f60fbd8ded` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r7-preflight` |
| branch | `codex/p5-f445h-e4r7-preflight` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r7-preflight/build/cargo-target` |

`464a3319..5c86c873` 只新增 E4R7/E4R8 两份 task 文档；production、tests、fixture、Cargo 和
lockfile diff 均为零。因此本次诊断的 production/tests 状态精确等于任务冻结候选
`464a3319`。开始时 `git status --short --branch` 只输出 branch header，tracked/untracked 状态均
clean；Cargo 只写 ignored `build/cargo-target`。

所有 exact/listing 命令均显式清除 `RUST_MIN_STACK`、`RUSTFLAGS`，没有自定义 worker stack；
设置 `CARGO_NET_OFFLINE=true`，没有访问 network。没有运行完整 lib/eval、combined、stable、
live 或 MongoDB，也没有派子 Agent、merge、rebase 或 push。

## 2. 共同 fixture 与 deadline owner

五条测试都使用
`ordinary::test_runtime::execution_control_with_deadline(...)`。该 fixture 的当前结构是：

```text
TestExecutionControl::with_deadline(deadline)
  cancellation = CancellationToken::new()
  execution_scope = ExecutionScope::request(cancellation.clone(), deadline)
  legacy deadline field = deadline

ExecutionControlApi::execution_scope()
  -> Ok(the current execution_scope clone)
```

因此五条都能在调用时取得完整 current scope；不存在
`ExecutionScopeAccessError::Unavailable`。deadline 的精确分类统一为：

- `ExecutionDeadlineSource::Request`；
- nesting `0`；
- request scope，不是 `derive(...)` 产生的 local child；
- `terminal_at(now)` 为 `InheritedDeadlineExceeded`，因为 request source 不归 lexical timeout
  scope所有；
- 前四条使用已经过期的真实 `Instant`，第五条使用 `now + 20ms`；
- 均未使用 scripted clock。

fixture 同时保留 legacy `deadline()` 字段，但 R4 production 路径不读取它：

```text
await_provider_unary
await_provider_stream_terminal
await_provider_publication
await_stream_item_publication
  -> current_scope::{from_execution|from_owned_execution}
  -> ExecutionControl::execution_scope()
  -> current_scope::wait
  -> terminal_at / cancellation_signals / effective_deadline
```

`async_stream_cancel.rs` 中唯一 `deadline_error(...)` 已是 `#[cfg(test)]` 的旧测试 helper；production
四条 wait 路径没有 `deadline()`、generic request token或 `poll_execution_budget()` fallback。

这组 tests 最初分别来自：

- `f6d6100b`（`runtime: unify service suspension boundary`）；
- `e7924bba`（`runtime: preserve typed stream deadline terminals`）。

它们早于 E1 internal carrier和 R4 current-scope切换。`b1faea53` 后
`TestExecutionControl` 已携带真实 scope，`bb64a182` 又把上述 wait 从 legacy
`deadline()`/generic token切到 current scope，但这五条旧断言没有同步迁移。

## 3. 有界命令与 exact 结果

实际执行了五条允许的 exact tests，各一次；父节点和本次 exact 的 failure shape一致，无需第二次
复跑。

| test | exit | 实际 failure | 旧 expected |
| --- | ---: | --- | --- |
| `pending_provider_unary_wakes_from_deadline_and_cancels_provider_request` | `101` | `async_stream_cancel.rs:1140`；调用链唯一产出 `RuntimeError::ScopeTerminal(InheritedDeadlineExceeded(request deadline))` | generic `ExecutionBudgetExceeded(DeadlineExceeded)` |
| `stream_terminal_item_and_publication_deadlines_remain_typed` | `101` | `:1278` 首个 terminal断言失败；实际为 `ProviderTerminal::DeadlineExceeded(ScopeTerminal(...request...))` | generic deadline terminal；后续旧 item/publication断言未执行 |
| `provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout` | `101` | `:1410` 明确打印 raw consumer terminal `Cancelled` | generic typed deadline stream error |
| `stream_item_deadline_remains_typed_through_provider_terminal` | `101` | `:1462` 明确打印 raw consumer结果 `End` | generic typed deadline stream error |
| `terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout` | `101` | `:1508` typed match失败；静态唯一 raw error为 `Cancelled` | generic typed deadline stream error |

每条 libtest summary 都是
`0 passed / 1 failed / 0 ignored / 394 filtered`。第一次命令包含初始约 `26s` compile；其余复用同一
target。没有通过 filter=0伪造成功。

另运行允许的 inventory-only 命令：

```text
cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --list
```

exit `0`，精确列出 `22 tests, 0 benchmarks`。其中 current-scope语义直接包括：

- provider publication保留 local deadline owner carrier；
- provider cancel在同刻deadline与ready result前赢；
- provider terminal/item publication观察 lease-child signal；
- real `program_stream`/`program_invocation` consumer保留 current local carrier；
- natural End与非-End cleanup分流。

本任务按合同只做 listing，没有重跑这 22 条；R4 的历史 `22/22` 证据没有被当前只读 tree改动。

## 4. 逐 test 正确语义与分类

### 4.1 Pending unary

`pending_provider_unary_wakes_from_deadline_and_cancels_provider_request`

- current scope：存在，request scope；
- deadline来源：request effective deadline，不是 legacy `deadline()` winner；
- provider future：永久 Pending，没有 provider-origin timeout；
- R4正确结果：`ProviderUnaryWaitTerminal::DeadlineExceeded` 携带 internal
  `ScopeTerminalCarrier(InheritedDeadlineExceeded)`，随后 `into_result()` 原样返回 carrier；
- cleanup：`provider_request.cancel()` 仍必须发生；
- 分类：**stale expectation only**。不得把 carrier改回 generic budget error。

### 4.2 Terminal、item 与 terminal-publication helper matrix

`stream_terminal_item_and_publication_deadlines_remain_typed`

首个实际失败已经证明 terminal helper返回 internal carrier。若按 current语义继续其余两个分支，
静态结果唯一为：

| 分支 | R4正确 helper结果 |
| --- | --- |
| provider terminal wait | `ProviderTerminal::DeadlineExceeded(RuntimeError::ScopeTerminal(...request...))` |
| item publication | carrier到 `StreamRuntimeResult` 边界时投影为 `StreamRuntimeError::Cancelled` |
| terminal publication wait | `ProviderPublication::DeadlineExceeded(RuntimeError::ScopeTerminal(...request...))` |

item publication 的签名是 raw `StreamRuntimeResult<T>`；E1 明确禁止
`ScopeTerminalCarrier` 进入 `OrdinaryRuntimeError`、request-heap stream wrapper或 wire payload。
`stream_runtime_error_from_eval` 因此把 internal terminal映射为 `StreamRuntimeError::Cancelled`。
这不是 deadline owner丢失：owner仍由真实 consumer的 current-scope wait保留。

分类：**一个旧 generic expectation加两个尚未执行但同样 stale 的子断言**；fixture无需修改。

### 4.3 Provider terminal 到 raw consumer

`provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout`

内部路径先正确形成：

```text
ProviderTerminal::DeadlineExceeded(ScopeTerminal(request deadline))
  -> finish_provider_stream
  -> publish_provider_deadline_terminal
```

它被识别为 deadline，因此没有进入 provider ordinary service-failure export。随后 carrier在 raw
stream边界按 E1合同投影为 `Cancelled`；exact test打印的实际值正是 `Cancelled`。真实
`program_stream`/`program_invocation` consumer会先由自己的 current request scope返回 carrier，
不会把 raw cancellation当成可 catch业务错误。

该 fixture的 provider future仍是永久 Pending，没有抛 provider-owned
`TimeoutError`，所以合法结果不是“provider typed timeout”。`provider_request.cancel()`和 stream
lifetime释放一次仍须保留为修复后的断言。

分类：**stale raw-stream expectation**，production行为正确。

### 4.4 Item publication 到 provider terminal

`stream_item_deadline_remains_typed_through_provider_terminal`

item wait先形成 request carrier，然后在 `StreamRuntimeResult` 边界变为 `Cancelled`。
`finish_provider_stream(Provider(Err(Cancelled)))` 执行本地 request/stream cancel。当前 test只
`tokio::spawn` consumer而没有“已进入 Pending poll” gate；若 cancel前 consumer已取得 channel，它会
看到 raw `Cancelled`，若 registry先被移除，后续 raw poll看到 `End`。本次和父节点都观察到后者。

这不是 cancellation与deadline winner竞态：deadline在 item wait中已经赢并形成 carrier；
`Cancelled`/`End` 只是之后的 raw cleanup可见形态。真实 scoped consumer在 wait入口/winner处先检查
同一 request deadline，因此返回 carrier，不把这个 cleanup `End` 当作 natural provider End。

分类：**stale expectation + test attachment race**，不是 production race。修复必须用手动 first
poll或明确 gate固定 raw consumer是否已经 Pending，不能继续依赖 spawn调度。

### 4.5 Backpressured terminal publication

`terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout`

capacity-one buffer先阻塞原 `End` publication；request scope的 +20ms deadline随后形成 carrier。
`publish_provider_deadline_terminal` 取消 provider request，并把 carrier按 raw stream边界规则发布为
`Cancelled`。consumer在50ms后取走 buffered item，再收到该 raw cancellation；因此 old generic
deadline match失败。

第五条既不是 provider-origin typed timeout，也不是 ancestor-cancel-vs-deadline race。两个固定 sleep
是 F419 fixture遗留，并违反 R4 对确定性 gate/scripted timing 的要求。它可用 already-expired
request deadline加明确 first-poll/backpressure顺序重写，不需要 production timer变化。

分类：**stale expectation + stale fixed-sleep fixture**。

## 5. 排除项

以下假设已排除：

1. **current scope缺失**：五个 control均返回 `Ok(ExecutionScope)`；没有
   `InvalidArtifact("current execution scope is unavailable...")`。
2. **legacy fallback仍在 production**：四条 wait只调用 current-scope组合；`deadline_error`仅为旧
   test helper。
3. **ancestor cancel同刻抢赢导致 deadline丢失**：五条在 deadline winner前都没有 cancel
   execution root；`provider_request.cancel()`发生在 carrier已选定之后，且不是 execution root
   token。R4 `terminal_at`/biased cancellation-first规则没有被这些 failures反证。
4. **provider typed timeout被错误降级**：五条 provider futures均不产生 provider ordinary
   `TimeoutError`或 provider budget error；唯一 deadline是 caller request scope。
5. **raw `End`是 natural provider completion**：第四条的 `End`来自 cancel后 test runtime registry
   已移除，不来自 `ProviderTerminal::Provider(Ok(_))` 或正常 `sink.end()`。
6. **需要 request-root fallback**：fixture本身已是调用时 request current scope；新增 fallback只会
   破坏 local/inherited owner语义。
7. **需要远端 ack**：这些 assertions只需保留本地 provider request cancel、stream/lifetime收束；
   不新增远端 acknowledgement或 exactly-once业务契约。

## 6. 唯一修复 owner 与允许写集

唯一 owner：

```text
P5-F445H-E4R7 async_stream_cancel legacy deadline test semantics closure
```

最小允许写集只有：

```text
runtime/eval/src/assembly_execution/async_stream_cancel.rs
  #[cfg(test)] mod tests 中上述五条 tests
```

虽然 tests与production同文件，后继 diff必须证明所有改动均位于 `#[cfg(test)] mod tests`。必要的
test-only rename、manual first-poll helper或局部断言可留在同一 module；不需要修改公共 fixture。

明确禁止：

- `async_stream_cancel.rs` 的非-test production段；
- `async_stream_cancel/current_scope.rs`；
- `async_stream_cancel/current_scope_tests.rs` 的已GREEN R4证据；
- `ordinary/test_runtime.rs`及其 `scoped_execution.rs`；
- `error.rs`、`ScopeTerminalCarrier`或 `stream_runtime_error_from_eval`；
- capability-context stream/scope core；
- program stream/invocation consumer；
- Cargo、manifest、lockfile及其它 docs/tasks。

修复后的五条应分别冻结：

1. unary/request deadline返回 inherited carrier并取消 provider request；
2. terminal/publication helper在 raw stream边界前保留 request carrier，item边界投影 cancel；
3. raw terminal只观察 internal cancel，同时 lifetime恰好释放一次；
4. item cancel场景先手动 poll到真实 Pending，再断言 raw `Cancelled`，消除 `End` attachment race；
5. backpressure场景无固定 sleep，断言 buffered item后是 raw internal cancel并保留本地 cleanup。

不得为了保留旧 test名称中的 “typed timeout” 恢复 generic
`ExecutionBudgetExceeded(DeadlineExceeded)`、把 carrier塞进 ordinary producer payload，或允许
request-root inference。

## 7. RED、GREEN 与完整 lib重验

当前 RED 已由本结果第3节五条 exact命令冻结，均 exit `101`。后继 test-only修复应给五条重写测试统一
非零 selector，例如：

```text
f445h_e4r7_stream_deadline
```

并要求：

```text
cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --list
  => 5 tests, 0 benchmarks

cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --nocapture
  => 5 passed / 0 failed

cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --list
  => 22 tests, 0 benchmarks

cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --nocapture
  => 22 passed / 0 failed

cargo check -p skiff-runtime-eval --tests --locked
cargo fmt --check
git diff --check
```

统一 selector可以通过重命名这五条 tests实现；不得删除 case或让旧 exact filter变成零测试后冒充
GREEN。若后继选择保留原名称，则五条 current full name必须各自 exact GREEN，并另用 listing证明
仍有五个实际 case。

完整 lib本任务未运行。由于当前还存在独立 E4R8 ordinary service-error consumer default-stack
blocker，昂贵 gate不得在两个修复叶子重复执行。E4R7和E4R8修复都集成到同一最终 tree后，由唯一 full
lib gate owner在默认 stack、独立 target上执行一次：

```text
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_NET_OFFLINE=true \
  CARGO_TARGET_DIR=<final-independent-target> \
  cargo test -p skiff-runtime-eval --locked --lib -- \
  --nocapture --test-threads=1
```

必须取得真实 libtest完整 summary、零 failure且无 `SIGABRT`；不能从 filtered exact或 abort前输出推算
395-test结果。任一后续 production/tests变化都会使该一次证据失效。

## 8. 最终判定

五条 failure是一个 test-only owner下的两类旧断言：

- direct helper仍期待 generic budget error；
- raw stream仍期待 internal deadline可作为 ordinary typed producer error穿过边界。

第四、第五条另有 stale调度/固定sleep fixture问题，但不改变 owner或实现方向。修复不需要
production、公共 fixture或新DAG分支，因此状态为 `READY_FOR_E4R7_FIX`，`USER_DECISION = NO`。

本 preflight唯一 tracked写入为本文；production/tests/fixture保持只读。
