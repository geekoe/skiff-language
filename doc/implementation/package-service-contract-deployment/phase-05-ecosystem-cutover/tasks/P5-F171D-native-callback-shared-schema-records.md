# P5-F171D：Native Callback Shared Schema Records

状态：Ready

## 直接父任务

- `P5-F171B-runtime-boundary-shared-schema-records-result.md`

## 当前断点

boundary已改为共享`Arc<Record>` map，但native callback adapter仍保存value-owned map，导致
runtime-native/eval无法编译。

## 范围

只修改`runtime/native/src/callback_adapter.rs`及其聚焦测试，并写result。

## 必须实现

- adapter/registry统一保存并传递shared record map，不clone record payload。
- 保持完整Package callback identity、跨Package隔离、closure校验和缓存语义。
- 不得恢复双模型或调用期转换。

## 验证

- `cargo test -p skiff-runtime-native`；
- `cargo check -p skiff-runtime-native`；
- `git diff --check`；
- 独立提交并写result。
