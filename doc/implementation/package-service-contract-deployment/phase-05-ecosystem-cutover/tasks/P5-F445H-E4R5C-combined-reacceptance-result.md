# P5-F445H-E4R5C combined reacceptance result

状态：

```text
PASS
E4R_COMPLETE = YES
I6_UNBLOCKED = YES
TASK_SCOPE_EXPANDED = NO
```

deadline listing/execution 与 combined listing/execution 均精确为 `5/5`。唯一一次完整
`skiff-runtime-eval` gate 在默认 libtest worker 栈、显式清除 `RUST_MIN_STACK` /
`RUSTFLAGS`、单线程 lib 顺序下取得全部五个 binary/doc-test 的合法完整 summary：
`411 passed / 0 failed / 0 ignored / 0 filtered`，没有 abort。locked check、fmt、diff 与九项
production 静态反向检查全部通过。

因此初次 R5 的 callback stack blocker、R6 preflight 观察到的五条 deadline 旧断言和
service-error consumer default-stack blocker均已在冻结组合候选上关闭；E4R 完成，I6 前置解除。
这不代表 F445H 或 Phase 05 整体完成。

## 1. 冻结候选与验收身份

| 项 | 值 |
| --- | --- |
| 验收开始 HEAD | `0a0ffa881c0bb9bebcd6f7e4a8e093a10a963228` |
| 验收开始 tree | `e6051bdcd89764753cefcc59510ef94f0a2b939d` |
| 冻结 production/tests commit | `bf55ede018526751a2db101a42900c4e07fe08a8` |
| 冻结 production/tests tree | `61323e4772061c3b50abc189712767bde716ea24` |
| 初次 R5 production/tests commit | `da49c17cb6e3c479ea649b936aab8614d3beface` |
| 初次 R5 production/tests tree | `0bdff47fad52aa52fea27bfd753db4bbf1213b6c` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance` |
| branch | `codex/p5-f445h-e4r5c-acceptance` |
| 独立 target | `/Users/geek/workspace/skiff-p5-f445h-e4r5c-acceptance/build/cargo-target` |

`0a0ffa88` 的唯一 parent 精确为 `bf55ede0`。`bf55ede0..0a0ffa88` 只有新增本验收任务合同
`P5-F445H-E4R5C-combined-reacceptance.md`，没有 production、tests、fixture、Cargo、manifest
或 lockfile 变化。验收开始和 result 写入前的 `git status --short --branch` 均只输出 branch
header；忽略的独立 Cargo target不构成候选写入。因此所有动态与静态 gate都针对同一冻结
production/tests tree，候选期间没有在途写入。

## 2. 精确命令与实际结果

所有 Cargo test/check 命令均设置 `CARGO_NET_OFFLINE=true`，使用上表独立 target，并显式执行
`env -u RUST_MIN_STACK -u RUSTFLAGS`。没有设置任何替代 test stack、编译 flags或在线依赖。

| # | 命令 | exit | 实际结果 |
| ---: | --- | ---: | --- |
| 1 | `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --list` | `0` | 精确 `5 tests, 0 benchmarks`。 |
| 2 | `cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --nocapture` | `0` | `5 passed / 0 failed / 0 ignored / 390 filtered`。 |
| 3 | `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --list` | `0` | combined binary精确 `5 tests, 0 benchmarks`；另外三个 test binaries为0匹配。 |
| 4 | `cargo test -p skiff-runtime-eval --locked f445h_e4r_combined -- --nocapture` | `0` | combined `5 passed / 0 failed / 0 ignored / 0 filtered`；其它 binaries为0执行。 |
| 5 | `cargo test -p skiff-runtime-eval --locked --no-fail-fast -- --test-threads=1` | `0` | 唯一完整 eval；全部 binary与doc test形成合法 summary，无 abort。 |
| 6 | `cargo check -p skiff-runtime-eval --locked` | `0` | PASS。 |
| 7 | `cargo fmt --check` | `0` | PASS，无输出。 |
| 8 | `git diff --check` | `0` | PASS，无输出。 |

完整 eval只运行一次。没有用 abort 前输出、filtered exact、提高栈对照或父节点结果推算完整
summary。

### 2.1 唯一完整 eval summary

| binary | inventory | passed | failed | ignored | filtered |
| --- | ---: | ---: | ---: | ---: | ---: |
| `skiff_runtime_eval` | 395 | 395 | 0 | 0 | 0 |
| `catch_fixture_closure` | 4 | 4 | 0 | 0 | 0 |
| `f445h_e4r_combined` | 5 | 5 | 0 | 0 | 0 |
| `representation_wrap_consumer` | 6 | 6 | 0 | 0 | 0 |
| doc tests | 1 | 1 | 0 | 0 | 0 |
| **总计** | **411** | **411** | **0** | **0** | **0** |

完整 lib binary 的只读 post-run listing确认实际 inventory：

```text
total=395
f445h_e4r_spine=23
f445h_e4r_timeout=11
f445h_e4r_concurrent=11
f445h_e4r_stream=22
f445h_e4r7_stream_deadline=5
```

combined 独立 binary inventory为5。完整执行没有 filter，因此上述 spine、timeout、
concurrent、stream、deadline 与 combined inventory均在本次完整 gate中真实执行，不只是存在。

### 2.2 blocker closure 的默认栈证据

完整 lib的 inventory与无过滤 `395/395` summary共同证明以下 blocker/target tests在默认
libtest worker栈执行：

- callback Ready/Pending：
  `f445h_e4r_spine_callback_ready_keeps_actor_segment`、
  `f445h_e4r_spine_callback_pending_reacquires_before_finalize`；
- activation Ready/Pending/failure：
  `f445h_e4r_stream_activation_unary_ready_keeps_actor_segment`、
  `...pending_releases_then_reacquires_before_finalize`、
  `...actual_evaluator_imports_provider_failure_once`；
- 先前 abort 的 service-error consumer
  `ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses`；该 test本身没有
  自定义栈。该模块另外四条 tests也在完整 lib中通过，其中一个既有 three-hop test的内部专用
  线程单独分类如下；
- 五条 `f445h_e4r7_stream_deadline_*` request carrier/raw cancellation tests；
- combined R1、R2、R3、R4 activation、R4 stream五条 tests。

callback Pending不再 stack overflow；activation Pending完成后先 reacquire再 finalize；
service-error consumer的 linked/unlinked/internal catch路径不再 abort。五条 deadline tests同时在
focused gate `5/5` 和本次完整 lib `395/395` 中通过。

仓库另有两处早于初次 R5、且不在 R6/R7/R8 diff中的 test-only `16 MiB` 专用线程：

- `ordinary/tests/service_error_consumer.rs` 的独立 three-hop diagnostic test；
- `ordinary/tests/source_inline_effect_e2e.rs` 的 source-inline overlay test。

它们不是上述曾 abort 的 `ordinary_exact_public...` test，也不覆盖 callback、activation、
deadline或combined路径。production和目标 tests没有 `RUST_MIN_STACK`/`RUSTFLAGS`/stack override；
这两个既有 test-local helper相对初次 R5精确无 diff，不是本次 stack closure的证据来源。

## 3. R6、R7、R8 修复点与写集边界

从初次 R5 production/tests commit `da49c17c` 到冻结候选 `bf55ede0`，production/tests/Cargo
范围只有三个 changed paths：

```text
runtime/eval/src/eval_context/actual_pending.rs
runtime/eval/src/eval_context/actual_pending/activation.rs
runtime/eval/src/assembly_execution/async_stream_cancel.rs
```

Cargo manifests和 `Cargo.lock` 无 diff。

### 3.1 Callback 与 activation private layout

callback call-site仍只构造同一个 prepared wait，并经过原通用 owner：

```rust
let wait = Box::pin(prepared.wait(&interpreter));
let completed = self.await_actual_pending(wait).await?;
completed.finalize(self.heap).map(Into::into)
```

activation call-site同样只改变 concrete future布局：

```rust
let wait = Box::pin(operation.wait());
let completed = self.await_actual_pending(wait).await?;
completed.finalize(self).map(Into::into)
```

两者都继续进入同一个 `EvalContext::await_actual_pending` →
`actual_pending::await_operation` → `ActorExecutionFrame::await_if_pending`。该 owner先 poll一次，
只有实际 `Pending` 才 suspend，完成后 resume，再返回 call-site finalize。R6/R8没有改变
prepared owner、wait次数、finalize次数/顺序、error import、request owner、E1/E2/E3或 Actor
frame。

production中 `prepared.wait(&interpreter)` 的唯一 call-site就是上述 private pinned box；
另一个同文本命中位于 `callback_native/prepared_state_tests.rs` 的 test-only prepared-owner unit
test。production中 `operation.wait()` 的唯一命中就是上述 activation private pinned box。

新增项只是两个函数体内的 private local `let wait`，原函数 visibility仍分别为 `pub(super)`和
`pub(in crate::eval_context)`；没有公共 boxing type、公共 API或 ABI变化。

### 3.2 R7 production diff 为零

R7 implementation `19234714` 只修改
`runtime/eval/src/assembly_execution/async_stream_cancel.rs`。该文件当前
`#[cfg(test)] mod tests` 从第973行开始；29个 zero-context diff hunk的新侧起点全部为第975行或
之后，test module之前的 hunk数为0。

