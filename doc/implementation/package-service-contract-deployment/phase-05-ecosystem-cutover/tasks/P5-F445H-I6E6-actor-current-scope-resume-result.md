# P5-F445H-I6E6 Actor current-scope consumer resume result

状态：

```text
IMPLEMENTATION_PASS
TASK_SCOPE_EXPANDED = NO
I6_ACTOR_COMPLETE = YES
```

Actor control、method 与 spawn 现在都在操作开始时消费 E1 交付的 current
`OwnedExecutionControl`。current/outer deadline 与 ancestor stop 直接竞争真实 outbound
lease；本地已提交 response 优先，scope winner drop 真实 waiter 并保留 late/duplicate fence。
method 的 30 秒 primitive 保持为操作自身上限，spawn 只在有效 receipt 后唤醒 worker。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| integration base | `e942efa99460ea2b9bf29f07d8dfe855c9715aff` / `46abc10c8fbdab6e70f2ea071539382dbf03a1be` |
| E1 implementation | `ba66719e03cbabde2e159b94761cc1a1c71b35d2` / `0b1972158d710c4355274f7fb272be292dcc7927` |
| task publication HEAD | `8628b37cd0056c550ef62ab40a5aa3e54b06baab` / `754b9ac86ead0a6012e06720024a4ee9ced5ece0` |
| I6E6 implementation | `b90be5785cd2a2f69c658592fd10ca28220ef69e` / `05874695fe48fb1e331ce5705dbcbdd966590bf4` |

implementation commit 同时包含合同要求的 production 与聚焦 tests；本 result 单独提交。

## 2. RED / GREEN

真实 RED 由直接父节点 I6D 在相同未实现 production 上冻结：

- `cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list` 为 `0 tests`；
- 对应 run 为 `0 passed`；
- Host Actor adapter 丢弃 `execution_control`，control 只等待 request-root cancellation，
  method 只竞争 request-root cancellation 与 primitive timer；
- Eval method primitive 从 outbound/request effective timeout 派生，不是固定的 30 秒 Actor
  primitive；
- task publication 只增加合同，未改变上述 production，因此该 RED 对本任务固定起点仍成立。

GREEN：

```text
cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --list
4 tests

cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --nocapture
4 passed / 0 failed

cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list
15 tests

cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture
15 passed / 0 failed
```

两个 selector listing 均非零，listing 与 execution 数量一致，共 `19/19` 通过。

## 3. 六类入口矩阵

| 入口 | current scope receipt | 真实 response owner | terminal / fence 证据 |
| --- | --- | --- | --- |
| get-or-create | borrowed/owned Host adapter 各读取一次 current scope，再调用 scoped concrete operation | `OutboundRequestLease` | ancestor stop drop waiter；registry lease/waiter 归零；late complete 被拒绝 |
| replace | 同上 | `OutboundRequestLease` | ancestor stop、request.cancel reason 与 late fence 由四入口真实循环 case 覆盖 |
| find | 同上；E1 `actor_find` receipt 已证明 Eval projection 把同一 owned control 交到 Host seam | `OutboundRequestLease` | 已提交 find response 与同刻 ready deadline 竞争时 response 胜出且不发 cancel |
| remove | 同上 | `OutboundRequestLease` | ancestor stop drop owner，late/duplicate response 被既有 registry fence 拒绝 |
| method | prepared operation 携带同一个 current control；Host operation start 读取一次 scope | `ActorMethodOutboundLease` | current/outer deadline、ancestor stop、root internal stop、primitive deadline、response-first、late/duplicate 与 lifecycle 归零均有 selector |
| spawn | canonical Eval spawn fixture 收到同一个 current control；Host operation start 读取一次 scope | `OutboundRequestLease` | valid receipt 才 wake；rejected receipt 与 scope-terminal late receipt 均不 wake |

Eval method 的 Ready、Pending continuation/heap boundary 与 invocation owner drop 分别有真实
prepared-operation case；canonical spawn case 同时验证 exact target 与 current carrier receipt。
既有 segment 切分与 continuation resume 没有增加新状态或公开元数据。

## 4. response、spawn fence 与 owner

control/spawn 沿用唯一 `OutboundRequestRegistry`。scoped wait 在现有
`OutboundRequestLease::terminal_signal()` 上观察“response 已从 registry 移除并本地提交”，
biased response branch 先消费现有 receiver，再完成 scope lease。scope branch 对同一个真实
lease 执行既有 cancel/drop；因此没有第二个 response registry，late response 找不到 owner。

method 沿用唯一 `ActorMethodOutboundRegistry` 与 `ActorMethodOutboundLease`。registry entry 内部增加
crate-private response-committed signal；entry 仍先从同一个 map 移除，再标记 commit 并发送现有
oneshot。scope winner发送既有 wire cancel hint、drop 同一个 method lease，然后保持内部 Pending，
让 Eval post-await checkpoint保留 deadline/error 的精确 owner。没有新增 public cancel、
yield 或 lifecycle metadata。

聚焦断言覆盖：

- control：pending request、active lease、scope waiter/timer 全部归零，late completion 返回 false；
- method：registry pending count 与 scope lifecycle 全部归零，late/duplicate outcome 返回 false；
- response-first：control 与 method 都返回已提交 response，且不发 cancellation hint；
- spawn：deadline 后 registry owner归零，late receipt 不可完成，worker wake signal保持未触发；
- valid spawn receipt 已通过既有 response schema/identity 校验后才调用 `wake_build`。

## 5. timeout 与结果投影

`prepare_actor_method` 固定建立 `30_000ms` primitive，不再读取 outbound/request effective
timeout。Host wire hint 使用 `min(current/outer scope remaining, primitive)`；测试分别覆盖
current 50ms、outer 50ms 与独立 1ms primitive。

scope deadline/ancestor stop 分支不返回普通 Actor capability error：真实 lease释放并发送
best-effort cancel hint后保持 Pending，由 Eval 已有 post-await checkpoint投影 exact terminal。
request-root cancellation 与 primitive deadline 的既有 typed Actor outcome 保持独立。

## 6. 实际写集

```text
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
runtime/eval/src/spawn_ops/canonical_tests.rs
runtime/host/src/capability_context/actor.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/capability_context/actor_method_outbound.rs
runtime/host/src/eval_capability_adapter/actor.rs
```

`prepared_operation.rs` 与 `spawn_ops.rs` 的 E1 carrier 接线已经满足当前任务，不需要 production
修改。没有修改 E1 shared API/`capabilities.rs`、native Actor、Router/wire、artifact/std、
Cargo manifest 或 lockfile。

## 7. 验证与边界

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --list` | PASS；`4 tests` |
| `cargo test -p skiff-runtime-eval f445h_i6_actor_scope -- --nocapture` | PASS；`4/4` |
| `cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list` | PASS；`15 tests` |
| `cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture` | PASS；`15/15` |
| `cargo check -p skiff-runtime-eval -p skiff-runtime-host --locked` | PASS；仅既有 warnings |
| `cargo fmt --all --check` | PASS |
| `git diff --check` | PASS |

首次并行复跑 Host 与首次双 crate check 遇到本机 `No space left on device`。两次都只执行当前
worktree 的 `cargo clean`，随后改为串行并完整通过上述命令；没有清理其它 worktree 或修改源码
来规避门禁。

未运行 full gate、stable instance、live/network、MongoDB、merge、rebase 或 push。

```text
I6_ACTOR_COMPLETE = YES
```
