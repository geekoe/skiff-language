# P5-F445H-I6E5 file current-scope consumer resume result

状态：

```text
IMPLEMENTATION_PASS
TASK_SCOPE_EXPANDED = NO
I6_FILE_COMPLETE = YES
```

E1 交付的 invocation-time `OwnedExecutionControl` 现在由 Host file adapter 在每次
direct/provider/source operation 开始时读取。六个 direct/provider operation 与 source `next`
统一经过一个 scoped lower-future owner；`FileIngest` 在 scope winner drop 路径同步清理 spill
临时文件，既有 `StagedFile` drop owner 保持不变。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| integration base commit | `e942efa99460ea2b9bf29f07d8dfe855c9715aff` |
| integration base tree | `46abc10c8fbdab6e70f2ea071539382dbf03a1be` |
| task publication HEAD | `8628b37cd0056c550ef62ab40a5aa3e54b06baab` |
| implementation commit | `05c5e2bfb567ebf2468e9e8039782339b2577bfa` |
| implementation tree | `cac6fec9aec4768e7a72521da2f6c94518fa90b3` |

implementation commit 相对 task publication HEAD 只修改合同允许的六个 production/test 文件，
没有修改 `store.rs`、`stream_runtime.rs`、E1 接口、DB state machine、Cargo 或 lockfile。

## 2. RED / GREEN

实现前的父节点 RED 为 Host `f445h_i6_file_scope` selector listing `0 tests`；E1 tree 中六个
direct/provider operation 与 source `next` 均显式丢弃 `_execution_control`，真实 Pending 只能等待
旧 lower future，current scope 无法唤醒它。

实现迭代中的正常竞争测试还捕获了一个真实 winner bug：若在 `tokio::select!` branch 内才调用
`completion.complete()`，未选中的 `lease.wait()` future 会先被 drop，Ready lower 被错误投影为
`Execution(Cancelled)`。最终实现把 completion commit 放进 lower wrapper，确保 lower Ready
先提交时不会被 select cleanup 或随后 signal 覆盖。

最终 GREEN：

```text
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list
6 tests

cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture
6 passed / 0 failed

cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --list
1 test

cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --nocapture
1 passed / 0 failed
```

两个 selector listing 均非零，listing 与 execution 数量一致。

## 3. 资源 owner 矩阵

| 资源 / effect | canonical owner | normal | scope terminal / drop |
| --- | --- | --- | --- |
| current scope read | `scoped_file_future` | 每次 operation 第一次 poll 读取一次 full scope | scope unavailable fail closed 为 decode |
| lease / waiter / timer | adapter `scoped_file_future` | lower wrapper 在返回 output 前调用 completion owner | lease winner drop lower；所有 lifecycle counter 归零 |
| six direct/provider lower futures | 既有 concrete `FileCapabilityContext` / store provider 选择 | 原 output 原样返回 | adapter 只 drop future，不声称撤销已开始的 blob/DB/spawn-blocking effect |
| source channel/pull waiter | 既有 `stream_runtime.rs` | lower item/End/error 先完成 adapter completion | adapter lease drop lower waiter；late item 与 late error sender 均无法再发布 |
| ingest spill path before finish | `FileIngest::drop` | `finish` take path 并转交 `StagedFile` | scope winner drop 同步 close file handle并删除 path |
| staged spill path after finish | 既有 `StagedFile::drop` | persist 既有 async cleanup | drop fallback 保持，未建立第二 supervisor |
| provider/store selection | 既有 `store.rs` | 不变 | adapter 外围唯一 acquire；store 层零新增 lease |
| exact deadline owner | current `ExecutionScope` + Eval post-await checkpoint | normal winner不重投影 | lower识别 deadline terminal；checkpoint 恢复 local/inherited owner与 source/site/nesting |

Host deadline、ancestor stop、late source和 cleanup cases结束后均断言：

```text
active_leases=0
active_waiters=0
active_timers=0
```

## 4. 行为与真实 receipt

### Host scoped owner

`RuntimeFileCapabilityContext` 的 create/read/readText/info/delete/createFromStream 及
`RuntimeOwnedFileSourceStreamContext::next_file_source_stream_item` 都调用同一个
`scoped_file_future`：

1. 从本次调用 carrier 读取 full current `ExecutionScope`；
2. acquire 唯一外围 lease；
3. lower wrapper 在 output Ready 后先 CAS completion；
4. biased select 只让尚未提交的 scope terminal drop lower；
5. scope winner用 carrier post-await poll 投影 cancellation或 deadline budget terminal；
6. 不把未知外部副作用报告为已撤销。

聚焦 receipt 覆盖 direct Ready、direct Pending current deadline、provider Pending inherited
request deadline、source Pending ancestor stop、normal-first 竞争、late item/error fence、
`FileIngest` spill drop清理与所有 owner归零。

### createFromStream 纵向 receipt

`f445h_i6_file_projection_to_pending_preserves_current_deadline_owner` 经过真实 linked
`std.file.createFromStream` native call、Eval projection/wrapper、file capability method与真实
Pending wait。recording lower收到 current carrier并在其 deadline lease上 Pending；deadline胜出后：

- lower post-await poll观察 `DeadlineExceeded`，而非 request-root cancellation；
- native wait只暴露内部 cancellation terminal，不伪装 ordinary外部错误或 rollback；
- 同一个 `ProgramExecutionContext` post-await checkpoint恢复
  `ScopeTerminal(LocalDeadlineExceeded)`，且 `is_owned_by(current_scope)`；
- lower wait只 drop一次，completion为零，scope lifecycle全部归零。

这条 receipt 与 Host scoped-owner tests 组合证明 carrier 从真实 Eval调用到 pending consumer，
以及生产 Host adapter 的 winner/cleanup语义；没有使用 network、stable、live或 MongoDB。

## 5. 验证

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| Host selector list | `cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list` | PASS；6 tests |
| Host selector run | `cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture` | PASS；6/6 |
| Eval selector list | `cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --list` | PASS；1 test |
| Eval selector run | `cargo test -p skiff-runtime-eval f445h_i6_file_projection_to_pending -- --nocapture` | PASS；1/1 |
| locked check | `cargo check -p skiff-runtime-host -p skiff-runtime-eval --locked` | PASS；仅既有 warnings |
| Rust format | `cargo fmt --check` | PASS |
| diff | `git diff --check` | PASS |

一次最终证据重建最初因本机磁盘 `No space left on device` 在 compile 阶段中断；只执行
`cargo clean` 清理本 worktree 可再生 build cache后，上表全部命令在最终 implementation tree
重新执行并 PASS。未运行 full gate。

## 6. 实际写集与反向搜索

实际写集：

```text
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/eval_capability_adapter/file_stream_tests.rs
runtime/host/src/host/file_runtime.rs
runtime/host/src/host/file_runtime/tests.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/file_create_from_stream.rs
runtime/eval/src/actor_executor/tests/actor_concurrent_continuation_tests/evaluator_actual_pending/support.rs
```

反向核对：

- `file_stream.rs` 中旧 `_execution_control = execution_control` 为零；
- `scoped_file_future(...)` 有七个 operation callsite；
- adapter/store/stream 三文件的 `acquire_lease()` 只有 adapter 一个 production命中；
- `FileIngest` 与 `StagedFile` 各保留一个明确 Drop owner；
- `store.rs`、`stream_runtime.rs`、E1共享接口、DB、Cargo/lockfile均无 diff。

```text
I6_FILE_COMPLETE = YES
```
