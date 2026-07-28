# P5-F445H-I6D time/file/Actor/response-source current scope result

状态：

```text
PARTIAL_IMPLEMENTATION
TASK_NOT_EXECUTABLE = YES
TASK_SCOPE_EXPANDED = YES
I6_D_COMPLETE = NO
D1_TIME_COMPLETE = NO
D2_RESPONSE_SINK_COMPLETE = YES
D3_FILE_COMPLETE = NO
D4_ACTOR_COMPLETE = NO
I6_J_I6D_COMBINED_CASE_UNBLOCKED = NO
READY = NO
```

本任务按合同把 D1 time、D2 Host response sink、D3 file、D4 Actor 派给四个独立
worktree/branch。四个分片共同冻结为只消费 I6-A 已有 invocation-time owned execution carrier，
不得新增公共 API 或吞并其它 production owner。

有界探查证明 I6-A 只把 carrier 存入 Eval crate 的 wrapper；D1、D3、D4 的实际 consumer seam
仍位于本合同禁止写的 Eval/native owner。三条分片均按停止条款保持 clean，没有影子实现。D2 可在
唯一允许文件内闭环，已集成为一个独立、可保留的实现检查点。

## 1. 候选身份

| 项 | 值 |
| --- | --- |
| 合同固定 implementation base commit | `8db08c539acaf0b3fc41733365f06e9883bdbdd8` |
| 合同固定 implementation base tree | `71123064dd0948d5946ad8c6312df909670794e0` |
| 父任务派发 HEAD | `baf2547d37e2f9103a360c9615fb29a9bb6584c9` |
| 父任务派发 tree | `f936f711eba2bd2ca73ce7b59e8d404004b6923f` |
| D2 source implementation commit | `d01c00094c666071873f741a3aa4991940d51dff` |
| D2 source implementation tree | `2590181fb6cc1d2e5e91ed6f35eee4ea4663c9a7` |
| D2 integrated commit | `44568435aee8e59fe2437c0cbc3e0a60f1315f50` |
| D2 integrated tree | `2590181fb6cc1d2e5e91ed6f35eee4ea4663c9a7` |
| parent worktree | `/Users/geek/workspace/skiff-p5-f445h-i6d-host-ops` |
| parent branch | `codex/p5-f445h-i6d-host-ops` |

D2 cherry-pick 后 tree 与 source implementation tree 相同；集成没有冲突或机械 constructor 跟随。

## 2. 分片结果与实际写集

| 分片 | worktree / branch | 提交与 tree | 实际写集 | 结论 |
| --- | --- | --- | --- | --- |
| D1 time | `/Users/geek/workspace/skiff-p5-f445h-i6d-d1-time` / `codex/p5-f445h-i6d-d1-time` | 无提交；停在派发 HEAD/tree | 空 | `TASK_NOT_EXECUTABLE` |
| D2 response sink | `/Users/geek/workspace/skiff-p5-f445h-i6d-d2-response` / `codex/p5-f445h-i6d-d2-response-sink` | `d01c00094c666071873f741a3aa4991940d51dff` / `2590181fb6cc1d2e5e91ed6f35eee4ea4663c9a7` | `runtime/capability-context/src/stream.rs` | PASS，已集成 |
| D3 file | `/Users/geek/workspace/skiff-p5-f445h-i6d-d3-file` / `codex/p5-f445h-i6d-d3-file` | 无提交；停在派发 HEAD/tree | 空 | `TASK_SCOPE_EXPANDED / TASK_NOT_EXECUTABLE` |
| D4 Actor | `/Users/geek/workspace/skiff-p5-f445h-i6d-d4-actor` / `codex/p5-f445h-i6d-d4-actor` | 无提交；停在派发 HEAD/tree | 空 | `TASK_NOT_EXECUTABLE` |

父任务相对派发 HEAD 的 production/test 实际写集精确为：

```text
runtime/capability-context/src/stream.rs
```

没有修改 Cargo/lockfile、compiler/artifact/std 签名、Actor/Router wire、legacy outbound、DB E4/O6
state machine、stable instance 或真实外部状态。

## 3. 三个合同 blocker

### 3.1 D1 time

`runtime/native/src/dispatch/time.rs` 的 generic `TimeContext` 只能调用
`runtime/native/src/capability.rs::NativeTimeCapability::poll_execution_budget()`。I6-A carrier
保存在 `runtime/eval/src/capabilities.rs::RuntimeNativeTimeCapabilityContext`，其 accessor 只在 Eval
crate 内可见；`RuntimeNativeInvocation` 也不携带 scope。

因此 paused-clock sleep 若要直接等待 current absolute deadline 与全部 signals，至少需要修改禁止写的
native capability trait 与对应 Eval implementation，或先增加另一个共享 carrier seam。D1 唯一写集
无法完成该行为。

### 3.2 D3 file

I6-A carrier 保存在 `RuntimeNativeFileCapability` 和
`RuntimeNativeFileSourceStreamCapability`，但两者位于禁止写的
`runtime/eval/src/capabilities.rs`，其 direct operation 与 source wait implementation 仍忽略
carrier。允许写的 Host `FileCapabilityContext` 只有 runtime/DB，`FileSourceStreamContext` 只有旧的
request-construction execution snapshot，更低层 `FileRuntime` 没有 execution scope。

因此 direct/provider/source Pending 的 current winner 必须先由 Eval owner把 invocation carrier
传入 Host file seam；当前 D3 写集无法完成。

### 3.3 D4 Actor