implementation parent与当前候选的第1–972行 SHA-256完全相同：

```text
6478d67ed6b5c620914e49c601e3bba419c09fe2b7dd9094951558bebd3db5ba
```

五条测试仍是五个独立 `#[tokio::test]`，统一非零 selector
`f445h_e4r7_stream_deadline`；没有 `#[ignore]`、`#[should_panic]`、删除/合并或零 filter伪证据。
它们分别冻结 inherited request carrier、raw `Cancelled`、attached consumer、
buffer/backpressure顺序与lifetime once语义。全 `runtime/eval/src` 和
`runtime/eval/tests` 的 `#[ignore]` 实际命中为0。

## 4. 九项 production 静态反向检查

### 4.1 Fail-closed diagnostic、静态 effect与实际 Pending

1. `F445H-E4 evaluator integration is required` 在 eval source/tests中为 **0 hits**；四个
   evaluator fail-closed diagnostic均已消失。
2. `maySuspend`为 **0 hits**；`may_suspend`为65 hits，均按路径分类为 artifact/ABI metadata、
   gateway/link signature校验、fixture/builder字段或 test expectation。没有命中参与 segment
   释放。
3. `native_call_suspends`为4 hits：`eval_context.rs` 的 `#[cfg(test)]` re-export和
   `actor_executor.rs` 内三条 unit expectations。production decision为0。
