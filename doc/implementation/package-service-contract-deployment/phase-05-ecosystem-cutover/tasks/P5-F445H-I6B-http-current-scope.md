# P5-F445H-I6B HTTP current-scope consumer

状态：Ready。消费I6-A冻结的invocation-time execution carrier，使HTTP unary、stream open与SSE open
在真实Pending期间受调用点current deadline与internal stop约束。

## 直接父节点

- `P5-F445H-I6A-shared-invocation-scope-checkpoint-result.md`
- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`

## 固定输入与边界

```text
base commit  8db08c539acaf0b3fc41733365f06e9883bdbdd8
base tree    71123064dd0948d5946ad8c6312df909670794e0
```

I6-A已让HTTP native projection取得调用时owned execution control。本节点只迁移HTTP lower consumer；
不修改共享carrier、E4 stream consumer、HTTP ingress、public std shape或service timeout。

## 实现要求

1. HTTP unary、body-stream open与SSE open在operation开始时读取I6-A carrier的full current scope。
2. 可见deadline为current effective deadline与
   `std.http.HttpClientRequest.timeoutMs` primitive中更早者；没有显式primitive时不得新增默认配置。
3. Pending waiter观察current scope全部signals与absolute deadline，不继续依赖request构造时冻结的
   relative `deadline_ms`和root token。
4. Current/request/outer deadline由E4 post-await checkpoint保留精确owner；primitive timeout作为当前
   lane普通`TimeoutError`，不得继续映射成
   `ProviderUnavailable("request timeout")`。
5. Winner固定后先本地settle并drop lower future，late response不能进入native finalize或caller heap。
6. HTTP write已经发出时outcome可以unknown；只使用reqwest/response/stream现有owner做best-effort收束，
   不声称撤销、不等待cleanup acknowledgement。
7. 已创建handle后的body/SSE `next`、natural End、break/return/error cleanup继续由E4 owner负责，
   本节点不得复制stream supervisor。

## 允许写集

Production：

```text
runtime/host/src/capability_context/effect_context.rs
runtime/host/src/host/http_client_runtime.rs
runtime/host/src/host/http_runtime/call_context.rs
runtime/host/src/host/http_runtime/request.rs
runtime/host/src/host/http_runtime/stream.rs
runtime/host/src/host/http_runtime/sse.rs
runtime/host/src/host/http_runtime/transport.rs
```

Tests：

```text
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs
runtime/host/src/host/http_runtime/tests/request.rs
runtime/host/src/host/http_runtime/tests/stream.rs
runtime/host/src/host/http_runtime/tests/sse.rs
runtime/host/src/host/http_client_runtime.rs
```

实际diff必须是最小子集。新增fake seam只能crate-private/test-only，不能改变public HTTP API。

## 禁止写集

- I6-A carrier、I6-C/D文件；
- E4 eval actual-Pending、program/source stream与canonical service；
- HTTP ingress/router/proxy/egress配置；
- public std/native schema、artifact/compiler；
- Cargo manifests、lockfile、真实network fixture。

## 任务内并行

父任务Agent可派最多两个有界子Agent，子Agent不得继续委派：

1. 一个只读分片核对unary/open/transport的共同deadline owner和现有fake seam；
2. 父Agent冻结内部call-context签名后，一个test-only分片可在独立worktree实现
   `current_scope.rs`与不重叠fixture。

父Agent独占production integration、timeout owner判断与统一验证。发现需要E4/public API/router修改时返回
`TASK_SCOPE_EXPANDED`。

## Test-first与验证

使用paused Tokio clock、scripted fake transport、barrier/oneshot/drop counter，不访问真实网络。

RED至少覆盖：

- root仍active时derived child deadline到达，旧future不醒；
- fake lower只看到root budget；
- primitive timeout返回ProviderUnavailable；
- scope winner后late response仍尝试finalize。

GREEN至少覆盖：

- `min(current, primitive)`及equal owner；
- ancestor stop与deadline同刻时internal stop优先且无业务error；
- unary/stream/SSE open全部使用current scope；
- late result不finalize，active waiter/timer/stream counter归零；
- cleanup不等待ack，也不承诺外部副作用撤销。

命令：

```bash
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
cargo check -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

Listing必须非零且与execution数量一致。不得运行完整crate/stage gate、network/stable/live/MongoDB。

反向搜索：

```bash
rg -n "CancellationSignals::from_tokens\\(\\[request\\.cancellation" runtime/host/src/host/http_client_runtime.rs
rg -n "frame_deadline_ms|deadline_ms" runtime/host/src/host/http_runtime runtime/host/src/host/http_client_runtime.rs
rg -n 'ProviderUnavailable\\(\"request timeout\"' runtime/host/src
```

剩余deadline命中必须分类为operation-start current或显式primitive，不能是request-construction snapshot。

## 交付

提交implementation，再新增
`P5-F445H-I6B-http-current-scope-result.md`并单独提交result。Result记录精确tree、实际写集、
RED/GREEN、非零计数、owner矩阵、反向搜索和I6-J HTTP case是否解除。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i6b-http
branch   codex/p5-f445h-i6b-http
```

最终clean；不得merge/rebase/push。启动五分钟内完成第一处production修改；不能安全修改时立即停止。
