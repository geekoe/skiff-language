# P5-F445H-O3 In-process service prepared operation result

状态：`IMPLEMENTATION_COMPLETE / O3_GREEN`。

canonical activation-relative unary service 已拆成同步 `prepare`、caller-free owned `wait` 和
resume 后 `finalize`。owned wait 的返回类型为 `Send + 'static`，以 `async move` 持有
provider interpreter/context/heap/invocation env/request/arguments；它不借 caller
`RequestHeap`、`Env`、`EvalContext` 或 Actor frame。现有 async 入口只薄组合三阶段，供 E4R
接入 E3 actual-Pending seam 前维持编译。

serverStream 的同步 setup、producer task、consumer `next()` 与各自 cleanup owner没有进入 unary
协议，也没有改动 legacy relay、公共 provider/request API、Actor 或 evaluator call site。

## 1. 输入与提交

| 项 | commit |
| --- | --- |
| production prerequisite | `d39ad5b0` |
| O1–O5 task checkpoint | `87e85911` |
| implementation | `010a6bcd609b29ef9f9bc8cb07905ddd8a36e252` |

implementation 写集精确为：

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary.rs`
- `runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary_tests.rs`

## 2. Test-first 证据

首先加入
`prepared_provider_unary_wait_does_not_borrow_caller_heap_or_env`，在没有 production API 时运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o3-in-process-service/build/cargo-target \
  cargo test -p skiff-runtime-eval \
  prepared_provider_unary_wait_does_not_borrow_caller_heap_or_env -- --nocapture
```

结果为预期 RED，exit `101`：

```text
cannot find function `prepare_provider_unary` in this scope
```

测试要求 wait future 满足 `Future + Send + 'static`；future 存活时继续分配 caller heap并修改
caller env，因此旧的 `execute_provider_unary(&mut EvalContext).await` 不能伪装通过。

实现第一版后又增加“already-fixed failure不得重复提交诊断”断言。完整 focused suite 得到第二个
真实 RED：30/31 通过，`provider_normal_and_fixed_outcomes_are_deferred_to_finalize` 失败，证明旧
finalize 路径会重新 export 已固定错误。随后改为 fixed carrier直接转发；只有原始 provider
错误才执行 provider-local export。

## 3. prepare / wait / finalize

### prepare

`prepare_provider_unary` 在 caller 同步 segment内完成：

1. callback contract、参数数量、effect guarantee和 boundary value plan检查；
2. caller package identity和 provider activation/context解析；
3. 独立 provider heap创建；
4. caller参数向 provider heap的 canonical materialization；
5. provider executable、type arguments、request和 execution owner捕获。

传给 provider executable 的 invocation env从 `Env::new()`建立，只复制 owned stream
capability和 type substitutions；不复制 caller slots或 `self`，因此不会把 caller heap handle
带入 wait。

### owned wait

`PreparedProviderUnary::wait(self)` 显式返回：

```text
impl Future<Output = CompletedProviderUnary> + Send + 'static
```

其 `async move` 只持 owned provider state。provider executable只在该 future首次 poll时启动，
future本身不会预 poll、预释放 Actor segment或重建 provider request；Ready/Pending仍由后继 E3
首次 poll观察。

`ProviderUnaryRequestOwner` 在未完成 wait被 drop时取消 provider。terminal状态区分
provider outcome、caller cancellation和deadline，保证：

- caller cancellation和deadline分支各取消 request一次；
- provider自身返回 cancellation时由 owner取消一次；
- terminal形成后 disarm drop guard；
- pending future被取消时只 drop一次，late completion不能进入 finalize。

### finalize

`CompletedProviderUnary` 只保存 owned provider heap和 raw outcome。`finalize` 才重新接收
caller heap：

- normal result按 canonical return plan物化回 caller；
-原始 provider error在 provider heap仍存活时导出 fixed service failure；
- already-fixed failure原样转发，不重复诊断；
- cancellation原样返回；
- materialization失败由 boundary checkpoint回滚，不留下 caller部分 import。

## 4. 自验收矩阵

| 任务合同 | production / 测试证据 |
| --- | --- |
| wait不借 caller heap/env/context | `wait -> impl Future + Send + 'static`；`prepared_provider_unary_wait_does_not_borrow_caller_heap_or_env` 在 future存活时独立修改两者 |
| owned context不捕获 caller Actor frame | `OwnedProgramExecutionContext`是唯一 context载体；prepared test确认借出的 provider context没有 Actor frame |
| Ready不强制 cut、Pending只启动一次 | 既有 `ready_provider_unary_returns_without_forced_yield`；pending completion测试；新 start oneshot只允许 provider future启动一次 |
| finalize前不写 caller | actual provider user-error wait前后比较 caller checkpoint/stats |
| normal result | test-only raw completion在 finalize前保持 caller heap为空，finalize返回 declared string |
| user error | 真实 provider throw只在 finalize导出，诊断恰好1次 |
| fixed failure | fixed carrier原样转发，新增诊断为0 |
| cancel/deadline | 既有 caller cancel、expired deadline和biased ordering测试均通过 |
| drop与late result | unpolled wait drop取消 request；in-flight future drop计数为1，late oneshot发送失败 |
|失败原子性 | nested provider graph在受限 caller heap中间失败后 checkpoint/stats完全恢复 |
| serverStream不回归 | 既有 normal/error/cancel/deadline/publication/item/lifetime/task-counter矩阵全部包含在31/31 focused suite |
| wrapper保持单一协议 | `execute_provider_unary`仅为 `prepare -> wait -> finalize` |

## 5. 验证

所有 Cargo 命令使用独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o3-in-process-service/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval async_stream_cancel -- --nocapture` | PASS：实际执行31/31 unit tests；其它 test binary为0个匹配测试，不计作证据 |
| `cargo test -p skiff-runtime-eval service_error_channel_contract_operation_restricted_service_diagnostic_real_lanes -- --nocapture` | PASS：实际执行1/1 |
| `cargo check -p skiff-runtime-eval --locked` | PASS |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

输出只有既有 linker dead-code、compiler-source/test unused import和
`service_error_channel.rs` unreachable-pattern warnings；本节点没有新增 warning。

production反向搜索：

```text
rg 'yield_now|unsafe|suspend_actor_segment|may_suspend|maySuspend' \
  runtime/eval/src/assembly_execution/async_stream_cancel/prepared_unary.rs
```

结果为空。implementation没有修改 `assembly_execution/mod.rs`、`eval_context.rs`、Actor、
host/native/service-db、manifest或 lockfile，也没有运行 stable、live或network。

## 6. E4R 后继接口

E4R 可按以下顺序接线，不需要复制 provider materialization、request terminal或 error export：

1. 当前同步 segment调用 `prepare_provider_unary(...)`；
2. 把 `prepared.wait()`交给 E3 `await_if_pending`；
3. E3第一次 poll为Ready时不切 segment，真实Pending时才 suspend/resume；
4. resume成功后调用 `completed.finalize(caller_heap)`。

当前 `execute_service_call(...).await` 仍是薄兼容入口；E4R负责删除 evaluator现有 pre-suspend
pair。serverStream继续走同步 `start_provider_stream`，不应交给 unary wait。
