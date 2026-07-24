# P5-F149：Authoring Reachable Package Closure

状态：Ready

## 父节点

- `P5-D83-generated-deployment-implicit-std-binding-audit-result.md`

## 写入范围与 owner

- `compiler/driver/authoring.rs`、其同 owner helper 与 tests。
- 不修改 pipeline implicit std 规则、deployment validator/schema、service binding 或 consumer。

## 完成标准

1. 从 compiled implementation exact `PackageRequirement` 出发，用本轮已加载/store-resolved candidates做 reachable BFS。
2. 每条边校验 package id、version、expected local ABI；传递 requirements 同样闭合。
3. generated deployment 只收到 reachable closure，未使用 artifact/std 不进入。
4. service 无显式 std dependency但源码/API使用 std时成功生成 `(callerBuild,"std")` exact binding。
5. 缺 pointer/record、ABI mismatch、缺传递 provider fail closed；显式 std仍被拒绝。

## 验证

- 先列出 authoring/generated deployment selector；运行新增 handoff、implicit std 与 generated deployment 聚焦测试。
- 目标格式、`git diff --check`；不运行完整 gate。
- 若需 schema/公共语义改变则停止。

## Worktree

- `/Users/geek/workspace/skiff-p5-f149`
- branch `codex/p5-f149-authoring-package-closure`
- 新的一次性会话；提交、不 push、不操作 stable。

