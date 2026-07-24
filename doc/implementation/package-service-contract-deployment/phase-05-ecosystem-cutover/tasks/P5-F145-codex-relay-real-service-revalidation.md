# P5-F145：Codex Relay 真实 Service 重验

状态：Ready

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 写入与目标

- worktree `/Users/geek/workspace/internals-p5-f145`
- branch `codex/p5-f145-codex-relay-revalidation`
- 只写 `codex-relay/` 及 Codex Relay 专属 workflow fixture。
- 生成 Service API 必须有 17/17 intended operations Available，无意外公开 callable。
- 30 条 HTTP routes 必须与公开 handler 精确对应；覆盖缺失、重复、错误 method/path 负例。
- 保持单一 package/service authoring，禁止 contract/deployment legacy 文件或 adapter。

## 验证

- Linked worktree 使用 temporary store 和显式 `SKIFF_ROOT`；先列出 selector。
- 运行 Codex Relay canonical type-check/test/service workflow、结构搜索、格式与 `git diff --check`。
- 若需要共享 Skiff 修改，停止并返回 blocker。提交、不 push、不操作 stable。

