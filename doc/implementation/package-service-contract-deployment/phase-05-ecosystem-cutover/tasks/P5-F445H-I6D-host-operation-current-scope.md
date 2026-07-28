# P5-F445H-I6D time/file/Actor/response-source current scope

状态：Ready。消费I6-A carrier，完成time、file、Actor control/method与Host response sink四个互不
重叠consumer，并由一个父任务Agent统一集成和验证。

## 直接父节点

- `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
- `P5-F445H-D1-internal-execution-stop-semantics-result.md`
- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`

## 固定输入

```text
base commit  8db08c539acaf0b3fc41733365f06e9883bdbdd8
base tree    71123064dd0948d5946ad8c6312df909670794e0
```

本任务不创建通用cleanup supervisor、lifecycle metadata或公开cancel。四个consumer共享I6-A内部carrier，
但保留各自resource owner。

## 四个实现分片

### D1 time

- `std.time.sleep`按invocation current scope等待；requested duration只决定normal wake，不是额外deadline。
- Paused clock证明derived deadline与ancestor stop可唤醒；同步date/time helper只经过既有checkpoint。
- 不新增语言`yield`或轮询request-root snapshot。

允许写：

```text
runtime/native/src/dispatch/time.rs
```

### D2 Host response sink

- response sink capacity/Pending wait使用current scope lease/signals/absolute deadline；
- winner后late capacity wake不能继续写response；
- natural End与非End cleanup继续由E4/现有`StreamConsumerCleanup`拥有。

允许写：

```text
runtime/capability-context/src/stream.rs
```

### D3 file

- file direct operation与`createFromStream`使用invocation carrier；
- current winner先本地settle/drop provider future，late value不能finalize或写caller heap；
- non-End继续现有幂等cleanup，natural End不误cleanup；
- staging/blob/DB effect开始后允许完成或unknown，不伪装撤销。

允许写：

```text
runtime/host/src/eval_capability_adapter/file_stream.rs
runtime/host/src/capability_context/store.rs
runtime/host/src/host/file_runtime.rs
```

### D4 Actor

- get/create/replace/find/remove、spawn control RPC wait使用current deadline/signals；
- Actor method deadline为`min(current effective deadline, existing 30s primitive)`；
- current deadline保留scope owner，30s primitive为普通lane `TimeoutError`；
- local registry/lease先移除，internal cancel frame只是best-effort hint，late outcome不命中。

允许写：

```text
runtime/eval/src/actor_dispatch.rs
runtime/eval/src/actor_dispatch/prepared_operation.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/host/src/capability_context/actor.rs
runtime/host/src/capability_context/actor_method_outbound.rs
```

## Test允许写集

```text
runtime/native/src/dispatch/time.rs
runtime/native/src/dispatch/file/tests.rs
runtime/capability-context/src/stream.rs
runtime/host/src/host/file_runtime/tests.rs
runtime/host/src/capability_context/actor/tests.rs
runtime/host/src/eval_capability_adapter/actor.rs
runtime/eval/src/actor_dispatch/prepared_operation_tests.rs
```

## 禁止写集

- DB E4/O6 state machines、canonical service、ordinary/source stream owner；
- 通用lifecycle metadata、cleanup grace/ack；
- public cancel/error、Actor/Router wire、legacy outbound；
- I6-A/B/C、compiler/artifact/std公开签名、Cargo/lockfile。

## 任务内并行

父任务Agent必须优先按上述D1–D4划分四个互不重叠的有界子Agent；每个子Agent使用独立worktree/branch，
只写自己分片文件并提交。子Agent不得继续委派。父Agent：

1. 冻结四个分片共同使用的I6-A carrier方式；
2. 检查写集不重叠后并行启动；
3. 接收并集成四个提交；
4. 统一处理仅由组合编译暴露的机械constructor跟随；
5. 运行完整聚焦矩阵并提交单一result。

某分片发现需要新公共API、其它production owner或多个实现方向时必须停止该分片并上报父Agent；父Agent
不得让其吞并。其它无关分片可以继续。

## Test-first与验证

四个分片分别保留真实RED：

- time：derived child下sleep仍poll root；
- response：sink capacity Pending不因current deadline/stop醒；
- file：provider/source Pending只认root，late/staging cleanup缺口；
- Actor：control只认root、method固定30s替代current。

GREEN必须证明current owner、ancestor stop无业务error、late隔离及各自resource counter归零。使用paused
clock、scripted fake provider/registry/barrier，不访问真实DB/blob/router/network。

命令：

```bash
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --list
cargo test -p skiff-runtime-native f445h_i6_time_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_file_scope -- --nocapture
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_actor_scope -- --nocapture
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --list
cargo test -p skiff-runtime-capability-context f445h_i6_response_sink_scope -- --nocapture
cargo check -p skiff-runtime-native -p skiff-runtime-capability-context \
  -p skiff-runtime-eval -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

所有listing非零且与execution一致。不得运行完整crate/stage gate、stable/live/network/MongoDB。

反向搜索：

```bash
rg -n "parts\\.cancellation\\.wait_cancelled|context\\.cancellation_token\\(\\)\\.wait_cancelled" runtime/host/src/{eval_capability_adapter,capability_context}
rg -n "send_with_cancellation\\(.*cancellation_token" runtime/capability-context/src/stream.rs
rg -n "FileIngest|StagedFile" runtime/host/src/host/file_runtime.rs
rg -n "30_000" runtime/eval/src/actor_dispatch.rs runtime/host/src/eval_capability_adapter/actor.rs
```

Root-only wait目标为0；30s只可作为与current取min的primitive。

## 交付

父Agent集成四个implementation提交后，新增
`P5-F445H-I6D-host-operation-current-scope-result.md`并提交。Result记录每个分片commit/tree、
实际写集、RED/GREEN、聚焦计数、组合检查、反向搜索、未决分片及I6-J相应cases是否解除。

```text
parent worktree /Users/geek/workspace/skiff-p5-f445h-i6d-host-ops
parent branch   codex/p5-f445h-i6d-host-ops
```

子worktree也必须直接位于`/Users/geek/workspace/`，完成集成后由父Agent删除已合入的子worktree和分支。
父worktree最终clean；不得merge到integration、rebase或push。启动五分钟内至少一个分片完成第一处
production修改。
