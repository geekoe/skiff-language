# P5-F445H-O4 Callback prepared state machine result

状态：`IMPLEMENTATION_COMPLETE / CALLBACK_GREEN / FULL_EVAL_GREEN`。

callback capability 已拆成同步 `prepare`、只拥有 owner 状态的 `wait`、以及回到 caller
segment 后执行的 `finalize`。跨等待生命周期不再借用 caller `RequestHeap`、`Env` 或
`EvalContext`，也不会继承 caller Actor frame；现有 async 入口只是薄组合这三个阶段，留给
E4R 接入 E3 的 actual-Pending seam。

## 1. 输入、提交与写集

| 项 | 值 |
| --- | --- |
| production prerequisite | `d39ad5b0` |
| task document checkpoint | `87e85911` |
| implementation | `acce0964` |
| direct parent | `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md` |
| direct parent | `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o4-callback` |
| branch | `codex/p5-f445h-o4-callback` |

implementation 写集精确为：

- `runtime/eval/src/assembly_execution/callback_native.rs`
- `runtime/eval/src/assembly_execution/callback_native/prepared.rs`
- `runtime/eval/src/assembly_execution/callback_native/prepared_state_tests.rs`
- `runtime/native/src/callback_adapter.rs`

没有修改 `assembly_execution/mod.rs`、`eval_context.rs`、Actor、其它 native dispatch、host、
manifest 或 lockfile，也没有运行 stable、live、network。

原 `callback_native.rs` 已从 553 行降到 467 行；新的 291 行 `prepared.rs` 独立拥有 callback
状态机，373 行行为矩阵位于测试 child file，没有继续把 owner 生命周期堆进原长文件。

## 2. Test-first 证据

先加入 owned guard 与 prepared state tests，再运行任务规定的两个 focused 命令。

eval RED：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo test -p skiff-runtime-eval callback_native -- --nocapture
```

exit `101`，真实缺口为：

- `CallbackOwnerWait` 不存在；
- `CallbackOwnerWaitOutcome` 不存在；
- `CompletedCallbackInvocation` 不存在；
- `prepare_owner_arguments` 不存在；
- `validate_callback_request_generation` 不存在。

native RED：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target \
  cargo test -p skiff-runtime-native callback_adapter -- --nocapture
```

exit `101`，真实缺口为：

- `try_lock_owner_heap_owned` 不存在；
- `CallbackAdapterError::OwnerStateUnavailable` 不存在。

两个 RED 都来自任务要求的 owned owner-heap / prepared state API 缺失，不依赖旧 exhaustive
match、伪造 Pending 或写集外组件。

## 3. Owner heap authority

`InProcessCallbackAdapter` 不再返回 borrowed `&Mutex<RequestHeap>`。它只提供
`try_lock_owner_heap_owned()`：

- clone 内部 `Arc<tokio::sync::Mutex<RequestHeap>>` 后返回 `OwnedMutexGuard`；
- guard 生命周期不借 adapter；
- 同一 callback 重入立即返回 `OwnerStateUnavailable`，不会等待自己持有的锁；
- guard drop 后同一 owner heap 可再次取得，owner 状态继续可见；
- adapter 没有新增独立的任意 heap mutation 方法。

eval 将这个 guard 封装为 invocation-scoped `CallbackOwnerWait`。prepared、completed、error、
cancel 和 future drop 的所有路径都只由该 guard 的单一所有权释放锁，没有第二个 unlock 或
重建 future 的路径。

## 4. Prepare / wait / finalize

### 4.1 Prepare

`prepare_interface_call` 在 caller 同步 segment 内完成：

1. 校验当前 request generation；
2. 只按 opaque owner activation id 查找 owner；
3. 经 capability table 校验 runtime/owner/generation/opaque id 与 lifetime；
4. downcast 精确 adapter；
5. 校验 canonical contract identity、空 type arguments、operation slot、method ABI 和 arity；
6. 切换到精确 owner activation；
7. 通过 `OwnedProgramExecutionContext::capture` 固化 owner execution context；
8. 取得 owned owner-heap guard；
9. 把 caller arguments 按 contract plan 物化到 owner heap；
10. 保存 owner receiver、owner args、executable、caller address、类型替换和 stream calling
    context。

参数物化前记录 owner heap checkpoint。任一参数失败时只截断本次新增 allocation并恢复 stats；
既有 owner heap 节点及其可见 mutation 语义不变。成功后不把该 checkpoint误当作完整 heap
transaction。

保存的 call env 是新建的 owner call env，只 clone 现有调用语义需要的 stream sink、
response stream sink、current stream item type 与 type substitutions；caller slot store及其中的
caller heap handles不会进入 wait。

### 4.2 Owned wait

`PreparedCallbackInvocation::wait`只持有：

- `OwnedMutexGuard<RequestHeap>`；
- `OwnedProgramExecutionContext`；
- owner call env；
- owned receiver、arguments、addresses、type arguments与返回 contract facts；
- borrowed `&Interpreter`。

