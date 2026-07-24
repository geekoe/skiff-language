# P5-F168：Compiler Lowering Package Schema Cutover Result

状态：Completed

## 直接父任务

- `P5-F168-compiler-lowering-package-schema-cutover.md`

## 交付

- executable type projection改为消费`PackageTypeRef::PackageSchema`；Package schema名义叶在
  File IR中仍只投影为opaque `unknown`，嵌套container与nullable形状保持。
- lowering接口执行fixture改为真实Package-owned输入：
  - `example.types`拥有`User`的canonical descriptor、schema record/index/type identity；
  - `example.payments`的ServiceContract只通过`PackageTypeRequirement`和
    `ContractTypeRef::PackageSchema`引用该类型；
  - direct Package alias `types.User`与Service alias `payments.User`由同一份record和精确Package
    local ABI输入依赖分析；
  - 接口实现直接把`payments.User`参数返回为`types.User`，证明两条引用解析为同一Package身份。
- File IR聚焦断言确认不包含Package id、Package schema type id、Package/Service alias限定名或边界
  schema字段；Package ABI identity没有复制到执行IR。
- lowering crate内已删除旧`PackageTypeRef::Contract`、`ContractTypeId`、
  `ContractSchemaType`与`boundary_schema`使用。

## 验证

通过：

```text
cargo test -p skiff-compiler-lowering executable_type_projection::tests
2 passed; 0 failed

cargo test -p skiff-compiler-lowering \
  source_file_lowering::interface_execution_tests::exact_interface_and_impl_contract_types_share_opaque_execution_projection \
  -- --exact
1 passed; 0 failed

rg "PackageTypeRef::Contract|ContractTypeId|ContractSchemaType|boundary_schema" compiler/lowering
no matches

git diff --check
passed
```

完整`cargo test -p skiff-compiler-lowering`仍受进入本任务前的测试环境/Package interface断面影响：
两次运行分别为`26 passed; 15 failed`和`37 passed; 4 failed`；并行测试是否先初始化全局prelude
registry造成数量波动，剩余另一类失败是父结果已记录的`pkg.Reader`不是interface。F168新增/迁移的
两个projection测试和真实Package/Service同身份接口测试均稳定通过。

`cargo check --workspace`已成功越过`skiff-compiler-lowering`，首个生产错误位于后续
`runtime/native/src/callback_adapter.rs`：仍导入已删除的`ContractSchemaType`/`ContractTypeId`并调用旧
`canonical_contract_type_id` accessor。

未修改artifact model、compiler input/source、runtime、service或package；未操作stable，未push。