4. production `binding_key`只在 `eval_native_call_with_stream_producer_arg` 识别
   `std.file.createFromStream` composite preparation；其真实切点仍由未改动的
   `await_if_pending` first poll决定。effect summary不决定 release。

从初次 R5到当前候选，`ActorExecutionFrame`、`eval_context.rs`、`program_execution.rs`和
concurrent scheduler均无 diff。当前 `await_if_pending` 仍明确执行“poll once；仅观察到
Pending才 suspend；完成后 resume”。

### 4.2 旧 helper、yield 与 concurrent fallback

- `suspend_actor_segment|resume_actor_segment`：0 hits；
- 语言级 `yield`、`nosuspend|no_suspend`、`sequential.*concurrent`：0 hits；
- `yield_now`命中都位于 `#[cfg(test)]` module/test child；
- concurrent production没有 sequential fallback；唯一相关命中是 test名
  `...fail_closed_without_fallback`；
- `async_stream_cancel.rs` 的既有 `fallback_source/fallback_stack`只为 restricted diagnostic
  补充 instruction source/stack，位于 unchanged owner，不是 current-scope、segment或
  compatibility fallback。

### 4.3 Timeout internal carrier

timeout statement/expression继续从 parent精确 `derive_timeout_child`，保留 child scope和owner
context，只对 `ScopeTerminalCarrier::is_owned_by(child_scope)` 的local owner物化
`TimeoutError`。inherited request carrier和ancestor cancel不由当前 wrapper物化。

`ScopeTerminalCarrier`仍只经 eval crate内部 re-export；`RuntimeError::ordinary_payload`和catch
projection对 `RuntimeError::ScopeTerminal(_)`都返回 `None`，wire/diagnostic不会导出 internal
carrier。只有精确 owner wrapper在验证 payload identity后创建 request-local exception。
timeout/carrier/error/wire owners相对初次 R5无 diff。

