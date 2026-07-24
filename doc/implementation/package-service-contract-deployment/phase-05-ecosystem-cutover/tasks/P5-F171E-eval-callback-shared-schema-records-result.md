# P5-F171E：Eval Callback Shared Schema Records Result

状态：Completed

## 直接父任务

- `P5-F171E-eval-callback-shared-schema-records.md`

## 交付

- eval `callback_native`的callback contract解析、alias递归解析和callback value materialization
  统一借用boundary公开的`ServiceSchemaRecords`。
- 目标文件不再保留value-owned Package schema map签名；descriptor lookup直接从共享
  `Arc<PackageSchemaTypeRecord>`解引用读取。
- 保持Package owner、stable key、type id、缺record、非callback descriptor和alias cycle的
  fail-closed语义。
- 直接测试fixture改用shared record map，并验证contract解析前后record `Arc`引用计数不变。

## 验证

通过：

```text
rg "BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>" \
  runtime/eval/src/assembly_execution/callback_native.rs
# no matches

git diff --check
```

尝试：

```text
cargo test --locked -p skiff-runtime-eval callback_native
```

测试二进制仍被F171/F172后续范围的旧执行代码阻断。首错位于
`assembly_execution/async_stream_cancel.rs`，其后错误位于
`boundary_materialization.rs`、`websocket_contract_plan.rs`及对应旧fixture；均仍引用已删除的
`ContractSchemaType`、`ContractTypeId`、`boundary_schema`或旧enum variants。编译输出没有
`callback_native.rs`错误，说明编译已越过本任务目标文件。
