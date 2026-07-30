# P5-F189：跨 Package Schema 传递闭包结果

状态：Completed

## 根因

Package schema record 已正确保留跨 Package named child 的
`packageId + stableSchemaKey + packageSchemaTypeId`，但 canonical store 的
`resolve_package_artifact_schema` 只读取被解析 Package 自己 index 中的公开 records。
因此 `llm-providers` 的公开根可以引用 `llm-api:LlmApiFormat`，Relay 编译拿到的
`ResolvedPackageSchema` 却缺少这个可达 child，ServiceContract closure 校验按设计失败关闭。

## 实现

- artifact model 提供一个统一的 canonical descriptor 直接 child 收集器，覆盖 record、
  structural/discriminated union、representation、alias、callback interface 和容器嵌套。
- canonical store 以 Package index records 为根，按 record descriptor 中的精确
  `packageId + stableSchemaKey + packageSchemaTypeId` 递归读取跨 Package records。
- store 完成递归读取后，继续执行完整 identity、缺失 child、owner/key、无环图校验；
  PackageArtifact 自身的 record refs 仍只精确对应本 Package index，没有复制 dependency records。
- `ResolvedPackageSchema` 明确承载“本 Package 的公开 index + 从这些根可达的 record closure”：
  index 仍是唯一 public-nameability authority，foreign child 不能通过依赖 Package 的公开查找接口
  冒充本包公开类型。
- compiler fixture 从 `PublishedPackageArtifact.resolved_package_schema_type_records` 构造依赖 schema，
  不再把本包自有 records 误当成完整 closure。

## 正负探针

- 正例：`relay -> llm-providers -> llm-api` 两层跨 Package child 由 store 自动解析，resolved
  record 集合精确包含三个可达 records。
- 缺失：任一 transitive child 未写入 canonical store 时解析失败。
- 多余：显式 resolved schema 携带不可达 record 时失败。
- owner/key 错误：child record 与引用中的 Package owner 或 stable key 不一致时失败。
- Package artifact 的 index/ref 多余、缺失和错误 owner/type id 既有负例继续通过。

## 真实链证据

在 `/Users/geek/workspace/internals-p5-f188` 使用 F189 Skiff worktree 和临时 canonical store：

- `agine.ai/llm-api` 发布成功；
- `agine.ai/llm-providers` 发布成功；
- Relay 不再出现
  `package schema closure is missing agine.ai/llm-api:LlmApiFormat:...`；
- Relay 随后暴露独立的 boundary projection blocker：
  `ServiceContractDefinition operations must contain at least one operation`。

该后续 blocker 由 P5-F191 负责；F189 没有通过 consumer 手补 schema record 绕过它。

## 验证

- `cargo test -p skiff-artifact-model -p skiff-compiler-projection-input
  -p skiff-deployment --no-fail-fast`
  - `114 + 7 + 50 passed / 0 failed`
- `cargo test -p skiff-compiler-contract -p skiff-compiler --no-fail-fast`
  - F189 相关 contract、service conformance、package schema 测试通过；
  - 既有 5 个失败中 4 个由 P5-F191 的 HTTP/PackageSymbol boundary blocker 造成，另一个是已过期
    std identity 常量；没有跨 Package closure 失败。
- `cargo check --workspace`：通过。
- `git diff --check`：通过。

## 不变量

- consumer 未复制 dependency 类型或手工补 record。
- PackageArtifact 未内联 dependency records，canonical store 继续独立寻址和去重。
- 未增加 legacy artifact fallback、名字猜测或缺失 child 容错。
- 未操作 stable instance，未 push。
