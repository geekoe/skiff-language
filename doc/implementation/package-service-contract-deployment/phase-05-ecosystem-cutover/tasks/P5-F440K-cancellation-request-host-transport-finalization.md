# P5-F440K Cancellation request / Host / transport finalization

状态：Ready。确定性实现 leaf；对应 F439A 冻结 DAG 的 **R2**。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`
- `P5-F440I-cancellation-native-eval-service-channel-result.md`

需要细节时只沿这三个父节点引用向上读取。

精确实现输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `1897df3f98c6ba1d4f8522c5003e295380ed54e3` | `483ba35924a46bf6e08052c6fb4a7c61e113535d` |

## 目标与唯一写集

把已冻结的 internal cancellation terminal贯穿 root request、Host capability adapter与transport completion：

- active root request取消时清除自身与全部child pending；
- cancellation不产生普通 response、payload、catch identity或service/transport error；
- Host/native/file/http/stream/actor工作仍被及时中止；
- deadline/instruction limit仍形成普通 `TimeoutError`响应。

唯一 production/test 写集：

- `runtime/request/**`
- `runtime/host/**`
- `runtime/transport/**`

另可新增本 leaf result。禁止修改 native、eval、capability-context、runtime/model、Router、compiler、
artifact、scripts、fixtures、其它 task/result或权威设计。不得派子 agent。

## 实现合同

### Request error与root completion

1. `RequestError::Cancelled` 只保留编码无关的 internal terminal classification；不能实现或返回
   `WirePayload`、`RuntimeErrorPayload`、Cancel catch identity或 ordinary response。
2. `RequestError::Eval` 必须先按 eval 的 `is_cancellation_terminal()` 分流；ordinary error才调用
   `ordinary_payload()` / `ordinary_catch_projection()`。不得重新增加 eval total projection。
3. Root request completion明确分成：
   - cancelled terminal：清 pending/lease/stream/child，禁止写 `ResponseEvent::Error`；
   - ordinary failure：继续走现有 response mapper；
   - success：保持现有 single terminal。
4. Router `request.cancel`与成功/错误同时到达时，只允许一个 terminal owner；晚到 success/error不得重开
   request或再写 frame。

### Host adapter

5. 删除 `runtime/host/src/error.rs`、eval capability adapter、native projection及stream wrapper中依赖
   `Box<dyn WirePayload>` / downcast来识别 cancellation 的旧 mixed-carrier路径。
6. Host层可以保留结构化 `Cancelled` variant/query；ordinary wrapper只能包装已验证的
   ordinary projection，不能把 terminal降级为 `InternalError`或`ProviderUnavailableError`。
7. HTTP/file/native operation、blocked stream send/next、outbound lease、actor method与child task在
   cancellation时继续 abort/wake/cleanup；删除 wire projection不能使 future永久 pending。
8. Actor deadline、operation deadline及execution budget deadline仍精确投影为 `TimeoutError`，可产生
   ordinary response；cancel/deadline同时 ready时保持权威 biased winner。

### Transport

9. `runtime/transport/src/response_mapper.rs`及其它 response writer只接收 ordinary errors。
   Cancellation terminal在类型/入口上 fail closed，不能被映射成 `response.error`。
10. Runtime writer关闭、Router session断开与client disconnect必须释放 pending/lease；仅活 caller遇到
    provider/runtime丢失时保持 `ProviderUnavailableError`，不能误标为用户取消。
11. 保留 `request.cancel` control frame及bounded reason；不增加新的公开 cancellation code/frame。
12. 本任务不实现 WebSocket JSON-RPC `connection.request.cancel`；它属于后续 RPC transport checkpoint。

## 测试先行

先落真实 red，至少覆盖：

1. root ingress active request取消后：
   - root active count为零；
   - child/native/stream pending均为零；
   - cancel telemetry/control settlement一次；
   - ordinary `response.error` frame为零。
2. `RequestError::Cancelled`与wrapped eval cancellation无 payload/catch/response。
3. Router cancel vs successful completion、ordinary error、deadline三类竞态各最多一个 terminal。
4. HTTP gateway/client disconnect、outbound lease drop、runtime writer close、stream early break。
5. actor method cancel与actor deadline分别为 internal terminal / `TimeoutError`。
6. HTTP/file/native operation被真实 AbortSignal/token唤醒。
7. transport mapper拒绝 cancellation，但 ordinary Timeout/ProviderUnavailable仍编码。

测试必须经过真实 request entry、Host adapter与transport mapper，不得只测孤立布尔 helper。

## 验证

先列出非零 selectors，再运行受影响 crate的 focused与完整 matrix，至少：

```bash
cargo test -p skiff-runtime-request
cargo test -p skiff-runtime-host
cargo test -p skiff-runtime-transport
cargo test -p runtime
cargo check -p skiff-runtime-request
cargo check -p skiff-runtime-host
cargo check -p skiff-runtime-transport
cargo check -p runtime
cargo fmt --all -- --check
git diff --check
```

若 `runtime`完整测试被与本任务无关的已知
`runtime/eval/src/runtime_http_gateway/tests.rs:384` stale test挡住，精确记录并继续运行本写集可执行的
crate/selectors；该单文件由独立 test repair owner处理，不得越界。

反向搜索：

```bash
rg -n 'CancelError|PlatformBuiltinErrorIdentity::Cancel' runtime/request runtime/host runtime/transport
rg -n 'Cancelled|is_cancel|response_error|ResponseEvent::Error|WirePayload' runtime/request runtime/host runtime/transport
```

第一条 production必须为零；命名清楚的 legacy rejection tests可保留并分类。第二条逐项说明 terminal
carrier、cleanup owner、ordinary-only mapper与control frame。

## 停止规则与交付

- 若必须先改 runtime/model finite platform registry，记录为后继 M0 blocker；不得越界。
- 若完成 root no-response语义要求修改 Router，返回 `TASK_SCOPE_EXPANDED`；不得越界。
- 不运行完整 verify、Router suite、live、instance、stable或chat smoke。

Result必须列出 red/green计数、root pending/telemetry/frame直接证据、Timeout与
ProviderUnavailable正例、runtime/model后继、reverse-search和clean状态。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440k-cancellation-request-host`
- branch：`codex/p5-f440k-cancellation-request-host`
- result：`P5-F440K-cancellation-request-host-transport-finalization-result.md`

Implementation 与 result 分开提交。不 merge/rebase/push。
