# P5-F146：Account 真实 Service 重验

状态：Ready

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 写入与目标

- worktree `/Users/geek/workspace/internals-p5-f146`
- branch `codex/p5-f146-account-revalidation`
- 只写 `skiff-platform/account/` 及 Account 专属 workflow fixture。
- 搜索并修复旧式 dotted service call（已知候选 `httpSession.read`），使用 canonical `alias/operation`。
- 21 条 routes 与公开 handler/availability receipt 精确对应；无遗留 service response policy。
- 覆盖错误调用地址、缺失/重复 route 负例。

## 验证

- Linked worktree 使用 temporary store 和显式 `SKIFF_ROOT`；先列出 selector。
- 运行 Account canonical package/service tests、route/receipt 检查、格式与 `git diff --check`。
- 若需要共享 Skiff 修改，停止并返回 blocker。提交、不 push、不操作 stable。

