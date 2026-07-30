# P5-F445H-I6E2 HTTP current-scope consumer resume

状态：Ready。消费 E1 已交付到 Host adapter 的 `OwnedExecutionControl`，使 HTTP unary、
body-stream open、SSE open 的真实等待受调用点 current scope约束。

## 直接父节点

- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6B-http-current-scope-result.md`
- `P5-F445H-I6E-invocation-carrier-delivery-preflight-result.md`

## 固定输入

```text
E1 implementation commit  ba66719e03cbabde2e159b94761cc1a1c71b35d2
E1 implementation tree    0b1972158d710c4355274f7fb272be292dcc7927
integration base commit   e942efa99460ea2b9bf29f07d8dfe855c9715aff
integration base tree     46abc10c8fbdab6e70f2ea071539382dbf03a1be
```

## 行为要求

1. 三个操作开始时从 E1 carrier读取完整 current scope；不得回退到请求构造时冻结的
   `deadline_ms`和root token。
2. unary、body-stream open、SSE open 的 pending lower future同时观察 current scope全部signals和
   absolute deadline；scope胜出时drop lower future并释放lease/timer。
3. `HttpClientRequest.timeoutMs` 是操作自身 primitive timeout；有效终止点为 current deadline与
   primitive timeout中更早者。没有显式值时不新增默认timeout。
4. current scope终止继续由E4 post-await checkpoint保留精确owner；primitive timeout映射为既有
   `std.service.TimeoutError`，不得误映射为ProviderUnavailable。
5. lower future先正常完成时先完成本地owner，再返回结果；同刻scope signal不得覆盖已提交结果。
6. scope胜出后的late lower完成不得交付response/stream handle；只做既有best-effort cleanup。
7. 本任务只管HTTP open。handle建立后的body/SSE `next`及natural/non-End cleanup仍由E4 stream owner
   负责。

## 唯一写集

Production：

```text
runtime/host/src/capability_context/http.rs
runtime/host/src/host/http_client_runtime.rs
runtime/host/src/host/http_runtime/transport.rs
```

Tests：

```text
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs
runtime/host/src/host/http_runtime/tests/request.rs
runtime/host/src/host/http_client_runtime.rs
```

若选择删除旧frame-deadline plumbing，可把同一owner下列文件加入本任务，但必须在result解释必要性：

```text
runtime/host/src/host/http_runtime/call_context.rs
runtime/host/src/host/http_runtime/request.rs
runtime/host/src/host/http_runtime/stream.rs
runtime/host/src/host/http_runtime/sse.rs
```

不得修改 E1 共享接口、Eval、HTTP ingress、Router、std/native公开面、Cargo/lockfile。

## 测试

必须先建立真实RED，再转GREEN。paused clock/fake lower至少覆盖：

- current deadline；
- outer timeout；
- ancestor/internal stop；
- primitive timeout及精确错误；
- normal completion竞争；
- late completion不交付；
- lease/timer/waiter归零；
- unary、body open、SSE open三种入口。

命令：

```text
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
cargo check -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

listing必须非零且与execution数量一致。

## 停止与禁止

若需要修改E4 stream consumer、真实network、HTTP ingress/Router、公开timeout/error surface或E1共享
carrier接口，提交 `TASK_SCOPE_EXPANDED` result并停止。禁止peer cancel、公开cancel/yield、full gate、
stable/live/network/Mongo、merge/rebase/push。

## 完成

分开提交implementation/tests与
`P5-F445H-I6E2-http-current-scope-resume-result.md`。result给出commit/tree、RED/GREEN计数、三入口
矩阵、owner/cleanup证据、实际写集，并标明 `I6_HTTP_COMPLETE = YES/NO`。worktree保持clean。
