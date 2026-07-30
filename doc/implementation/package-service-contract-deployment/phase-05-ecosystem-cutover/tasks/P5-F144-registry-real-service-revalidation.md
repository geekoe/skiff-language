# P5-F144：Registry 真实 Service 重验

状态：Ready

## 父节点

- `P5-H33-c2-real-service-revalidation-batch.md`

## 写入与目标

- worktree `/Users/geek/workspace/skiff-packages-p5-f144`
- branch `codex/p5-f144-registry-revalidation`
- 只写 `registry/` 及该仓库中 Registry 专属测试 fixture。
- 验证生成 Service API 的 20 个 intended operations 全部 Available；任何额外/缺失 operation fail。
- 通过真实 service/package 测试证明 immutable record、release pointer、active deployment pointer 的写入、读取、
  冲突与不存在负例；不得恢复 Registry 特权或 developer-owned contract/deployment。

## 验证

- 使用隔离 artifact/store 和显式 `SKIFF_ROOT`；先列出真实测试。
- 运行 Registry package/service 聚焦测试、authoring/receipt 检查、格式与 `git diff --check`。
- 若需要共享 Skiff 修改，停止并返回 blocker。提交、不 push、不操作 stable。

