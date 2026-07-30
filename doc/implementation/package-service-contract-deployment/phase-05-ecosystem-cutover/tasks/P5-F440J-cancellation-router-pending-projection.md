# P5-F440J Cancellation Router pending / projection follower

状态：Ready。确定性实现 leaf；对应 F439A 冻结 DAG 的 **Q0**。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`
- `P5-F440E-cancellation-runtime-terminal-checkpoint-result.md`

需要细节时只沿这三个父节点引用向上读取。

精确实现输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `5e26079dbdc851e3528d3ad8dbf809cf9b7fd29c` | `23cba855509ad4509215458125d97fadcf93d686` |

## 目标与唯一写集

从 Router 的 ordinary error公开面删除 cancellation，同时保留内部
`request.cancel` 控制帧、pending exactly-once cleanup和有界取消原因。

唯一 production/test 写集：

- `router/**`

另可新增本 leaf result。禁止修改 Rust runtime/model/compiler/artifact、scripts、fixtures、service root、
其它 task/result或权威设计。不得派子 agent。

## 实现合同

1. Router公开 runtime/service error registry不再接受 `CancelError`：
   - `PLATFORM_SERVICE_ERROR_IDENTITIES` 删除该值；
   - fixed-service platform envelope、control `response.error`及其它 runtime payload validator均
     fail closed；
   - `runtimeErrorStatus` 删除 `CancelError -> 499` ordinary HTTP error映射；
   - 不新增 replacement code、alias或“未知错误回退为取消”。
2. `request.cancel` frame/header/reason registry继续保留；它是 control plane，不是 ordinary error。
3. Caller abort、HTTP/client disconnect、timeout、backpressure、Router shutdown与protocol failure仍通过单一
   pending owner最多发送一次适当 `request.cancel`，并最多执行一次 `finishPending` / lease cleanup。
4. Runtime-originated cancellation control终止对应 pending，但不能生成/转发普通 `response.error`。
   late success、late error和duplicate cancel均不能重新打开或二次结束 pending。
5. Provider/runtime disconnect对仍活 caller保持 `ProviderUnavailableError`；不能误分类为用户取消。
6. Timeout保持普通 `TimeoutError`与现有HTTP timeout status。Timeout winner可先结束caller并发送
   `request.cancel`，但不得产生 `CancelError`。
7. HTTP连接已经关闭、没有写出普通响应时，Router内部 telemetry使用499表达client-closed observation可保留；
   它不是错误payload/status projection，result须明确分类。
8. 本任务不实现 WebSocket JSON-RPC broker、业务request id、peer `$/cancelRequest`或新 external manifest。

## 测试先行

先增加或改成真实 red，至少证明：

1. fixed-service `PlatformError { builtinErrorIdentity: "CancelError" }` 被拒绝；
2. runtime/control `response.error { code: "CancelError" }` 被拒绝，不能投影为HTTP 499；
3. timeout仍映射为 `TimeoutError` / 504并发送一次 timeout `request.cancel`；
4. caller abort与client disconnect只清理一次并发送一次对应 cancel reason；
5. runtime-originated cancel、runtime disconnect、late/duplicate response竞争时pending最多一个terminal；
6. provider disconnect保持 ProviderUnavailable；
7. fixed/control error channel继续互斥。

复用真实 runtimeDispatcher、protocol、raw HTTP、assembly unary/stream tests；不得只测孤立 helper。

## 验证

先列出精确非零 Vitest selectors，再运行受影响 focused suites，至少覆盖：

```bash
pnpm --dir router test -- tests/protocol.test.ts
pnpm --dir router test -- tests/runtime-registry-dispatch.test.ts
pnpm --dir router test -- tests/runtime-assembly-unary-dispatch.test.ts
pnpm --dir router test -- tests/raw-http.test.ts
pnpm --dir router test -- tests/assembly-http-gateway-stream.test.ts
pnpm --dir router typecheck
git diff --check
```

若项目脚本不接受上述单文件形式，可使用等价的 `pnpm --dir router exec vitest run ...`，但 result必须给出
实际命令、listed/executed数量。不得安装新依赖；若依赖不存在，记录精确 blocker并完成静态/typecheck证据。

反向搜索：

```bash
rg -n 'CancelError' router/src router/tests
rg -n 'request\.cancel|finishPending|499' router/src router/tests
```

第一条 production必须为零；只允许命名清楚的negative rejection tests。第二条逐项分类 control owner、
pending owner与允许保留的client-closed telemetry。

## 停止规则与交付

- 若必须修改 runtime wire schema owner或runtime/model才能严格拒绝，返回 `TASK_SCOPE_EXPANDED`，不得越界。
- 不运行完整 verify、Rust suite、live、instance或stable。
- Result列出 red/green计数、竞态证据、Timeout/ProviderUnavailable正例、reverse-search分类和clean状态。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440j-cancellation-router`
- branch：`codex/p5-f440j-cancellation-router`
- result：`P5-F440J-cancellation-router-pending-projection-result.md`

Implementation 与 result 分开提交。不 merge/rebase/push。
