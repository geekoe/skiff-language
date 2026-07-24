# P5-F171C：WebSocket Shared Schema Records Result

状态：Completed

## 直接父任务

- `P5-F171C-websocket-shared-schema-records.md`

## 交付

- `artifact-model::websocket_ingress_context`及其内部递归校验链路改为直接借用
  `BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>`。
- Package schema record解析只解引用共享`Arc`，不复制record payload，也不新增旧service-owned
  schema兼容入口。
- 保持ServiceContract requirements、Package owner、stable key、type id、传递闭包、循环检测和
  fail-closed语义。
- 测试fixture使用shared records，并验证helper调用前后record `Arc`引用计数不变。

## 验证

通过：

```text
cargo test --locked -p skiff-artifact-model
# 109 passed; 0 failed

cargo check --locked -p skiff-artifact-model
git diff --check
```

## 下游断面

runtime eval的WebSocket contract plan应把admission固定的shared record map直接传给
`websocket_ingress_context`，不得生成value-owned副本。