get/create/replace/find/remove 的 carrier consumer seam 位于禁止写的
`runtime/eval/src/capabilities.rs::NativeActorCapability` implementation；spawn 的 current execution
seam 位于禁止写的 `runtime/eval/src/spawn_ops.rs`。允许写的 Actor dispatch文件只覆盖 method，
Host actor context仍只拿到 request-root cancellation。

因此即使单独修改 method，也不能满足合同要求的 Actor control + spawn 完整分片；需要先重发包含精确
Eval carrier owner的合同或共享 seam checkpoint。

这三个 blocker 都是写入 owner/合同范围被实际调用链证伪，不是可以由父任务机械 constructor 跟随处理的
组合编译问题。

## 4. D2 RED / GREEN 与语义证据

真实 RED：

- deadline 与 ancestor-stop 两个测试在旧实现上不能唤醒 capacity Pending；三测试为
  `1 passed / 2 failed`。
- normal capacity completion 的竞争顺序补充测试曾返回 cancellation；精确为
  `0 passed / 1 failed`。

GREEN：

1. `HttpResponseStreamCapabilityContext` 从 invocation execution取得 current `ExecutionScope`，
   acquisition建立 lease。
2. lower sink同时取得原 root token和lease child cancellation；current deadline/ancestor stop
   winner会drop send future，late capacity notify不能再写 response。
3. normal send branch先调用 completion owner `complete()`，再返回 lower output；scope loser不能把
   normal capacity completion误投影为 cancellation。
4. scope terminal只返回内部 `StreamRuntimeError::Cancelled`，不调用 sink `end`/`fail`。
5. natural End与非End cleanup仍由既有 `StreamConsumerCleanup` 拥有。

聚焦测试精确为 `4 listed / 4 passed`。deadline、ancestor-stop、normal completion 后的
`active_leases/waiters/timers` 均归零；scope winner 后 fake sink `pending=0`、`writes=0`，
late notify不增写。natural End cleanup cancel为 `0`，非End为 `1`。

## 5. 组合验证

| 层级 | 命令 | owner | 代码状态 | 结果 | 覆盖 |
| --- | --- | --- | --- | --- | --- |
| focused list | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list` | parent | `44568435` | `0 tests`，不满足合同 | D1缺失 |
| focused run | `cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture` | parent | `44568435` | `0 passed / 113 filtered` | D1缺失 |
| focused list | `cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list` | parent | `44568435` | 所有test binary均 `0 tests` | D3缺失 |
| focused run | `cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture` | parent | `44568435` | 所有test binary均 `0 passed` | D3缺失 |
| focused list | `cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list` | parent | `44568435` | 所有test binary均 `0 tests` | D4缺失 |
| focused run | `cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture` | parent | `44568435` | 所有test binary均 `0 passed` | D4缺失 |
| focused list | `cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --list` | parent | `44568435` | PASS；`4 tests` | D2 |
| focused run | `cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --nocapture` | parent | `44568435` | PASS；`4/4` | D2 |
| combined check | `cargo check -p skiff-runtime-native -p skiff-runtime-capability-context -p skiff-runtime-eval -p skiff-runtime-host --locked` | parent | `44568435` | PASS；仅既有 warnings | 四包接线 |
| format | `cargo fmt --check` | parent | `44568435` | PASS | Rust格式 |
| diff | `git diff --check` | parent | `44568435` | PASS | working tree |

没有运行完整 crate/stage gate、stable/live/network/MongoDB。

## 6. 反向搜索

```text
rg "parts.cancellation.wait_cancelled|context.cancellation_token().wait_cancelled" ...
  runtime/host/src/capability_context/actor.rs:405
  runtime/host/src/eval_capability_adapter/actor.rs:432
```

root-only Actor wait仍有 `2` 个 production命中，对应 D4 blocker。

```text
rg "send_with_cancellation(.*cancellation_token" runtime/capability-context/src/stream.rs
  0 hits
```

D2旧的单行 root-only sink形态为零；当前调用显式同时携带 root token与
`child_cancellation`。

`FileIngest|StagedFile` 在 `file_runtime.rs` 保持现有 `10` 个定义/使用命中；D3没有改变 staging
owner，也没有伪装撤销已开始的 blob/DB effect。

`30_000` 搜索仍有 `4` 个命中，其中 production
`runtime/eval/src/actor_dispatch.rs:107` 仍为固定 primitive fallback，尚未与 current effective
deadline取min；其余为现有测试。该 production命中对应 D4 blocker。

## 7. DAG 与重发要求

- I6-J response sink局部 case：`UNBLOCKED_BY_D2`；
- I6-J time case：`BLOCKED_BY_D1_CARRIER_SEAM`；
- I6-J file case：`BLOCKED_BY_D3_EVAL_CARRIER_SEAM`；
- I6-J Actor case：`BLOCKED_BY_D4_EVAL_AND_SPAWN_CARRIER_SEAMS`；
- I6-J 的 I6-D combined case：`NOT_UNBLOCKED`。

重发前需要把实际 carrier consumer owner固化进新合同。最小影响面至少包括
`runtime/native/src/capability.rs`、`runtime/eval/src/capabilities.rs` 和
`runtime/eval/src/spawn_ops.rs` 中与三条分片直接对应的 seam及其聚焦 fixture；也可以先发一个更窄的
内部 carrier seam checkpoint，再重新扇出 time/file/Actor。两种方式会改变当前写集与依赖，父任务不能
在本合同内自行选择或实现。

D2提交是有效的局部实现检查点，但当前代码状态不是完整 I6-D 预验收候选，不能宣称 Ready。
