# P5-F170：Compiler Driver Package Schema Cutover Result

状态：Completed

## 直接父任务

- `P5-F170-compiler-driver-package-schema-cutover.md`

## 交付

- canonical contract dependency重建现在把`PackageCompileInput`中同一批
  `ResolvedPackageSchema`原样交给input validation；不再从ServiceContract推导或合成schema。
- `GeneratedServiceDeploymentInput`新增显式
  `PackageSchemaTypeId -> PackageSchemaTypeRecord`输入，deployment projection直接消费该closure。
- authoring路径把本轮编译产物的`resolved_package_schema_type_records`传给generated deployment，
  保持当前Package与全部精确依赖Package的canonical record。
- compiler公共入口删除`definition_contract_type_id`与`definition_contract_type_ref`旧re-export；
  没有加入兼容分支。
- driver fixture补齐Package schema index/record字段；generated deployment聚焦调用者全部显式传递
  schema record集合。
- 新增精确schema批次测试，覆盖成功基线以及缺schema、版本、build和ABI错配fail closed。

## 验证

通过：

```text
cargo check -p skiff-compiler
passed

cargo test -p skiff-compiler --lib \
  pipeline::tests::exact_package_schema_batch_rejects_missing_version_build_and_abi_mismatch \
  -- --exact
1 passed; 0 failed

rg "definition_contract_type_id|definition_contract_type_ref|boundary_schema" compiler/driver
no matches

git diff --check
passed
```

完整`cargo test -p skiff-compiler`仍被尚未迁移的compiler集成fixture阻塞：
`file_ir_execution_type_representation.rs`与`websocket_ingress.rs`仍构造旧service-owned schema；
generated deployment测试进入真实compile后还命中既有
`external named types cannot enter package schema v1`断面。driver lib运行为`10 passed; 5 failed`，
5个失败均是旧PackageArtifact JSON缺schema字段或同一external-named-type断面。

`cargo check --workspace`已成功越过compiler driver；首个生产错误位于后续`runtime/eval`的旧
`ContractSchemaType`/`ContractTypeId`与`boundary_schema`接线。

未修改input/source/lowering、deployment、artifact model或runtime；未操作stable，未push。
