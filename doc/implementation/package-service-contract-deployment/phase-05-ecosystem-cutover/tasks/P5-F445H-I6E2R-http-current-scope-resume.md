# P5-F445H-I6E2R HTTP current-scope consumer resume

状态：Ready。恢复 I6E2，并把 E1 已送达但在 Host HTTP adapter显式丢弃的 carrier继续转发到具体
HTTP context；随后完成 unary、body-stream open、SSE open 的 current-scope consumer。

## 直接父节点

- `P5-F445H-I6E2-http-current-scope-resume-result.md`
- `P5-F445H-I6E1-shared-carrier-delivery-checkpoint-result.md`
- `P5-F445H-I6B-http-current-scope-result.md`

## 固定输入

```text
E1 implementation commit  ba66719e03cbabde2e159b94761cc1a1c71b35d2
E1 implementation tree    0b1972158d710c4355274f7fb272be292dcc7927
resume base commit        105a1f776c120455f962572e02ac4ed821f5c4e6
resume base tree          2f41a195dfa275dc0907ddb08455018b57c476e6
```

## 修订后的完整行为

1. `runtime/host/src/eval_capability_adapter/http.rs` 三个入口不再
   `let _execution_control = execution_control`；只把同一个owned control转发给具体context，不建立
   第二个carrier、不在adapter acquire lease。
2. unary、body-stream open、SSE open开始时读取完整current scope，不再用request构造时冻结的
   relative deadline/root token驱动pending。
3. pending lower future观察current scope全部signals与absolute deadline；scope胜出时drop lower
   future并清lease/timer，late lower completion不得交付。
4. `HttpClientRequest.timeoutMs`是操作自身primitive；与current deadline取更早者。没有显式值不新增
   默认。primitive timeout映射既有`std.service.TimeoutError`，不得映射为ProviderUnavailable。
5. normal completion先本地提交则不被同刻signal覆盖；current scope终止由既有post-await checkpoint
   保留精确owner。
6. 只处理open；handle建立后的stream/SSE `next`与cleanup仍属于E4 stream。

## 唯一写集

Production：

```text
runtime/host/src/eval_capability_adapter/http.rs
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

若为删除旧frame-deadline plumbing确实必须，可同owner增加
`http_runtime/{call_context,request,stream,sse}.rs`，result逐项解释；不得修改Eval/E1 shared API、
HTTP ingress、Router、std/native、Cargo/lockfile。

## RED / GREEN

真实RED必须先证明adapter丢carrier或真实lower pending忽略current scope；随后paused-clock/fake lower
覆盖三入口、current/outer deadline、ancestor/internal stop、primitive timeout精确错误、normal竞争、
late completion及owner归零。

```text
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
cargo check -p skiff-runtime-host --locked
cargo fmt --check
git diff --check
```

listing必须非零且与execution数量一致。

## 停止与完成

若仍需E4 stream、真实network、HTTP ingress/Router、公开timeout/error surface或E1 shared API变更，
提交 `TASK_SCOPE_EXPANDED` result并停止。禁止full gate、stable/live/network/Mongo、
merge/rebase/push。

分开提交implementation/tests与
`P5-F445H-I6E2R-http-current-scope-resume-result.md`。result给出commit/tree、RED/GREEN、三入口矩阵、
owner/cleanup、实际写集及 `I6_HTTP_COMPLETE = YES/NO`。worktree保持clean。
