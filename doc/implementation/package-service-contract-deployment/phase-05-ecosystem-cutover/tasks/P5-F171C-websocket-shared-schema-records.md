# P5-F171C：WebSocket Shared Schema Records

状态：Ready

## 直接父任务

- `P5-F171B-runtime-boundary-shared-schema-records-result.md`

## 当前断点

`artifact-model::websocket_ingress_context`仍要求value-owned Package schema map，runtime eval只有
admission固定的`Arc<Record>` map；调用会被迫复制record。

## 范围

只修改artifact-model的WebSocket ingress schema helper及其测试，并写result。

## 必须实现

- helper直接借用shared record map，或使用不复制payload的只读lookup抽象。
- 保持Package owner/stable key/type id、closure及fail-closed语义。
- 不得新增兼容旧service-owned schema的重载。

## 验证

- artifact-model相关测试；
- `cargo check -p skiff-artifact-model`；
- `git diff --check`；
- 独立提交并写result。
