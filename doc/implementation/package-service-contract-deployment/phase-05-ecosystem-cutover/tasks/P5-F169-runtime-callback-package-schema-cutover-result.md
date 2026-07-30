# P5-F169：Runtime Callback Package Schema Cutover Result

状态：Completed

## 直接父任务

- `P5-F169-runtime-callback-package-schema-cutover.md`

## 交付

- callback adapter、native adapter descriptor与callback projection统一保存完整
  `PackageSchemaTypeRef(packageId, stableSchemaKey, PackageSchemaTypeId)`，不再保存
  service-owned type id。
- adapter只接收admission阶段已解析的
  `PackageSchemaTypeId -> PackageSchemaTypeRecord`只读集合；创建adapter时立即校验：
  - callback record存在且owner/key/id三元组精确匹配；
  - descriptor确实是callback interface；
  - operation集合与admitted record一致；
  - 参数和返回值的Package schema传递闭包完整。
- eval callback native从boundary capability request中的resolved records解析callback descriptor，
  沿Package-owned alias/representation闭包查找真实callback interface，并在注册capability前拒绝缺record、
  owner/key/id错配、非callback descriptor和SCC。
- native adapter registry的key改为`adapter identity + 完整PackageSchemaTypeRef`；相同stable key但不同
  Package的callback adapter可同时注册且不会混用。
- capability carrier使用完整`PackageSchemaTypeRef`的严格JSON身份，调用时与adapter保存的完整身份重新比较；
  参数/返回materialization继续使用adapter固定的in-memory Package records，不访问artifact store、
  Package index或provider source。
- callback handle生命周期、取消、调用次数与原错误传播路径未改变。

## 验证

通过：

```text
cargo test --locked -p skiff-runtime-native
63 passed; 0 failed

rg "ContractTypeId|ContractSchemaType" \
  runtime/native/src/callback_adapter.rs \
  runtime/eval/src/assembly_execution/callback_native.rs
0 matches

git diff --check
passed
```

聚焦测试覆盖完整Package identity正例、相同stable key跨Package registry隔离、owner/key/id逐项错配、
非callback descriptor及缺少嵌套Package record。

以下命令已执行，但被本任务范围外的后续runtime consumer断面阻断：

```text
cargo test --locked -p skiff-runtime-eval callback
cargo check --locked --workspace
```

两者均已越过`runtime/native/src/callback_adapter.rs`与
`runtime/eval/src/assembly_execution/callback_native.rs`。当前runtime首错位于
`runtime/eval/src/assembly_execution/async_stream_cancel.rs`、
`boundary_materialization.rs`和`websocket_contract_plan.rs`仍引用已删除的
`ContractSchemaType`/`ContractTypeId`/`boundary_schema`；workspace并行还暴露F168负责的
`compiler/lowering`旧`PackageTypeRef::Contract`。因此eval callback单测会随下一批普通/stream/WebSocket
consumer迁移后恢复可执行，本任务没有越界修改这些owner。

## 下游断面

后续eval consumer必须从admitted resolved schema handoff取得同一Package records集合，并传给普通、
stream与WebSocket materialization；不得重新引入ServiceContract schema accessor或filesystem resolver。
