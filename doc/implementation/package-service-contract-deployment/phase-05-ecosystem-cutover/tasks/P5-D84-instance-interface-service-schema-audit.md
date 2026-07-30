# P5-D84：Instance Interface Service Schema 审计

状态：Ready（只读）

## 父节点

- 直接父节点：`P5-F145-codex-relay-real-service-revalidation-result.md`
- 该 result 向上追溯到 C2 batch 和唯一权威设计。

## 审计目标

- 从 Codex Relay `api.yml`/source public surface 追到 PackageArtifact boundary projection、Service API visibility 和
  contract public type/callback schema。
- 区分 interface declaration、instance method、callback interface、internal helper 和真正 public executable callable。
- 解释 `CodexRelayProxyClient` 为何把 `std.http.HttpRequest` 纳入 service schema materialization。
- 判断缺口属于 consumer authoring 还是共享 compiler owner；列出最小修复、正负探针及 17/17/30 后续遮挡。
- 返回 `READY_TO_IMPLEMENT` 或需要用户决定的精确 `DESIGN_BLOCKED`。

只读，不修改、不运行完整 gate、不操作 stable。