### 4.4 Stream current scope

以下三个 production child相对初次 R5均无 diff：

```text
program_stream/current_scope.rs
program_invocation/current_scope.rs
assembly_execution/async_stream_cancel/current_scope.rs
```

三者从调用时 execution control取得精确 `ExecutionScope`，使用
`terminal_at(Instant::now())`、`cancellation_signals()`与`effective_deadline()`。两个 raw
consumer给 `next_with_cancellation` 的额外 token iterator均为 `std::iter::empty()`。
`request-root|root-scope|generic token|cancellation_token()`为0 hits；scope不可取得时 fail
closed，没有 request-root/generic token fallback。

### 4.5 Natural End 与 cleanup

`StreamConsumerCleanup::reached_end()`先记录 runtime natural End，再调用
`disarm_after_end()`；后者只有 `has_reached_end()`成立时才能 disarm。program stream和
invocation loops只在真实 `StreamPoll::End`投影调用 `reached_end()`。

binary HTTP payload中的逻辑 `HttpBoundaryResponseStreamEvent::End`只停止迭代，不调用
`reached_end()`，因此 guard drop仍发起本地 cleanup。break/return/error/future drop同样不
disarm，只承诺同步发起本地 cancel/cleanup；supervised state只允许一次 finalization claim。
这些 owners相对初次 R5无 diff。

### 4.6 DB O6 与同步 DbQuery

DB operation/transaction/lease继续只通过
`runtime/eval/src/program_db/wait.rs::await_operation`进入
`ActorExecutionFrame::await_if_pending`；transaction复用同一 owner，没有复制 wait owner。

`DbQuery` route仍为：

```text
eval_context.rs LinkedExprIr::DbQuery
  -> program_db.rs::eval_program_db_query_value
  -> db_eval.rs::eval_query_value
  -> materialize query IR
```

该 route没有 DB store operation、`program_db::wait::await_operation`或 `ExternalWait`，保持
first-poll Ready同步例外。`program_db.rs`、`program_db/**`和`db_eval.rs`相对初次 R5全部无
diff。

### 4.7 组合写集与兼容边界

R6/R8只改变两个 private future的栈/堆布局；R7 non-test production diff为零。冻结候选没有新增
public API、Cargo/lockfile变化、`RUST_MIN_STACK`/`RUSTFLAGS`依赖、test stack配置、
dual-read/dual-write、legacy adapter或compatibility路径。两个既有 test-local stack helper已在
2.2按路径分类，且不在候选 diff。

因此初次 R5 的全部静态结论在当前组合候选仍成立，没有残留 production owner、重复 owner或
fallback。

## 5. T05–T12 执行证据映射

以下沿用初次 R5冻结的测试/production映射；本次状态不再只是 inventory存在，而是完整 lib
`395/395` 无过滤实际通过，并由对应 combined case补充真实 public/integration入口。

| ID | 实际 tests | production路径 | 本次执行证据 |
| --- | --- | --- | --- |
| T05 | timeout statement normal/max/parent restore、expression value/child restore、zero-ms real root arms | `eval_context.rs` timeout root → `eval_context/timeout.rs` child scope | timeout inventory `11/11`随完整 lib通过；combined R2通过 |
| T06 | local owner inner-catch miss/outer-catch hit、ordinary catch/rethrow | `timeout.rs::materialize_owned_timeout` → request-local exception/catch/rethrow | 完整 lib通过 |
| T07 | nested inner earlier、outer earlier、equal absolute deadline owner | nested timeout wrappers + `is_owned_by` | 完整 lib通过 |
| T08 | inherited request deadline not extended/materialized/caught | inherited `ExecutionScopeTerminal`透传 | 完整 lib通过 |
| T09 | ancestor cancel same-poll priority/lifecycle zero、future drop parent/lifecycle zero | E1 scope priority/lifecycle + timeout wrapper | 完整 lib通过 |
| T10 | pure CPU loop checkpoint、generated array chunk、instruction-count accounting、shared current/derived scope control | `eval_context/checkpoint.rs`与真实 evaluator entry/chunk/backedge | spine inventory `23/23`随完整 lib通过；combined R1通过 |
| T11 | serial dependency/fence/value tail/source-order error/outer-terminal priority，以及 Actor concurrent Ready/Pending/Error | `eval_context/concurrent.rs` → E2 scheduler → E3 Actor bridge | concurrent inventory `11/11`随完整 lib通过；combined R3通过 |
| T12 | winner/loser cleanup、late heap isolation、Actor error/parent restore；stream natural End、non-End cleanup、current child scope与provider publication | concurrent winner/cleanup；三个 stream current-scope child；`StreamConsumerCleanup` | stream inventory `22/22`随完整 lib通过；combined R4 activation/stream通过 |

