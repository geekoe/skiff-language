# P5-F171D：Native Callback Shared Schema Records Result

状态：Completed

## 直接父任务

- `P5-F171B-runtime-boundary-shared-schema-records-result.md`

## 交付

- `InProcessCallbackAdapter`及其local/native构造入口统一使用boundary公开的
  `ServiceSchemaRecords`，即`BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>`。
- adapter保存schema时只克隆map key和record `Arc`，不克隆record或descriptor payload；
  后续callback参数、返回值校验直接借用同一组shared records。
- 保留完整Package owner/stable key/type id校验、callback operation一致性、closure校验、
  adapter registry跨Package隔离及原有缓存语义。
- 测试夹具统一使用shared records；新增回归测试以`Arc::ptr_eq`证明adapter保留admitted
  record的同一对象，并用引用计数证明仅增加一个shared owner。

## 验证

通过：

```text
cargo test -p skiff-runtime-native
# 64 passed; 0 failed

cargo check -p skiff-runtime-native
git diff --check
```

## 下游断面

eval callback materialization可直接把boundary request携带的`ServiceSchemaRecords`借给native
adapter；不得恢复value-owned `PackageSchemaTypeRecord` map或调用期payload转换。
