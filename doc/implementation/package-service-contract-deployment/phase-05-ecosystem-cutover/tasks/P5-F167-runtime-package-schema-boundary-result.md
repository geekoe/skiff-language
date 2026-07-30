# P5-F167：Runtime Package Schema Boundary Result

状态：Completed

## 直接父任务

- `P5-F167-runtime-package-schema-boundary.md`

## 交付

- `ServiceLinkableContractPlan`、callback capability request、schema closure与
  `ServiceValuePlan`统一消费`PackageSchemaTypeId -> PackageSchemaTypeRecord`。
- `ContractTypeRef::PackageSchema`在每次解析（包括命中已编译plan缓存前）严格校验Package owner、
  `stableSchemaKey`、map key与record自身的`PackageSchemaTypeId`。
- schema closure与callback interface识别沿record的canonical descriptor递归，不复制或重建
  ServiceContract schema；缺record、owner/key/id错配、未解析type parameter与SCC均fail closed。
- record、structural/discriminated union、representation、enumeration、nullable、collection/map、
  builtin、HTTP与WebSocket materialization路径保持；普通值仍拒绝alias与callback interface。
- `RuntimeTypePlan`名义、union、union branch与record kind identity使用包含
  `packageId + stableSchemaKey + PackageSchemaTypeId`的canonical Package identity，不使用display label。
- 聚焦测试已迁移到Package-owned schema fixture，并新增owner/key/id错配（含缓存命中后错配）和
  Package nominal identity保留覆盖。

## 验证

通过：

```text
cargo test -p skiff-runtime-boundary
171 passed; 0 failed

git diff --check
passed
```

`cargo check --workspace`已成功越过`skiff-runtime-boundary`，首个生产错误位于后续
`runtime/native/src/callback_adapter.rs`接线：仍导入已删除的`ContractSchemaType`/
`ContractTypeId`并调用已删除的`canonical_contract_type_id`。

未操作stable，未push，未修改loader、eval、native、host或consumer service。
