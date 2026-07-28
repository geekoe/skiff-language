# P5-F445H-I6E2R HTTP current-scope consumer resume result

状态：

```text
IMPLEMENTATION_PASS
TASK_SCOPE_EXPANDED = NO
I6_HTTP_COMPLETE = YES
```

E1 已交付到 Host HTTP adapter 的同一个 owned execution control 现在继续进入 unary、
body-stream open 与 SSE open 的 concrete Host consumer。三个 current-scope operation 在开始时读取
完整 `ExecutionScope`，并用 scope lease 与 HTTP primitive timeout 共同监督真实 lower future；
handle 建立后的 stream/SSE `next` 与 cleanup 未进入本任务。

## 1. 候选身份

| 项 | commit / tree |
| --- | --- |
| E1 implementation commit | `ba66719e03cbabde2e159b94761cc1a1c71b35d2` |
| E1 implementation tree | `0b1972158d710c4355274f7fb272be292dcc7927` |
| resume base commit | `105a1f776c120455f962572e02ac4ed821f5c4e6` |
| resume base tree | `2f41a195dfa275dc0907ddb08455018b57c476e6` |
| task publication HEAD | `a9f0fa5f9bfd7d60403d5570d143de0c686c236b` |
| implementation commit | `860a2a9cd098a531354891591fbe386b8e0ad7b3` |
| implementation tree | `e2568e944e9313aec68f3745452a4208358d8024` |

## 2. RED / GREEN

真实 RED 由直接父 result
`P5-F445H-I6E2-http-current-scope-resume-result.md` 固化：E1 carrier 到达
`RuntimeHttpClientCapabilityContext` 后，三个 adapter method 都执行
`let _execution_control = execution_control`，随后调用不带 carrier 的 concrete dispatch。因此真实
lower pending 只能观察 request-construction 时冻结的 root token / relative deadline，不能观察
current scope。

GREEN：

```text
cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list
11 tests / 0 benchmarks

cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture
11 passed / 0 failed / 303 filtered
```

listing 与 execution 均为 11，数量一致。所有 selector 测试使用 fake lower；deadline case 使用已经
到达的 absolute `Instant`，不等待真实 wall clock，也不访问 network。

## 3. 三入口与 winner 矩阵

| 入口 / 行为 | production path | 动态证据 |
| --- | --- | --- |
| unary | adapter owned control → `dispatch_http_request_with_current_scope` → request lower wrapper | `f445h_i6_http_current_scope_unary_open_observes_ancestor_stop` |
| body-stream open | adapter owned control → `dispatch_http_stream_with_current_scope` → body-open lower wrapper | `f445h_i6_http_current_scope_body_stream_open_observes_ancestor_stop` |
| SSE open | adapter owned control → `dispatch_http_sse_with_current_scope` → SSE-open lower wrapper | `f445h_i6_http_current_scope_sse_open_observes_ancestor_stop` |
| current deadline | current scope owns effective absolute deadline；scope winner drops lower | `f445h_i6_http_current_scope_current_deadline_stops_pending_lower` |
| outer deadline | current scope observes inherited outer effective deadline | `f445h_i6_http_current_scope_outer_deadline_stops_pending_lower` |
| ancestor stop | full scope ancestor signals wake lease | three entry tests above |
| internal parent stop | dropped parent lease propagates child-scope stop | `f445h_i6_http_current_scope_internal_parent_stop_stops_pending_lower` |
| primitive timeout | `timeoutMs` timer wins as existing `TimeoutError` payload | `f445h_i6_http_current_scope_primitive_timeout_is_timeout_error` |
| min(current, primitive) | earlier current absolute deadline wins over later primitive | `f445h_i6_http_current_scope_earlier_current_deadline_beats_primitive_timeout` |
| normal/signal competition | biased lower branch commits a ready normal result before same-turn signal | `f445h_i6_http_current_scope_ready_lower_commits_before_same_turn_signal` |
| late completion fence | scope winner drops lower receiver; later sender cannot deliver | `f445h_i6_http_current_scope_late_lower_completion_cannot_deliver` |
| full carrier read | owned carrier returns nesting and effective absolute deadline of current scope | `f445h_i6_http_current_scope_owned_carrier_exposes_full_current_scope` |

## 4. Owner、timeout 与 cleanup

- Adapter 只按值转发 E1 已有的 owned control；没有建立第二 carrier，也没有 acquire lease。
- Concrete `*_with_current_scope` method 在构造/等待 lower 之前读取一次完整 current scope。
- Shared Host HTTP lower supervisor 是 lease、scope/primitive winner、lower-future drop 与 late-delivery
  fence 的唯一 owner。lower ready branch排在 scope/primitive branch 前；一旦本地取得 normal output，
  同 turn signal 不会覆盖该 output，已有 post-await checkpoint仍保留 current-scope终止的精确 owner。
- scope winner返回内部 cancellation terminal；current local deadline的精确 ordinary projection继续由
  已有 post-await checkpoint拥有，未在 HTTP consumer复制。
- `timeoutMs`只作为 HTTP operation primitive；current path向旧 transport传 `frame_deadline_ms = None`，
  没有显式 timeout时不创建 primitive timer。reqwest timeout和零 timeout都映射
  `TimeoutError`，不再映射 `ProviderUnavailable`。
- 每个 scope/timeout/cancel/normal/late case 都在结束后断言
  `active_leases = active_waiters = active_timers = 0`。scope/primitive胜出会 drop lower future；
  body/SSE handle只保留既有 stream cancellation，不携带 operation lease child。

## 5. 实际写集

Production：

```text
runtime/host/src/eval_capability_adapter/http.rs
runtime/host/src/host/http_client_runtime.rs
runtime/host/src/host/http_runtime/transport.rs
```

Tests：

```text
runtime/host/src/host/http_runtime/tests/mod.rs
runtime/host/src/host/http_runtime/tests/current_scope.rs
runtime/host/src/host/http_runtime/tests/request.rs
```

共 6 个文件，`743 additions / 69 deletions`。没有修改 Eval/E1 shared API、
`runtime/host/src/capability_context/http.rs`、HTTP ingress、Router、std/native、Cargo/lockfile，
也没有修改 E4 handle-established stream/SSE `next` owner。

## 6. 验证

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --list` | PASS；11 tests |
| `cargo test -p skiff-runtime-host f445h_i6_http_current_scope -- --nocapture` | PASS；11 / 11 |
| `cargo check -p skiff-runtime-host --locked` | PASS；仅既有 warnings |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

未运行 full gate；未访问或启动 stable/live/network/MongoDB；未 merge、rebase或 push。首次 test-profile
编译曾耗尽 worktree构建缓存空间，随后只用 `cargo clean` 清理该 worktree可重建的 `target` 产物后继续。

```text
I6_HTTP_COMPLETE = YES
```
