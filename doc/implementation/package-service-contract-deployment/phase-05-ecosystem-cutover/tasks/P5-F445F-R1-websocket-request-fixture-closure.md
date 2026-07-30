# P5-F445F-R1 WebSocket request fixture closure

状态：Ready。独立、test-only baseline closure。

## 直接父节点

- `P5-F445F-scoped-execution-control-checkpoint-result.md`

父result已在任务初始HEAD独立复现唯一失败：
`websocket_connect_target_requires_real_handler_and_exact_plan` 的fixture仍构造空
`rpc_profiles`，而current artifact identity要求精确
`jsonrpc-2.0-text`。

## 实现与边界

只修改
`runtime/request/src/websocket_connect_target.rs` 的test module：

- import `GatewayWebSocketRpcProfile`；
- fixture使用
  `vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text]`；
- 保留测试其余handler/plan/negative语义。

不得修改production validator、放宽profile、增加兼容或触碰scoped-control实现。

## 验证与提交

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-r1-request-fixture/build/cargo-target \
  cargo test -p skiff-runtime-request \
  websocket_connect_target_requires_real_handler_and_exact_plan -- --exact --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-r1-request-fixture/build/cargo-target \
  cargo test -p skiff-runtime-request --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445f-r1-request-fixture/build/cargo-target \
  cargo fmt --check
git diff --check
```

worktree：

`/Users/geek/workspace/skiff-p5-f445f-r1-request-fixture`

branch：

`codex/p5-f445f-r1-request-fixture`

提交implementation，再只新增并提交：

`P5-F445F-R1-websocket-request-fixture-closure-result.md`

最终clean。不得派子Agent、merge/rebase/push、stable/live/network。
