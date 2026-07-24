# P5-F171C2：WebSocket Schema Record Lookup Result

状态：Completed

## 直接父任务

- `P5-F171C2-websocket-schema-record-lookup.md`

## 交付

- `websocket_ingress_context`使用标准库`Borrow<PackageSchemaTypeRecord>`作为最小只读lookup约束。
- helper及其递归closure校验只从map取得`&PackageSchemaTypeRecord`，因此同时支持：
  - publication的`BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>`；
  - runtime的`BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>`。
- 两条路径都不复制或转换record payload；未增加旧service-owned schema入口。
- owner、stable key、type id、requirements、closure和cycle的fail-closed校验保持不变。
- 测试分别覆盖owned publication map和shared runtime map；shared路径调用前后`Arc`
  strong count不变。

## 验证

通过：

```text
cargo test --locked -p skiff-artifact-model
# 110 passed; 0 failed

cargo check --locked -p skiff-artifact-model
cargo check --locked -p skiff-deployment
git diff --check
```
