# P5-F166：Runtime Package Schema Hydration Result

状态：Completed

## 直接父任务

- `P5-F166-runtime-package-schema-hydration.md`

## 交付

- `RuntimeAssemblyContentResolver`新增精确`PackageSchemaTypeRecordRef`解析入口；
  filesystem resolver直接复用canonical artifact store的content-addressed record读取。
- runtime loader在assembly可见前一次性加载每个ServiceContract声明的全部
  `PackageTypeRequirement`，并形成不可变`ResolvedServiceSchema`：
  - 先验证ServiceContract exact identity；
  - 验证record path返回的owner/type id、descriptor重算hash与stable key；
  - 验证required集合与resolved records集合完全相等；
  - 从operation参数、返回、错误、stream和callback roots重新计算完整传递闭包并要求精确相等；
  - 拒绝缺child、未require/额外record及v1 recursive SCC。
- `ServiceContractStore`按contract保留独立validated closure，同时按
  `PackageSchemaTypeId`共享不可变record payload；eval和后续boundary无需持有filesystem resolver。
- public-only仍由Package schema生成/发布入口保证；runtime不读取PackageSchemaIndex，也不按active
  PackageArtifact、version或provider source猜类型。
- 所有现有runtime assembly test resolver均显式实现新增解析接口；没有空schema fallback。

## 验证

通过：

```text
cargo check --locked -p skiff-runtime-loader

cargo test --locked -p skiff-runtime-loader
12 passed; 0 failed

git diff --check
passed
```

聚焦测试覆盖真实filesystem record、加载后删除record仍保持in-memory closure、新admission fail closed、
共享record `Arc`去重、缺失、额外、未require、错hash/owner/key、closure缺child及recursive SCC。

扩展检查：

```text
cargo check --locked -p skiff-runtime-linker -p skiff-runtime-host \
  -p skiff-runtime-package-test
```

该命令被下一断面`runtime/boundary`仍引用已删除的`ContractSchemaType`、`ContractTypeId`、
`ContractTypeRef::Contract/PackagePublic`阻断；loader及其新增resolver接口已先完成编译。

## 下游断面

下一任务应让runtime boundary/value-plan直接读取admitted `ResolvedServiceSchema`中的Package-owned
records，随后再迁移eval、callback及Host execution fixtures；不得恢复contract-owned schema map或让
execution路径重新读取artifact store。
