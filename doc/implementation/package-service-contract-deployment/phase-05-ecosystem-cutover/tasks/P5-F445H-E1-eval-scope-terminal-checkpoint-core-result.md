# P5-F445H-E1 Eval scope, terminal and checkpoint core result

状态：`IMPLEMENTATION_COMPLETE / EVAL_GREEN`。

E1 已完成 current owned execution control、单调时钟、内部 scope terminal、owner-aware checkpoint
core，以及四个 F445H-E4 前置的明确 fail-closed compile bridge。本节点没有物化
`UserException<TimeoutError>`，没有修改 catch/exceptions owner，也没有触碰 capability、request、
host、native、stream、artifact、compiler 或 Router。

## 1. 输入与提交

| 项 | commit |
| --- | --- |
| production base | `27618e61` |
| task document | `0bd34f25` |
| implementation | `f254d11e` |
| owned clock correction | `f04a7c82` |

implementation 写集精确为：

- `runtime/eval/src/program_execution.rs`
- `runtime/eval/src/program_execution/execution_scope.rs`
- `runtime/eval/src/program_execution/execution_scope_tests.rs`
- `runtime/eval/src/error.rs`
- `runtime/eval/src/error/scope_terminal.rs`
- `runtime/eval/src/error/scope_terminal_tests.rs`
- `runtime/eval/src/eval_context.rs`

新增 production 责任分别放在 244 行的 `execution_scope.rs` 与 40 行的
`scope_terminal.rs`；两个既有超长 root 只增加字段、variant、module 和薄接线。`eval_context.rs`
只增加四个 compile bridge arm、稳定诊断 helper 与直接测试。

## 2. Test-first 证据

先加入四个明确 bridge arm，使 eval 不再被 F445G 的 non-exhaustive match 阻断；随后只加入 E1
contract tests 并运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target \
  cargo test -p skiff-runtime-eval program_execution_scope -- --nocapture
```

结果为预期 RED，exit `101`：

- `program_execution::execution_scope` 尚不存在；
- `error::ScopeTerminalCarrier` 与 `RuntimeError::ScopeTerminal` 尚不存在。

RED 没有来自旧 exhaustive match 或写集外组件。随后才实现 E1 production core。

### 2.1 独立验收 correction

独立验收发现初始实现只让 `OwnedProgramExecutionContext` 保存 current execution control，
`borrow()` 仍会重建 production clock，因而 scripted clock 无法跨 owned round-trip 继续同一调用
序列。先增加单一 focused test：

```text
cargo test -p skiff-runtime-eval \
  program_execution_scope_owned_round_trip_preserves_current_scripted_clock_sequence \
  -- --nocapture
```

修正前该测试按预期 RED，exit `101`：capture 前第一个 checkpoint 消耗 scripted call 1，
`owned.borrow()` 后的第二个 checkpoint 没有在 scripted call 2 越过 deadline。测试共享同一个
clock queue/counter，没有重建替代 clock。

`f04a7c82` 让 owned context 与 current control 一起 clone/capture `ExecutionClock`，并在 borrow
后恢复该实例。修正后同一测试 PASS，call count 精确为 2，第二个 checkpoint 产生原 local owner
terminal。

## 3. Current scope 与 clock

`ProgramExecutionContext` 现在构造时立即 capture `OwnedExecutionControl`。每次
`execution()` 都从当前 owned control 产生新的 borrowed view；context clone 可安装 child owned
control，parent 不被原地修改。`OwnedProgramExecutionContext::{capture,borrow}` 因此保留调用时的
child current scope，而不是退回 request-start snapshot。

timeout child derivation 使用 eval-private monotonic clock：

- production 为 `Instant::now()`；
- tests 为 scripted clock；
- 普通 duration 精确 `checked_add`；
- 超大合法 `duration_ms` 通过 64 次以内的单调二分钳到该平台可表示的最远毫秒 deadline；
- scope unavailable 与 nesting derive failure 都稳定返回 `InvalidArtifact`。

测试覆盖 child/drop parent restoration、owned round-trip、inner-earlier、outer-earlier、equal
deadline outer-only owner，以及 scripted clock 第三次 checkpoint 越界。

## 4. Internal terminal 与 checkpoint

`ScopeTerminalCarrier` 保留完整 `ExecutionScopeTerminal` / `EffectiveDeadline`，并提供当前 scope
owner 精确匹配：

- ancestor cancel 立即归一为 `RuntimeError::Cancelled`；
- local / inherited deadline 保持 `RuntimeError::ScopeTerminal`；
- diagnostic/source wrappers 可递归恢复 carrier；
- ordinary payload、ordinary catch、`OrdinaryRuntimeError` 与 request-heap-owned stream wire
  wrapper 全部拒绝该 terminal；
- generic deadline budget error 会重新读取 current scope 和 clock，恢复 local/inherited owner；
- instruction limit 继续使用既有 `ExecutionBudgetExceeded`。

统一 checkpoint 顺序为 clock、scope terminal、instruction accounting、budget poll、generic
deadline owner recovery。kind/units 可表达 function entry、loop condition、backedge、lane
start/end、tail start 和 generated chunk；本节点没有迁移 evaluator call sites。

## 5. Compile bridge

以下四个 arm 均返回稳定 `InvalidArtifact`，明确说明必须由 F445H-E4 接线：

- `LinkedStmtIr::Timeout`
- `LinkedStmtIr::Concurrent`
- `LinkedExprIr::Timeout`
- `LinkedExprIr::ConcurrentValue`

没有 wildcard、顺序退化、body/plan 丢弃或部分语义；既有 `ValueBlock` 未修改。

## 6. 验证

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-e1-scope-core/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval program_execution_scope -- --nocapture` | PASS：9/9 |
| `cargo test -p skiff-runtime-eval scope_terminal -- --nocapture` | PASS：4/4 |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo test -p skiff-runtime-eval --no-fail-fast` | PASS：233 unit、10 integration、1 doc-test |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 linker dead-code、compiler-source unused import 和
`service_error_channel.rs` unreachable-pattern warnings；本节点没有新增 warning 或既有失败。

## 7. 后继合同

E2/E3 可消费本节点的 current owned scope 与 checkpoint types。E4 负责：

- 用真实 timeout/concurrent evaluator 语义替换四个 bridge arm；
- 只在匹配当前 source/nesting 的 timeout wrapper 中物化 local `TimeoutError`；
- inherited/request terminal 穿过当前 timeout 和 ordinary catch；
- 把分散 evaluator checkpoint call sites 迁到本节点统一 helper；
- 完成 stream/catch/actual-Pending 接线。

I6 仍负责 production host/native adapter 的 invocation-time scope propagation。本节点没有为这些
后继新增 fallback 或兼容路径。
