# P5-F148：Agine 真实 Service 重验

状态：Ready

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 写入与目标

- worktree `/Users/geek/workspace/internals-p5-f148`
- branch `codex/p5-f148-agine-revalidation`
- 只写 `agine/` 及 Agine 专属 workflow fixture。
- 零 ordinary service-call operation 是合法结果，不得为 HTTP/WebSocket ingress 伪造 Service API。
- HTTP/WebSocket ingress routes/handlers 与 service-call availability receipt 分别精确验证。
- 真实 caller `aihub/managedLlm.streamChat(input)` 必须以 canonical alias/operation 和 `for` stream consumption 编译。

## 验证

- Linked worktree只运行 type-check/test/canonical workflow，使用 temporary store 与显式 `SKIFF_ROOT`；不得 build/dev/start。
- 验证零 operation receipt、HTTP/WebSocket ingress、AIHub stream dependency compile、格式与 `git diff --check`。
- 不运行最终 chat smoke；若需要共享 Skiff 修改，停止并返回 blocker。提交、不 push、不操作 stable。