它不持有 caller heap、caller env、`EvalContext` 或 caller Actor frame。wait 调用精确 owner
executable一次，不在首次 poll 后 drop/restart。`OwnedProgramExecutionContext`当前没有
`actor_execution_frame` 字段；prepare 和 wait 都额外 fail closed，若后续 capture错误地带入
frame，会返回 `InvalidArtifact`，不会复用 caller scheduler lease。

### 4.3 Finalize

`CompletedCallbackInvocation`显式交回 owner guard与 owner terminal：

- normal result在 guard仍有效时按 provider-owned contract plan导入 caller heap，随后释放 guard；
- method error与 cancel不导入 caller heap，直接释放 guard并保留原错误；
- result materialization error同样由 RAII释放 guard；
- future在 Pending时被 drop/abort，guard随 future释放；
- method error、cancel与drop不会回滚已经成功写入的 owner状态，保持原有可见性；
- `finalize(self, ...)`消费 outcome，类型系统阻止第二次导入。

原 `execute_interface_call`现在只组合：

```text
prepare_interface_call(...)
  -> prepared.wait(interpreter).await
  -> completed.finalize(caller_heap)
```

它当前保持原调用行为；E4R可把同一个 `prepared.wait(...)` future交给 E3
`await_if_pending`，无需重写 callback owner 状态机。

## 5. 自验收矩阵

| 任务合同 | production 证据 | 测试证据 |
| --- | --- | --- |
| owned owner heap入口 | adapter `try_lock_owner_heap_owned` | `owned_owner_heap_guard_is_exclusive_and_released_once` |
| wait期间caller heap/env独立可访问 | prepared字段无caller borrow；owner call env为新值 | `pending_wait_owns_only_owner_state_and_invokes_once` |
| Ready/Pending只执行一次 | 单一 consuming wait；无restart路径 | `ready_wait_invokes_once_and_finalizes_once`、`pending_wait_owns_only_owner_state_and_invokes_once` |
| 参数失败恢复checkpoint | `prepare_owner_arguments` checkpoint/rollback | `parameter_prepare_failure_restores_owner_checkpoint` |
| normal result先导入后释放 | completed outcome持guard至finalize | `finalize_imports_owner_heap_result_before_releasing_guard` |
| method error/cancel保留owner mutation | terminal不回滚成功owner state | `method_error_and_cancel_release_guard_without_rollback` |
| Pending drop不重启且释放一次 | future单一拥有guard | `dropped_pending_wait_releases_guard_once_without_restart` |
| generation固定错误不变 | generation helper仍映射 `CapabilityUnavailable` | `generation_mismatch_keeps_the_fixed_unavailable_error` |
| slot/method ABI边界不变 | adapter projection与 `operation(slot, methodAbi)` | native adapter 既有 operation bounds/mapping tests |
| 递归 evaluator不持caller Actor frame | owned context无frame字段；prepare/wait双重fail closed | `prepared_recursive_wait_owns_context_without_actor_frame` |
| reentry不死锁 | owned try-lock立即失败 | native owned-guard exclusivity test |

## 6. 验证

所有 Cargo 命令均使用：

```text
/Users/geek/workspace/skiff-p5-f445h-o4-callback/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval callback_native -- --nocapture` | PASS：实际执行 11/11 unit tests；其它 test binary 0 个匹配，不计数 |
| `cargo test -p skiff-runtime-native callback_adapter -- --nocapture` | PASS：实际执行 8/8 unit tests |
| `cargo test -p skiff-runtime-eval --locked --no-fail-fast` | PASS：272/272 unit、4/4 catch integration、6/6 representation integration、1/1 doc test |
| `cargo check -p skiff-runtime-eval -p skiff-runtime-native --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 compiler-source unused、linker dead-code、ordinary test unused import和
`service_error_channel.rs` unreachable-pattern warnings；本节点没有新增 warning。

反向检查确认：

- production 不再存在 borrowed `owner_heap()`入口；
- prepared wait字段没有 `&mut RequestHeap`、`&mut Env`或`EvalContext`；
- caller `context.heap` / `context.env`只在同步 prepare中使用；
- 没有新增 `unsafe`、`yield_now`、future restart、pre-suspend或 Actor修改。

## 7. 后继接口与决策

E4R可直接消费：

1. `callback_native::prepare_interface_call(...)`；
2. `PreparedCallbackInvocation::wait(&Interpreter)`；
3. E3 actual-Pending seam返回后调用
   `CompletedCallbackInvocation::finalize(&mut RequestHeap)`。

当前没有需要用户决定的语义，也没有命中 `TASK_SCOPE_EXPANDED`：

- owner heap可形成安全 owned guard；
- caller Actor frame无需且不会被捕获；
- parameter failure、method error、cancel与drop均保留既有可见性；
- operation/generation/method ABI与固定 unavailable error未改变。
