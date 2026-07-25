# P5-F263 Suspending WebSocket ingress result

状态：COMPLETE。

## 实现

- 删除 `websocket_ingress_context` 对 `maySuspend=true` 的旧拒绝；WebSocket ingress 仍严格要求
  unary、无 throws、无 callbacks 且 `NotCancellable`。
- Package boundary projection 对 canonical
  `event: std.websocket.WebSocketIngressEvent<Context>` 保留 `NotCancellable`，不再因实现挂起而错误
  投影成 `Cooperative`。普通可挂起 callable 仍保持 `Cooperative`。
- 权威 gateway/runtime 文档明确：每个入站消息只 dispatch 一次；同一连接一次只有一个 active
  receive，后续消息按到达顺序排队；连接关闭只结算 active transport dispatch 一次；`connection.send`
  保持非挂起。

## 验证

- `cargo test -p skiff-artifact-model websocket_ingress::tests -- --nocapture`：5/5。
- `cargo test -p skiff-compiler --test websocket_ingress -- --nocapture`：4/4。正向 fixture 在
  WebSocket handler 中真实调用 `std.time.sleep`，投影为 `maySuspend=true + NotCancellable`，
  并成功生成 deployment；原有泛型、类型身份负例继续通过。
- `cargo test -p skiff-compiler-projection package_artifact::boundary -- --nocapture`：6/6。
- `pnpm --dir router exec vitest run tests/websocket-connection-lifecycle.test.ts`：9/9。该 suite 覆盖
  挂起 receive 的恢复、同连接顺序、连接关闭时 active dispatch 仅终止一次、队列清理和计数归零。
- `cargo check --workspace`：通过。
- `git diff --check`：通过。

真实 AIHub 使用本任务 worktree 的 CLI 和 fresh store
`/tmp/p5-f251-final.9oNWjb/store` 完成发布：

- `websocket`：Available，operation identity 前缀 `ebffba63`；
- `handleAihubHttp`：Available；
- service protocol identity 前缀 `03881940`；
- deployment revision 前缀 `sha256-933c7081`；
- artifact identity 前缀 `9088c05b`；
- deployment、contract 和 package pointers 均写入成功。

AIHub 的 `managedLlm.streamChat` / `validateChat` 仍因独立的 write + same-heap 问题 Unavailable，
但它们不是 ingress operation，也未阻止本次 deployment。Agine 的下一独立 blocker 是尚未重新发布的
Agent package pointer，不属于 F263。

未 push、未操作 stable、未执行磁盘清理。
