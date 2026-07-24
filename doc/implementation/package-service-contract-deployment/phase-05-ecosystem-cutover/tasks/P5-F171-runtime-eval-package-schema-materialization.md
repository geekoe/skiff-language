# P5-F171：Runtime Eval Package Schema Materialization

状态：Ready

## 直接父任务

- `P5-F169-runtime-callback-package-schema-cutover-result.md`

## 当前断点

eval的普通边界值物化和async stream/cancel仍从已删除的
`ServiceContract.boundary_schema`读取`ContractSchemaType`，尚未消费admission固定的Package records。

## 范围

只修改`runtime/eval`中的`boundary_materialization`、`async_stream_cancel`及其直接聚焦测试，并写
result。不得修改callback、WebSocket、loader、boundary、native、host或compiler。

## 必须实现

- 普通参数、返回值、typed error、stream item和取消路径直接消费admission传入的
  `PackageSchemaTypeId -> PackageSchemaTypeRecord`只读集合。
- 物化计划继续由runtime boundary编译；eval不得访问文件系统、Package index或重新解析contract。
- 命名类型保留Package owner、stable key、type id；完整identity进入缓存键。
- 缺record、owner/key/id错配、未闭合引用在调用/stream建立前fail closed。
- 保持值所有权、detach、错误、取消、backpressure和资源释放语义。
- 删除范围内旧`ContractTypeId`、`ContractSchemaType`和`boundary_schema`使用。

## 验证

- 相关eval聚焦测试；crate恢复编译后运行`cargo test -p skiff-runtime-eval`；
- 普通、typed error、stream item、取消以及跨Package同名隔离覆盖；
- 旧符号在范围文件无命中；
- `cargo check --workspace`首错越过范围文件；
- `git diff --check`；
- 独立提交并写`P5-F171-runtime-eval-package-schema-materialization-result.md`。
