# P5-D83：Generated Deployment 隐式 Std Binding 审计

状态：Ready（只读）

## 父节点

- 直接父节点：`P5-F144-registry-real-service-revalidation-result.md`
- 同类证据：`P5-F145-codex-relay-real-service-revalidation-result.md`
- 两者向上追溯到 C2 batch 和唯一权威设计。

## 审计目标

- 从 compiler 内建 std requirement 的产生点追到 package closure、generated deployment input、binding vector 和
  deployment validation。
- 解释为何 artifact root 已有 exact std record/pointer 仍被判 unbound。
- 区分 package dependency binding 与 service dependency binding；禁止建议 consumer 显式声明 builtin std。
- 列出 canonical owner、最小修复面、正负探针和会失效的证据。
- 返回 `READY_TO_IMPLEMENT` 或需要用户决定的精确 `DESIGN_BLOCKED`。

只读，不修改、不运行完整 gate、不操作 stable。

