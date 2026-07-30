# P5-D83：Generated Deployment 隐式 Std Binding 审计结果

结论：`READY_TO_IMPLEMENT`

## 父节点链

- `P5-D83-generated-deployment-implicit-std-binding-audit.md`
- 向上追溯到 F144/F145 results、C2 batch 和唯一权威设计。

## 根因与 owner

- compiler pipeline 正确生成 compiler-owned exact std `PackageRequirement`，并正确拒绝 consumer 显式声明 std。
- authoring compile 的 `available` 包含 store 中 std，但 generated deployment 只收到显式 `dependencies`。
- validator随后无法为 implementation 的 exact std requirement 找到 candidate，报 unbound。
- canonical owner 是 `compiler/driver/authoring` 的 store-backed reachable package closure assembly。
- 不能把整个 `available` 传入；unused artifact 会被 deployment projection 判为 unreachable。必须从 implementation exact
  requirements 做 BFS，校验 id/version/local ABI，并递归闭合传递 requirements。

无需改变 schema、service binding 或 consumer authoring。