五条 R7 deadline tests在T12 current-scope/raw boundary上补充并真实执行：

- raw boundary前保留 `ScopeTerminalCarrier(InheritedDeadlineExceeded)`；
- raw boundary后只暴露内部 `StreamRuntimeError::Cancelled`；
- attached consumer不再竞态性观察 `End`；
- buffered item先于blocked cancellation terminal；
- provider request cancel与stream lifetime精确收束一次。

## 6. 九组 actual-Pending / 同步例外映射

| # | group | 实际 tests / 语义 | 本次执行证据 |
| ---: | --- | --- | --- |
| 1 | Emit projected | Ready保留segment；Pending完成前reacquire | 完整 lib通过 |
| 2 | Emit detached | Ready保留segment；Pending只切一次 | 完整 lib通过 |
| 3 | Emit canonical wire | Ready首 poll完成；Pending恢复同一send一次 | 完整 lib通过 |
| 4 | remote interface | Ready保留segment；Pending在finalize前reacquire | 完整 lib通过 |
| 5 | callback | Ready保留segment；Pending在caller-heap finalize前reacquire | 完整 lib通过；初次 R5 stack blocker已关闭 |
| 6 | Actor dispatch | Ready保留segment；Pending在finalize前reacquire | 完整 lib通过 |
| 7 | legacy outbound | unary Pending；serverStream同步Ready例外 | 完整 lib通过 |
| 8 | native/composite | native Ready/Pending、WebSocket sync error、DbQuery Ready、`createFromStream` Pending success/drop | 完整 lib通过 |
| 9 | activation-relative | unary Ready/Pending/failure import；serverStream同步Ready例外 | 完整 lib通过；combined R4 activation通过 |

WebSocket、legacy/activation serverStream和DbQuery保持同步，不因静态 effect预释放。callback与
activation concrete wait仅在private call-site装箱，九组仍共享同一个 actual-Pending/E3 owner。

## 7. Blocking、non-blocking、warnings与残余风险

### Blocking

无。deadline、combined、完整 eval、locked check、fmt、diff和静态反向检查全部通过。

### Non-blocking / warnings

Cargo输出只有候选已有且未被本 gate配置为deny的 warnings：

- `skiff-compiler-source`：27个 unused import/dead-code类 warning；
- `skiff-runtime-linker`：32个 dead-code类 warning；
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`：test-only unused
  `LinkedCallTarget` import；
- `runtime/eval/src/assembly_execution/service_error_channel.rs`：既有
  `PlatformBuiltinErrorIdentity` unreachable-pattern warning。

R6/R8两个 changed call-site和R7 changed test module没有新增 warning。完整 eval每个
binary/doc-test均为0 ignored；eval source/tests的 `#[ignore]` 搜索同样为0。

两个既有 test-only 16MiB helper已在2.2分类；它们不是本次 blocker测试、不是候选新增，也不影响
默认栈下 callback/activation/`ordinary_exact_public...` closure。

### 未运行与残余风险

依合同禁止面，没有启动或访问：

- stable instance、本地服务或live selector；
- network；
- MongoDB；
- loop-risk stress或其它压力环境；
- 其它仓库。

这些面保留未验证风险，但不属于本次 hermetic E4R reacceptance gate，也不是 blocker。

## 8. 交付边界

唯一 tracked写入是本 result文件。没有修改 production、tests、fixture、Cargo、manifest、
lockfile或其它文档；没有派子 Agent，没有 merge、rebase或push。result commit hash由最终交付
消息记录，提交后再次确认 worktree clean。
